use crate::atmb::model::Address;
use crate::utils::retry_wrapper;
use color_eyre::eyre::{bail, eyre};
use dotenvy::dotenv;
use serde::{Deserialize, Serialize};
use smarty_rust_sdk::sdk::authentication::SecretKeyCredential;
use smarty_rust_sdk::sdk::batch::Batch;
use smarty_rust_sdk::sdk::error::SmartyError;
use smarty_rust_sdk::sdk::options::{Options, OptionsBuilder};
use smarty_rust_sdk::us_street_api::client::USStreetAddressClient;
use smarty_rust_sdk::us_street_api::lookup::{Lookup, MatchStrategy};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

/// A free trial account is limited to 1000 lookups per month.
/// So we use multiple accounts to avoid the limitation.
///
/// As there are ~1700 atmb location currently, we need at least 2 accounts.
pub struct SmartyClientProxy {
    clients: Vec<SmartyClient>,
    state: Mutex<Vec<ClientState>>,
    cursor: AtomicUsize,
}

impl SmartyClientProxy {
    pub fn new() -> color_eyre::Result<Self> {
        let credentials = Self::credentials();
        let clients = credentials
            .into_iter()
            .map(|(id, secret)| SmartyClient::new(id, secret))
            .collect::<Result<Vec<_>, _>>()?;
        let state = clients.iter().map(|_| ClientState::default()).collect();
        log::info!("Loaded [{}] Smarty credential(s)", clients.len());
        Ok(Self {
            clients,
            state: Mutex::new(state),
            cursor: AtomicUsize::new(0),
        })
    }

    pub async fn inquire_address(&self, address: Address) -> color_eyre::Result<AdditionalInfo> {
        let total_clients = self.clients.len();
        let mut last_err = None;

        for _ in 0..total_clients {
            let idx = match self.next_client_idx() {
                Some(i) => i,
                None => break,
            };

            match self.clients[idx].inquire_address(address.clone()).await {
                Ok(info) => {
                    self.update_state(idx, true);
                    return Ok(info);
                }
                Err(e) => {
                    // still advance lookup count so we don't hammer one account forever
                    self.update_state(idx, false);
                    log::warn!("Smarty client [{}] failed: {:?}", idx, e);
                    last_err = Some(e);
                    continue;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| eyre!("all Smarty clients failed or are unavailable")))
    }

    fn next_client_idx(&self) -> Option<usize> {
        let total = self.clients.len();
        for _ in 0..total {
            let idx = self.cursor.fetch_add(1, Ordering::AcqRel) % total;
            let state = self.state.lock().unwrap();
            let available = state.get(idx).map_or(false, ClientState::is_available);
            drop(state);
            if available {
                return Some(idx);
            }
        }
        None
    }

    fn update_state(&self, idx: usize, success: bool) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(s) = state.get_mut(idx) {
                if success {
                    s.lookups += 1;
                    s.consecutive_failures = 0;
                } else {
                    s.consecutive_failures += 1;
                }
            }
        }
    }

    /// load authentication credentials from environment variables
    ///
    /// CREDENTIALS=`ID1`=`SECRET1`[,`ID2`=`SECRET2`]*
    fn credentials() -> Vec<(String, String)> {
        let _ = dotenv();
        std::env::var("CREDENTIALS")
            .map(|credentials| {
                credentials
                    .split(',')
                    .map(|pair| {
                        let mut iter = pair.split('=');
                        (
                            iter.next().unwrap().to_string(),
                            iter.next().unwrap().to_string(),
                        )
                    })
                    .collect()
            })
            .expect("`CREDENTIALS` environment variable must be set")
    }
}

#[derive(Default)]
struct ClientState {
    lookups: u32,
    consecutive_failures: u32,
}

impl ClientState {
    fn is_available(&self) -> bool {
        const MAX_LOOKUPS: u32 = 1000;
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;
        self.lookups < MAX_LOOKUPS && self.consecutive_failures < MAX_CONSECUTIVE_FAILURES
    }
}

