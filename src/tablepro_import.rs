//! TablePro v1/v2 connection bundles. Format references are recorded in docs/TABLEPRO_IMPORT.md.
use crate::tableplus_import::{Candidate, Loaded, MAX_FILE};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::Path,
};
use zeroize::Zeroizing;

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
    if bytes.starts_with(b"TPRO") {
        envelope(&bytes)?;
        Ok(Loaded::Locked(bytes))
    } else {
        parse(&bytes).map(Loaded::Preview)
    }
}
fn envelope(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() <= 65 || bytes.len() > MAX_FILE || !bytes.starts_with(b"TPRO\x01") {
        return Err(
            "Invalid or unsupported TablePro encrypted export (expected version 1).".into(),
        );
    }
    Ok(())
}
pub fn unlock(bytes: &[u8], password: &str) -> Result<Vec<Candidate>, String> {
    envelope(bytes)?;
    if password.len() > 4096 {
        return Err("Export password is too long.".into());
    }
    let mut key = Zeroizing::new([0u8; 32]);
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(600_000).unwrap(),
        &bytes[5..37],
        password.as_bytes(),
        key.as_mut(),
    );
    let key = ring::aead::LessSafeKey::new(
        ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, key.as_ref())
            .map_err(|_| "Invalid encryption key.")?,
    );
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(&bytes[37..49])
        .map_err(|_| "Invalid encryption nonce.")?;
    let mut clear = Zeroizing::new(bytes[49..].to_vec());
    let data = key
        .open_in_place(nonce, ring::aead::Aad::empty(), &mut clear)
        .map_err(|_| "Incorrect export password or damaged file.")?;
    parse(data)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Candidate>, String> {
    if bytes.len() > MAX_FILE {
        return Err("Connection export exceeds the 8 MiB limit.".into());
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid TablePro JSON export.")?;
    let version = root["formatVersion"]
        .as_u64()
        .filter(|v| matches!(v, 1 | 2))
        .ok_or("Unsupported TablePro format version.")?;
    let connections = root["connections"]
        .as_array()
        .ok_or("Missing TablePro connections.")?;
    if connections.is_empty() || connections.len() > 2000 {
        return Err("Expected between 1 and 2000 connections.".into());
    }
    let mut groups = HashMap::new();
    if version == 2 {
        for group in root["groups"].as_array().into_iter().flatten() {
            let id = text(group, "ref");
            if id.is_empty() || groups.insert(id, group).is_some() {
                return Err("Invalid or duplicate group reference.".into());
            }
        }
    }
    let mut refs = HashSet::new();
    let mut result = Vec::new();
    for (index, entry) in connections.iter().enumerate() {
        let reference = if version == 1 {
            index.to_string()
        } else {
            text(entry, "ref").to_owned()
        };
        if reference.is_empty() || !refs.insert(reference.clone()) {
            return Err("Invalid or duplicate connection reference.".into());
        }
        let group = if version == 1 {
            text(entry, "groupName").to_owned()
        } else {
            let mut path = Vec::new();
            let mut visited = HashSet::new();
            let mut id = text(entry, "groupRef");
            while !id.is_empty() {
                if !visited.insert(id) || visited.len() > 32 {
                    return Err("Cyclic or excessive group nesting.".into());
                }
                let group = groups.get(id).ok_or("Unknown group reference.")?;
                path.push(text(group, "name"));
                id = text(group, "parentRef");
            }
            path.reverse();
            path.join(" / ")
        };
        let ssh = &entry["sshConfig"];
        let ssl = &entry["sslConfig"];
        let mode: String = text(ssl, "mode")
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let tls = match mode.as_str() {
            "" | "disabled" | "disable" => "0",
            "preferred" | "prefer" | "required" | "require" | "verifyca" | "verifyidentity"
            | "verifyfull" => "1",
            _ => return Err("Unknown TLS mode in TablePro export.".into()),
        };
        let kind = text(entry, "type");
        let database = if kind.eq_ignore_ascii_case("redis") {
            entry.get("redisDatabase").cloned().unwrap_or(json!(0))
        } else {
            entry["database"].clone()
        };
        let mapped = json!({
            "Driver": kind, "ConnectionName": text(entry,"name"),
            "DatabaseHost": text(entry,"host"), "DatabasePort": entry["port"],
            "DatabaseName": database, "DatabaseUser": text(entry,"username"),
            "DatabasePath": text(entry,"database"), "tLSMode": tls,
            "isOverSSH": ssh["enabled"].as_bool().unwrap_or(false),
            "ServerAddress": text(ssh,"host"), "ServerPort": ssh["port"], "ServerUser": text(ssh,"username")
        });
        let mut warnings = Vec::new();
        let connection = crate::tableplus_import::convert(&mapped, &mut warnings);
        for warning in &mut warnings {
            *warning = warning.replace("TablePlus", "TablePro");
        }
        warnings.push("Tags, colors, saved queries, startup commands, custom timeouts and credential profiles are not imported. Review settings before connecting.".into());
        let mut candidate = Candidate {
            name: text(entry, "name").to_owned(),
            driver: kind.to_owned(),
            group: (!group.is_empty()).then_some(group),
            profile: None,
            warnings,
            error: None,
            selected: false,
            password: None,
        };
        let unsupported = (!kind.eq_ignore_ascii_case("sqlite")
            && entry["port"].as_u64().is_none_or(|p| p == 0 || p > 65535))
            || ["host", "database", "username"]
                .iter()
                .any(|key| text(entry, key).contains('$'))
            || text(entry, "host").starts_with('/')
            || (ssh["enabled"].as_bool() == Some(true)
                && ssh.get("port").is_some_and(|p| {
                    !p.is_null() && p.as_u64().is_none_or(|p| p == 0 || p > 65535)
                }))
            || entry.get("tunnelCommand").is_some_and(|v| !v.is_null())
            || !text(entry, "sshProfileId").is_empty()
            || !text(entry, "credentialProfileRef").is_empty()
            || !text(entry, "credentialProfileName").is_empty()
            || entry["additionalFields"]
                .as_object()
                .is_some_and(|v| !v.is_empty())
            || ssh["jumpHosts"].as_array().is_some_and(|v| !v.is_empty())
            || !text(ssh, "remoteFilePath").is_empty()
            || !text(ssl, "caCertificatePath").is_empty()
            || !text(ssl, "clientCertificatePath").is_empty()
            || !text(ssl, "clientKeyPath").is_empty();
        if unsupported {
            candidate.error = Some("This connection uses advanced transport, authentication, certificate or driver settings that require manual setup; no simplified connection will be imported.".into());
        } else {
            match connection {
                Ok(connection) => {
                    let profile = crate::connection_profiles::Profile::draft(
                        candidate.name.clone(),
                        connection,
                    );
                    match profile.validate() {
                        Ok(()) => {
                            candidate.profile = Some(profile);
                            candidate.selected = true;
                        }
                        Err(error) => candidate.error = Some(error),
                    }
                }
                Err(error) => candidate.error = Some(error),
            }
        }
        if let Some(password) = root["credentials"][&reference]["password"].as_str() {
            candidate.password = Some(Zeroizing::new(password.to_owned()));
        }
        result.push(candidate);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle() -> Value {
        json!({"formatVersion":1,"connections":[{"name":"Sample","type":"PostgreSQL","host":"localhost","port":5432,"database":"sample","username":"demo","groupName":"Local"}]})
    }
    #[test]
    fn supported_engines_groups_credentials_and_duplicate_refs() {
        for (kind, port) in [
            ("PostgreSQL", 5432),
            ("MySQL", 3306),
            ("MariaDB", 3306),
            ("Redis", 6379),
            ("SQLite", 1),
        ] {
            let mut data = bundle();
            data["connections"][0]["type"] = json!(kind);
            data["connections"][0]["port"] = json!(port);
            data["connections"][0]["database"] = json!("/tmp/example.sqlite");
            data["credentials"] = json!({"0":{"password":"secret"}});
            let rows = parse(&serde_json::to_vec(&data).unwrap()).unwrap();
            assert!(rows[0].selected, "{kind}: {:?}", rows[0].error);
            assert_eq!(rows[0].group.as_deref(), Some("Local"));
            assert_eq!(
                rows[0].password.as_deref().map(String::as_str),
                Some("secret")
            );
        }
        let mut data = bundle();
        data["formatVersion"] = json!(2);
        data["connections"][0]["ref"] = json!("c");
        data["connections"][0]["groupRef"] = json!("child");
        data["groups"] =
            json!([{"ref":"root","name":"Team"},{"ref":"child","name":"Dev","parentRef":"root"}]);
        assert_eq!(
            parse(&serde_json::to_vec(&data).unwrap()).unwrap()[0]
                .group
                .as_deref(),
            Some("Team / Dev")
        );
        data["groups"][0]["parentRef"] = json!("child");
        assert!(parse(&serde_json::to_vec(&data).unwrap()).is_err());
    }
    #[test]
    fn rejects_unsupported_versions_and_does_not_downgrade_transports() {
        let mut data = bundle();
        data["formatVersion"] = json!(3);
        assert!(parse(&serde_json::to_vec(&data).unwrap()).is_err());
        for field in ["tunnelCommand", "additionalFields"] {
            let mut data = bundle();
            data["connections"][0][field] = json!({"custom":"value"});
            let rows = parse(&serde_json::to_vec(&data).unwrap()).unwrap();
            assert!(!rows[0].selected);
            assert!(rows[0].profile.is_none());
        }
        let mut data = bundle();
        data["connections"][0]["type"] = json!("MongoDB");
        assert!(
            parse(&serde_json::to_vec(&data).unwrap()).unwrap()[0]
                .profile
                .is_none()
        );
    }
    #[test]
    fn apple_cryptokit_export_is_compatible() {
        let rows = unlock(
            include_bytes!("fixtures/tablepro-v1.tablepro"),
            "fixture-only",
        )
        .unwrap();
        assert_eq!(rows[0].name, "Swift fixture");
        assert!(rows[0].selected);
    }
    #[test]
    fn encrypted_export_authentication_and_limits() {
        let mut bytes = b"TPRO\x01".to_vec();
        bytes.extend_from_slice(&[7; 32]);
        bytes.extend_from_slice(&[8; 12]);
        let mut key = [0u8; 32];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(600_000).unwrap(),
            &[7; 32],
            b"sample",
            &mut key,
        );
        let key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key).unwrap(),
        );
        let mut data = serde_json::to_vec(&bundle()).unwrap();
        key.seal_in_place_append_tag(
            ring::aead::Nonce::assume_unique_for_key([8; 12]),
            ring::aead::Aad::empty(),
            &mut data,
        )
        .unwrap();
        bytes.extend(data);
        assert!(unlock(&bytes, "sample").unwrap()[0].selected);
        assert!(unlock(&bytes, "wrong").is_err());
        *bytes.last_mut().unwrap() ^= 1;
        assert!(unlock(&bytes, "sample").is_err());
        assert!(unlock(&bytes[..40], "sample").is_err());
        assert!(parse(&vec![b' '; MAX_FILE + 1]).is_err());
    }
}
