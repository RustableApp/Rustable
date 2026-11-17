//! Saved connection metadata for the MCP server, read from the desktop app's
//! eframe storage. Profiles hold no secrets; passwords stay in the system
//! credential store. The file is re-read only when its size or mtime changes.
use crate::connection_profiles::{ConnectionKind, ConnectionProfiles, Profile};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::SystemTime,
};

/// The subset of the desktop workspace the server needs. Unknown fields are ignored.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Workspace {
    saved_connections: ConnectionProfiles,
}

pub(super) fn engine(kind: &ConnectionKind) -> Option<&'static str> {
    match kind {
        ConnectionKind::SQLite { .. } => Some("SQLite"),
        ConnectionKind::Postgres(_) => Some("PostgreSQL"),
        ConnectionKind::Mysql(_) => Some("MySQL"),
        ConnectionKind::MariaDb(_) => Some("MariaDB"),
        ConnectionKind::Redis(_) => None,
    }
}

/// Address without credentials: user@host:port/database, or a SQLite path.
pub(super) fn target(kind: &ConnectionKind) -> String {
    match kind {
        ConnectionKind::SQLite { path } => path.display().to_string(),
        ConnectionKind::Postgres(p) => format!("{}@{}:{}/{}", p.user, p.host, p.port, p.database),
        ConnectionKind::Mysql(p) | ConnectionKind::MariaDb(p) => {
            format!("{}@{}:{}/{}", p.user, p.host, p.port, p.database)
        }
        ConnectionKind::Redis(_) => String::new(),
    }
}

pub(super) fn parse(text: &str) -> Result<ConnectionProfiles, String> {
    let entries: HashMap<String, String> =
        ron::from_str(text).map_err(|e| format!("Could not read Rustable settings: {e}"))?;
    let Some(workspace) = entries.get("workspace") else {
        return Ok(ConnectionProfiles::default());
    };
    ron::from_str::<Workspace>(workspace)
        .map(|workspace| workspace.saved_connections)
        .map_err(|e| format!("Could not read saved connections: {e}"))
}

pub(super) struct Catalog {
    path: Option<PathBuf>,
    stamp: Option<(Option<SystemTime>, u64)>,
    profiles: ConnectionProfiles,
    /// Optional allow-list of profile names or ids from `--connection`.
    allow: Vec<String>,
}

impl Catalog {
    pub(super) fn new(path: Option<PathBuf>, allow: Vec<String>) -> Self {
        Self { path, stamp: None, profiles: ConnectionProfiles::default(), allow }
    }

    pub(super) fn default_path() -> Option<PathBuf> {
        eframe::storage_dir(crate::APP_NAME).map(|directory| directory.join("app.ron"))
    }

