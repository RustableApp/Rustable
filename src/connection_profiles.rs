//! Saved connection definitions contain addresses and key paths, never credentials.
//! Vector order is display order. A profile belongs to one group or to the root.
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::HashSet, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshOptions {
    #[serde(default)]
    pub ignore_config: bool,
    pub host: String,
    /// Zero inherits OpenSSH configuration; existing nonzero ports remain overrides.
    pub port: u16,
    pub user: String,
    /// Optional filename only. Key bytes and passphrases belong to session state.
    pub identity_file: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresProfile {
    /// When SSH is enabled this is the database host as seen by the SSH server.
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub tls: crate::connection::PostgresTlsMode,
    pub ssh: Option<SshOptions>,
}
impl Default for PostgresProfile {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            database: "postgres".into(),
            user: "postgres".into(),
            tls: crate::connection::PostgresTlsMode::VerifyFull,
            ssh: None,
        }
    }
}
impl PostgresProfile {
    /// Credentials are supplied at connect time and never copied back into the profile.
    pub fn with_password(&self, password: String) -> crate::connection::PostgresConfig {
        crate::connection::PostgresConfig {
            host: self.host.clone(),
            port: self.port,
            database: self.database.clone(),
            user: self.user.clone(),
            password,
            tls: self.tls,
            ssh: self.ssh.as_ref().map(|ssh| crate::ssh_tunnel::Config {
                host: ssh.host.clone(),
                port: ssh.port,
                user: ssh.user.clone(),
                identity_file: ssh.identity_file.clone(),
                known_hosts_file: None,
                config_file: ssh.ignore_config.then(|| PathBuf::from("/dev/null")),
                remote_host: self.host.clone(),
                remote_port: self.port,
            }),
        }
    }
}
impl From<&crate::connection::PostgresConfig> for PostgresProfile {
    fn from(config: &crate::connection::PostgresConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            database: config.database.clone(),
            user: config.user.clone(),
            tls: config.tls,
            ssh: config.ssh.as_ref().map(|ssh| SshOptions { ignore_config: ssh.config_file.as_deref() == Some(std::path::Path::new("/dev/null")),
                host: ssh.host.clone(),
                port: ssh.port,
                user: ssh.user.clone(),
                identity_file: ssh.identity_file.clone(),
            }),
        }
    }
}

