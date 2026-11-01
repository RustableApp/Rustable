/// Ephemeral connection credentials; deliberately excluded from preferences and Debug.
#[derive(Clone)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub tls: bool,
    pub ssh: Option<crate::ssh_tunnel::Config>,
}
impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            user: "postgres".into(),
            password: String::new(),
            tls: true,
            ssh: None,
        }
    }
}
impl PostgresConfig {
    pub fn label(&self) -> String {
        format!(
            "{}@{}:{}/{}",
            self.user, self.host, self.port, self.database
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_labels_never_include_passwords_and_tls_defaults_on() {
        let config = PostgresConfig {
            password: "secret-value".into(),
            ..Default::default()
        };
        assert!(config.tls);
        assert_eq!(config.label(), "postgres@localhost:5432/postgres");
        assert!(!config.label().contains(&config.password));
    }
}

/// MySQL/MariaDB credentials exist only for the live session, never persisted
/// or included in Debug output. Both engines use this wire configuration.
#[derive(Clone)]
pub struct MysqlConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub tls: bool,
    pub root_certificate: Option<std::path::PathBuf>,
    pub ssh: Option<crate::ssh_tunnel::Config>,
    pub connect_timeout: std::time::Duration,
    pub read_timeout: std::time::Duration,
}

impl Default for MysqlConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 3306,
            database: String::new(),
            user: "root".into(),
            password: String::new(),
            tls: true,
            root_certificate: None,
            ssh: None,
            connect_timeout: std::time::Duration::from_secs(5),
            read_timeout: std::time::Duration::from_secs(120),
        }
    }
}

