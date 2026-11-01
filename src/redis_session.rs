//! Native Redis transport and bounded browser operations. Owned by one worker.
use crate::redis_settings::Settings;
use redis::{ConnectionLike, IntoConnectionInfo};

pub(crate) struct Session {
    connection: Option<redis::Connection>,
}
#[derive(Debug)]
pub(crate) struct ConnectFailure {
    pub passed: Vec<crate::connection_probe::ProbeField>,
    pub failed: Vec<crate::connection_probe::ProbeField>,
    pub message: String,
}
#[derive(Clone, Debug)]
pub(crate) struct KeyPage {
    pub cursor: u64,
    pub keys: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub key: Vec<u8>,
    pub kind: String,
    pub ttl_ms: i64,
    pub length: Option<u64>,
    pub value: redis::Value,
    pub truncated: bool,
}
impl Session {
    pub fn disconnected() -> Self {
        Self { connection: None }
    }
    pub fn connect(settings: &Settings, password: &str) -> Result<Self, String> {
        Self::connect_diagnostic(settings, password).map_err(|error| error.message)
    }
    pub fn is_connected(&self) -> bool {
        self.connection
            .as_ref()
            .is_some_and(ConnectionLike::is_open)
    }
    pub fn connect_diagnostic(settings: &Settings, password: &str) -> Result<Self, ConnectFailure> {
        use crate::connection_probe::ProbeField::*;
        let failure = |passed: Vec<_>, failed: Vec<_>, message: String| ConnectFailure {
            passed,
            failed,
            message,
        };
        settings
            .validate()
            .map_err(|message| failure(vec![], vec![Host, Port], message))?;
        let address = if settings.tls {
            redis::ConnectionAddr::TcpTls {
                host: settings.host.clone(),
                port: settings.port,
                insecure: false,
                tls_params: None,
            }
        } else {
            redis::ConnectionAddr::Tcp(settings.host.clone(), settings.port)
        };
        // Authenticate first, then SELECT explicitly so an invalid database is
        // reported independently from a successful transport and authentication.
        let mut auth = redis::RedisConnectionInfo::default();
        if !settings.username.is_empty() {
            auth = auth.set_username(&settings.username);
        }
        if !password.is_empty() || !settings.username.is_empty() {
            auth = auth.set_password(password);
        }
        let info = address
            .into_connection_info()
            .map_err(|e| failure(vec![], vec![Host, Port], e.to_string()))?
            .set_redis_settings(auth);
        let client = if let Some(certificate) =
            settings.root_certificate.as_ref().filter(|_| settings.tls)
        {
            let pem = std::fs::read(certificate).map_err(|e| {
                failure(
                    vec![],
                    vec![Tls],
                    format!("Cannot read Redis CA certificate: {e}"),
                )
            })?;
            redis::Client::build_with_tls(
                info,
                redis::TlsCertificates {
                    client_tls: None,
                    root_cert: Some(pem),
                },
            )
        } else {
            redis::Client::open(info)
        }
        .map_err(|e| failure(vec![], vec![Tls], format!("Redis configuration: {e}")))?;
        let mut transport = vec![Host, Port];
        if settings.tls {
            transport.push(Tls);
        }
        let connection = client
            .get_connection_with_timeout(settings.connect_timeout)
            .map_err(|e| {
                if e.kind() == redis::ErrorKind::AuthenticationFailed
                    || matches!(e.code(), Some("WRONGPASS" | "NOAUTH"))
                {
                    failure(
                        transport.clone(),
                        vec![User, Password],
                        format!("Redis authentication: {e}"),
                    )
                } else {
                    failure(vec![], transport.clone(), format!("Redis connection: {e}"))
                }
            })?;
        connection
            .set_read_timeout(Some(settings.read_timeout))
            .map_err(|e| failure(vec![], transport.clone(), e.to_string()))?;
        connection
            .set_write_timeout(Some(settings.read_timeout))
            .map_err(|e| failure(vec![], transport.clone(), e.to_string()))?;
        let mut session = Self {
            connection: Some(connection),
        };
        // PING detects missing credentials even when a server does not require an
        // explicit AUTH exchange. Redis ACLs must permit PING for this workspace.
        session
            .query(&redis::cmd("PING"))
            .map_err(|message| failure(transport.clone(), vec![User, Password], message))?;
        transport.extend([User, Password]);
        session
            .query(redis::cmd("SELECT").arg(settings.database))
            .map_err(|message| failure(transport.clone(), vec![Database], message))?;
        Ok(session)
    }
    fn query(&mut self, command: &redis::Cmd) -> Result<redis::Value, String> {
        let conn = self
            .connection
            .as_mut()
            .ok_or("Redis connection closed. Reconnect before continuing.")?;
        match command.query(conn) {
            Ok(value) => Ok(value),
            Err(error) => {
                if error.is_io_error() || !conn.is_open() {
                    self.connection = None;
                    Err(format!(
                        "Redis connection lost: {error}. A submitted write may have completed; verify before retrying."
                    ))
                } else {
                    Err(match error.code() {
                        Some(code) => format!("{code}: {error}"),
                        None => error.to_string(),
                    })
                }
            }
        }
    }
    fn typed<T: redis::FromRedisValue>(&mut self, command: &redis::Cmd) -> Result<T, String> {
        redis::from_redis_value(self.query(command)?).map_err(|e| e.to_string())
    }
    pub fn scan(&mut self, cursor: u64, pattern: &[u8], count: usize) -> Result<KeyPage, String> {
        let mut command = redis::cmd("SCAN");
        command
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(count.clamp(1, 1000));
        let (cursor, keys) = self.typed(&command)?;
        Ok(KeyPage { cursor, keys })
    }
    pub fn inspect(&mut self, key: &[u8]) -> Result<Snapshot, String> {
        let kind: String = self.typed(redis::cmd("TYPE").arg(key))?;
        let ttl_ms: i64 = self.typed(redis::cmd("PTTL").arg(key))?;
        let (length, mut value, mut truncated) = match kind.as_str() {
            "none" => (Some(0), redis::Value::Nil, false),
            "string" => {
                let length: u64 = self.typed(redis::cmd("STRLEN").arg(key))?;
                let value = self.query(redis::cmd("GETRANGE").arg(key).arg(0).arg(65535))?;
                (Some(length), value, length > 65536)
            }
            "list" => {
                let length: u64 = self.typed(redis::cmd("LLEN").arg(key))?;
                (
                    Some(length),
                    self.query(redis::cmd("LRANGE").arg(key).arg(0).arg(199))?,
                    length > 200,
                )
            }
            "zset" => {
                let length: u64 = self.typed(redis::cmd("ZCARD").arg(key))?;
                (
                    Some(length),
                    self.query(
                        redis::cmd("ZRANGE")
                            .arg(key)
                            .arg(0)
                            .arg(199)
                            .arg("WITHSCORES"),
                    )?,
                    length > 200,
                )
            }
            "hash" | "set" => {
                let length: u64 =
                    self.typed(redis::cmd(if kind == "hash" { "HLEN" } else { "SCARD" }).arg(key))?;
                let (cursor, values): (u64, Vec<redis::Value>) = self.typed(
                    redis::cmd(if kind == "hash" { "HSCAN" } else { "SSCAN" })
                        .arg(key)
                        .arg(0)
                        .arg("COUNT")
                        .arg(200),
                )?;
                (Some(length), redis::Value::Array(values), cursor != 0)
            }
            "stream" => {
                let length: u64 = self.typed(redis::cmd("XLEN").arg(key))?;
                (
                    Some(length),
                    self.query(
                        redis::cmd("XRANGE")
                            .arg(key)
                            .arg("-")
                            .arg("+")
                            .arg("COUNT")
                            .arg(200),
                    )?,
                    length > 200,
                )
            }
            _ => {
                return Err(format!(
                    "Key type {kind} is not supported by the value inspector."
                ));
            }
        };
        truncated |= bound_preview(&mut value, &mut (256 * 1024), &mut 1000);
        Ok(Snapshot {
            key: key.to_vec(),
            kind,
            ttl_ms,
            length,
            value,
            truncated,
        })
    }
    pub fn execute(&mut self, text: &str, read_only: bool) -> Result<Vec<redis::Value>, String> {
        let commands = crate::redis_commands::parse(text)?;
        // Validate the entire batch before its first write.
        for args in &commands {
            crate::redis_commands::validate(args, read_only)?;
        }
        let mut results = Vec::new();
        for args in commands {
            let name = std::str::from_utf8(&args[0]).map_err(|_| "Command name must be UTF-8.")?;
            let mut command = redis::cmd(name);
            for arg in &args[1..] {
                command.arg(arg);
            }
            match self.query(&command) {
                Ok(value) => results.push(value),
                Err(error) => {
                    return Err(format!(
                        "Command {} failed: {error}. {} preceding command(s) completed; Redis batches are not automatically rolled back.",
                        results.len() + 1,
                        results.len()
                    ));
                }
            }
        }
        Ok(results)
    }
    /// Compare-and-set avoids overwriting a concurrent update, and preserves TTL.
    pub fn replace_string(
        &mut self,
        key: &[u8],
        expected: &[u8],
        replacement: &[u8],
        read_only: bool,
    ) -> Result<(), String> {
        if read_only {
            return Err("Unlock editing before saving a value.".into());
        }
        if replacement.len() > 65536 {
            return Err("String editor limit is 64 KiB. Use commands for larger values.".into());
        }
        let script = "if redis.call('TYPE',KEYS[1]).ok ~= 'string' then return 0 end if redis.call('GET',KEYS[1]) ~= ARGV[1] then return 0 end redis.call('SET',KEYS[1],ARGV[2],'KEEPTTL') return 1";
        let updated: i64 = self.typed(
            redis::cmd("EVAL")
                .arg(script)
                .arg(1)
                .arg(key)
                .arg(expected)
                .arg(replacement),
        )?;
        if updated != 1 {
            return Err(
                "The key expired, changed type, or was modified. Refresh before editing again."
                    .into(),
            );
        }
        Ok(())
    }
    pub fn delete(&mut self, key: &[u8], read_only: bool) -> Result<bool, String> {
        if read_only {
            return Err("Unlock editing before deleting a key.".into());
        }
        let deleted: i64 = self.typed(redis::cmd("UNLINK").arg(key))?;
        Ok(deleted != 0)
    }
    pub fn expire(
        &mut self,
        key: &[u8],
        milliseconds: Option<u64>,
        read_only: bool,
    ) -> Result<bool, String> {
        if read_only {
            return Err("Unlock editing before changing expiry.".into());
        }
        let result: i64 = if let Some(ttl) = milliseconds {
            if ttl == 0 || ttl > i64::MAX as u64 {
                return Err("TTL must be a positive number of milliseconds.".into());
            }
            self.typed(redis::cmd("PEXPIRE").arg(key).arg(ttl))?
        } else {
            self.typed(redis::cmd("PERSIST").arg(key))?
        };
        Ok(result != 0)
    }
}

