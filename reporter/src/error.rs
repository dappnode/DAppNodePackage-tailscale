use std::fmt;

#[derive(Debug)]
pub enum Error {
    InvalidLoginUrl,
    InvalidManagerUrl,
    InvalidManagerOrigin,
    InvalidStatus,
    LocalApiUnavailable,
    LocalApiResponse(reqwest::StatusCode),
    LocalApiRead,
    LocalApiTooLarge,
    PayloadSerialization,
    PayloadTooLarge,
    ManagerUnavailable,
    ManagerResponse(reqwest::StatusCode),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLoginUrl => f.write_str("invalid Tailscale login URL"),
            Self::InvalidManagerUrl => f.write_str("invalid DAPPMANAGER_URL"),
            Self::InvalidManagerOrigin => {
                f.write_str("DAPPMANAGER_URL must be an HTTP origin without credentials")
            }
            Self::InvalidStatus => f.write_str("invalid LocalAPI status"),
            Self::LocalApiUnavailable => f.write_str("LocalAPI unavailable"),
            Self::LocalApiResponse(status) => write!(f, "LocalAPI returned HTTP {status}"),
            Self::LocalApiRead => f.write_str("cannot read LocalAPI"),
            Self::LocalApiTooLarge => f.write_str("LocalAPI response too large"),
            Self::PayloadSerialization => f.write_str("cannot serialize status"),
            Self::PayloadTooLarge => f.write_str("status payload exceeds data-send limit"),
            Self::ManagerUnavailable => f.write_str("Dappmanager unavailable"),
            Self::ManagerResponse(status) => write!(f, "Dappmanager returned HTTP {status}"),
        }
    }
}

impl std::error::Error for Error {}