impl std::fmt::Display for MysqlConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}:{}/{}", self.user, self.host, self.port, self.database)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<&MysqlConfig> for mysql::OptsBuilder {
    fn from(config: &MysqlConfig) -> Self {
        let tls = config.tls.then(|| mysql::SslOpts::default()
            .with_root_cert_path(config.root_certificate.clone()));
        Self::default()
            .ip_or_hostname(Some(config.host.clone()))
            .tcp_port(config.port)
            .user(Some(config.user.clone()))
            .pass(Some(config.password.clone()))
            .db_name((!config.database.is_empty()).then(|| config.database.clone()))
            // Don't silently switch transports and bypass the requested TLS policy.
            .prefer_socket(false)
            .ssl_opts(tls)
            .tcp_connect_timeout(Some(config.connect_timeout))
            .read_timeout(Some(config.read_timeout))
            .write_timeout(Some(std::time::Duration::from_secs(10)))
            .max_allowed_packet(Some(32 * 1024 * 1024))
            .additional_capabilities(mysql::consts::CapabilityFlags::CLIENT_SESSION_TRACK)
            .stmt_cache_size(32)
            .init(vec!["SET NAMES utf8mb4"])
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod mysql_tests {
    use super::*;
    #[test]
    fn configuration_preserves_tls_identity_and_bounds_transport() {
        let config = MysqlConfig { password: "secret-sentinel".into(), ..Default::default() };
        let opts = mysql::Opts::from(mysql::OptsBuilder::from(&config));
        assert!(opts.get_ssl_opts().is_some());
        assert!(!opts.get_ssl_opts().unwrap().skip_domain_validation());
        assert!(!opts.get_ssl_opts().unwrap().accept_invalid_certs());
        assert!(!opts.get_prefer_socket());
        assert_eq!(opts.get_db_name(), None);
        assert_eq!(opts.get_tcp_port(), 3306);
        assert_eq!(opts.get_max_allowed_packet(), Some(32 * 1024 * 1024));
        assert!(!config.to_string().contains("secret-sentinel"));
        assert_eq!(config.to_string(), "root@localhost:3306/");
    }

    #[test]
    #[ignore = "requires disposable TLS MySQL/MariaDB fixture and RUSTABLE_MYSQL_TEST_CA"]
    fn live_tls_verifies_ca_hostname_and_rejects_plaintext() {
        use mysql::prelude::Queryable;
        let port = std::env::var("RUSTABLE_MYSQL_TLS_TEST_PORT").unwrap().parse().unwrap();
        assert!([43318, 43319].contains(&port));
        let config = MysqlConfig {
            host: "localhost".into(), port, user: "rustable".into(),
            password: "rustable-test-only".into(), database: "rustable_test".into(),
            root_certificate: Some(std::env::var_os("RUSTABLE_MYSQL_TEST_CA").unwrap().into()),
            ..Default::default()
        };
        let mut conn = mysql::Conn::new(mysql::OptsBuilder::from(&config)).unwrap();
        let (_, cipher): (String, String) = conn.query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'").unwrap().unwrap();
        assert!(!cipher.is_empty(), "connection must actually use TLS");
        let version: String = conn.query_first("SELECT VERSION()").unwrap().unwrap();
        eprintln!("TLS verified: {version}; {cipher}");
        let mut untrusted = config.clone(); untrusted.root_certificate = None;
        let error = mysql::Conn::new(mysql::OptsBuilder::from(&untrusted)).expect_err("untrusted CA must be rejected").to_string();
        assert!(error.to_lowercase().contains("certificate"), "expected certificate verification failure: {error}");
        let mut wrong_host = config.clone(); wrong_host.host = "127.0.0.1".into();
        let error = mysql::Conn::new(mysql::OptsBuilder::from(&wrong_host)).expect_err("wrong hostname must be rejected").to_string();
        assert!(error.to_lowercase().contains("not valid for name"), "expected hostname verification failure: {error}");
        let mut plaintext = config.clone(); plaintext.tls = false;
        assert!(mysql::Conn::new(mysql::OptsBuilder::from(&plaintext)).is_err(), "fixture requires encrypted transport");
        let value: String = conn.query_first("SELECT '日本 🦀'").unwrap().unwrap();
        assert_eq!(value, "日本 🦀");
    }

    #[test]
    #[ignore = "requires disposable TLS MySQL/MariaDB fixture and RUSTABLE_MYSQL_TEST_CA"]
    fn live_tls_transport_destination_keeps_original_peer_verification() {
        use mysql::prelude::Queryable;
        let port = std::env::var("RUSTABLE_MYSQL_TLS_TEST_PORT").unwrap().parse().unwrap();
        assert!([43318,43319].contains(&port));
        let config = MysqlConfig { host:"localhost".into(), port:1, user:"rustable".into(),
            password:"rustable-test-only".into(), database:"rustable_test".into(),
            root_certificate:Some(std::env::var_os("RUSTABLE_MYSQL_TEST_CA").unwrap().into()), ..Default::default() };
        let destination = std::net::SocketAddr::from(([127,0,0,1],port));
        let opts = mysql::OptsBuilder::from(&config).tcp_destination(Some(destination));
        let mut conn = mysql::Conn::new(opts).unwrap();
        let (_, cipher): (String,String) = conn.query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'").unwrap().unwrap();
        assert!(!cipher.is_empty());
        assert_eq!(conn.query_first::<String,_>("SELECT '東京 🦀'").unwrap().as_deref(),Some("東京 🦀"));
        let mut wrong_name = config.clone(); wrong_name.host = "wrong-peer.invalid".into();
        let error = mysql::Conn::new(mysql::OptsBuilder::from(&wrong_name).tcp_destination(Some(destination))).unwrap_err().to_string();
        assert!(error.to_lowercase().contains("not valid for name"), "must verify original name rather than transport address: {error}");
        let mut untrusted = config; untrusted.root_certificate = None;
        let error = mysql::Conn::new(mysql::OptsBuilder::from(&untrusted).tcp_destination(Some(destination))).unwrap_err().to_string();
        assert!(error.to_lowercase().contains("certificate"), "transport override must not bypass trust: {error}");
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_config_connects_with_utf8_and_rejects_untrusted_tls() {
        use mysql::prelude::Queryable;
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let mut config = MysqlConfig {
            host: "127.0.0.1".into(), port, user: "rustable".into(),
            password: "rustable-test-only".into(), database: "rustable_test".into(),
            ..Default::default()
        };
        // Test servers have no publicly trusted certificate. Never fall back to plaintext.
        assert!(mysql::Conn::new(mysql::OptsBuilder::from(&config)).is_err());
        config.tls = false; // Explicit plaintext only for these loopback fixtures.
        let mut conn = mysql::Conn::new(mysql::OptsBuilder::from(&config)).unwrap();
        let charset: String = conn.query_first("SELECT @@character_set_results").unwrap().unwrap();
        assert_eq!(charset, "utf8mb4");
        let value: String = conn.query_first("SELECT '日本 🦀'").unwrap().unwrap();
        assert_eq!(value, "日本 🦀");
        assert_eq!(conn.transaction_status(), Some(false));
    }
}