/// Persistable MySQL/MariaDB address and TLS settings. Passwords are session-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MysqlProfile {
    pub host: String,
    pub port: u16,
    /// Empty selects no default database, allowing the database browser to choose.
    pub database: String,
    pub user: String,
    pub tls: bool,
    pub root_certificate: Option<PathBuf>,
    pub ssh: Option<SshOptions>,
    pub connect_timeout: std::time::Duration,
    pub read_timeout: std::time::Duration,
}
impl Default for MysqlProfile {
    fn default() -> Self { Self::from(&crate::connection::MysqlConfig::default()) }
}
impl From<&crate::connection::MysqlConfig> for MysqlProfile {
    fn from(config: &crate::connection::MysqlConfig) -> Self {
        Self { host: config.host.clone(), port: config.port, database: config.database.clone(),
            user: config.user.clone(), tls: config.tls, root_certificate: config.root_certificate.clone(),
            ssh: config.ssh.as_ref().map(|ssh| SshOptions { ignore_config: ssh.config_file.as_deref() == Some(std::path::Path::new("/dev/null")), host:ssh.host.clone(),port:ssh.port,user:ssh.user.clone(),identity_file:ssh.identity_file.clone() }),
            connect_timeout: config.connect_timeout, read_timeout: config.read_timeout }
    }
}
impl MysqlProfile {
    pub fn with_password(&self, password: String) -> crate::connection::MysqlConfig {
        crate::connection::MysqlConfig { host: self.host.clone(), port: self.port, database: self.database.clone(),
            user: self.user.clone(), password, tls: self.tls, root_certificate: self.root_certificate.clone(),
            ssh: self.ssh.as_ref().map(|ssh| crate::ssh_tunnel::Config { host:ssh.host.clone(),port:ssh.port,user:ssh.user.clone(),identity_file:ssh.identity_file.clone(),known_hosts_file:None,config_file:ssh.ignore_config.then(|| PathBuf::from("/dev/null")),remote_host:self.host.clone(),remote_port:self.port }),
            connect_timeout: self.connect_timeout, read_timeout: self.read_timeout }
    }
    pub fn validate(&self) -> Result<(), String> {
        required(&self.host, "Database host")?;
        required(&self.user, "Database user")?;
        if self.port == 0 { return Err("Database port must be between 1 and 65535.".into()); }
        if self.connect_timeout.is_zero() || self.read_timeout.is_zero() {
            return Err("Connection and read timeouts must be greater than zero.".into());
        }
        if let Some(ssh) = &self.ssh {
            required(&ssh.host, "SSH host")?;
            if ssh.identity_file.as_ref().is_some_and(|path| path.as_os_str().is_empty()) {
                return Err("Choose an SSH key file or leave it unset to use the agent.".into());
            }
        }
        if self.root_certificate.as_ref().is_some_and(|path| path.as_os_str().is_empty()) {
            return Err("Choose a CA certificate file or leave it unset to use system roots.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionKind {
    SQLite { path: PathBuf },
    Postgres(PostgresProfile),
    Mysql(MysqlProfile),
    MariaDb(MysqlProfile),
    Redis(Box<crate::redis_settings::Settings>),
}
impl ConnectionKind {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Redis(settings) => settings.validate(),
            Self::SQLite { path } if path.as_os_str().is_empty() => {
                Err("Choose a SQLite file.".into())
            }
            Self::SQLite { .. } => Ok(()),
            Self::Mysql(config) | Self::MariaDb(config) => config.validate(),
            Self::Postgres(config) => {
                required(&config.host, "Database host")?;
                required(&config.database, "Database name")?;
                required(&config.user, "Database user")?;
                if config.port == 0 {
                    return Err("Database port must be between 1 and 65535.".into());
                }
                if let Some(ssh) = &config.ssh {
                    required(&ssh.host, "SSH host")?;
                    if ssh
                        .identity_file
                        .as_ref()
                        .is_some_and(|path| path.as_os_str().is_empty())
                    {
                        return Err(
                            "Choose an SSH key file or leave it unset to use the agent.".into()
                        );
                    }
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub group: Option<GroupId>,
    pub environment: String,
    pub color: [u8; 3],
    pub connection: ConnectionKind,
    /// Secure-store lookup marker, set automatically on Save/Connect.
    /// Lookup is keyed by this profile ID; no secret is serialized.
    #[serde(default)]
    pub remember_password: bool,
}
impl Profile {
    /// New drafts receive their durable ID only when inserted into the collection.
    pub fn draft(name: String, connection: ConnectionKind) -> Self {
        Self {
            id: ProfileId(0),
            name,
            group: None,
            environment: "Local".into(),
            color: [44, 111, 80],
            connection,
            remember_password: false,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        required(&self.name, "Connection name")?;
        self.connection.validate()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ConnectionProfiles {
    pub groups: Vec<Group>,
    pub profiles: Vec<Profile>,
    next_id: u64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredProfiles {
    groups: Vec<Group>,
    profiles: Vec<Profile>,
    next_id: u64,
}
impl<'de> Deserialize<'de> for ConnectionProfiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = StoredProfiles::deserialize(deserializer)?;
        let mut result = Self {
            groups: stored.groups,
            profiles: stored.profiles,
            next_id: stored.next_id,
        };
        result.repair_ids();
        Ok(result)
    }
}

impl ConnectionProfiles {
    /// Ordered nonsecret matches. Group names match every member; querying never
    /// mutates the collection or changes the order used by drag-and-drop.
    pub fn matching_profiles(&self, query: &str) -> Vec<ProfileId> {
        let query = query.trim().to_lowercase();
        self.profiles
            .iter()
            .filter(|profile| {
                if query.is_empty() {
                    return true;
                }
                let contains = |text: &str| text.to_lowercase().contains(&query);
                let group = profile
                    .group
                    .and_then(|id| self.groups.iter().find(|group| group.id == id));
                if contains(&profile.name)
                    || contains(&profile.environment)
                    || contains(group.map_or("Ungrouped", |group| group.name.as_str()))
                {
                    return true;
                }
                match &profile.connection {
                    ConnectionKind::Redis(config) => contains("Redis") || contains(&format!("{}@{}:{}/{}", config.username, config.host, config.port, config.database)),
                    ConnectionKind::SQLite { path } => {
                        contains("SQLite") || contains(&path.to_string_lossy())
                    }
                    ConnectionKind::Mysql(config) | ConnectionKind::MariaDb(config) => {
                        contains(if matches!(profile.connection, ConnectionKind::MariaDb(_)) { "MariaDB" } else { "MySQL" })
                            || contains(&format!("{}@{}:{}/{}", config.user, config.host, config.port, config.database))
                            || config.ssh.as_ref().is_some_and(|ssh| {
                                contains(&format!("{}@{}:{}", ssh.user, ssh.host, ssh.port))
                            })
                    }
                    ConnectionKind::Postgres(config) => {
                        contains("PostgreSQL")
                            || contains(&format!(
                                "{}@{}:{}/{}",
                                config.user, config.host, config.port, config.database
                            ))
                            || config.ssh.as_ref().is_some_and(|ssh| {
                                contains(&format!("{}@{}:{}", ssh.user, ssh.host, ssh.port))
                            })
                    }
                }
            })
            .map(|profile| profile.id)
            .collect()
    }
    /// Duplicate only persisted metadata: credentials are not part of this model.
    pub fn duplicate_profile(&mut self, id: ProfileId) -> Result<ProfileId, String> {
        let mut copy = self
            .profile(id)
            .ok_or("Connection no longer exists.")?
            .clone();
        let base = format!("{} copy", copy.name);
        let mut name = base.clone();
        let mut suffix = 2_u64;
        while self
            .profiles
            .iter()
            .any(|profile| profile.name.to_lowercase() == name.to_lowercase())
        {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        copy.id = ProfileId(0);
        copy.remember_password = false;
        if let ConnectionKind::Redis(settings) = &mut copy.connection && let Some(sentinel) = &mut settings.sentinel { sentinel.remember_password = false; }
        copy.name = name;
        self.add_profile(copy)
    }
    pub fn profile(&self, id: ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }
    pub fn in_group(&self, group: Option<GroupId>) -> impl Iterator<Item = &Profile> {
        self.profiles
            .iter()
            .filter(move |profile| profile.group == group)
    }
    pub fn add_group(&mut self, name: String) -> Result<GroupId, String> {
        required(&name, "Group name")?;
        let id = GroupId(self.allocate());
        self.groups.push(Group {
            id,
            name: name.trim().into(),
        });
        Ok(id)
    }
    pub fn rename_group(&mut self, id: GroupId, name: String) -> Result<(), String> {
        required(&name, "Group name")?;
        let group = self
            .groups
            .iter_mut()
            .find(|group| group.id == id)
            .ok_or("Group no longer exists.")?;
        group.name = name.trim().into();
        Ok(())
    }
    /// Removing a group preserves its connections, moving them to the root in order.
    pub fn remove_group(&mut self, id: GroupId) -> Result<(), String> {
        let index = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .ok_or("Group no longer exists.")?;
        self.groups.remove(index);
        for profile in &mut self.profiles {
            if profile.group == Some(id) {
                profile.group = None;
            }
        }
        Ok(())
    }
    /// `index` is the final zero-based display position, clamped to the list's end.
    pub fn move_group(&mut self, id: GroupId, index: usize) -> Result<(), String> {
        let old = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .ok_or("Group no longer exists.")?;
        let group = self.groups.remove(old);
        self.groups.insert(index.min(self.groups.len()), group);
        Ok(())
    }
    pub fn add_profile(&mut self, mut profile: Profile) -> Result<ProfileId, String> {
        profile.validate()?;
        self.validate_group(profile.group)?;
        profile.id = ProfileId(self.allocate());
        profile.name = profile.name.trim().into();
        let id = profile.id;
        self.profiles.push(profile);
        Ok(id)
    }
    pub fn update_profile(&mut self, mut profile: Profile) -> Result<(), String> {
        profile.validate()?;
        self.validate_group(profile.group)?;
        let old = self
            .profiles
            .iter_mut()
            .find(|old| old.id == profile.id)
            .ok_or("Connection no longer exists.")?;
        profile.name = profile.name.trim().into();
        *old = profile;
        Ok(())
    }
    pub fn remove_profile(&mut self, id: ProfileId) -> Result<Profile, String> {
        let index = self
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .ok_or("Connection no longer exists.")?;
        Ok(self.profiles.remove(index))
    }
    /// `index` is the final position within the destination group, excluding the moved item.
    /// Invalid IDs never mutate the collection; dropping past the last item appends.
    pub fn move_profile(
        &mut self,
        id: ProfileId,
        group: Option<GroupId>,
        index: usize,
    ) -> Result<(), String> {
        self.validate_group(group)?;
        let old = self
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .ok_or("Connection no longer exists.")?;
        let mut profile = self.profiles.remove(old);
        profile.group = group;
        let insert = self
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.group == group)
            .nth(index)
            .map_or(self.profiles.len(), |(index, _)| index);
        self.profiles.insert(insert, profile);
        Ok(())
    }
    fn validate_group(&self, id: Option<GroupId>) -> Result<(), String> {
        if id.is_some_and(|id| id.0 == 0 || !self.groups.iter().any(|group| group.id == id)) {
            Err("Destination group no longer exists.".into())
        } else {
            Ok(())
        }
    }
    fn allocate(&mut self) -> u64 {
        let occupied: HashSet<u64> = self
            .groups
            .iter()
            .map(|group| group.id.0)
            .chain(self.profiles.iter().map(|profile| profile.id.0))
            .collect();
        let id = free_id(&occupied, self.next_id);
        self.next_id = id.wrapping_add(1).max(1);
        id
    }
    fn repair_ids(&mut self) {
        // Reserve every existing ID first so repairing an early corrupt entry cannot
        // steal a valid later entry's identity. Duplicate group references target the first.
        let mut reserved: HashSet<u64> = self
            .groups
            .iter()
            .map(|group| group.id.0)
            .chain(self.profiles.iter().map(|profile| profile.id.0))
            .filter(|id| *id != 0)
            .collect();
        let original_profile_ids: Vec<_> = self.profiles.iter().map(|profile| profile.id).collect();
        let mut seen = HashSet::new();
        for id in self
            .groups
            .iter_mut()
            .map(|group| &mut group.id.0)
            .chain(self.profiles.iter_mut().map(|profile| &mut profile.id.0))
        {
            if *id == 0 || !seen.insert(*id) {
                *id = free_id(&reserved, self.next_id);
                reserved.insert(*id);
                seen.insert(*id);
                self.next_id = id.wrapping_add(1).max(1);
            }
        }
        for (profile, original) in self.profiles.iter_mut().zip(original_profile_ids) {
            if profile.id != original {
                profile.remember_password = false;
                if let ConnectionKind::Redis(settings) = &mut profile.connection && let Some(sentinel) = &mut settings.sentinel { sentinel.remember_password = false; }
            }
        }
        let groups: HashSet<GroupId> = self.groups.iter().map(|group| group.id).collect();
        for profile in &mut self.profiles {
            if profile
                .group
                .is_some_and(|id| id.0 == 0 || !groups.contains(&id))
            {
                profile.group = None;
            }
        }
        self.next_id = free_id(&reserved, self.next_id);
    }
}

fn free_id(occupied: &HashSet<u64>, start: u64) -> u64 {
    let mut id = start.max(1);
    while occupied.contains(&id) {
        id = id.wrapping_add(1).max(1);
    }
    id
}
fn required(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(format!(
            "{field} must not be empty or contain NUL characters."
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    fn draft(name: &str) -> Profile {
        Profile::draft(
            name.into(),
            ConnectionKind::Postgres(PostgresProfile::default()),
        )
    }

    #[test]
    fn legacy_ron_tls_booleans_keep_their_security_policy() {
        for (legacy, expected) in [("true", crate::connection::PostgresTlsMode::VerifyFull), ("false", crate::connection::PostgresTlsMode::Disable)] {
            let mut storage = Storage::default();
            eframe::set_value(&mut storage, "profile", &PostgresProfile::default());
            let encoded = storage.0.get_mut("profile").unwrap();
            assert!(encoded.contains("\"verify_full\""));
            *encoded = encoded.replace("\"verify_full\"", legacy);
            let restored: PostgresProfile = eframe::get_value(&storage, "profile").unwrap();
            assert_eq!(restored.tls, expected);
        }
    }

    #[test]
    fn ssh_config_defaults_allow_explicit_overrides_and_can_be_disabled() {
        let mut profile = PostgresProfile { ssh: Some(SshOptions {
            host: "gateway-alias".into(), user: "override-user".into(), port: 2222,
            identity_file: Some("/tmp/override-key".into()), ..Default::default()
        }), ..Default::default() };
        let config = profile.with_password(String::new()).ssh.unwrap();
        assert!(config.config_file.is_none());
        assert_eq!(config.port, 2222);
        assert_eq!(config.user, "override-user");
        profile.ssh.as_mut().unwrap().ignore_config = true;
        let config = profile.with_password(String::new()).ssh.unwrap();
        assert_eq!(config.config_file, Some(PathBuf::from("/dev/null")));
        assert_eq!(config.identity_file, Some(PathBuf::from("/tmp/override-key")));
        let old: SshOptions = serde_json::from_str(r#"{"host":"alias","port":0,"user":"","identity_file":null}"#).unwrap();
        assert!(!old.ignore_config);
    }

    #[test]
    fn redis_profile_roundtrips_with_settings_and_no_embedded_password() {
        let profile = Profile::draft("Redis".into(), ConnectionKind::Redis(Box::new(crate::redis_settings::Settings {
            host: "cache.example.test".into(), database: 3, username: "app".into(),
            root_certificate: Some("/tmp/ca.crt".into()), ..Default::default()
        })));
        assert!(profile.validate().is_ok());
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "redis", &profile);
        let restored: Profile = eframe::get_value(&storage, "redis").unwrap();
        assert_eq!(restored, profile);
        let ConnectionKind::Redis(config) = restored.connection else { panic!() };
        let serialized = serde_json::to_string(&config).unwrap();
        assert!(!serialized.contains("password"));
        assert_eq!(config.database, 3);
        // Boxing must preserve the pre-existing persisted enum representation.
        let legacy = serde_json::json!({ "Redis": serde_json::to_value(config.as_ref()).unwrap() });
        let loaded: ConnectionKind = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(loaded).unwrap(), legacy);
    }
    #[test]
    fn ssh_inheritance_roundtrips_without_changing_legacy_explicit_ports() {
        let inherited = SshOptions::default();
        assert_eq!(inherited.port, 0);
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "ssh", &inherited);
        let restored: SshOptions = eframe::get_value(&storage, "ssh").unwrap();
        assert_eq!(restored, inherited);
        storage.0.insert(
            "ssh".into(),
            "(host: \"alias\", port: 22, user: \"old-user\", identity_file: None)".into(),
        );
        let legacy: SshOptions = eframe::get_value(&storage, "ssh").unwrap();
        assert_eq!(legacy.port, 22);
        assert_eq!(legacy.user, "old-user");
        let mut profile = draft("Inherited SSH");
        if let ConnectionKind::Postgres(config) = &mut profile.connection {
            config.ssh = Some(SshOptions { ignore_config: false,
                host: "alias".into(),
                ..Default::default()
            });
        }
        assert!(profile.validate().is_ok());
    }

    fn order(model: &ConnectionProfiles, group: Option<GroupId>) -> Vec<String> {
        model
            .in_group(group)
            .map(|profile| profile.name.clone())
            .collect()
    }
    #[test]
    fn searches_nonsecret_metadata_and_group_members_without_reordering() {
        let mut model = ConnectionProfiles::default();
        let group = model.add_group("Analytics équipe".into()).unwrap();
        let mut a = draft("Orders");
        a.group = Some(group);
        a.environment = "Staging".into();
        if let ConnectionKind::Postgres(config) = &mut a.connection {
            config.host = "db.example.test".into();
            config.database = "warehouse".into();
            config.user = "analyst".into();
            config.port = 6432;
            config.ssh = Some(SshOptions { ignore_config: false,
                host: "gateway.example.test".into(),
                user: "ssh-user".into(),
                ..Default::default()
            });
        }
        let a = model.add_profile(a).unwrap();
        let mut b = Profile::draft(
            "Local".into(),
            ConnectionKind::SQLite {
                path: "/tmp/fixtures/report.sqlite".into(),
            },
        );
        b.group = Some(group);
        let b = model.add_profile(b).unwrap();
        let c = model.add_profile(draft("Elsewhere")).unwrap();
        for query in [
            " ORDERS ",
            "STAGING",
            "db.example",
            "WAREHOUSE",
            "ANALYST",
            "6432",
            "GATEWAY",
            "SSH-USER",
        ] {
            assert_eq!(model.matching_profiles(query), vec![a], "{query}");
        }
        assert_eq!(model.matching_profiles("ANALYTICS ÉQUIPE"), vec![a, b]);
        assert_eq!(model.matching_profiles("report.sqlite"), vec![b]);
        assert_eq!(model.matching_profiles("ungrouped"), vec![c]);
        assert!(model.matching_profiles("no matches").is_empty());
        assert_eq!(model.matching_profiles("   "), vec![a, b, c]);
        assert_eq!(
            model
                .profiles
                .iter()
                .map(|profile| profile.id)
                .collect::<Vec<_>>(),
            vec![a, b, c]
        );
    }
    #[test]
    fn duplicate_preserves_transport_group_style_and_allocates_unique_names_and_ids() {
        let mut model = ConnectionProfiles::default();
        let group = model.add_group("Servers".into()).unwrap();
        let mut original = draft("Warehouse");
        original.group = Some(group);
        original.environment = "Production".into();
        original.color = [120, 30, 40];
        if let ConnectionKind::Postgres(config) = &mut original.connection {
            config.ssh = Some(SshOptions { ignore_config: false,
                host: "gateway.test".into(),
                user: "operator".into(),
                identity_file: Some("/keys/qa".into()),
                ..Default::default()
            });
        }
        let source = model.add_profile(original).unwrap();
        model.add_profile(draft("WAREHOUSE COPY")).unwrap();
        let first = model.duplicate_profile(source).unwrap();
        let second = model.duplicate_profile(source).unwrap();
        assert_ne!(source, first);
        assert_ne!(first, second);
        assert_eq!(model.profile(first).unwrap().name, "Warehouse copy 2");
        assert_eq!(model.profile(second).unwrap().name, "Warehouse copy 3");
        let source_profile = model.profile(source).unwrap();
        let copy = model.profile(first).unwrap();
        assert_eq!(source_profile.connection, copy.connection);
        assert_eq!(source_profile.group, copy.group);
        assert_eq!(source_profile.environment, copy.environment);
        assert_eq!(source_profile.color, copy.color);
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "profiles", &model);
        let restored: ConnectionProfiles = eframe::get_value(&storage, "profiles").unwrap();
        assert_eq!(restored.profiles, model.profiles);
        let before = model.profiles.clone();
        assert!(model.duplicate_profile(ProfileId(u64::MAX)).is_err());
        assert_eq!(model.profiles, before);
    }
    #[test]
    fn moving_connections_and_deleting_groups_preserves_identity_and_order() {
        let mut model = ConnectionProfiles::default();
        let group = model.add_group("Production".into()).unwrap();
        let a = model.add_profile(draft("a")).unwrap();
        let b = model.add_profile(draft("b")).unwrap();
        let c = model.add_profile(draft("c")).unwrap();
        model.move_profile(c, None, 0).unwrap();
        assert_eq!(order(&model, None), ["c", "a", "b"]);
        model.move_profile(a, Some(group), 0).unwrap();
        model.move_profile(b, Some(group), 0).unwrap();
        assert_eq!(order(&model, Some(group)), ["b", "a"]);
        assert_eq!(order(&model, None), ["c"]);
        model.remove_group(group).unwrap();
        assert_eq!(order(&model, None), ["c", "b", "a"]);
        assert_eq!(model.profile(a).unwrap().name, "a");
        assert!(model.move_profile(a, Some(group), 0).is_err());
        assert_eq!(order(&model, None), ["c", "b", "a"]);
    }
    #[test]
    fn persistence_repairs_corrupt_ids_and_orphans_without_dropping_profiles() {
        let mut model = ConnectionProfiles::default();
        let group = model.add_group("One".into()).unwrap();
        model.groups.push(Group {
            id: group,
            name: "Duplicate".into(),
        });
        for name in ["a", "b", "c"] {
            model.add_profile(draft(name)).unwrap();
        }
        model.profiles[0].id = ProfileId(0);
        model.profiles[1].id = model.profiles[2].id;
        model.profiles[0].group = Some(GroupId(999));
        model.profiles[1].group = Some(group);
        let stable = model.profiles[2].id;
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "profiles", &model);
        let mut loaded: ConnectionProfiles = eframe::get_value(&storage, "profiles").unwrap();
        assert_eq!(loaded.profiles.len(), 3);
        let ids: Vec<_> = loaded
            .groups
            .iter()
            .map(|group| group.id.0)
            .chain(loaded.profiles.iter().map(|profile| profile.id.0))
            .collect();
        assert!(!ids.contains(&0));
        assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len());
        assert_eq!(loaded.profiles[1].id, stable);
        assert_eq!(loaded.profiles[0].group, None);
        assert_eq!(loaded.profiles[1].group, Some(group));
        let next = loaded.add_profile(draft("new")).unwrap();
        assert!(!ids.contains(&next.0));
    }
    #[test]
    fn remembered_password_is_opt_in_and_duplicate_does_not_share_credentials() {
        let mut model = ConnectionProfiles::default();
        let mut profile = draft("Remembered");
        profile.remember_password = true;
        let id = model.add_profile(profile).unwrap();
        let copied = model.duplicate_profile(id).unwrap();
        assert!(!model.profile(copied).unwrap().remember_password);
        let mut storage = Storage::default();
        eframe::set_value(
            &mut storage,
            "old-profile",
            &Profile::draft(
                "Legacy".into(),
                ConnectionKind::Postgres(PostgresProfile::default()),
            ),
        );
        let legacy = storage.0["old-profile"]
            .replace("remember_password:false", "")
            .replace(",)", ")");
        storage.0.insert("old-profile".into(), legacy);
        let restored: Profile = eframe::get_value(&storage, "old-profile").unwrap();
        assert!(!restored.remember_password);
        model.profiles[0].connection = ConnectionKind::Redis(Box::new(crate::redis_settings::Settings {
            sentinel: Some(crate::redis_settings::Sentinel { remember_password: true, ..Default::default() }), ..Default::default()
        }));
        model.profiles[0].id = ProfileId(0);
        eframe::set_value(&mut storage, "repaired", &model);
        let repaired: ConnectionProfiles = eframe::get_value(&storage, "repaired").unwrap();
        assert!(
            !repaired.profiles[0].remember_password,
            "reassigned identities must not inherit secure-store references"
        );
        let ConnectionKind::Redis(settings) = &repaired.profiles[0].connection else { panic!() };
        assert!(!settings.sentinel.as_ref().unwrap().remember_password);
    }

    #[test]
    fn saved_profiles_cannot_capture_session_passwords_or_private_key_bytes() {
        let config = PostgresProfile {
            ssh: Some(SshOptions { ignore_config: false,
                host: "bastion.test".into(),
                user: "alice".into(),
                identity_file: Some("/keys/example".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let session = config.with_password("TOP_SECRET_PASSWORD".into());
        assert_eq!(session.password, "TOP_SECRET_PASSWORD");
        let mut model = ConnectionProfiles::default();
        model
            .add_profile(Profile::draft(
                "Server".into(),
                ConnectionKind::Postgres(config),
            ))
            .unwrap();
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "profiles", &model);
        let text = &storage.0["profiles"];
        assert!(!text.replace("remember_password", "").contains("password"));
        assert!(!text.contains("TOP_SECRET"));
        assert!(!text.contains("PRIVATE KEY"));
        let restored: ConnectionProfiles = eframe::get_value(&storage, "profiles").unwrap();
        assert_eq!(restored.profiles, model.profiles);
    }
    #[test]
    fn crud_validates_before_mutating_and_group_moves_use_final_positions() {
        let mut model = ConnectionProfiles::default();
        assert!(model.add_group(" ".into()).is_err());
        assert!(model.add_profile(draft("")).is_err());
        let a = model.add_group("a".into()).unwrap();
        let b = model.add_group("b".into()).unwrap();
        model.move_group(a, 1).unwrap();
        assert_eq!(
            model
                .groups
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            [b, a]
        );
        model.rename_group(a, "renamed".into()).unwrap();
        let id = model.add_profile(draft("db")).unwrap();
        let mut changed = model.profile(id).unwrap().clone();
        changed.name = "new".into();
        changed.group = Some(a);
        model.update_profile(changed).unwrap();
        assert_eq!(model.profile(id).unwrap().name, "new");
        assert_eq!(model.remove_profile(id).unwrap().group, Some(a));
        assert!(model.remove_profile(id).is_err());
        assert!(model.profile(id).is_none());
    }
}

#[cfg(test)]
mod mysql_profile_tests {
    use super::*;
    #[test]
    fn mysql_profile_roundtrip_never_retains_credentials() {
        let config = crate::connection::MysqlConfig { host: "db.example.test".into(), port: 3307,
            database: "metrics".into(), user: "reader".into(), password: "PRIVATE_PASSWORD_SENTINEL".into(),
            root_certificate: Some("/certs/company.pem".into()), ..Default::default() };
        let profile = MysqlProfile::from(&config);
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("password") && !json.contains(&config.password));
        assert!(!format!("{profile:?}").contains(&config.password));
        let restored: MysqlProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, profile);
        assert!(restored.validate().is_ok());
    }
    #[test]
    fn mysql_ssh_profile_restores_destination_without_credentials() {
        let profile = MysqlProfile { host:"db.internal".into(),port:3307,
            ssh:Some(SshOptions { ignore_config: false, host:"gateway".into(),user:"ssh-user".into(),identity_file:Some("/keys/client".into()),..Default::default() }),..Default::default() };
        let config = profile.with_password("SESSION_SECRET".into());
        let ssh = config.ssh.as_ref().unwrap();
        assert_eq!((&ssh.remote_host,ssh.remote_port),(&profile.host,3307));
        assert_eq!(ssh.host,"gateway");
        assert_eq!(ssh.port,0);
        assert!(ssh.known_hosts_file.is_none() && ssh.config_file.is_none());
        assert_eq!(MysqlProfile::from(&config),profile);
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("SESSION_SECRET"));
        assert_eq!(serde_json::from_str::<MysqlProfile>(&json).unwrap(),profile);
        assert!(profile.validate().is_ok());
        let mut invalid = profile;
        invalid.ssh.as_mut().unwrap().host.clear();
        assert!(invalid.validate().unwrap_err().contains("SSH host"));
    }
    #[test]
    fn mysql_gateway_search_finds_profiles_without_matching_private_key_paths() {
        let mut model = ConnectionProfiles::default();
        let config = MysqlProfile { host:"db.internal".into(),database:"warehouse".into(),
            ssh:Some(SshOptions { ignore_config: false, host:"gateway.example.test".into(),user:"jump-user".into(),port:2222,
                identity_file:Some("/private/key-sentinel".into()) }),..Default::default() };
        let mysql = model.add_profile(Profile::draft("Orders".into(),ConnectionKind::Mysql(config.clone()))).unwrap();
        let maria = model.add_profile(Profile::draft("Reporting".into(),ConnectionKind::MariaDb(config))).unwrap();
        let direct = model.add_profile(Profile::draft("Direct".into(),ConnectionKind::Mysql(MysqlProfile::default()))).unwrap();
        for query in [" GATEWAY.EXAMPLE ","JUMP-USER","2222","DB.INTERNAL","WAREHOUSE"] {
            assert_eq!(model.matching_profiles(query),vec![mysql,maria],"{query}");
        }
        assert!(model.matching_profiles("key-sentinel").is_empty());
        assert_eq!(model.matching_profiles(""),vec![mysql,maria,direct]);
        assert_eq!(model.matching_profiles("MariaDB"),vec![maria]);
    }

    #[test]
    fn mysql_profile_defaults_and_validation_allow_database_selection_later() {
        let mut profile: MysqlProfile = serde_json::from_str("{}").unwrap();
        assert!(profile.tls);
        assert_eq!(profile.port, 3306);
        assert!(profile.database.is_empty());
        assert!(profile.validate().is_ok());
        profile.port = 0; assert!(profile.validate().is_err());
        profile.port = 3306;
        profile.connect_timeout = std::time::Duration::ZERO; assert!(profile.validate().is_err());
        profile.connect_timeout = std::time::Duration::from_secs(5);
        profile.read_timeout = std::time::Duration::ZERO; assert!(profile.validate().is_err());
        profile.read_timeout = std::time::Duration::from_secs(120);
        profile.root_certificate = Some(PathBuf::new()); assert!(profile.validate().is_err());
        profile.root_certificate = None;
        profile.host = "  ".into(); assert!(profile.validate().is_err());
    }
}
