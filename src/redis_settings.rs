//! Saved Redis settings contain no credentials.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SentinelSeed {
    pub host: String,
    pub port: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Sentinel {
    pub service: String,
    pub seeds: Vec<SentinelSeed>,
    pub username: String,
    pub tls: bool,
    pub root_certificate: Option<PathBuf>,
    pub client_certificate: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub remember_password: bool,
}
impl Default for Sentinel {
    fn default() -> Self {
        Self {
            service: "mymaster".into(),
            seeds: vec![SentinelSeed {
                host: "localhost".into(),
                port: 26379,
            }],
            username: String::new(),
            tls: true,
            root_certificate: None,
            client_certificate: None,
            client_key: None,
            remember_password: false,
        }
    }
}
impl Sentinel {
    fn validate(&self) -> Result<(), String> {
        validate_identity(self.tls, &self.client_certificate, &self.client_key)?;
        if self.service.trim().is_empty() || self.service.len() > 256 || self.service.contains('\0')
        {
            return Err(
                "Enter a Sentinel service name of 1–256 bytes without NUL characters.".into(),
            );
        }
        if self.seeds.is_empty() || self.seeds.len() > 8 {
            return Err("Configure 1–8 Sentinel addresses.".into());
        }
        let mut seen = std::collections::HashSet::new();
        for seed in &self.seeds {
            if seed.host.trim().is_empty()
                || seed.host.len() > 1024
                || seed.host.contains('\0')
                || seed.port == 0
            {
                return Err("Each Sentinel needs a host and a port between 1 and 65535.".into());
            }
            if !seen.insert((&seed.host, seed.port)) {
                return Err("Remove duplicate Sentinel addresses.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub host: String,
    pub port: u16,
    pub database: u32,
    pub cluster: bool,
    pub cluster_seeds: Vec<SentinelSeed>,
    pub sentinel: Option<Sentinel>,
    pub username: String,
    pub tls: bool,
    pub root_certificate: Option<PathBuf>,
    pub client_certificate: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 6379,
            database: 0,
            cluster: false,
            cluster_seeds: Vec::new(),
            sentinel: None,
            username: String::new(),
            tls: true,
            root_certificate: None,
            client_certificate: None,
            client_key: None,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        validate_identity(self.tls, &self.client_certificate, &self.client_key)?;
        if let Some(sentinel) = &self.sentinel {
            if self.cluster {
                return Err("Choose either Cluster or Sentinel deployment.".into());
            }
            sentinel.validate()?;
        }
        if self.cluster {
            if self.cluster_seeds.len() > 7 { return Err("Configure at most seven alternate Cluster seeds.".into()); }
            let mut seen = std::collections::HashSet::new();
            seen.insert((self.host.as_str(), self.port));
            for seed in &self.cluster_seeds {
                if seed.host.is_empty() || seed.host.len() > 253 || seed.port == 0
                    || seed.host.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@' | '#' | '?'))
                    || (seed.host.contains(':') && seed.host.parse::<std::net::Ipv6Addr>().is_err())
                { return Err("Each alternate Cluster seed needs a host name or IP address and a valid port.".into()); }
                if !seen.insert((seed.host.as_str(), seed.port)) { return Err("Remove duplicate Cluster seed addresses.".into()); }
            }
        }
        if self.cluster && self.database != 0 {
            return Err("Redis Cluster only supports database zero.".into());
        }
        if self.sentinel.is_none() && self.host.trim().is_empty() {
            return Err("Enter a Redis host.".into());
        }
        if self.sentinel.is_none() && self.port == 0 {
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

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_pem(path: &std::path::Path, label: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Cannot read Redis {label}: {e}"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(format!("Redis {label} exceeds the 1 MiB limit."));
    }
    Ok(bytes)
}

fn validate_identity(tls: bool, certificate: &Option<PathBuf>, key: &Option<PathBuf>) -> Result<(), String> {
    if certificate.is_some() != key.is_some() {
        return Err("Configure both the Redis client certificate and private key.".into());
    }
    if !tls && certificate.is_some() {
        return Err("Redis client certificates require verified TLS.".into());
    }
    if certificate.iter().chain(key.iter()).any(|p| p.as_os_str().is_empty()) {
        return Err("Redis client certificate and key paths cannot be empty.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_identity_requires_verified_tls_and_paired_paths() {
        let mut settings: Settings = serde_json::from_str(r#"{"host":"legacy"}"#).unwrap();
        assert!(settings.client_certificate.is_none());
        assert!(settings.client_key.is_none());
        settings.client_certificate = Some("client.pem".into());
        assert!(settings.validate().unwrap_err().contains("both"));
        settings.client_key = Some("client.key".into());
        assert!(settings.validate().is_ok());
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&encoded).unwrap(), settings);
        settings.tls = false;
        assert!(settings.validate().unwrap_err().contains("verified TLS"));
        settings.tls = true;
        settings.sentinel = Some(Sentinel { client_key: Some("sentinel.key".into()), ..Default::default() });
        assert!(settings.validate().unwrap_err().contains("both"));
        settings.sentinel.as_mut().unwrap().client_certificate = Some("sentinel.pem".into());
        assert!(settings.validate().is_ok());
        settings.sentinel.as_mut().unwrap().tls = false;
        assert!(settings.validate().unwrap_err().contains("verified TLS"));
    }

    #[test]
    fn cluster_seed_profiles_are_bounded_validated_and_backward_compatible() {
        let old: Settings = serde_json::from_str(r#"{"cluster":true,"host":"legacy"}"#).unwrap();
        assert!(old.cluster_seeds.is_empty());
        let mut settings = Settings { cluster: true, cluster_seeds: vec![SentinelSeed { host: "alternate".into(), port: 6379 }], ..Default::default() };
        assert!(settings.validate().is_ok());
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(), settings);
        settings.cluster_seeds[0].host = settings.host.clone();
        assert!(settings.validate().unwrap_err().contains("duplicate"));
        settings.cluster_seeds[0].host = "redis://unsafe".into();
        assert!(settings.validate().is_err());
        settings.cluster_seeds = (0..8).map(|i| SentinelSeed { host: format!("seed-{i}"), port: 6379 }).collect();
        assert!(settings.validate().is_err());
    }
    #[test]
    fn sentinel_settings_are_bounded_separate_from_data_auth_and_backward_compatible() {
        let old: Settings = serde_json::from_str(r#"{"host":"legacy","database":4}"#).unwrap();
        assert!(old.sentinel.is_none());
        let mut settings = Settings {
            host: String::new(),
            port: 0,
            database: 4,
            username: "data-user".into(),
            sentinel: Some(Sentinel {
                username: "discovery-user".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(settings.validate().is_ok());
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&encoded).unwrap(),
            settings
        );
        settings.cluster = true;
        assert!(settings.validate().is_err());
        settings.cluster = false;
        let sentinel = settings.sentinel.as_mut().unwrap();
        sentinel.seeds.push(sentinel.seeds[0].clone());
        assert!(settings.validate().unwrap_err().contains("duplicate"));
        settings.sentinel.as_mut().unwrap().seeds.clear();
        assert!(settings.validate().is_err());
        settings.sentinel = Some(Sentinel::default());
        settings.sentinel.as_mut().unwrap().service = "a".repeat(257);
        assert!(settings.validate().is_err());
    }
    #[test]
    fn old_profiles_remain_standalone_and_cluster_requires_database_zero() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"host":"redis.example","database":2}"#).unwrap();
        assert!(!settings.cluster);
        assert!(settings.validate().is_ok());
        settings.cluster = true;
        assert!(settings.validate().unwrap_err().contains("database zero"));
        settings.database = 0;
        assert!(settings.validate().is_ok());
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
    }
}
