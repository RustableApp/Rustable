//! Explicit file import. No source-app discovery, keychain reads or connections.
use crate::connection_profiles::{
    ConnectionKind, ConnectionProfiles, MysqlProfile, PostgresProfile, Profile, ProfileId,
    SshOptions,
};
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use eframe::egui;
use serde_json::Value;
use std::{collections::HashMap, io::Read, path::Path};
use zeroize::Zeroizing;

type ImportResult = (Vec<(ProfileId, Zeroizing<String>)>, usize);

pub const MAX_FILE: usize = 8 * 1024 * 1024;
pub struct Candidate {
    pub name: String,
    pub driver: String,
    pub group: Option<String>,
    pub profile: Option<Profile>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub selected: bool,
    pub password: Option<Zeroizing<String>>,
}
pub enum Loaded {
    Locked(Zeroizing<Vec<u8>>),
    Preview(Vec<Candidate>),
}
pub fn read(path: &Path) -> Result<Loaded, String> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)
        .map_err(|_| "Cannot open the selected file.")?
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read the selected file.")?;
    if bytes.len() > MAX_FILE {
        return Err("Connection export exceeds the 8 MiB limit.".into());
    }
    if bytes.starts_with(&[3, 1]) {
        validate_envelope(&bytes)?;
        Ok(Loaded::Locked(bytes))
    } else {
        parse(&bytes).map(Loaded::Preview)
    }
}
fn validate_envelope(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_FILE
        || bytes.len() < 82
        || !bytes.starts_with(&[3, 1])
        || !(bytes.len() - 66).is_multiple_of(16)
    {
        return Err(
            "Invalid or unsupported encrypted export. Expected RNCryptor version 3.".into(),
        );
    }
    Ok(())
}
pub fn unlock(bytes: &[u8], password: &str) -> Result<Vec<Candidate>, String> {
    parse(&decrypt(bytes, password)?)
}
fn decrypt(bytes: &[u8], password: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    validate_envelope(bytes)?;
    if password.len() > 4096 {
        return Err("Export password is too long.".into());
    }
    let mut mac_key = Zeroizing::new([0_u8; 32]);
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA1,
        std::num::NonZeroU32::new(10_000).unwrap(),
        &bytes[10..18],
        password.as_bytes(),
        mac_key.as_mut(),
    );
    let end = bytes.len() - 32;
    ring::hmac::verify(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, mac_key.as_ref()),
        &bytes[..end],
        &bytes[end..],
    )
    .map_err(|_| "Incorrect export password or damaged file.")?;
    // Authenticate before decrypting; never expose plaintext on a bad MAC.
    let mut key = Zeroizing::new([0_u8; 32]);
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA1,
        std::num::NonZeroU32::new(10_000).unwrap(),
        &bytes[2..10],
        password.as_bytes(),
        key.as_mut(),
    );
    let mut plaintext = Zeroizing::new(bytes[34..end].to_vec());
    let decoded = cbc::Decryptor::<aes::Aes256>::new_from_slices(key.as_ref(), &bytes[18..34])
        .map_err(|_| "Invalid export encryption parameters.")?
        .decrypt_padded::<Pkcs7>(&mut plaintext)
        .map_err(|_| "Damaged encrypted export.")?;
    let len = decoded.len();
    plaintext.truncate(len);
    Ok(plaintext)
}
fn text(v: &Value, name: &str) -> String {
    match &v[name] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
fn flag(v: &Value, name: &str) -> bool {
    matches!(&v[name], Value::Bool(true)) || matches!(text(v, name).as_str(), "1" | "true" | "YES")
}
fn port(v: &Value, name: &str, default: u16) -> Result<u16, String> {
    let s = text(v, name);
    if s.is_empty() {
        return Ok(default);
    }
    s.parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| format!("Invalid {name}; expected a port from 1 to 65535."))
}
fn nonempty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn flatten_records<'a>(
    records: &'a [Value],
    parents: &[String],
    out: &mut Vec<(&'a Value, Vec<String>)>,
    visited: &mut usize,
) -> Result<(), String> {
    if parents.len() > 32 {
        return Err("Group nesting exceeds 32 levels.".into());
    }
    for record in records {
        *visited += 1;
        if *visited > 2000 {
            return Err("Export contains too many connection/group records.".into());
        }
        if record.get("Driver").is_none() && record.get("Name").is_some() {
            let mut path = parents.to_vec();
            if let Some(name) = nonempty(text(record, "Name")) {
                path.push(name);
            }
            for key in ["connections", "groups"] {
                if let Some(children) = record.get(key) {
                    let children = children
                        .as_array()
                        .ok_or("Invalid group contents in export.")?;
                    flatten_records(children, &path, out, visited)?;
                }
            }
        } else {
            out.push((record, parents.to_vec()));
            if out.len() > 1000 {
                return Err("Export exceeds 1000 connections.".into());
            }
        }
    }
    Ok(())
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Candidate>, String> {
    if bytes.len() > MAX_FILE {
        return Err("Connection export exceeds the 8 MiB limit.".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "Unsupported file. Choose a TablePlus .tableplusconnection export or its unencrypted JSON; property lists and other encrypted formats are not supported.")?;
    let records = if let Some(a) = value.as_array() {
        a
    } else {
        value
            .get("Connections")
            .and_then(Value::as_array)
            .ok_or("No TablePlus connection records found in this file.")?
    };
    if records.is_empty() || records.len() > 1000 {
        return Err("Choose an export containing between 1 and 1000 connections.".into());
    }
    let groups: HashMap<String, (String, String)> = value
        .get("ConnectionGroups")
        .or_else(|| value.get("Groups"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            records
                .iter()
                .filter(|r| r.get("Driver").is_none() && r.get("Name").is_some()),
        )
        .map(|r| (text(r, "ID"), (text(r, "Name"), text(r, "GroupID"))))
        .collect();
    let mut flat = Vec::new();
    flatten_records(records, &[], &mut flat, &mut 0)?;
    let mut out = Vec::new();
    for (r, parent_names) in flat {
        if r.get("Driver").is_none() && r.get("Name").is_some() {
            continue;
        }
        let mut c = Candidate {
            name: text(r, "ConnectionName"),
            driver: text(r, "Driver"),
            group: None,
            profile: None,
            warnings: vec![],
            error: None,
            selected: false,
            password: None,
        };
        if c.name.is_empty() {
            c.name = "Unnamed connection".into();
        }
        let mut id = text(r, "GroupID");
        let mut names = vec![];
        let mut seen = std::collections::HashSet::new();
        while !id.is_empty() {
            if !seen.insert(id.clone()) {
                c.warnings
                    .push("Cyclic group hierarchy was flattened.".into());
                break;
            }
            let Some((name, parent)) = groups.get(&id) else {
                c.warnings
                    .push("Group name was not included; imported as Ungrouped.".into());
                names.clear();
                break;
            };
            names.push(name.clone());
            id = parent.clone();
        }
        names.reverse();
        c.group = if names.is_empty() {
            nonempty(text(r, "GroupName"))
        } else {
            Some(names.join(" / "))
        };
        if !parent_names.is_empty() {
            c.group = Some(parent_names.join(" / "));
        }
        if names.len() > 1 || parent_names.len() > 1 {
            c.warnings
                .push("Nested groups become a single group path.".into());
        }
        match convert(r, &mut c.warnings) {
            Ok(kind) => {
                let mut profile = Profile::draft(c.name.clone(), kind);
                profile.environment = nonempty(text(r, "Enviroment"))
                    .or_else(|| nonempty(text(r, "Environment")))
                    .unwrap_or_else(|| "Local".into());
                let color = text(r, "statusColor");
                let color = color.trim_start_matches('#');
                if color.len() == 6 && color.is_ascii()
                    && let Ok(rgb) = u32::from_str_radix(color, 16) {
                    profile.color = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
                }
                if let Err(e) = profile.validate() {
                    c.error = Some(e);
                } else {
                    c.profile = Some(profile);
                    c.selected = true;
                }
            }
            Err(e) => c.error = Some(e),
        }
        let password = text(r, "DatabasePassword");
        // Command-line credential modes must never become saved plaintext passwords.
        if !password.is_empty() && text(r, "DatabasePasswordMode") == "0" {
            c.password = Some(Zeroizing::new(password));
        } else {
            c.warnings
                .push("Enter the database password after import if required.".into());
        }
        if !text(r, "StartupCommands").is_empty() || !text(r, "StartupBashScript").is_empty() {
            c.warnings
                .push("Startup SQL and shell commands are not imported or executed.".into());
        }
        if !text(r, "DatabasePasswordMode").is_empty() && text(r, "DatabasePasswordMode") != "0" {
            c.warnings.push(
                "Password provider settings are not imported; enter credentials manually.".into(),
            );
        }
        out.push(c);
    }
    if out.is_empty() {
        return Err("No TablePlus connections found.".into());
    }
    Ok(out)
}
pub(super) fn convert(v: &Value, warnings: &mut Vec<String>) -> Result<ConnectionKind, String> {
    let driver = text(v, "Driver").to_ascii_lowercase();
    if !matches!(
        driver.as_str(),
        "postgresql" | "postgres" | "mysql" | "mariadb" | "sqlite" | "redis"
    ) {
        return Err("This database engine is not supported by Rustable.".into());
    }
    if flag(v, "isUseSocket") {
        return Err("Unix socket connections must be configured manually.".into());
    }
    if driver == "sqlite" {
        if flag(v, "isOverSSH") || !text(v, "DatabaseKeyPassword").is_empty() {
            return Err(
                "Remote or encrypted SQLite connections must be configured manually.".into(),
            );
        }
        warnings.push("Check the SQLite file path on this computer before connecting.".into());
        return Ok(ConnectionKind::SQLite {
            path: text(v, "DatabasePath").into(),
        });
    }
    let ssh = if flag(v, "isOverSSH") {
        if driver == "redis" {
            return Err(
                "Redis SSH tunnels are not supported; no direct connection will be substituted."
                    .into(),
            );
        }
        warnings.push("SSH passwords/private keys are not imported. Configure an SSH agent or select your key after import; the gateway must already be trusted.".into());
        Some(SshOptions {
            host: text(v, "ServerAddress"),
            port: port(v, "ServerPort", 22)?,
            user: text(v, "ServerUser"),
            identity_file: None,
        })
    } else {
        None
    };
    let tls_mode = text(v, "tLSMode");
    let tls = tls_mode != "0";
    warnings.push("Review the imported TLS mode before connecting; TablePlus TLS options are not identical to Rustable’s.".into());
    if tls {
        warnings.push(
            "TLS uses Rustable's certificate verification. Review TLS settings before connecting."
                .into(),
        );
    }
    if !text(v, "TlsKeyName").is_empty()
        || v.get("TlsKeyPaths")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|p| p.as_str().is_some_and(|s| !s.is_empty())))
    {
        warnings.push("Custom TLS certificates/private keys are not imported. Re-select certificates in the connection editor.".into());
    }
    if !text(v, "DatabaseUserRole").is_empty()
        || !text(v, "Authenticator").is_empty()
        || v.get("OtherOptions")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    {
        warnings.push("Advanced role/authentication/driver options need manual review.".into());
    }
    let host = text(v, "DatabaseHost");
    let user = text(v, "DatabaseUser");
    let database = text(v, "DatabaseName");
    Ok(match driver.as_str() {
        "postgresql" | "postgres" => ConnectionKind::Postgres(PostgresProfile {
            host,
            port: port(v, "DatabasePort", 5432)?,
            database: if database.is_empty() {
                warnings
                    .push("No default PostgreSQL database was exported; using postgres.".into());
                "postgres".into()
            } else {
                database
            },
            user,
            tls,
            ssh,
        }),
        "mysql" | "mariadb" => {
            let c = MysqlProfile {
                host,
                port: port(v, "DatabasePort", 3306)?,
                database,
                user,
                tls,
                ssh,
                ..Default::default()
            };
            if driver == "mysql" {
                ConnectionKind::Mysql(c)
            } else {
                ConnectionKind::MariaDb(c)
            }
        }
        "redis" => ConnectionKind::Redis(crate::redis_settings::Settings {
            host,
            port: port(v, "DatabasePort", 6379)?,
            database: if database.is_empty() {
                0
            } else {
                database
                    .parse()
                    .map_err(|_| "Invalid Redis database number.")?
            },
            username: user,
            tls,
            ..Default::default()
        }),
        _ => unreachable!(),
    })
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Duplicates {
    Skip,
    KeepBoth,
}
pub fn is_duplicate(store: &ConnectionProfiles, c: &Candidate) -> bool {
    c.profile.as_ref().is_some_and(|p| {
        store
            .profiles
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&p.name) || e.connection == p.connection)
    })
}
/// Apply to a clone: validation failure leaves the saved collection untouched.
pub fn apply(
    store: &mut ConnectionProfiles,
    candidates: &mut [Candidate],
    duplicates: Duplicates,
    passwords: bool,
) -> Result<ImportResult, String> {
    let mut next = store.clone();
    let mut credentials = Vec::new();
    let mut count = 0;
    for c in candidates
        .iter_mut()
        .filter(|c| c.selected && c.profile.is_some())
    {
        if duplicates == Duplicates::Skip && is_duplicate(&next, c) {
            continue;
        }
        let mut p = c.profile.clone().unwrap();
        let base = p.name.clone();
        let mut suffix = 2;
        while next
            .profiles
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&p.name))
        {
            p.name = format!("{base} ({suffix})");
            suffix += 1;
        }
        if let Some(name) = &c.group {
            p.group = Some(match next.groups.iter().find(|g| g.name == *name) {
                Some(g) => g.id,
                None => next.add_group(name.clone())?,
            });
        }
        p.remember_password = passwords && c.password.is_some();
        let id = next.add_profile(p)?;
        if passwords && let Some(password) = &c.password {
            credentials.push((id, Zeroizing::new(password.to_string())));
        }
        count += 1;
    }
    *store = next;
    Ok((credentials, count))
}