    pub(super) fn location(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    fn refresh(&mut self) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or("Rustable's settings directory is unavailable on this platform")?;
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.profiles = ConnectionProfiles::default();
                self.stamp = None;
                return Ok(());
            }
            Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
        };
        let stamp = (metadata.modified().ok(), metadata.len());
        if self.stamp == Some(stamp) {
            return Ok(());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        self.profiles = parse(&text)?;
        self.stamp = Some(stamp);
        Ok(())
    }

    fn allowed(&self, profile: &Profile) -> bool {
        self.allow.is_empty()
            || self.allow.iter().any(|entry| {
                entry.eq_ignore_ascii_case(&profile.name)
                    || entry.trim_start_matches('#') == profile.id.0.to_string()
            })
    }

    /// SQL connections the server may use, in the app's display order.
    pub(super) fn profiles(&mut self) -> Result<Vec<Profile>, String> {
        self.refresh()?;
        Ok(self
            .profiles
            .profiles
            .iter()
            .filter(|profile| engine(&profile.connection).is_some() && self.allowed(profile))
            .cloned()
            .collect())
    }

    pub(super) fn group_name(&self, profile: &Profile) -> Option<String> {
        let id = profile.group?;
        self.profiles.groups.iter().find(|group| group.id == id).map(|group| group.name.clone())
    }

    /// Resolves an exact name, then a case-insensitive name, then a numeric id.
    pub(super) fn resolve(&mut self, reference: &str) -> Result<Profile, String> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Err("Provide a connection name from list_connections.".into());
        }
        let profiles = self.profiles()?;
        let pick = |matches: Vec<&Profile>| -> Option<Result<Profile, String>> {
            match matches.as_slice() {
                [] => None,
                [profile] => Some(Ok((*profile).clone())),
                several => Some(Err(format!(
                    "Several saved connections are named {reference:?}. Use an id instead: {}.",
                    several.iter().map(|p| p.id.0.to_string()).collect::<Vec<_>>().join(", ")
                ))),
            }
        };
        if let Some(found) = pick(profiles.iter().filter(|p| p.name == reference).collect()) {
            return found;
        }
        if let Some(found) =
            pick(profiles.iter().filter(|p| p.name.eq_ignore_ascii_case(reference)).collect())
        {
            return found;
        }
        if let Ok(id) = reference.trim_start_matches('#').parse::<u64>()
            && let Some(profile) = profiles.iter().find(|p| p.id.0 == id)
        {
            return Ok(profile.clone());
        }
        let hidden = self.profiles.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(reference));
        Err(match hidden {
            Some(profile) if engine(&profile.connection).is_none() => {
                format!("{reference:?} is a Redis connection. The MCP server supports SQLite, PostgreSQL, MySQL and MariaDB.")
            }
            Some(_) => format!("{reference:?} is not enabled for this MCP server (see its --connection options)."),
            None => format!("No saved connection named {reference:?}. Call list_connections for available names."),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection_profiles::{PostgresProfile, Profile};

    fn storage(profiles: &ConnectionProfiles) -> String {
        // Mirror eframe: the workspace is a RON string nested in a RON map, and
        // the real workspace carries many more fields than this server reads.
        #[derive(serde::Serialize)]
        struct DesktopWorkspace<'a> {
            recent: Vec<PathBuf>,
            saved_connections: &'a ConnectionProfiles,
            history: Vec<String>,
            page_size: usize,
        }
        let workspace = ron::to_string(&DesktopWorkspace {
            recent: vec!["/tmp/x".into()],
            saved_connections: profiles,
            history: vec!["SELECT 1".into()],
            page_size: 500,
        })
        .unwrap();
        let mut entries = HashMap::new();
        entries.insert("workspace".to_owned(), workspace);
        entries.insert("connection-window-sizes".to_owned(), "{}".to_owned());
        ron::ser::to_string_pretty(&entries, Default::default()).unwrap()
    }

    fn sample() -> ConnectionProfiles {
        let mut profiles = ConnectionProfiles::default();
        profiles
            .add_profile(Profile::draft(
                "Analytics".into(),
                ConnectionKind::Postgres(PostgresProfile::default()),
            ))
            .unwrap();
        profiles
            .add_profile(Profile::draft(
                "Local file".into(),
                ConnectionKind::SQLite { path: "/tmp/local.sqlite".into() },
            ))
            .unwrap();
        profiles
            .add_profile(Profile::draft(
                "Cache".into(),
                ConnectionKind::Redis(Box::default()),
            ))
            .unwrap();
        profiles
    }

    fn catalog(profiles: &ConnectionProfiles, allow: Vec<String>) -> (tempfile::TempDir, Catalog) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.ron");
        std::fs::write(&path, storage(profiles)).unwrap();
        (directory, Catalog::new(Some(path), allow))
    }

    #[test]
    fn reads_desktop_storage_and_lists_only_sql_connections() {
        let (_directory, mut catalog) = catalog(&sample(), Vec::new());
        let names: Vec<_> = catalog.profiles().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Analytics", "Local file"]);
        assert_eq!(catalog.resolve("analytics").unwrap().name, "Analytics");
        let id = catalog.resolve("Local file").unwrap().id.0;
        assert_eq!(catalog.resolve(&format!("#{id}")).unwrap().name, "Local file");
        assert!(catalog.resolve("Cache").unwrap_err().contains("Redis"));
        assert!(catalog.resolve("missing").unwrap_err().contains("list_connections"));
    }

    #[test]
    fn allow_list_hides_other_connections() {
        let (_directory, mut catalog) = catalog(&sample(), vec!["local FILE".into()]);
        let names: Vec<_> = catalog.profiles().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Local file"]);
        assert!(catalog.resolve("Analytics").unwrap_err().contains("not enabled"));
    }

    #[test]
    fn duplicate_names_require_an_id_and_missing_storage_is_empty() {
        let mut profiles = sample();
        profiles
            .add_profile(Profile::draft(
                "Analytics".into(),
                ConnectionKind::SQLite { path: "/tmp/other.sqlite".into() },
            ))
            .unwrap();
        let (directory, mut catalog) = catalog(&profiles, Vec::new());
        assert!(catalog.resolve("Analytics").unwrap_err().contains("Several"));
        std::fs::remove_file(directory.path().join("app.ron")).unwrap();
        assert!(catalog.profiles().unwrap().is_empty());
        assert_eq!(parse("{}").unwrap().profiles.len(), 0);
    }
}
