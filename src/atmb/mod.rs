use crate::atmb::model::Mailbox;
use crate::atmb::page::{CountryPage, LocationDetailPage, StatePage};
use crate::utils::retry_wrapper;
use color_eyre::eyre::{bail, eyre};
use futures::StreamExt;
use log::info;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION,
    USER_AGENT,
};
use reqwest::Client;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Duration};

pub mod model;
pub mod page;

const BASE_URL: &str = "https://www.anytimemailbox.com";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

const US_HOME_PAGE_URL: &str = "/l/usa";
const CACHE_DIR: &str = "cache";
const CACHE_HTML_DIR: &str = "cache/html";
const CACHE_BASE_MAILBOXES: &str = "cache/mailboxes_base.json";
const CACHE_DETAIL_FILE: &str = "cache/mailboxes_detail.json";

/// HTTP client for obtaining information from ATMB
struct ATMBClient {
    client: Client,
}

impl ATMBClient {
    fn new() -> color_eyre::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .gzip(true)
                .brotli(true)
                .default_headers(Self::default_headers())
                .build()?,
        })
    }

    fn default_headers() -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(USER_AGENT, HeaderValue::from_static(UA));
        map.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"));
        map.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        // force identity to avoid receiving compressed body when decoding support is limited
        map.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        map.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
        map.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        map
    }

    /// get the content of a page
    ///
    /// * `url_path` - the path of the page, can be either a full URL or a relative path
    async fn fetch_page(&self, url_path: &str) -> color_eyre::Result<String> {
        let url = if url_path.starts_with("http") {
            url_path
        } else {
            &format!("{}{}", BASE_URL, url_path)
        };
        let body = retry_wrapper(3, || async {
            let resp = self.client.get(url).send().await?;
            let status = resp.status();
            let text = resp.text().await?;
            if !status.is_success() {
                let snippet: String = text.chars().take(300).collect();
                return Err(eyre!("http status: {}, body: {}", status, snippet));
            }
            Ok(text)
        })
        .await?;

        // small delay to reduce chance of being rate-limited
        sleep(Duration::from_millis(150)).await;
        Ok(body)
    }
}

fn ensure_dir(path: &str) -> color_eyre::Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