/// File IO and password derivation stay off the rendering thread.
pub struct Dialog {
    tablepro: bool,
    pending: Option<std::sync::mpsc::Receiver<Result<Option<Loaded>, String>>>,
    locked: Option<Zeroizing<Vec<u8>>>,
    password: Zeroizing<String>,
    candidates: Vec<Candidate>,
    duplicates: Duplicates,
    passwords: bool,
    error: Option<String>,
}
pub enum Decision {
    Cancel,
    Import,
}
impl Dialog {
    pub fn pick(ctx: egui::Context) -> Self { Self::pick_source(ctx, false) }
    pub fn pick_source(ctx: egui::Context, tablepro: bool) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .set_title(if tablepro { "Import TablePro connections" } else { "Import TablePlus connections" })
                .add_filter("Connection export", if tablepro { &["tablepro", "json"] } else { &["tableplusconnection", "json"] })
                .pick_file()
                .map(|path| if tablepro { crate::tablepro_import::read(&path) } else { read(&path) })
                .transpose();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Self {
            tablepro,
            pending: Some(rx),
            locked: None,
            password: Zeroizing::new(String::new()),
            candidates: vec![],
            duplicates: Duplicates::Skip,
            passwords: false,
            error: None,
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, store: &ConnectionProfiles) -> Option<Decision> {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(Some(Loaded::Locked(bytes))) => self.locked = Some(bytes),
                        Ok(Some(Loaded::Preview(rows))) => {
                            self.locked = None;
                            self.candidates = rows;
                            self.error = None;
                        }
                        Ok(None) => return Some(Decision::Cancel),
                        Err(e) => self.error = Some(e),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error =
                        Some("Import worker stopped. Please choose the file again.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let mut decision = None;
        egui::Modal::new(egui::Id::new("tableplus-import")).show(ctx, |ui| {
            ui.set_width(600.0_f32.min(ctx.content_rect().width() - 48.0));
            ui.heading(if self.tablepro { "Import from TablePro" } else { "Import from TablePlus" });
            if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if self.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading export…");
                });
            } else if self.locked.is_some() {
                ui.label("Enter the export password. Decryption happens locally.");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut *self.password)
                        .password(true)
                        .hint_text("Export password"),
                );
                if ui.button("Unlock").clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    let bytes = self.locked.as_ref().unwrap().clone();
                    let password =
                        std::mem::replace(&mut self.password, Zeroizing::new(String::new()));
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.pending = Some(rx);
                    let ctx = ctx.clone();
                    let tablepro = self.tablepro;
                    std::thread::spawn(move || {
                        let _ = tx.send(
                            (if tablepro { crate::tablepro_import::unlock(&bytes, &password) } else { unlock(&bytes, &password) }).map(|rows| Some(Loaded::Preview(rows))),
                        );
                        ctx.request_repaint();
                    });
                }
            } else if !self.candidates.is_empty() {
                ui.label("Select connections to add. Existing connections are never overwritten.");
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() * 0.55).min(420.0))
                    .show(ui, |ui| {
                        for (i, candidate) in self.candidates.iter_mut().enumerate() {
                            ui.push_id(i, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_enabled(
                                        candidate.profile.is_some(),
                                        egui::Checkbox::new(
                                            &mut candidate.selected,
                                            &candidate.name,
                                        ),
                                    );
                                    ui.weak(&candidate.driver);
                                    if is_duplicate(store, candidate) {
                                        ui.weak("Already saved");
                                    }
                                });
                                if let Some(error) = &candidate.error {
                                    ui.colored_label(ui.visuals().error_fg_color, error);
                                }
                                ui.collapsing("Settings and notices", |ui| {
                                    ui.label(format!(
                                        "Group: {}",
                                        candidate.group.as_deref().unwrap_or("Ungrouped")
                                    ));
                                    if let Some(p) = &candidate.profile {
                                        ui.label(format!("Environment: {}", p.environment));
                                        // Profiles contain configuration only, never credentials.
                                        match &p.connection {
                                            ConnectionKind::SQLite { path } => {
                                                ui.label(format!("File: {}", path.display()));
                                            }
                                            ConnectionKind::Postgres(c) => {
                                                ui.label(format!(
                                                    "{}:{} · {} · User: {} · TLS: {} · SSH: {}",
                                                    c.host,
                                                    c.port,
                                                    c.database,
                                                    c.user,
                                                    c.tls,
                                                    c.ssh.is_some()
                                                ));
                                            }
                                            ConnectionKind::Mysql(c)
                                            | ConnectionKind::MariaDb(c) => {
                                                ui.label(format!(
                                                    "{}:{} · {} · User: {} · TLS: {} · SSH: {}",
                                                    c.host,
                                                    c.port,
                                                    c.database,
                                                    c.user,
                                                    c.tls,
                                                    c.ssh.is_some()
                                                ));
                                            }
                                            ConnectionKind::Redis(c) => {
                                                ui.label(format!(
                                                    "{}:{} · DB {} · User: {} · TLS: {}",
                                                    c.host, c.port, c.database, c.username, c.tls
                                                ));
                                            }
                                        }
                                    }
                                    for warning in &candidate.warnings {
                                        ui.label(warning);
                                    }
                                });
                                ui.separator();
                            });
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Duplicates:");
                    ui.selectable_value(&mut self.duplicates, Duplicates::Skip, "Skip");
                    ui.selectable_value(&mut self.duplicates, Duplicates::KeepBoth, "Keep both");
                });
                ui.checkbox(
                    &mut self.passwords,
                    "Save included database passwords in the OS credential store",
                );
                let count = self
                    .candidates
                    .iter()
                    .filter(|c| c.selected && c.profile.is_some())
                    .count();
                if ui
                    .add_enabled(
                        count > 0,
                        egui::Button::new(format!("Import {count} selected")),
                    )
                    .clicked()
                {
                    decision = Some(Decision::Import);
                }
            }
            if ui.button("Cancel").clicked() {
                decision = Some(Decision::Cancel);
            }
        });
        decision
    }
    pub fn apply(
        &mut self,
        store: &mut ConnectionProfiles,
    ) -> Result<ImportResult, String> {
        apply(store, &mut self.candidates, self.duplicates, self.passwords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn official_rncryptor_vectors_and_tampering() {
        // https://github.com/RNCryptor/RNCryptor-Spec/blob/master/vectors/v3/password
        let mut fields = HashMap::new();
        let mut count = 0;
        for line in include_str!("fixtures/rncryptor-v3-password.txt").lines() {
            if line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                fields.insert(key.trim(), value.trim());
                if key == "ciphertext_hex" {
                    let mut bytes = hex(value);
                    assert_eq!(
                        &*decrypt(&bytes, fields["password"]).unwrap(),
                        &hex(fields["plaintext_hex"])
                    );
                    assert!(decrypt(&bytes, "wrong password").is_err());
                    bytes[34] ^= 1;
                    assert!(decrypt(&bytes, fields["password"]).is_err());
                    count += 1;
                }
            }
        }
        assert!(count >= 6);
    }
    fn sample() -> Vec<Candidate> {
        parse(br##"{"ConnectionGroups":[{"ID":"g","Name":"Development"}],"Connections":[{"ConnectionName":"Local","Driver":"MySQL","DatabaseHost":"localhost","DatabasePort":"3306","DatabaseUser":"root","DatabaseName":"test","tLSMode":0,"GroupID":"g","statusColor":"#123456","DatabasePasswordMode":0,"DatabasePassword":"test-secret","StartupBashScript":"echo never"}]}"##).unwrap()
    }
    #[test]
    fn imports_groups_colors_and_credentials_separately_and_skips_duplicates() {
        let mut rows = sample();
        let mut store = ConnectionProfiles::default();
        let (credentials, count) = apply(&mut store, &mut rows, Duplicates::Skip, true).unwrap();
        assert_eq!(count, 1);
        assert_eq!(&**credentials[0].1, "test-secret");
        assert_eq!(store.profiles[0].color, [0x12, 0x34, 0x56]);
        assert_eq!(store.groups[0].name, "Development");
        let serialized = serde_json::to_string(&store).unwrap();
        assert!(!serialized.contains("test-secret"));
        assert!(!serialized.contains("echo never"));
        assert_eq!(
            apply(&mut store, &mut rows, Duplicates::Skip, false)
                .unwrap()
                .1,
            0
        );
        assert_eq!(
            apply(&mut store, &mut rows, Duplicates::KeepBoth, false)
                .unwrap()
                .1,
            1
        );
        assert_eq!(store.profiles[1].name, "Local (2)");
        assert!(!store.profiles[1].remember_password);
        assert_ne!(store.profiles[0].id, store.profiles[1].id);
    }
    #[test]
    fn invalid_records_never_import_and_json_errors_do_not_echo_secrets() {
        let mut rows = parse(br#"[{"Driver":"Redis","ConnectionName":"Tunnel","isOverSSH":1},{"Driver":"MongoDB"},{"Driver":"MySQL","DatabasePort":"invalid"}]"#).unwrap();
        assert!(rows.iter().all(|c| !c.selected && c.error.is_some()));
        let mut store = ConnectionProfiles::default();
        assert_eq!(
            apply(&mut store, &mut rows, Duplicates::Skip, false)
                .unwrap()
                .1,
            0
        );
        assert!(!parse(b"secret").err().unwrap().contains("secret"));
        assert!(unlock(&[3, 1], "password").is_err());
    }
    #[test]
    fn failed_validation_leaves_store_unchanged() {
        let mut rows = sample();
        rows[0].profile.as_mut().unwrap().name.clear();
        let mut store = ConnectionProfiles::default();
        let before = serde_json::to_string(&store).unwrap();
        assert!(apply(&mut store, &mut rows, Duplicates::KeepBoth, true).is_err());
        assert_eq!(serde_json::to_string(&store).unwrap(), before);
    }
    #[test]
    fn preview_and_password_prompt_render_without_mutation() {
        let ctx = egui::Context::default();
        let store = ConnectionProfiles::default();
        let mut dialog = Dialog {
            tablepro: false,
            pending: None,
            locked: None,
            password: Zeroizing::new(String::new()),
            candidates: sample(),
            duplicates: Duplicates::Skip,
            passwords: false,
            error: None,
        };
        let output = ctx.run(Default::default(), |ctx| {
            assert!(dialog.show(ctx, &store).is_none());
        });
        assert!(!output.shapes.is_empty());
        dialog.locked = Some(Zeroizing::new(vec![3, 1]));
        let output = ctx.run(Default::default(), |ctx| {
            assert!(dialog.show(ctx, &store).is_none());
        });
        assert!(!output.shapes.is_empty());
        assert!(store.profiles.is_empty());
    }
    #[test]
    fn nested_export_groups_preserve_all_connections() {
        let value = serde_json::json!([
            {"Name":"Development","connections":[{"Driver":"Redis","ConnectionName":"Cache","DatabaseHost":"localhost","tLSMode":0}],"groups":[
                {"Name":"SQL","connections":[{"Driver":"MySQL","ConnectionName":"Database","DatabaseHost":"localhost","DatabaseUser":"root","tLSMode":0}],"groups":[]}
            ]},
            {"Name":"Empty","connections":[],"groups":[]},
            {"Driver":"SQLite","ConnectionName":"File","DatabasePath":"/tmp/example.sqlite"}
        ]);
        let rows = parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.selected));
        assert_eq!(rows[0].group.as_deref(), Some("Development"));
        assert_eq!(rows[1].group.as_deref(), Some("Development / SQL"));
        assert_eq!(rows[2].group, None);
    }

    #[test]
    #[ignore = "Explicit local export verification; requires path and export password environment variables"]
    fn local_export_compatibility() {
        let path = std::env::var("RUSTABLE_TEST_EXPORT").unwrap();
        let password = Zeroizing::new(std::env::var("RUSTABLE_TEST_EXPORT_PASSWORD").unwrap());
        let loaded = read(Path::new(&path)).unwrap();
        let rows = match loaded {
            Loaded::Locked(bytes) => unlock(&bytes, &password).unwrap(),
            Loaded::Preview(rows) => rows,
        };
        let accepted = rows.iter().filter(|r| r.profile.is_some()).count();
        let grouped = rows.iter().filter(|r| r.group.is_some()).count();
        eprintln!(
            "Export check: {} connections, {accepted} importable, {grouped} grouped",
            rows.len()
        );
        for row in &rows {
            if let Some(error) = &row.error {
                eprintln!("Unsupported record: {error}");
            }
        }
        assert!(!rows.is_empty());
        // Preview only: never apply or connect to user-supplied data.
    }

    #[test]
    fn supported_engines_and_password_providers() {
        for driver in ["PostgreSQL", "MySQL", "MariaDB", "Redis", "SQLite"] {
            let json = serde_json::json!([{"Driver":driver,"ConnectionName":driver,"DatabaseHost":"localhost","DatabaseUser":"user","DatabasePath":"/tmp/example.sqlite","DatabasePasswordMode":1,"DatabasePassword":"command-not-password"}]);
            let rows = parse(&serde_json::to_vec(&json).unwrap()).unwrap();
            assert!(rows[0].profile.is_some(), "{}: {:?}", driver, rows[0].error);
            assert!(rows[0].password.is_none());
        }
    }
}
