use std::fs::File;
use std::io::{BufWriter, Write};

use csv::ReaderBuilder;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;

use atmb_us_physical::atmb::page::LocationDetailPage;
use atmb_us_physical::utils::retry_wrapper;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

#[derive(Debug, Deserialize)]
struct CsvRecord {
    name: String,
    street: String,
    city: String,
    state: String,
    zip: String,
    price: String,
    link: String,
    #[serde(rename = "rdi")]
    rdi: String,
    #[serde(rename = "CMRA")]
    cmra: String,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let input_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "result/mailboxes.csv".to_string());
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_path(&input_path)?;

    let log_path = "result/verify.log";
    if let Some(parent) = std::path::Path::new(log_path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut log = BufWriter::new(File::create(log_path)?);

    let client = reqwest::Client::builder()
        .default_headers(default_headers())
        .build()?;

    for rec in reader.deserialize::<CsvRecord>() {
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                writeln!(log, "CSV parse error: {:?}", e)?;
                continue;
            }
        };

        let page_html = match fetch_page(&client, &rec.link).await {
            Ok(html) => html,
            Err(e) => {
                writeln!(log, "[FAIL] {} - fetch error: {:?}", rec.link, e)?;
                continue;
            }
        };

        let parsed: String = match LocationDetailPage::parse_html(&page_html) {
            Ok(p) => p.street(),
            Err(e) => {
                writeln!(log, "[FAIL] {} - parse error: {:?}", rec.link, e)?;
                continue;
            }
        };

        if normalize(&parsed) == normalize(&rec.street) {
            writeln!(log, "[OK] {} matches ({})", rec.link, parsed)?;
        } else {
            writeln!(
                log,
                "[MISMATCH] {} - csv: [{}] parsed: [{}]",
                rec.link, rec.street, parsed
            )?;
        }
    }

    log.flush()?;
    Ok(())
}

fn default_headers() -> HeaderMap {
    let mut map = HeaderMap::new();
    map.insert(USER_AGENT, HeaderValue::from_static(UA));
    map
}

async fn fetch_page(client: &reqwest::Client, url: &str) -> color_eyre::Result<String> {
    retry_wrapper(3, || async {
        Ok(client.get(url).send().await?.text().await?)
    })
    .await
}

fn normalize(s: &str) -> String {
    s.trim().replace("  ", " ").to_lowercase()
}