fn cache_key(url: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn read_cached_html(url: &str) -> Option<String> {
    let path = Path::new(CACHE_HTML_DIR).join(format!("{}.html", cache_key(url)));
    fs::read_to_string(path).ok()
}

fn write_cached_html(url: &str, body: &str) -> color_eyre::Result<()> {
    ensure_dir(CACHE_HTML_DIR)?;
    let path = Path::new(CACHE_HTML_DIR).join(format!("{}.html", cache_key(url)));
    fs::write(path, body)?;
    Ok(())
}

fn load_base_mailboxes() -> Option<Vec<Mailbox>> {
    fs::read_to_string(CACHE_BASE_MAILBOXES)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save_base_mailboxes(mailboxes: &[Mailbox]) -> color_eyre::Result<()> {
    ensure_dir(CACHE_DIR)?;
    let json = serde_json::to_string(mailboxes)?;
    fs::write(CACHE_BASE_MAILBOXES, json)?;
    Ok(())
}

fn load_detail_cache() -> HashMap<String, String> {
    fs::read_to_string(CACHE_DETAIL_FILE)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_detail_cache(map: &HashMap<String, String>) -> color_eyre::Result<()> {
    ensure_dir(CACHE_DIR)?;
    fs::write(CACHE_DETAIL_FILE, serde_json::to_string(map)?)?;
    Ok(())
}

pub struct ATMBCrawl {
    client: ATMBClient,
}

#[derive(Debug, Clone)]
pub struct CrawlWarning {
    pub name: String,
    pub link: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct CrawlResult {
    pub mailboxes: Vec<Mailbox>,
    pub warnings: Vec<CrawlWarning>,
}

impl ATMBCrawl {
    pub fn new() -> color_eyre::Result<Self> {
        Ok(Self {
            client: ATMBClient::new()?,
        })
    }

    pub async fn fetch(&self) -> color_eyre::Result<CrawlResult> {
        if let Some(mailboxes) = load_base_mailboxes() {
            info!("Loaded mailboxes from cache");
            return self.update_street2_for_mailbox(mailboxes).await;
        }
        // we're only interested in US, so hardcode here.
        let country_html = self.fetch_page_cached(US_HOME_PAGE_URL).await?;
        let country_page = CountryPage::parse_html(&country_html).map_err(|e| {
            eyre!(
                "failed to parse country page: {:?}, body_prefix={}",
                e,
                &country_html.chars().take(200).collect::<String>()
            )
        })?;

        let state_pages = self.fetch_state_pages(&country_page).await?;
        let total_num = state_pages.iter().map(|sp| sp.len()).sum::<usize>();

        let mailboxes = state_pages
            .into_iter()
            .filter_map(|sp| match sp.to_mailboxes() {
                Ok(mailboxes) => Some(mailboxes),
                Err(e) => {
                    log::error!("cannot convert state page to mailboxes: {:?}", e);
                    None
                }
            })
            .flatten()
            .collect::<Vec<_>>();

        if mailboxes.len() != total_num {
            bail!("Some mailboxes cannot be fetched");
        }

        let _ = save_base_mailboxes(&mailboxes);
        // visit every mailbox detail page to get the address line 2
        self.update_street2_for_mailbox(mailboxes)
            .await
            .map_err(|e| eyre!("Some mailbox's detail cannot be fetched: {:?}", e))
    }

    async fn fetch_page_cached(&self, url: &str) -> color_eyre::Result<String> {
        if let Some(body) = read_cached_html(url) {
            info!("Cache hit for {}", url);
            return Ok(body);
        }
        let body = self.client.fetch_page(url).await?;
        let _ = write_cached_html(url, &body);
        Ok(body)
    }

    async fn update_street2_for_mailbox(
        &self,
        mailboxes: Vec<Mailbox>,
    ) -> color_eyre::Result<CrawlResult> {
        let total_mailboxes = mailboxes.len();
        let mut warnings: Vec<CrawlWarning> = Vec::new();
        let detail_cache = Arc::new(AsyncMutex::new(load_detail_cache()));

        let mailboxes = futures::stream::iter(mailboxes)
            .enumerate()
            .map(|(idx, mut mailbox)| {
                let link = mailbox.link.clone();
                let cache = detail_cache.clone();
                async move {
                    if let Some(street) = {
                        let guard = cache.lock().await;
                        guard.get(&link).cloned()
                    } {
                        mailbox.address.line1 = street;
                        return Ok(mailbox);
                    }
                    let fut = || async {
                        info!(
                            "[{}/{}] fetching the detail page of [{}]...",
                            idx + 1,
                            total_mailboxes,
                            mailbox.name
                        );
                        self.fetch_location_detail_page(&mailbox.link)
                            .await
                            .map(|detail_page| {
                                mailbox.address.line1 = detail_page.street();
                                mailbox
                            })
                    };
                    let res = fut().await.map_err(|err| {
                        let err = eyre!("cannot fetch detail page for: [{}]: {:?}", link, err);
                        log::error!("{:?}", err);
                        err
                    })?;

                    if let Ok(mut guard) = cache.try_lock() {
                        guard.insert(link.clone(), res.address.line1.clone());
                    } else {
                        let mut guard = cache.lock().await;
                        guard.insert(link.clone(), res.address.line1.clone());
                    }
                    Ok(res)
                }
            })
            .buffer_unordered(3)
            .collect::<Vec<_>>()
            .await;

        let mut suc_list = Vec::new();
        for result in mailboxes {
            match result {
                Ok(mailbox) => suc_list.push(mailbox),
                Err(err) => {
                    let link = extract_link_from_error(&err);
                    warnings.push(CrawlWarning {
                        name: link.clone().unwrap_or_else(|| "unknown".to_string()),
                        link: link.unwrap_or_else(|| "unknown".to_string()),
                        reason: format!("{:?}", err),
                    });
                }
            }
        }

        let cache_guard = detail_cache.lock().await;
        let _ = save_detail_cache(&cache_guard);

        Ok(CrawlResult {
            mailboxes: suc_list,
            warnings,
        })
    }

    async fn fetch_state_pages(
        &self,
        country_page: &CountryPage,
    ) -> color_eyre::Result<Vec<StatePage>> {
        let total_states = country_page.states.len();
        let state_pages: Vec<color_eyre::Result<StatePage>> =
            futures::stream::iter(&country_page.states)
                .enumerate()
                .map(|(idx, state_html_info)| {
                    info!(
                        "[{}/{total_states}] fetching [{}] state page...",
                        idx + 1,
                        state_html_info.name()
                    );
                    async move {
                        let state_html = self.fetch_page_cached(state_html_info.url()).await?;
                        Ok(StatePage::parse_html(&state_html)?)
                    }
                })
                // limit concurrent requests to 5
                .buffer_unordered(5)
                .collect()
                .await;

        if state_pages
            .iter()
            .filter_map(|state_page| match state_page {
                Err(e) => {
                    log::error!("cannot fetch state: {:?}", e);
                    Some(())
                }
                _ => None,
            })
            .count()
            != 0
        {
            bail!("Some states cannot be fetched");
        }
        Ok(state_pages
            .into_iter()
            .map(|state_page| state_page.unwrap())
            .collect())
    }

    async fn fetch_location_detail_page(
        &self,
        mailbox_link: &str,
    ) -> color_eyre::Result<LocationDetailPage> {
        let html = self.client.fetch_page(mailbox_link).await?;
        Ok(LocationDetailPage::parse_html(&html)?)
    }
}

fn extract_link_from_error(err: &color_eyre::eyre::Error) -> Option<String> {
    let msg = format!("{:?}", err);
    msg.split('[')
        .nth(2)
        .and_then(|s| s.split(']').next())
        .map(|s| s.to_string())
}
