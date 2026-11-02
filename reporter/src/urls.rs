use std::str::FromStr;

use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};

use crate::Error;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LoginUrl(String);

impl AsRef<str> for LoginUrl {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for LoginUrl {
    type Err = Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        // Check the wire representation before URL parsing can normalize it.
        // Preserve valid query strings exactly; never truncate a login URL.
        if raw.len() > 512
            || !raw.starts_with("https://login.tailscale.com/a/")
            || !raw.is_ascii()
            || raw
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            || raw.contains(['\\', '"', '<', '>'])
        {
            return Err(Error::InvalidLoginUrl);
        }
        let url = Url::parse(raw).map_err(|_| Error::InvalidLoginUrl)?;
        if url.scheme() != "https"
            || url.host_str() != Some("login.tailscale.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
            || !url.path().starts_with("/a/")
            || url.path().len() <= 3
        {
            return Err(Error::InvalidLoginUrl);
        }
        Ok(Self(raw.to_owned()))
    }
}

pub(crate) fn optional_login_url<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<LoginUrl>, D::Error> {
    let raw = Option::<String>::deserialize(deserializer)?;
    // Invalid links must not discard an otherwise useful NeedsLogin status.
    Ok(raw.and_then(|url| url.parse().ok()))
}

pub struct ManagerEndpoint(Url);

impl ManagerEndpoint {
    pub(crate) fn as_url(&self) -> &Url {
        &self.0
    }
}

impl FromStr for ManagerEndpoint {
    type Err = Error;

    fn from_str(base: &str) -> Result<Self, Self::Err> {
        let base = Url::parse(base).map_err(|_| Error::InvalidManagerUrl)?;
        if base.scheme() != "http"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(Error::InvalidManagerOrigin);
        }
        let mut endpoint = base
            .join("data-send")
            .map_err(|_| Error::InvalidManagerOrigin)?;
        endpoint
            .query_pairs_mut()
            .append_pair("key", "tailscale.status");
        Ok(Self(endpoint))
    }
}