struct SmartyClient {
    client: USStreetAddressClient,
}

impl SmartyClient {
    fn new(auth_id: impl Into<String>, auth_token: impl Into<String>) -> color_eyre::Result<Self> {
        Ok(Self {
            client: USStreetAddressClient::new(Self::options(auth_id, auth_token))?,
        })
    }

    async fn inquire_address(&self, address: Address) -> color_eyre::Result<AdditionalInfo> {
        retry_wrapper(3, || async { self._inquire_address(address.clone()).await }).await
    }

    async fn _inquire_address(&self, address: Address) -> color_eyre::Result<AdditionalInfo> {
        let mut batch = Batch::default();
        batch.push(Lookup::from(address))?;
        self.client
            .send(&mut batch)
            .await
            .map_err(|e| map_smarty_err(e))?;
        let resp = batch
            .records()
            .into_iter()
            .next()
            .ok_or_else(|| eyre!("no response from Smarty"))?;
        resp.clone().try_into()
    }

    fn authentication(
        auth_id: impl Into<String>,
        auth_token: impl Into<String>,
    ) -> Box<SecretKeyCredential> {
        SecretKeyCredential::new(auth_id.into(), auth_token.into())
    }

    fn options(auth_id: impl Into<String>, auth_token: impl Into<String>) -> Options {
        OptionsBuilder::new(Some(Self::authentication(auth_id, auth_token)))
            .with_license("us-core-cloud")
            .with_retries(3)
            .build()
    }
}

impl From<Address> for Lookup {
    fn from(address: Address) -> Self {
        Self {
            zipcode: address.full_zip(),
            street: address.line1,
            city: address.city,
            state: address.state,
            match_strategy: MatchStrategy::Enhanced,
            ..Default::default()
        }
    }
}

fn map_smarty_err(err: SmartyError) -> color_eyre::eyre::Error {
    match err {
        SmartyError::HttpError { code, detail } => eyre!("smarty http error: {code} - {detail}"),
        SmartyError::RequestProcess(e) => eyre!("smarty request error: {}", e),
        SmartyError::Middleware(e) => eyre!("smarty middleware error: {}", e),
        other => eyre!("smarty error: {:?}", other),
    }
}

#[derive(Debug)]
pub struct AdditionalInfo {
    pub cmra: YesOrNo,
    pub rdi: Rdi,
}

#[derive(Debug, PartialEq, Eq, Serialize, Ord, PartialOrd)]
#[serde(rename_all = "PascalCase")]
#[repr(u8)]
pub enum Rdi {
    Residential,
    Commercial,
    Unknown,
}

impl TryFrom<String> for Rdi {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "residential" => Ok(Rdi::Residential),
            "commercial" => Ok(Rdi::Commercial),
            "" => Ok(Rdi::Unknown),
            _ => Err(value),
        }
    }
}

impl AdditionalInfo {
    pub fn is_cmra(&self) -> bool {
        self.cmra == YesOrNo::Y
    }

    pub fn is_residential(&self) -> bool {
        self.rdi == Rdi::Residential
    }
}

impl TryFrom<Lookup> for AdditionalInfo {
    type Error = color_eyre::eyre::Error;

    fn try_from(lookup: Lookup) -> Result<Self, Self::Error> {
        if lookup.results.is_empty() {
            bail!("no results found: {:?}", lookup);
        }
        let candidate = lookup.results.into_iter().next().unwrap();

        Ok(Self {
            cmra: YesOrNo::try_from(candidate.analysis.dpv_cmra)
                .map_err(|e| eyre!("failed to parse CMRA: {}", e))?,
            rdi: Rdi::try_from(candidate.metadata.rdi)
                .map_err(|e| eyre!("failed to parse RDI: {}", e))?,
        })
    }
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum YesOrNo {
    N,
    Y,
}

impl TryFrom<String> for YesOrNo {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "y" => Ok(YesOrNo::Y),
            "n" => Ok(YesOrNo::N),
            _ => Err(value),
        }
    }
}
