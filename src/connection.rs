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
