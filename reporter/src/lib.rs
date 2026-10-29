mod error;
mod urls;

use std::{path::Path, time::Duration};

use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

pub use error::Error;
use urls::optional_login_url;
pub use urls::{LoginUrl, ManagerEndpoint};

pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_SOCKET: &str = "/run/tailscale/tailscaled.sock";
pub const DEFAULT_MANAGER: &str = "http://dappmanager.dappnode";
const STATUS_URL: &str = "http://local-tailscaled.sock/localapi/v0/status?peers=false";
const MAX_STATUS_BYTES: usize = 64 * 1024;
const MAX_PAYLOAD_BYTES: usize = 700;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum State {
    NoState,
    NeedsLogin,
    NeedsMachineAuth,
    Stopped,
    Starting,
    Running,
    #[serde(other)]
    Unavailable,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    version: u8,
    state: State,
    online: bool,
    auth_url: Option<LoginUrl>,
}

#[derive(Deserialize)]
struct LocalStatus {
    #[serde(rename = "BackendState")]
    state: State,
    #[serde(rename = "AuthURL", default, deserialize_with = "optional_login_url")]
    auth_url: Option<LoginUrl>,
    #[serde(rename = "Self")]
    self_status: Option<SelfStatus>,
}

#[derive(Deserialize)]
struct SelfStatus {
    #[serde(rename = "Online", default)]
    online: bool,
}

impl Snapshot {
    pub fn unavailable() -> Self {
        Self {
            version: 1,
            state: State::Unavailable,
            online: false,
            auth_url: None,
        }
    }

    fn from_json(body: &[u8]) -> Result<Self, Error> {
        let status: LocalStatus = serde_json::from_slice(body).map_err(|_| Error::InvalidStatus)?;
        let state = status.state;
        let auth_url = status.auth_url.filter(|_| state == State::NeedsLogin);
        Ok(Self {
            version: 1,
            state,
            online: state == State::Running && status.self_status.is_some_and(|s| s.online),
            auth_url,
        })
    }

    fn payload(&self, updated_at: u64) -> Result<Vec<u8>, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            #[serde(flatten)]
            snapshot: &'a Snapshot,
            updated_at: u64,
        }
        let body = serde_json::to_vec(&Payload {
            snapshot: self,
            updated_at,
        })
        .map_err(|_| Error::PayloadSerialization)?;
        if body.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::PayloadTooLarge);
        }
        Ok(body)
    }
}

fn http_client() -> reqwest::ClientBuilder {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .retry(reqwest::retry::never())
}

pub struct Reporter {
    local: Client,
    manager: Client,
    endpoint: ManagerEndpoint,
    delivered: Option<(Snapshot, Instant)>,
    delivery_failed: bool,
    status_failed: bool,
}

impl Reporter {
    pub fn new(socket: &Path, endpoint: ManagerEndpoint) -> Result<Self, reqwest::Error> {
        Ok(Self {
            local: http_client().unix_socket(socket).build()?,
            manager: http_client().build()?,
            endpoint,
            delivered: None,
            delivery_failed: false,
            status_failed: false,
        })
    }

    async fn read_status(&self) -> Result<Snapshot, Error> {
        let mut response = self
            .local
            .get(STATUS_URL)
            .send()
            .await
            .map_err(|_| Error::LocalApiUnavailable)?;
        if !response.status().is_success() {
            return Err(Error::LocalApiResponse(response.status()));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::LocalApiRead)? {
            if body.len() + chunk.len() > MAX_STATUS_BYTES {
                return Err(Error::LocalApiTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Snapshot::from_json(&body)
    }

    pub async fn publish(&mut self, snapshot: Snapshot, updated_at: u64) -> Result<(), Error> {
        let body = snapshot.payload(updated_at)?;
        let response = self
            .manager
            .post(self.endpoint.as_url().clone())
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(body)
            .send()
            .await
            .map_err(|_| Error::ManagerUnavailable)?;
        // Do not follow redirects, print response bodies, or log private URLs.
        if !response.status().is_success() {
            return Err(Error::ManagerResponse(response.status()));
        }
        self.delivered = Some((snapshot, Instant::now()));
        Ok(())
    }

    pub async fn tick(&mut self, updated_at: u64) {
        let snapshot = match self.read_status().await {
            Ok(snapshot) => {
                if self.status_failed {
                    eprintln!("Tailscale status is available again.");
                }
                self.status_failed = false;
                snapshot
            }
            Err(_) => {
                if !self.status_failed {
                    eprintln!("Tailscale status unavailable; clearing reported login link.");
                }
                self.status_failed = true;
                Snapshot::unavailable()
            }
        };
        let should_send = self.delivered.as_ref().is_none_or(|(previous, sent_at)| {
            previous != &snapshot || sent_at.elapsed() >= HEARTBEAT_INTERVAL
        });
        if should_send {
            match self.publish(snapshot, updated_at).await {
                Ok(()) => {
                    if self.delivery_failed {
                        eprintln!("Tailscale status reporting to Dappmanager resumed.");
                    }
                    self.delivery_failed = false;
                }
                Err(_) => {
                    if !self.delivery_failed {
                        eprintln!("Cannot report Tailscale status to Dappmanager; retrying.");
                    }
                    self.delivery_failed = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