// Limit retained inspector data as well as rendered text. Redis replies must
// still be decoded first; COUNT and these limits are not network byte limits.
fn bound_preview(value: &mut redis::Value, bytes: &mut usize, nodes: &mut usize) -> bool {
    if *nodes == 0 {
        *value = redis::Value::Nil;
        return true;
    }
    *nodes -= 1;
    match value {
        redis::Value::BulkString(value) => {
            let keep = value.len().min(*bytes).min(65536);
            let truncated = keep != value.len();
            value.truncate(keep);
            *bytes -= keep;
            truncated
        }
        redis::Value::Array(values) | redis::Value::Set(values) => {
            let mut truncated = false;
            let mut keep = 0;
            for value in values.iter_mut() {
                if *nodes == 0 || *bytes == 0 {
                    break;
                }
                truncated |= bound_preview(value, bytes, nodes);
                keep += 1;
            }
            truncated |= keep != values.len();
            values.truncate(keep);
            truncated
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Settings {
        Settings {
            host: "127.0.0.1".into(),
            port: 16379,
            tls: false,
            ..Default::default()
        }
    }

    #[test]
    #[ignore = "requires disposable Redis on localhost:16379"]
    fn live_pagination_binary_empty_large_expiry_and_timeout() {
        let mut session = Session::connect(&settings(), "rustable-redis-test-only").unwrap();
        let prefix = format!("rustable-page-test:{}:", std::process::id());
        for index in 0..250 {
            session
                .query(redis::cmd("SET").arg(format!("{prefix}{index}")).arg(""))
                .unwrap();
        }
        let mut cursor = 0;
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        loop {
            let page = session
                .scan(cursor, format!("{prefix}*").as_bytes(), 10)
                .unwrap();
            pages += 1;
            seen.extend(page.keys);
            cursor = page.cursor;
            if cursor == 0 {
                break;
            }
            assert!(pages < 10000);
        }
        assert_eq!(seen.len(), 250);
        assert!(pages > 1);
        let key = format!("{prefix}0");
        assert_eq!(
            session.inspect(key.as_bytes()).unwrap().value,
            redis::Value::BulkString(vec![])
        );
        session
            .replace_string(key.as_bytes(), b"", &[0, 255, 128], false)
            .unwrap();
        assert_eq!(
            session.inspect(key.as_bytes()).unwrap().value,
            redis::Value::BulkString(vec![0, 255, 128])
        );
        session
            .query(redis::cmd("SET").arg(&key).arg(vec![255u8; 100000]))
            .unwrap();
        let snapshot = session.inspect(key.as_bytes()).unwrap();
        assert!(snapshot.truncated);
        assert_eq!(snapshot.length, Some(100000));
        assert!(matches!(snapshot.value, redis::Value::BulkString(bytes) if bytes.len() == 65536));
        session.expire(key.as_bytes(), Some(1), false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(session.inspect(key.as_bytes()).unwrap().kind, "none");
        // A rejected later command must not allow the earlier write to execute.
        assert!(
            session
                .execute(&format!("SET {key} unintended\nSELECT 1"), false)
                .is_err()
        );
        assert_eq!(session.inspect(key.as_bytes()).unwrap().kind, "none");
        for key in seen {
            session.delete(&key, false).unwrap();
        }
        let mut timed = Session::connect(
            &Settings {
                read_timeout: std::time::Duration::from_millis(50),
                ..settings()
            },
            "rustable-redis-test-only",
        )
        .unwrap();
        // Direct private transport call exercises a stalled reply; UI forbids BLPOP.
        assert!(
            timed
                .query(redis::cmd("BLPOP").arg(&key).arg(0))
                .unwrap_err()
                .contains("may have completed")
        );
        assert!(!timed.is_connected());
        assert!(
            timed
                .execute("PING", true)
                .unwrap_err()
                .contains("Reconnect")
        );
    }

    #[test]
    #[ignore = "requires disposable Redis on localhost:16379"]
    fn live_acl_username_and_passwordless_identity() {
        let mut admin = Session::connect(&settings(), "rustable-redis-test-only").unwrap();
        let username = format!("rustable-test-{}", std::process::id());
        admin
            .query(
                redis::cmd("ACL")
                    .arg("SETUSER")
                    .arg(&username)
                    .arg("reset")
                    .arg("on")
                    .arg(">test-password")
                    .arg("+ping")
                    .arg("+select")
                    .arg("+get")
                    .arg("~rustable-test:*"),
            )
            .unwrap();
        let config = Settings {
            username: username.clone(),
            ..settings()
        };
        let mut session = Session::connect(&config, "test-password").unwrap();
        assert!(
            session
                .execute("SET rustable-test:key forbidden", false)
                .unwrap_err()
                .contains("NOPERM")
        );
        assert!(Session::connect(&config, "wrong").is_err());
        admin
            .query(
                redis::cmd("ACL")
                    .arg("SETUSER")
                    .arg(&username)
                    .arg("nopass"),
            )
            .unwrap();
        let mut passwordless = Session::connect(&config, "").unwrap();
        assert!(
            passwordless
                .execute("SET rustable-test:key forbidden", false)
                .unwrap_err()
                .contains("NOPERM")
        );
        admin
            .query(redis::cmd("ACL").arg("DELUSER").arg(username))
            .unwrap();
    }

    #[test]
    fn preview_limits_retained_nested_values() {
        let mut value = redis::Value::Array(vec![redis::Value::BulkString(vec![0; 100000]); 20]);
        assert!(bound_preview(&mut value, &mut (256 * 1024), &mut 1000));
        let redis::Value::Array(values) = value else {
            panic!()
        };
        assert_eq!(values.len(), 4);
        assert!(
            values.iter().all(
                |value| matches!(value, redis::Value::BulkString(bytes) if bytes.len() == 65536)
            )
        );
    }
    #[test]
    #[ignore = "requires disposable Redis TLS fixture on localhost:16380"]
    fn live_verified_tls_rejects_untrusted_and_wrong_hostname() {
        let settings = Settings {
            port: 16380,
            tls: true,
            root_certificate: Some(
                std::env::var_os("RUSTABLE_REDIS_TEST_CA")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| "/tmp/rustable-redis-tls/ca.crt".into()),
            ),
            ..settings()
        };
        Session::connect(&settings, "rustable-redis-test-only").unwrap();
        assert!(
            Session::connect(
                &Settings {
                    root_certificate: None,
                    ..settings.clone()
                },
                "rustable-redis-test-only"
            )
            .is_err()
        );
        assert!(
            Session::connect(
                &Settings {
                    host: "localhost".into(),
                    ..settings
                },
                "rustable-redis-test-only"
            )
            .is_err()
        );
    }
    #[test]
    fn stalled_handshake_obeys_connect_timeout() {
        use std::{
            net::TcpListener,
            time::{Duration, Instant},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let started = Instant::now();
        let result = Session::connect(
            &Settings {
                port,
                connect_timeout: Duration::from_millis(75),
                ..settings()
            },
            "test-only",
        );
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        peer.join().unwrap();
    }

    #[test]
    #[ignore = "requires disposable Redis on localhost:16379"]
    fn live_binary_types_expiry_guards_and_conflicts() {
        let mut session = Session::connect(&settings(), "rustable-redis-test-only").unwrap();
        let prefix = format!("rustable-test:{}:", std::process::id());
        let key = [prefix.as_bytes(), b"\xff\x00"].concat();
        let quoted = crate::redis_commands::quote(&key);
        session
            .execute(&format!("SET {quoted} 'old' PX 60000"), false)
            .unwrap();
        assert!(
            session
                .replace_string(&key, b"old", b"changed", true)
                .is_err()
        );
        session
            .replace_string(&key, b"old", b"changed", false)
            .unwrap();
        let snapshot = session.inspect(&key).unwrap();
        assert_eq!(snapshot.kind, "string");
        assert_eq!(snapshot.key, key);
        assert_eq!(snapshot.length, Some(7));
        assert!(!snapshot.truncated);
        assert!(snapshot.ttl_ms > 0 && snapshot.ttl_ms <= 60000);
        assert_eq!(
            snapshot.value,
            redis::Value::BulkString(b"changed".to_vec())
        );
        assert!(
            session
                .replace_string(&key, b"old", b"lost update", false)
                .is_err()
        );
        assert!(session.expire(&key, None, false).unwrap());
        assert_eq!(session.inspect(&key).unwrap().ttl_ms, -1);
        let mut cursor = 0;
        let mut found = false;
        loop {
            let page = session
                .scan(cursor, format!("{prefix}*").as_bytes(), 100)
                .unwrap();
            found |= page.keys.contains(&key);
            cursor = page.cursor;
            if cursor == 0 {
                break;
            }
        }
        assert!(found);
        assert!(session.delete(&key, true).is_err());
        assert!(session.delete(&key, false).unwrap());
        assert_eq!(session.inspect(&key).unwrap().kind, "none");
        for (kind, command) in [
            ("hash", "HSET k f v"),
            ("list", "RPUSH k a b"),
            ("set", "SADD k a b"),
            ("zset", "ZADD k 1 a"),
            ("stream", "XADD k * f v"),
        ] {
            let key = format!("{prefix}{kind}");
            session
                .execute(
                    &command.replace(
                        " k ",
                        &format!(" {} ", crate::redis_commands::quote(key.as_bytes())),
                    ),
                    false,
                )
                .unwrap();
            assert_eq!(session.inspect(key.as_bytes()).unwrap().kind, kind);
            session.delete(key.as_bytes(), false).unwrap();
        }
        let wrong = Session::connect_diagnostic(&settings(), "wrong")
            .err()
            .unwrap();
        assert!(
            wrong
                .passed
                .contains(&crate::connection_probe::ProbeField::Host)
        );
        assert_eq!(
            wrong.failed,
            vec![
                crate::connection_probe::ProbeField::User,
                crate::connection_probe::ProbeField::Password
            ]
        );
        let invalid = Session::connect_diagnostic(
            &Settings {
                database: 999999,
                ..settings()
            },
            "rustable-redis-test-only",
        )
        .err()
        .unwrap();
        assert!(
            invalid
                .passed
                .contains(&crate::connection_probe::ProbeField::Password)
        );
        assert_eq!(
            invalid.failed,
            vec![crate::connection_probe::ProbeField::Database]
        );
    }
}
