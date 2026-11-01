//! Saved Redis settings contain no credentials.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub host: String,
    pub port: u16,
    pub database: u32,
    pub username: String,
    pub tls: bool,
    pub root_certificate: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 6379,
            database: 0,
            username: String::new(),
            tls: true,
            root_certificate: None,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Enter a Redis host.".into());
        }
        if self.port == 0 {
            return Err("Redis port must be between 1 and 65535.".into());
        }
        for (name, timeout) in [
            ("Connect", self.connect_timeout),
            ("Read", self.read_timeout),
        ] {
            if timeout.is_zero() || timeout > Duration::from_secs(300) {
                return Err(format!(
                    "{name} timeout must be greater than zero and at most 300 seconds."
                ));
            }
        }
        Ok(())
    }
}
