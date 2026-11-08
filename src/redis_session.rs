//! Native Redis transport and bounded browser operations. Owned by one worker.
use crate::redis_settings::Settings;
use redis::{ConnectionLike, IntoConnectionInfo};

pub(crate) struct Session {
    connection: Option<redis::Connection>,
    cluster: Option<Box<crate::redis_cluster::Router>>,
    source_host: String,
    tls: bool,
    sentinel_primary: bool,
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
pub(crate) struct KeyRow {
    pub key: Vec<u8>,
    pub kind: String,
    pub ttl_ms: i64,
    pub value: redis::Value,
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
        Self { connection: None, cluster: None, source_host: String::new(), tls: false, sentinel_primary: false }
    }
    pub fn connect(settings: &Settings, password: &str) -> Result<Self, String> {
        Self::connect_diagnostic(settings, password).map_err(|error| error.message)
    }
    pub fn is_connected(&self) -> bool {
        self.cluster.as_ref().map_or_else(|| self.connection
            .as_ref()
            .is_some_and(ConnectionLike::is_open), |router|router.is_connected())
    }
    pub fn connect_diagnostic(settings: &Settings, password: &str) -> Result<Self, ConnectFailure> {
        if !settings.cluster || settings.cluster_seeds.is_empty() { return Self::connect_inner(settings,password,true,false); }
        use crate::connection_probe::ProbeField::{Host, Port};
        settings.validate().map_err(|message| ConnectFailure { passed: vec![], failed: vec![Host, Port], message })?;
        let deadline = std::time::Instant::now() + settings.connect_timeout;
        let addresses = std::iter::once((settings.host.as_str(), settings.port)).chain(settings.cluster_seeds.iter().map(|seed| (seed.host.as_str(), seed.port)));
        let mut last = ConnectFailure { passed: vec![], failed: vec![Host, Port], message: "Cluster seed connection deadline reached".into() };
        for (index, (host, port)) in addresses.enumerate() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() { break; }
            let budget = remaining / (settings.cluster_seeds.len() + 1 - index) as u32;
            let candidate = Settings { host: host.into(), port, cluster_seeds: vec![], connect_timeout: budget, read_timeout: settings.read_timeout.min(budget), ..settings.clone() };
            match Self::connect_inner(&candidate, password, true, false) {
                Ok(mut session) => {
                    if std::time::Instant::now() >= deadline { last.message = "Cluster seed connection deadline reached after discovery".into(); break; }
                    if let Some(router) = &mut session.cluster {
                        router.restore_timeouts(settings).map_err(|message| ConnectFailure { passed: vec![], failed: vec![Host, Port], message })?;
                        router.remember_seeds(settings);
                    }
                    return Ok(session);
                }
                Err(error) => last = error,
            }
        }
        last.message = format!("No configured Cluster seed connected: {}", last.message);
        Err(last)
    }
    pub(crate) fn cluster_node(settings: &Settings, password: &str) -> Result<redis::Connection,String> {
        Self::connect_inner(settings,password,false,false).map_err(|e|e.message)?.connection.ok_or("Redis node did not connect".into())
    }
    pub fn connect_with_sentinel(settings: &Settings, password: &str, sentinel_password: &str) -> Result<Self, String> {
        if settings.sentinel.is_some() {
            crate::redis_sentinel::connect(settings, password, sentinel_password)
        } else { Self::connect(settings, password) }
    }
    pub fn reconnect(&mut self, settings: &Settings, password: &str, sentinel_password: &str) -> Result<(), String> {
        if let Some(router) = &mut self.cluster {
            let endpoint = router.reconnect()?;
            self.source_host = endpoint.host;
            Ok(())
        } else {
            *self = Self::connect_with_sentinel(settings, password, sentinel_password)?;
            Ok(())
        }
    }
    pub(crate) fn raw_node(settings: &Settings, password: &str) -> Result<redis::Connection, String> {
        Self::connect_inner(settings, password, false, true).map_err(|e| e.message)?.connection.ok_or("Redis transport did not connect".into())
    }
    // Only the Sentinel resolver calls this after ROLE and SELECT succeed.
    pub(crate) fn verified_primary(connection: redis::Connection, settings: &Settings) -> Self {
        Self { connection: Some(connection), cluster: None, source_host: settings.host.clone(), tls: settings.tls, sentinel_primary: true }
    }
    fn connect_inner(settings: &Settings, password: &str, discover: bool, setup_only: bool) -> Result<Self, ConnectFailure> {
        use crate::connection_probe::ProbeField::*;
        let failure = |passed: Vec<_>, failed: Vec<_>, message: String| ConnectFailure {
            passed,
            failed,
            message,
        };
        settings
            .validate()
            .map_err(|message| failure(vec![], vec![Host, Port], message))?;
        // Password-only callers cannot supply independent discovery credentials.
        // Never reinterpret a Sentinel profile as a direct data-node connection.
        if settings.sentinel.is_some() {
            return Err(failure(vec![], vec![Host], "This Sentinel profile requires the discovery-credential connection path.".into()));
        }
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
        let client = if settings.tls && (settings.root_certificate.is_some() || settings.client_certificate.is_some()) {
            let read_pem = |path: &std::path::Path, label: &str| -> Result<Vec<u8>, ConnectFailure> {
                crate::redis_settings::read_pem(path, label)
                    .map_err(|message| failure(vec![], vec![Tls], message))
            };
            let root_cert = settings.root_certificate.as_deref().map(|path| read_pem(path, "CA certificate")).transpose()?;
            let client_tls = match (&settings.client_certificate, &settings.client_key) {
                (Some(cert), Some(key)) => Some(redis::ClientTlsConfig {
                    client_cert: read_pem(cert, "client certificate")?,
                    client_key: read_pem(key, "client private key")?,
                }),
                _ => None, // Settings validation enforces paired identity paths.
            };
            redis::Client::build_with_tls(info, redis::TlsCertificates { client_tls, root_cert })
        } else {
            redis::Client::open(info)
        }
        .map_err(|e| failure(vec![], vec![Tls], format!("Redis configuration: {e}")))?;
        let mut transport = vec![Host, Port];
        if settings.tls {
            transport.push(Tls);
        }
        let tunnel = crate::redis_settings::start_tunnel(settings, &std::sync::atomic::AtomicBool::new(false))
            .map_err(|message| failure(vec![], vec![SshHost], message))?;
        let opened = if let Some(tunnel) = &tunnel {
            client.get_connection_via(std::net::SocketAddr::from(([127, 0, 0, 1], tunnel.local_port())), settings.connect_timeout)
        } else { client.get_connection_with_timeout(settings.connect_timeout) };
        let mut connection = opened
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
        if let Some(tunnel) = tunnel { connection.retain_transport(Box::new(tunnel)); }
        connection
            .set_read_timeout(Some(settings.read_timeout))
            .map_err(|e| failure(vec![], transport.clone(), e.to_string()))?;
        connection
            .set_write_timeout(Some(settings.read_timeout))
            .map_err(|e| failure(vec![], transport.clone(), e.to_string()))?;
        let mut session = Self {
            connection: Some(connection),
            cluster: None,
            source_host: settings.host.clone(),
            tls: settings.tls,
            sentinel_primary: false,
        };
        if setup_only { return Ok(session); }
        // PING detects missing credentials even when a server does not require an
        // explicit AUTH exchange. Redis ACLs must permit PING for this workspace.
        session
            .query(&redis::cmd("PING"))
            .map_err(|message| failure(transport.clone(), vec![User, Password], message))?;
        transport.extend([User, Password]);
        if !settings.cluster {
            session.query(redis::cmd("SELECT").arg(settings.database))
                .map_err(|message| failure(transport.clone(),vec![Database],message))?;
        } else if discover {
            let value=session.query(redis::cmd("CLUSTER").arg("SHARDS"))
                .map_err(|message|failure(transport.clone(),vec![Database],message))?;
            let topology=crate::redis_topology::Cluster::decode(value,&settings.host,settings.tls)
                .map_err(|message|failure(transport.clone(),vec![Database],message))?;
            let connection=session.connection.take().unwrap();
            let mut router = crate::redis_cluster::Router::new(settings.clone(),password.to_owned(),connection,topology.routes());
            router.remember_discovery(&topology);
            session.cluster=Some(Box::new(router));
        }
        Ok(session)
    }
    fn query(&mut self, command: &redis::Cmd) -> Result<redis::Value, String> {
        if let Some(router)=&mut self.cluster {return router.query(command);}
        let conn = self
            .connection
            .as_mut()
            .ok_or("Redis connection closed. Reconnect before continuing.")?;
        match command.query(conn) {
            Ok(value) => Ok(value),
            Err(error) => {
                if self.sentinel_primary && error.code() == Some("READONLY") {
                    self.connection = None;
                    return Err("Sentinel primary is now read-only. Reconnect to discover the current primary. The rejected command was not replayed.".into());
                }
                if error.is_io_error() || error.kind() == redis::ErrorKind::Parse || !conn.is_open() {
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
    fn query_bounded(&mut self, command: &redis::Cmd) -> Result<redis::Value, String> {
        const LIMIT: usize = 1024 * 1024;
        if let Some(router) = &mut self.cluster { return router.query_bounded(command, LIMIT); }
        let conn = self.connection.as_mut().ok_or("Redis connection closed. Reconnect before continuing.")?;
        let result = conn.req_packed_commands_bounded(&command.get_packed_command(), &[LIMIT]).and_then(|mut values| values.remove(0).extract_error());
        match result {
            Ok(value) => Ok(value),
            Err(error) if error.is_io_error() || error.kind() == redis::ErrorKind::Parse || !conn.is_open() => { self.connection = None; Err(format!("Redis search connection closed: {error}. Reconnect before retrying. Nothing was replayed.")) }
            Err(error) => Err(error.to_string()),
        }
    }
    fn typed<T: redis::FromRedisValue>(&mut self, command: &redis::Cmd) -> Result<T, String> {
        redis::from_redis_value(self.query(command)?).map_err(|e| e.to_string())
    }
    #[cfg(test)]
    pub fn scan(&mut self, cursor: u64, pattern: &[u8], count: usize) -> Result<KeyPage, String> {
        self.scan_filtered(cursor, pattern, count, "")
    }
    pub fn scan_filtered(
        &mut self,
        cursor: u64,
        pattern: &[u8],
        count: usize,
        kind: &str,
    ) -> Result<KeyPage, String> {
        if let Some(router)=&mut self.cluster {return router.scan(cursor,pattern,count,kind);}
        let mut command = redis::cmd("SCAN");
        command
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(count.clamp(1, 1000));
        if !kind.is_empty() {
            command.arg("TYPE").arg(kind);
        }
        let (cursor, keys) = self.typed(&command)?;
        Ok(KeyPage { cursor, keys })
    }

    fn pipeline(&mut self, pipeline: &redis::Pipeline) -> Result<Vec<redis::Value>, String> {
        if let Some(router)=&mut self.cluster {return router.read_pipeline(pipeline);}
        let connection = self
            .connection
            .as_mut()
            .ok_or("Redis connection closed. Reconnect before continuing.")?;
        match pipeline.query(connection) {
            Ok(values) => Ok(values),
            Err(error) => {
                if error.is_io_error() || error.kind() == redis::ErrorKind::Parse || !connection.is_open() || (self.sentinel_primary && error.code() == Some("READONLY")) {
                    self.connection = None;
                }
                Err(format!("Redis preview: {error}"))
            }
        }
    }
    /// Two pipelined read rounds per chunk, rather than one network trip per cell.
    pub fn key_rows(&mut self, keys: &[Vec<u8>]) -> Result<Vec<KeyRow>, String> {
        let mut rows = Vec::new();
        for chunk in keys.chunks(200) {
            let mut metadata = redis::pipe();
            for key in chunk {
                metadata.cmd("TYPE").arg(key).cmd("PTTL").arg(key);
            }
            let values = self.pipeline(&metadata)?;
            let mut previews = redis::pipe();
            let mut kinds = Vec::new();
            for (key, pair) in chunk.iter().zip(values.as_chunks::<2>().0) {
                let kind: String =
                    redis::from_redis_value(pair[0].clone()).map_err(|e| e.to_string())?;
                let ttl: i64 =
                    redis::from_redis_value(pair[1].clone()).map_err(|e| e.to_string())?;
                match kind.as_str() {
                    "string" => {
                        previews.cmd("GETRANGE").arg(key).arg(0).arg(255);
                    }
                    "hash" => {
                        previews.cmd("HSCAN").arg(key).arg(0).arg("COUNT").arg(4);
                    }
                    "set" => {
                        previews.cmd("SSCAN").arg(key).arg(0).arg("COUNT").arg(4);
                    }
                    "list" => {
                        previews.cmd("LRANGE").arg(key).arg(0).arg(3);
                    }
                    "zset" => {
                        previews
                            .cmd("ZRANGE")
                            .arg(key)
                            .arg(0)
                            .arg(3)
                            .arg("WITHSCORES");
                    }
                    "TSDB-TYPE" => { previews.cmd("TS.GET").arg(key); }
                    "vectorset" => { previews.cmd("VCARD").arg(key); }
                    "stream" => {
                        previews
                            .cmd("XRANGE")
                            .arg(key)
                            .arg("-")
                            .arg("+")
                            .arg("COUNT")
                            .arg(2);
                    }
                    _ => {
                        previews.cmd("TYPE").arg(key);
                    }
                }
                kinds.push((kind, ttl));
            }
            // A key may change type between rounds. Keep its metadata and let
            // selection obtain a fresh inspector instead of hiding the whole page.
            let previews = match self.pipeline(&previews) {
                Ok(values) => values,
                Err(_) if self.is_connected() => vec![redis::Value::Nil; chunk.len()],
                Err(error) => return Err(error),
            };
            for ((key, (kind, ttl_ms)), mut value) in chunk.iter().zip(kinds).zip(previews) {
                if matches!(kind.as_str(), "hash" | "set")
                    && let redis::Value::Array(parts) = &mut value
                    && parts.len() == 2
                {
                    value = parts.pop().unwrap();
                }
                bound_preview(&mut value, &mut 4096, &mut 32);
                rows.push(KeyRow {
                    key: key.clone(),
                    kind,
                    ttl_ms,
                    value,
                });
            }
        }
        Ok(rows)
    }
    #[cfg(test)]
    pub fn inspect(&mut self, key: &[u8]) -> Result<Snapshot, String> {
        self.inspect_view(key).map(|v| v.snapshot)
    }
    pub fn inspect_view(&mut self, key: &[u8]) -> Result<crate::redis_value::Inspection, String> {
        use crate::redis_value::{Cursor, PageRequest};
        let mut meta = redis::pipe();
        meta.cmd("TYPE").arg(key).cmd("PTTL").arg(key);
        let values = self.pipeline(&meta)?;
        let kind: String = redis::from_redis_value(values[0].clone()).map_err(|e| e.to_string())?;
        let ttl_ms: i64 = redis::from_redis_value(values[1].clone()).map_err(|e| e.to_string())?;
        if kind == "none" {
            return Ok(crate::redis_value::Inspection {
                snapshot: Snapshot {
                    key: key.to_vec(),
                    kind,
                    ttl_ms,
                    length: Some(0),
                    value: redis::Value::Nil,
                    truncated: false,
                },
                page: crate::redis_value::Page {
                    cursor: Cursor::Offset(0),
                    next: None,
                    pattern: b"*".to_vec(),
                    display: Default::default(),
                },
            });
        }
        let request = PageRequest {
            key: key.to_vec(),
            cursor: Cursor::first(&kind, b"*"),
            kind,
            pattern: b"*".to_vec(),
        };
        self.read_value_page(&request, Some(ttl_ms))
    }
    pub fn value_page(
        &mut self,
        request: &crate::redis_value::PageRequest,
    ) -> Result<crate::redis_value::Inspection, String> {
        self.read_value_page(request, None)
    }
    // First read: length + data in one pipeline after TYPE/PTTL. Subsequent
    // reads include TYPE/PTTL/length/data in one exchange; never a preflight.
    fn read_value_page(
        &mut self,
        request: &crate::redis_value::PageRequest,
        known_ttl: Option<i64>,
    ) -> Result<crate::redis_value::Inspection, String> {
        if request.kind == "TSDB-TYPE" { return self.read_timeseries_page(request, known_ttl); }
        if request.kind == "vectorset" { return self.read_vectors_page(request, known_ttl); }
        use crate::redis_value::{Cursor, Display, Inspection, PAGE_SIZE, Page, STRING_PAGE};
        let key = &request.key;
        let mut pipe = redis::pipe();
        if known_ttl.is_none() {
            pipe.cmd("TYPE").arg(key).cmd("PTTL").arg(key);
        }
        let json = request.kind == "ReJSON-RL";
        let length_command = match request.kind.as_str() {
            "ReJSON-RL" => "JSON.GET",
            "string" => "STRLEN",
            "hash" => "HLEN",
            "set" => "SCARD",
            "list" => "LLEN",
            "zset" => "ZCARD",
            "stream" => "XLEN",
            kind => {
                return Err(format!(
                    "Key type {kind} is not supported by the value inspector."
                ));
            }
        };
        pipe.cmd(length_command).arg(key);
        match (&request.cursor, request.kind.as_str()) {
            (Cursor::Offset(0), "ReJSON-RL") => {}
            (Cursor::Scan(cursor), "hash" | "set" | "zset") => {
                pipe.cmd(match request.kind.as_str() {
                    "hash" => "HSCAN",
                    "set" => "SSCAN",
                    _ => "ZSCAN",
                })
                .arg(key)
                .arg(cursor)
                .arg("MATCH")
                .arg(&request.pattern)
                .arg("COUNT")
                .arg(PAGE_SIZE);
            }
            (Cursor::Offset(offset), "string" | "list" | "zset") => {
                let count = if request.kind == "string" {
                    STRING_PAGE
                } else {
                    PAGE_SIZE
                };
                let end = offset
                    .checked_add(count - 1)
                    .filter(|end| *end <= i64::MAX as u64)
                    .ok_or("Page offset is out of range.")?;
                pipe.cmd(match request.kind.as_str() {
                    "string" => "GETRANGE",
                    "list" => "LRANGE",
                    _ => "ZRANGE",
                })
                .arg(key)
                .arg(offset)
                .arg(end);
                if request.kind == "zset" {
                    pipe.arg("WITHSCORES");
                }
            }
            (Cursor::Stream(start), "stream") => {
                pipe.cmd("XRANGE")
                    .arg(key)
                    .arg(start)
                    .arg("+")
                    .arg("COUNT")
                    .arg(PAGE_SIZE + 1);
            }
            _ => {
                return Err(
                    "This page cursor does not match the key type. Refresh the key.".into(),
                );
            }
        }
        let mut values = self.pipeline(&pipe)?;
        let ttl_ms = if let Some(ttl) = known_ttl {
            ttl
        } else {
            let actual: String =
                redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?;
            let ttl: i64 = redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?;
            if actual != request.kind {
                return Err("The key expired or changed type. Refresh before continuing.".into());
            }
            ttl
        };
        let length: u64 = if json {
            match values.first() {
                Some(redis::Value::BulkString(bytes)) => bytes.len() as u64,
                _ => return Err("JSON key disappeared. Refresh before continuing.".into()),
            }
        } else {
            redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?
        };
        let mut value = values.remove(0);
        let next = match &request.cursor {
            Cursor::Timestamp { .. } => return Err("Timestamp cursor requires a time-series key.".into()),
            Cursor::Lex(_) => return Err("Lexical cursor requires a vector-set key.".into()),
            Cursor::Scan(_) => {
                let (cursor, rows): (u64, Vec<redis::Value>) =
                    redis::from_redis_value(value).map_err(|e| e.to_string())?;
                value = redis::Value::Array(rows);
                (cursor != 0).then_some(Cursor::Scan(cursor))
            }
            Cursor::Offset(offset) => {
                let count = match &value {
                    redis::Value::BulkString(b) => b.len(),
                    redis::Value::Array(v) => v.len() / if request.kind == "zset" { 2 } else { 1 },
                    _ => 0,
                } as u64;
                (count > 0 && offset + count < length).then_some(Cursor::Offset(offset + count))
            }
            Cursor::Stream(_) => {
                if let redis::Value::Array(rows) = &mut value {
                    let more = rows.len() > PAGE_SIZE as usize;
                    rows.truncate(PAGE_SIZE as usize);
                    if more {
                        let id = rows.last().and_then(|row| {
                            if let redis::Value::Array(v) = row {
                                v.first()
                            } else {
                                None
                            }
                        });
                        if let Some(redis::Value::BulkString(id)) = id {
                            let mut start = vec![b'('];
                            start.extend_from_slice(id);
                            Some(Cursor::Stream(start))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };
        let limited = bound_preview(&mut value, &mut (256 * 1024), &mut 8192);
        // Do not imply the skipped tail of an oversized response was displayed.
        let snapshot = Snapshot {
            key: key.clone(),
            kind: request.kind.clone(),
            ttl_ms,
            length: Some(length),
            value,
            truncated: limited
                || (request.kind == "string" && (next.is_some() || request.cursor.offset() != 0)),
        };
        let display = Display::new(&snapshot, &request.cursor);
        Ok(Inspection {
            snapshot,
            page: Page {
                cursor: request.cursor.clone(),
                next,
                pattern: request.pattern.clone(),
                display,
            },
        })
    }
    fn read_vectors_page(&mut self, request: &crate::redis_value::PageRequest, known_ttl: Option<i64>) -> Result<crate::redis_value::Inspection, String> {
        use crate::redis_value::{Display, Inspection, Page};
        let boundary = crate::redis_vectors::boundary(&request.cursor)?;
        let mut pipe = redis::pipe();
        if known_ttl.is_none() { pipe.cmd("TYPE").arg(&request.key).cmd("PTTL").arg(&request.key); }
        pipe.cmd("VCARD").arg(&request.key);
        pipe.cmd("EVAL_RO").arg(crate::redis_vectors::PAGE_SCRIPT).arg(1).arg(&request.key).arg(boundary);
        let mut values = self.pipeline(&pipe)?;
        let ttl_ms = if let Some(ttl) = known_ttl { ttl } else {
            if values.len() != 4 { return Err("Invalid vector page response count.".into()); }
            let actual: String = redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?;
            let ttl = redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?;
            if actual != "vectorset" { return Err("The key expired or changed type. Refresh before continuing.".into()); }
            ttl
        };
        if values.len() != 2 { return Err("Invalid vector page response count.".into()); }
        let length = redis::from_redis_value(values.remove(0)).map_err(|e| e.to_string())?;
        let (value, next) = crate::redis_vectors::page(values.remove(0), &request.cursor)?;
        let snapshot = Snapshot { key: request.key.clone(), kind: request.kind.clone(), ttl_ms, length: Some(length), value, truncated: false };
        let display = Display::new(&snapshot, &request.cursor);
        Ok(Inspection { snapshot, page: Page { cursor: request.cursor.clone(), next, pattern: request.pattern.clone(), display } })
    }
    fn read_timeseries_page(&mut self, request: &crate::redis_value::PageRequest, known_ttl: Option<i64>) -> Result<crate::redis_value::Inspection, String> {
        use crate::redis_value::{Cursor, Display, Inspection, Page, PAGE_SIZE};
        let Cursor::Timestamp { start, end } = request.cursor else { return Err("Invalid time-series cursor. Refresh the key.".into()); };
        if start > i64::MAX as u64 || end.is_some_and(|end| end > i64::MAX as u64 || end < start) { return Err("Time-series timestamp exceeds Redis range.".into()); }
        let mut pipe = redis::pipe();
        if known_ttl.is_none() { pipe.cmd("TYPE").arg(&request.key).cmd("PTTL").arg(&request.key); }
        pipe.cmd("TS.RANGE").arg(&request.key).arg(start).arg(end.map_or_else(|| "+".into(), |end| end.to_string())).arg("COUNT").arg(PAGE_SIZE + 1);
        let mut replies = self.pipeline(&pipe)?;
        let ttl_ms = if let Some(ttl) = known_ttl { ttl } else {
            if replies.len() != 3 { return Err("Invalid time-series page response.".into()); }
            let actual: String = redis::from_redis_value(replies.remove(0)).map_err(|e| e.to_string())?;
            let ttl = redis::from_redis_value(replies.remove(0)).map_err(|e| e.to_string())?;
            if actual != "TSDB-TYPE" { return Err("The key expired or changed type. Refresh before continuing.".into()); }
            ttl
        };
        if replies.len() != 1 { return Err("Invalid time-series response count.".into()); }
        let (value, next) = timeseries_page(replies.remove(0), start, end)?;
        let snapshot = Snapshot { key: request.key.clone(), kind: request.kind.clone(), ttl_ms, length: None, value, truncated: false };
        let display = Display::new(&snapshot, &request.cursor);
        Ok(Inspection { snapshot, page: Page { cursor: request.cursor.clone(), next, pattern: request.pattern.clone(), display } })
    }
    pub fn search_mutate(&mut self, mutation: crate::redis_search_manage::Mutation, read_only: bool) -> Result<crate::redis_search::Reply, String> {
        let command = mutation.command(read_only)?;
        let value = self.query(&command)?;
        if value != redis::Value::Okay { return Err("Unexpected index-operation reply. Verify the index before retrying.".into()); }
        Ok(crate::redis_search::Reply::MutationCompleted(mutation))
    }
    pub fn search(
        &mut self,
        request: crate::redis_search::Request,
    ) -> Result<crate::redis_search::Reply, String> {
        if let crate::redis_search::Request::Metadata(names) = &request {
            let rows = if let Some(router) = &mut self.cluster {
                router.search_metadata(names)?
            } else {
                let connection = self.connection.as_mut().ok_or("Redis connection closed. Reconnect before continuing.")?;
                let result = crate::redis_search_catalog::fetch(connection, names);
                if !connection.is_open() { self.connection = None; }
                result?
            };
            return Ok(crate::redis_search::Reply::Metadata(rows));
        }
        let command = request.command()?;
        let started = std::time::Instant::now();
        let value = self.query(&command)?;
        let mut reply = request.decode(value)?;
        if let crate::redis_search::Reply::Profile { profile, .. } = &mut reply {
            profile.client_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
        Ok(reply)
    }
    pub fn diagnostics(
        &mut self,
        request: &crate::redis_diagnostics::Request,
    ) -> Result<crate::redis_diagnostics::Report, String> {
        let mut pipe = redis::pipe();
        match request {
            crate::redis_diagnostics::Request::VectorSimilar { key, element } => {
                if key.len() > 4096 || element.len() > 65536 { return Err("Vector search supports keys up to 4 KiB and element names up to 64 KiB.".into()); }
                let value = self.query_bounded(redis::cmd("VSIM").arg(key).arg("ELE").arg(element).arg("WITHSCORES").arg("COUNT").arg(20).arg("EF").arg(100))?;
                Ok(crate::redis_diagnostics::Report { vector_matches: Some(Box::new(crate::redis_vector_matches::Matches::decode(value, key, element)?)), ..Default::default() })
            }
            crate::redis_diagnostics::Request::VectorElement { key, element } => {
                if key.len() > 4096 || element.len() > 65536 { return Err("Vector inspection supports keys up to 4 KiB and element names up to 64 KiB.".into()); }
                let value = self.query(redis::cmd("EVAL_RO").arg(crate::redis_vector_element::SCRIPT).arg(1).arg(key).arg(element))?;
                Ok(crate::redis_diagnostics::Report { vector_element: Some(Box::new(crate::redis_vector_element::Element::decode(value, element)?)), ..Default::default() })
            }
            crate::redis_diagnostics::Request::TimeSeries(key) => {
                if key.len() > 4096 { return Err("Metadata inspection supports keys up to 4,096 bytes.".into()); }
                let value = self.query(redis::cmd("EVAL_RO").arg(crate::redis_module_info::TIMESERIES_SCRIPT).arg(1).arg(key))?;
                crate::redis_module_info::timeseries(value, key)
            }
            crate::redis_diagnostics::Request::Cluster => {
                let value = self.query(redis::cmd("CLUSTER").arg("SHARDS"))?;
                let mut report=crate::redis_topology::Cluster::decode(value, &self.source_host, self.tls)?.report();
                if self.cluster.is_some() {report.footer="Topology reported by the connected node. Key browsing scans primaries from the routing snapshot; reconnect to reload topology after changes.".into();}
                Ok(report)
            }
            crate::redis_diagnostics::Request::Server => {
                pipe.cmd("INFO").cmd("SLOWLOG").arg("GET").arg(50);
                crate::redis_diagnostics::server(self.pipeline(&pipe)?)
            }
            crate::redis_diagnostics::Request::Keys(keys) => {
                if keys.is_empty() || keys.len() > 1000 {
                    return Err("Select 1–1,000 loaded keys for analysis.".into());
                }
                for key in keys {
                    pipe.cmd("MEMORY")
                        .arg("USAGE")
                        .arg(key)
                        .arg("SAMPLES")
                        .arg(5);
                }
                crate::redis_diagnostics::keys(keys, self.pipeline(&pipe)?)
            }
        }
    }
    pub fn bulk_mutate(&mut self, mutation: &crate::redis_mutation::Mutation, read_only: bool,
        cancel: &std::sync::atomic::AtomicBool) -> Result<crate::redis_cluster::WriteReport<bool>, String> {
        self.cluster.as_mut().ok_or("Cluster bulk routing requires a Cluster connection.")?.bulk(mutation, read_only, cancel)
    }
    pub fn mutate(
        &mut self,
        mutation: &crate::redis_mutation::Mutation,
        read_only: bool,
    ) -> Result<u64, String> {
        mutation.validate(read_only)?;
        let mut command = redis::cmd("EVAL");
        let rename = mutation.action == crate::redis_mutation::Action::Rename;
        // Declare every accessed key, including the destination. Cluster routing
        // can then reject CROSSSLOT before the script executes any operation.
        command.arg(crate::redis_mutation::SCRIPT)
            .arg(mutation.keys.len() + usize::from(rename))
            .arg(&mutation.keys);
        if rename { command.arg(&mutation.field); }
        command.arg(mutation.action_name())
            .arg(&mutation.kind)
            .arg(&mutation.field)
            .arg(&mutation.value)
            .arg(&mutation.expected)
            .arg(mutation.length)
            .arg(mutation.ttl_ms.map(|n| n.to_string()).unwrap_or_default());
        self.typed(&command)
    }
    pub fn review_archive(
        &mut self,
        archive: &crate::redis_archive::Archive,
        database: u32,
    ) -> Result<crate::redis_archive::Review, String> {
        archive.validate()?;
        if self.cluster.is_some() {
            crate::redis_archive::require_database(0, database)?;
            let mut pipe = redis::pipe();
            for entry in &archive.entries {
                pipe.cmd("EVAL_RO").arg(crate::redis_archive::REVIEW_SCRIPT).arg(1).arg(&entry.key);
            }
            let values = self.pipeline(&pipe)?;
            let mut review = crate::redis_archive::Review {
                database, server_now_ms: 0, targets: Vec::with_capacity(values.len()), existing: 0, expired: 0,
            };
            for (entry, value) in archive.entries.iter().zip(values) {
                let part = crate::redis_archive::review_reply(value, database, 1)?;
                review.server_now_ms = review.server_now_ms.max(part.server_now_ms);
                review.expired += usize::from(entry.expires_at_ms.is_some_and(|n| n <= part.server_now_ms));
                review.existing += part.existing;
                review.targets.extend(part.targets);
            }
            return Ok(review);
        }
        let mut command = redis::cmd("EVAL_RO");
        command
            .arg(crate::redis_archive::REVIEW_SCRIPT)
            .arg(archive.entries.len());
        for entry in &archive.entries {
            command.arg(&entry.key);
        }
        let mut review = crate::redis_archive::review_reply(
            self.query(&command)?,
            database,
            archive.entries.len(),
        )?;
        review.expired = archive
            .entries
            .iter()
            .filter(|e| e.expires_at_ms.is_some_and(|n| n <= review.server_now_ms))
            .count();
        Ok(review)
    }
    pub fn export_archive(
        &mut self,
        keys: &[Vec<u8>],
        database: u32,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<crate::redis_archive::ExportReport, String> {
        use crate::redis_archive::*;
        use std::sync::atomic::Ordering;
        if keys.is_empty()
            || keys.len() > MAX_KEYS
            || keys.iter().any(|k| k.len() > 4096)
            || keys.iter().map(Vec::len).sum::<usize>() > 1024 * 1024
        {
            return Err("Select 1–1,000 keys, up to 4 KiB each and 1 MiB of names.".into());
        }
        if self.cluster.is_some() {
            require_database(0, database)?;
        }
        let mut report = ExportReport {
            archive: Archive {
                created_at_ms: 0,
                database,
                entries: Vec::new(),
            },
            skipped: Vec::new(),
        };
        for batch in keys.chunks(16) {
            if cancel.load(Ordering::Acquire) {
                return Err("Export cancelled; no archive was saved.".into());
            }
            let ExportBatch { timestamp, entries, skipped } = if self.cluster.is_some() {
                // Sixteen per-key atomic scripts, fanned out in one read exchange.
                // Cross-slot Lua is forbidden even when keys share a primary.
                let mut pipe = redis::pipe();
                for key in batch {
                    pipe.cmd("EVAL_RO").arg(EXPORT_SCRIPT).arg(1).arg(key);
                }
                let mut combined = ExportBatch { timestamp: 0, entries: Vec::new(), skipped: Vec::new() };
                for (key, value) in batch.iter().zip(self.pipeline(&pipe)?) {
                    let part = export_reply(value, std::slice::from_ref(key))?;
                    combined.timestamp = combined.timestamp.max(part.timestamp);
                    combined.entries.extend(part.entries);
                    combined.skipped.extend(part.skipped);
                }
                combined
            } else {
                let mut command = redis::cmd("EVAL_RO");
                command.arg(EXPORT_SCRIPT).arg(batch.len());
                for key in batch { command.arg(key); }
                export_reply(self.query(&command)?, batch)?
            };
            if report.archive.created_at_ms == 0 {
                report.archive.created_at_ms = timestamp;
            }
            report.archive.entries.extend(entries);
            report.skipped.extend(skipped);
            if !report.archive.entries.is_empty() {
                report.archive.validate()?;
            }
        }
        Ok(report)
    }
    pub fn import_archive(
        &mut self,
        archive: &crate::redis_archive::Archive,
        collision: crate::redis_archive::Collision,
        expiry: crate::redis_archive::Expiry,
        read_only: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<crate::redis_archive::ImportReport, String> {
        use crate::redis_archive::*;
        use std::sync::atomic::Ordering;
        if read_only {
            return Err("Read-only mode prevents importing Redis keys.".into());
        }
        archive.validate()?;
        let mut report = ImportReport {
            outcomes: archive
                .entries
                .iter()
                .map(|entry| Outcome {
                    key: entry.key.clone(),
                    state: State::NotAttempted,
                    message: String::new(),
                })
                .collect(),
            stopped: None,
        };
        for (index, batch) in archive.entries.chunks(16).enumerate() {
            if cancel.load(Ordering::Acquire) {
                report.stopped = Some("Cancelled; remaining keys were not attempted.".into());
                break;
            }
            if let Some(router) = &mut self.cluster {
                let batch_report = router.import_batch(batch, collision, expiry);
                let offset = index * 16;
                for (target, outcome) in report.outcomes[offset..offset + batch.len()].iter_mut().zip(batch_report.outcomes) {
                    *target = outcome;
                }
                if let Some(error) = batch_report.stopped {
                    report.stopped = Some(error);
                    break;
                }
                continue;
            }
            let mut command = redis::cmd("EVAL");
            command.arg(IMPORT_SCRIPT).arg(batch.len());
            for entry in batch {
                command.arg(&entry.key);
            }
            command
                .arg(if collision == Collision::Replace {
                    "replace"
                } else {
                    "skip"
                })
                .arg(if expiry == Expiry::Persistent {
                    "persistent"
                } else {
                    "preserve"
                });
            for entry in batch {
                command
                    .arg(entry.expires_at_ms.unwrap_or(-1))
                    .arg(&entry.dump);
            }
            let result = self
                .query(&command)
                .and_then(|value| import_reply(value, batch.len()));
            let offset = index * 16;
            match result {
                Ok(outcomes) => {
                    for (row, (state, message)) in report.outcomes[offset..offset + batch.len()]
                        .iter_mut()
                        .zip(outcomes)
                    {
                        row.state = state;
                        row.message = message;
                    }
                }
                Err(error) => {
                    for row in &mut report.outcomes[offset..offset + batch.len()] {
                        row.state = State::Unknown;
                        row.message =
                            "Submitted batch outcome is unconfirmed. Inspect before retrying."
                                .into();
                    }
                    report.stopped = Some(error);
                    break;
                }
            }
        }
        Ok(report)
    }
    pub fn publish(
        &mut self,
        channel: &[u8],
        payload: &[u8],
        read_only: bool,
    ) -> Result<u64, String> {
        if read_only {
            return Err("Read-only mode prevents publishing.".into());
        }
        if channel.is_empty() || channel.len() > 4096 || payload.len() > 65536 {
            return Err("Publish exceeds the channel or payload limit.".into());
        }
        match self.query(redis::cmd("PUBLISH").arg(channel).arg(payload))? {
            redis::Value::Int(count) => {
                u64::try_from(count).map_err(|_| "Invalid PUBLISH receiver count".into())
            }
            _ => Err("Invalid PUBLISH response".into()),
        }
    }
    pub fn execute(&mut self, text: &str, read_only: bool) -> Result<Vec<redis::Value>, String> {
        let commands = crate::redis_commands::parse(text)?;
        // Validate the entire batch before its first write.
        for args in &commands {
            crate::redis_commands::validate(args, read_only)?;
            if self.cluster.is_some() {
                let name=std::str::from_utf8(&args[0]).map_err(|_|"Command name must be UTF-8.")?;
                let mut command=redis::cmd(name);command.arg(&args[1..]);
                crate::redis_cluster::command_slot(&command)?;
            }
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

fn timeseries_page(value: redis::Value, start: u64, end: Option<u64>) -> Result<(redis::Value, Option<crate::redis_value::Cursor>), String> {
    use crate::redis_value::{Cursor, PAGE_SIZE};
    let redis::Value::Array(mut rows) = value else { return Err("Invalid time-series samples.".into()); };
    if rows.len() > PAGE_SIZE as usize + 1 { return Err("Time-series response exceeds the requested page limit.".into()); }
    let mut previous = None;
    for row in &rows {
        let redis::Value::Array(pair) = row else { return Err("Invalid time-series sample.".into()); };
        if pair.len() != 2 { return Err("Invalid time-series sample width.".into()); }
        let redis::Value::Int(timestamp) = pair[0] else { return Err("Invalid time-series timestamp.".into()); };
        if timestamp < 0 || (timestamp as u64) < start || end.is_some_and(|end| timestamp as u64 > end) || previous.is_some_and(|n| timestamp <= n) { return Err("Time-series timestamps are outside the requested range or unordered.".into()); }
        match &pair[1] {
            redis::Value::BulkString(bytes) if bytes.len() <= 128 && std::str::from_utf8(bytes).ok().and_then(|s| s.parse::<f64>().ok()).is_some() => {},
            redis::Value::SimpleString(value) if value.len() <= 128 && value.parse::<f64>().is_ok() => {},
            redis::Value::Double(_) => {},
            _ => return Err("Invalid time-series sample value.".into()),
        }
        previous = Some(timestamp);
    }
    let more = rows.len() > PAGE_SIZE as usize;
    rows.truncate(PAGE_SIZE as usize);
    let next = if more {
        let redis::Value::Array(pair) = rows.last().unwrap() else { unreachable!() };
        let redis::Value::Int(timestamp) = pair[0] else { unreachable!() };
        Some(Cursor::Timestamp { start: timestamp as u64 + 1, end })
    } else { None };
    Ok((redis::Value::Array(rows), next))
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
    fn cluster_startup_falls_back_and_restores_normal_operation_timeout() {
        use std::{io::Write, net::TcpListener, time::Duration};
        let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = unavailable.local_addr().unwrap().port();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(unavailable);
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            loop {
                let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                match command[0].as_slice() {
                    b"CLIENT" => socket.write_all(b"+OK\r\n").unwrap(),
                    b"PING" => socket.write_all(b"+PONG\r\n").unwrap(),
                    b"CLUSTER" => { assert_eq!(command[1], b"SHARDS"); break; }
                    _ => panic!("unexpected connection command"),
                }
            }
            write!(socket, "*1\r\n*4\r\n+slots\r\n*2\r\n:0\r\n:16383\r\n+nodes\r\n*1\r\n*10\r\n+id\r\n+{}\r\n+role\r\n+master\r\n+health\r\n+online\r\n+endpoint\r\n+127.0.0.1\r\n+port\r\n:{port}\r\n", "a".repeat(40)).unwrap();
            let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
            assert_eq!(command, [b"GET".to_vec(), b"key".to_vec()]);
            std::thread::sleep(Duration::from_millis(500));
            socket.write_all(b"$5\r\nvalue\r\n").unwrap();
        });
        let mut settings = Settings { host: "127.0.0.1".into(), port: dead_port, cluster: true, tls: false,
            connect_timeout: Duration::from_millis(300), read_timeout: Duration::from_secs(2),
            cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: dead_port }], ..Default::default() };
        assert!(Session::connect(&settings, "").err().unwrap().contains("duplicate"));
        settings.cluster_seeds[0].port = port;
        let mut session = Session::connect(&settings, "").unwrap();
        assert_eq!(session.query(redis::cmd("GET").arg("key")).unwrap(), redis::Value::BulkString(b"value".to_vec()));
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "disposable Redis 8.4 scripts/test-redis-vectors.sh fixture"]
    fn live_vector_pages_preserve_binary_elements_and_enforce_byte_limits() {
        use crate::redis_value::{Cursor, PageRequest};
        assert_eq!(std::env::var("RUSTABLE_REDIS_VECTOR_QA").as_deref(), Ok("1"));
        let mut session = Session::connect(&Settings { host: "127.0.0.1".into(), port: 16382, tls: false, ..Default::default() }, "rustable-vector-test-only").unwrap();
        let similarity_key = b"rustable:vector:similarity";
        for (name, x, y) in [(vec![0,255], 1, 0), (b"side".to_vec(), 0, 1), (b"opposite".to_vec(), -1, 0)] {
            session.query(redis::cmd("VADD").arg(similarity_key).arg("VALUES").arg(2).arg(x).arg(y).arg(name)).unwrap();
        }
        let request = crate::redis_diagnostics::Request::VectorSimilar { key: similarity_key.to_vec(), element: vec![0,255] };
        let result = session.diagnostics(&request).unwrap().vector_matches.unwrap();
        assert_eq!(result.rows.iter().map(|row| row.score.as_str()).collect::<Vec<_>>(), ["1.0000", "0.5000", "0.0000"]);
        assert_eq!(result.rows[0].element, [0,255]);
        session.query(redis::cmd("DEL").arg(similarity_key)).unwrap();
        assert!(session.diagnostics(&request).unwrap().vector_matches.unwrap().rows.is_empty());
        let key = b"rustable:vector:pages";
        let mut names: Vec<Vec<u8>> = (0..435).map(|i| format!("member:{i:04}").into_bytes()).collect();
        names.extend([vec![], vec![0], vec![255, 0]]);
        let mut setup = redis::pipe();
        for name in &names { setup.cmd("VADD").arg(key).arg("VALUES").arg(3).arg(1).arg(0).arg(0).arg(name); }
        setup.query::<Vec<redis::Value>>(session.connection.as_mut().unwrap()).unwrap();
        names.sort();
        let mut page = session.inspect_view(key).unwrap();
        assert_eq!(page.snapshot.kind, "vectorset");
        assert_eq!(page.snapshot.length, Some(names.len() as u64));
        let mut seen = Vec::new();
        loop {
            let rows: Vec<Vec<u8>> = redis::from_redis_value(page.snapshot.value).unwrap();
            assert!(rows.len() <= 200);
            seen.extend(rows);
            let Some(cursor) = page.page.next else { break; };
            page = session.value_page(&PageRequest { key: key.to_vec(), kind: "vectorset".into(), cursor, pattern: b"*".to_vec() }).unwrap();
        }
        assert_eq!(seen, names);
        let similar = session.diagnostics(&crate::redis_diagnostics::Request::VectorSimilar { key: key.to_vec(), element: vec![255,0] }).unwrap().vector_matches.unwrap();
        assert_eq!(similar.rows.len(), 20);
        assert!(similar.rows.iter().all(|row| row.score == "1.0000"));
        assert!(session.diagnostics(&crate::redis_diagnostics::Request::VectorSimilar { key: key.to_vec(), element: b"missing".to_vec() }).err().unwrap().contains("element not found"));

        let detail_request = crate::redis_diagnostics::Request::VectorElement { key: key.to_vec(), element: vec![255, 0] };
        let detail = session.diagnostics(&detail_request).unwrap().vector_element.unwrap();
        assert_eq!(detail.components, ["1", "0", "0"]);
        assert!(detail.attributes.is_none());
        let attributes = br#"{"id":9007199254740993,"active":true,"label":"Camera"}"#;
        session.query(redis::cmd("VSETATTR").arg(key).arg([255u8, 0].as_slice()).arg(attributes)).unwrap();
        let detail = session.diagnostics(&detail_request).unwrap().vector_element.unwrap();
        let json = detail.attributes.unwrap();
        assert!(json.json_source.unwrap().contains("9007199254740993"));
        assert!(!json.json_tokens.is_empty());
        session.query(redis::cmd("VSETATTR").arg(key).arg([255u8, 0].as_slice()).arg(format!("\"{}\"", "x".repeat(65536)))).unwrap();
        assert!(session.diagnostics(&detail_request).err().unwrap().contains("64 KiB"));
        assert!(session.diagnostics(&crate::redis_diagnostics::Request::VectorElement { key: key.to_vec(), element: b"missing".to_vec() }).err().unwrap().contains("no longer exists"));

        session.query(redis::cmd("VADD").arg(key).arg("VALUES").arg(3).arg(1).arg(0).arg(0).arg(vec![b'z';65537])).unwrap();
        let request = PageRequest { key: key.to_vec(), kind: "vectorset".into(), cursor: Cursor::Lex(Some(b"member:9999".to_vec())), pattern: b"*".to_vec() };
        assert!(session.value_page(&request).err().unwrap().contains("64 KiB"));
        session.query(redis::cmd("DEL").arg(key)).unwrap();
        session.query(redis::cmd("SET").arg(key).arg("changed")).unwrap();
        assert!(session.value_page(&request).is_err());
        session.query(redis::cmd("DEL").arg(key)).unwrap();
    }
    #[test]
    fn malformed_ordinary_replies_cannot_leak_into_bounded_searches() {
        use std::{io::{Read, Write}, net::TcpListener, time::Duration};
        for pipeline in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut parser = redis::Parser::new();
                loop {
                    let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                    match command[0].as_slice() {
                        b"CLIENT" | b"SELECT" => socket.write_all(b"+OK\r\n").unwrap(),
                        b"PING" => socket.write_all(b"+PONG\r\n").unwrap(),
                        b"GET" => break,
                        _ => panic!("unexpected command"),
                    }
                }
                socket.write_all(b"?malformed\r\n").unwrap();
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).unwrap(), 0);
            });
            let mut session = Session::connect(&Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() }, "").unwrap();
            if pipeline { let mut pipe = redis::pipe(); pipe.cmd("GET").arg("key"); assert!(session.pipeline(&pipe).is_err()); }
            else { assert!(session.query(redis::cmd("GET").arg("key")).is_err()); }
            assert!(session.connection.is_none());
            assert!(session.query_bounded(redis::cmd("VSIM").arg("key").arg("ELE").arg("source")).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn bounded_responses_preserve_one_exchange_and_close_on_limit() {
        use std::{io::{Read, Write}, net::TcpListener, time::Duration};
        let exact = b"$3\r\nabc\r\n";
        assert_eq!(redis::Parser::new().parse_value_bounded(exact.as_slice(), exact.len()).unwrap(), redis::Value::BulkString(b"abc".to_vec()));
        assert!(redis::Parser::new().parse_value_bounded(exact.as_slice(), exact.len()-1).unwrap_err().to_string().contains("byte limit"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            let read = |socket: &mut std::net::TcpStream, parser: &mut redis::Parser| -> Vec<Vec<u8>> { redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap() };
            loop {
                let command = read(&mut socket, &mut parser);
                match command[0].as_slice() {
                    b"CLIENT" | b"SELECT" => socket.write_all(b"+OK\r\n").unwrap(),
                    b"ASKING" => break,
                    _ => panic!("unexpected startup command"),
                }
            }
            assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
            // Both requests arrive before either response, even with read limits.
            socket.write_all(b"+OK\r\n*2\r\n$3\r\none\r\n$3\r\n0.9\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
            socket.write_all(b"-NOPERM search denied\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
            socket.write_all(b"$1000000\r\n").unwrap();
            let _ = socket.write_all(&[b'x';256]);
            // No attempt to reuse the partially consumed reply or replay VSIM.
            let mut byte = [0];
            match socket.read(&mut byte) { Ok(0) => {}, Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::UnexpectedEof) => {}, result => panic!("unexpected request after limit: {result:?}") }
        });
        let mut connection = Session::raw_node(&Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() }, "").unwrap();
        let mut pipeline = redis::pipe();
        pipeline.cmd("ASKING").cmd("VSIM").arg("vectors").arg("ELE").arg("one").arg("COUNT").arg(20);
        let replies = connection.req_packed_commands_bounded(&pipeline.get_packed_pipeline(), &[128,1024]).unwrap();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0], redis::Value::Okay);
        assert_eq!(redis::cmd("PING").query::<String>(&mut connection).unwrap(), "PONG");
        let command = redis::cmd("VSIM").arg("vectors").arg("ELE").arg("one").get_packed_command();
        let error = connection.req_packed_commands_bounded(&command, &[128]).unwrap().pop().unwrap().extract_error().unwrap_err();
        assert!(error.to_string().contains("search denied"), "{error}");
        let error = connection.req_packed_commands_bounded(&command, &[128]).unwrap_err();
        assert!(error.to_string().contains("byte limit"));
        assert!(!redis::ConnectionLike::is_open(&connection));
        server.join().unwrap();
    }
    #[test]
    fn vector_page_requests_preserve_pipelined_exchange_budget() {
        use std::{io::Write, net::TcpListener, time::Duration};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            let read = |socket: &mut std::net::TcpStream, parser: &mut redis::Parser| -> Vec<Vec<u8>> { redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap() };
            loop {
                let command = read(&mut socket, &mut parser);
                match command[0].as_slice() {
                    b"CLIENT" | b"SELECT" => socket.write_all(b"+OK\r\n").unwrap(),
                    b"PING" => socket.write_all(b"+PONG\r\n").unwrap(),
                    b"TYPE" => break,
                    _ => panic!("unexpected startup command"),
                }
            }
            assert_eq!(read(&mut socket, &mut parser)[0], b"PTTL");
            socket.write_all(b"$9\r\nvectorset\r\n:-1\r\n").unwrap();
            for (index, boundary) in [b"-".as_slice(), b"(a".as_slice()].into_iter().enumerate() {
                if index == 1 {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"TYPE");
                    assert_eq!(read(&mut socket, &mut parser)[0], b"PTTL");
                }
                assert_eq!(read(&mut socket, &mut parser), [b"VCARD".to_vec(), b"series".to_vec()]);
                let command = read(&mut socket, &mut parser);
                assert_eq!(command, [b"EVAL_RO".to_vec(), crate::redis_vectors::PAGE_SCRIPT.as_bytes().to_vec(), b"1".to_vec(), b"series".to_vec(), boundary.to_vec()]);
                // The complete page pipeline must arrive before any response.
                if index == 1 { socket.write_all(b"$9\r\nvectorset\r\n:-1\r\n").unwrap(); }
                socket.write_all(b":1\r\n*2\r\n*1\r\n$1\r\nb\r\n:0\r\n").unwrap();
            }
            let detail = read(&mut socket, &mut parser);
            assert_eq!(detail, [b"EVAL_RO".to_vec(), crate::redis_vector_element::SCRIPT.as_bytes().to_vec(), b"1".to_vec(), b"series".to_vec(), vec![0,255]]);
            socket.write_all(b"-NOPERM vector detail denied\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let mut session = Session::connect(&Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() }, "").unwrap();
        assert_eq!(session.inspect_view(b"series").unwrap().page.display.rows.len(), 1);
        assert_eq!(session.value_page(&crate::redis_value::PageRequest { key: b"series".to_vec(), kind: "vectorset".into(), cursor: crate::redis_value::Cursor::Lex(Some(b"a".to_vec())), pattern: b"*".to_vec() }).unwrap().page.display.rows.len(), 1);
        let error = session.diagnostics(&crate::redis_diagnostics::Request::VectorElement { key: b"series".to_vec(), element: vec![0,255] }).err().unwrap();
        assert!(error.contains("NOPERM"));
        session.query(&redis::cmd("PING")).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn timeseries_page_requests_preserve_pipelined_exchange_budget() {
        use std::{io::Write, net::TcpListener, time::Duration};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            let read = |socket: &mut std::net::TcpStream, parser: &mut redis::Parser| -> Vec<Vec<u8>> { redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap() };
            loop {
                let command = read(&mut socket, &mut parser);
                match command[0].as_slice() {
                    b"CLIENT" | b"SELECT" => socket.write_all(b"+OK\r\n").unwrap(),
                    b"PING" => socket.write_all(b"+PONG\r\n").unwrap(),
                    b"TYPE" => break,
                    _ => panic!("unexpected startup command"),
                }
            }
            assert_eq!(read(&mut socket, &mut parser)[0], b"PTTL");
            socket.write_all(b"$9\r\nTSDB-TYPE\r\n:-1\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser), ["TS.RANGE", "series", "0", "+", "COUNT", "201"].map(|s| s.as_bytes().to_vec()));
            socket.write_all(b"*1\r\n*2\r\n:0\r\n$1\r\n1\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"TYPE");
            assert_eq!(read(&mut socket, &mut parser)[0], b"PTTL");
            assert_eq!(read(&mut socket, &mut parser), ["TS.RANGE", "series", "1", "+", "COUNT", "201"].map(|s| s.as_bytes().to_vec()));
            // All three follow-up requests arrive before any reply.
            socket.write_all(b"$9\r\nTSDB-TYPE\r\n:-1\r\n*0\r\n").unwrap();
            let metadata = read(&mut socket, &mut parser);
            assert_eq!(metadata[0], b"EVAL_RO");
            assert_eq!(metadata[1], crate::redis_module_info::TIMESERIES_SCRIPT.as_bytes());
            assert_eq!(&metadata[2..], &[b"1".to_vec(), b"series".to_vec()]);
            socket.write_all(b"-NOPERM metadata denied\r\n").unwrap();
            // Permission errors must not trigger an uncapped TS.INFO fallback.
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let mut session = Session::connect(&Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() }, "").unwrap();
        assert_eq!(session.inspect_view(b"series").unwrap().page.display.rows.len(), 1);
        assert!(session.value_page(&crate::redis_value::PageRequest { key: b"series".to_vec(), kind: "TSDB-TYPE".into(), cursor: crate::redis_value::Cursor::Timestamp { start: 1, end: None }, pattern: b"*".to_vec() }).unwrap().page.display.rows.is_empty());
        let denied = session.diagnostics(&crate::redis_diagnostics::Request::TimeSeries(b"series".to_vec())).err().unwrap();
        assert!(denied.contains("NOPERM"));
        session.query(&redis::cmd("PING")).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn timeseries_pages_validate_order_bounds_and_lookahead() {
        use crate::redis_value::{Cursor, PAGE_SIZE};
        let row = |timestamp| redis::Value::Array(vec![redis::Value::Int(timestamp), redis::Value::BulkString(b"1.25".to_vec())]);
        let (value, next) = timeseries_page(redis::Value::Array((0..=PAGE_SIZE).map(|n| row((n * 10) as i64)).collect()), 0, None).unwrap();
        assert!(matches!(value, redis::Value::Array(rows) if rows.len() == PAGE_SIZE as usize));
        assert_eq!(next, Some(Cursor::Timestamp { start: 1991, end: None }));
        assert!(timeseries_page(redis::Value::Array(vec![row(1), row(1)]), 0, None).is_err());
        assert!(timeseries_page(redis::Value::Array(vec![row(0)]), 1, None).is_err());
        assert!(timeseries_page(redis::Value::Array(vec![row(-1)]), 0, None).is_err());
        assert!(timeseries_page(redis::Value::Array((0..=PAGE_SIZE+1).map(|n| row(n as i64)).collect()), 0, None).is_err());
        assert_eq!(timeseries_page(redis::Value::Array(vec![row(i64::MAX)]), i64::MAX as u64, None).unwrap().1, None);
        assert_eq!(timeseries_page(redis::Value::Array(vec![]), 0, None).unwrap().1, None);
    }
    #[test]
    #[ignore = "disposable six-node scripts/test-redis-cluster-failover.sh fixture"]
    fn live_cluster_replica_promotion_recovers_after_all_old_primaries_stop() {
        use std::time::{Duration, Instant};
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_FAILOVER_QA").as_deref(), Ok("1"));
        let password = "rustable-cluster-test-only";
        let settings = Settings { host: "127.0.0.1".into(), port: 16600, tls: false, cluster: true, ..Default::default() };
        let mut nodes: Vec<_> = (16600..=16605).map(|port| Session::raw_node(&Settings { port, ..settings.clone() }, password).unwrap()).collect();
        let mut replicas = Vec::new();
        for (index, node) in nodes.iter_mut().enumerate() {
            let role: Vec<redis::Value> = redis::cmd("ROLE").query(node).unwrap();
            if role[0] == redis::Value::BulkString(b"slave".to_vec()) {
                let primary: u16 = redis::from_redis_value(role[2].clone()).unwrap();
                assert!((16600..=16602).contains(&primary));
                replicas.push((primary, index));
            } else { assert!(index < 3); }
        }
        assert_eq!(replicas.len(), 3);
        let mut session = Session::connect(&settings, password).unwrap();
        let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(&mut nodes[0]).unwrap(), "127.0.0.1", false).unwrap();
        let routes = topology.routes();
        let key = (0..10000).map(|i| format!("rustable:promotion:{i}")).find(|key| {
            let slot = crate::redis_cluster::command_slot(redis::cmd("GET").arg(key)).unwrap().unwrap();
            routes[slot as usize].as_ref().is_some_and(|e| e.port == 16600)
        }).unwrap();
        redis::cmd("SET").arg(&key).arg(0).query::<()>(&mut nodes[0]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let replicated: u64 = redis::cmd("WAIT").arg(1).arg(1000).query(&mut nodes[0]).unwrap();
            if replicated >= 1 { break; }
            assert!(Instant::now() < deadline, "fixture replica did not catch up");
        }
        let data_replica = replicas.iter().find(|(primary, _)| *primary == 16600).unwrap().1;
        redis::cmd("READONLY").query::<()>(&mut nodes[data_replica]).unwrap();
        assert_eq!(redis::cmd("GET").arg(&key).query::<redis::Value>(&mut nodes[data_replica]).unwrap(), redis::Value::BulkString(b"0".to_vec()), "replicated value before promotion");
        for &(primary, replica) in &replicas {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let role: Vec<redis::Value> = redis::cmd("ROLE").query(&mut nodes[replica]).unwrap();
                if role.get(3) == Some(&redis::Value::BulkString(b"connected".to_vec())) { break; }
                assert!(Instant::now() < deadline, "replica link not ready for planned promotion");
                std::thread::sleep(Duration::from_millis(50));
            }
            let representative_slot = routes.iter().position(|e| e.as_ref().is_some_and(|e| e.port == primary)).unwrap();
            redis::cmd("CLUSTER").arg("FAILOVER").query::<()>(&mut nodes[replica]).unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let role: Vec<redis::Value> = redis::cmd("ROLE").query(&mut nodes[replica]).unwrap();
                let old_role: Vec<redis::Value> = redis::cmd("ROLE").query(&mut nodes[(primary - 16600) as usize]).unwrap();
                if role[0] == redis::Value::BulkString(b"master".to_vec()) && old_role[0] == redis::Value::BulkString(b"slave".to_vec()) {
                    let agreed = nodes.iter_mut().all(|node| {
                        let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(node).unwrap(), "127.0.0.1", false);
                        topology.is_ok_and(|topology| topology.routes()[representative_slot].as_ref().is_some_and(|e| e.port == 16600 + replica as u16))
                    });
                    if agreed { break; }
                }
                assert!(Instant::now() < deadline, "manual promotion did not complete: primary {primary}, replica {}, roles {role:?} / {old_role:?}", 16600 + replica);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let converged = nodes.iter_mut().all(|node| {
                let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(node).unwrap(), "127.0.0.1", false);
                topology.is_ok_and(|topology| topology.routes().iter().all(|e| e.as_ref().is_some_and(|e| (16603..=16605).contains(&e.port))))
            });
            if converged { break; }
            assert!(Instant::now() < deadline, "promoted routes did not converge");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(redis::cmd("GET").arg(&key).query::<redis::Value>(&mut nodes[data_replica]).unwrap(), redis::Value::BulkString(b"0".to_vec()), "promoted owner before old-primary shutdown");
        for node in &mut nodes[..3] { let _ = redis::cmd("SHUTDOWN").arg("NOSAVE").query::<redis::Value>(node); }
        assert_eq!(redis::cmd("GET").arg(&key).query::<redis::Value>(&mut nodes[data_replica]).unwrap(), redis::Value::BulkString(b"0".to_vec()), "promoted owner after old-primary shutdown");
        assert!(session.query(redis::cmd("INCR").arg(&key)).is_err());
        session.reconnect(&settings, password, "").unwrap();
        assert_eq!(session.query(redis::cmd("GET").arg(&key)).unwrap(), redis::Value::BulkString(b"0".to_vec()));
        assert_eq!(session.query(redis::cmd("INCR").arg(&key)).unwrap(), redis::Value::Int(1));
        session.query(redis::cmd("DEL").arg(&key)).unwrap();
    }
    #[test]
    #[ignore = "destructive final check in disposable scripts/test-redis-cluster.sh fixture"]
    fn live_cluster_reconnect_uses_discovered_node_after_seed_shutdown() {
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_RECOVERY_QA").as_deref(), Ok("1"));
        let settings = Settings { host: "127.0.0.1".into(), port: 16400, tls: false, cluster: true, ..Default::default() };
        let mut session = Session::connect(&settings, "rustable-cluster-test-only").unwrap();
        let mut shutdown = Session::raw_node(&settings, "rustable-cluster-test-only").unwrap();
        let _ = redis::cmd("SHUTDOWN").arg("NOSAVE").query::<redis::Value>(&mut shutdown);
        assert!(session.query(&redis::cmd("PING")).is_err());
        assert!(!session.is_connected());
        session.reconnect(&settings, "rustable-cluster-test-only", "").unwrap();
        assert!(session.is_connected());
        assert_eq!(session.query(&redis::cmd("PING")).unwrap(), redis::Value::SimpleString("PONG".into()));
        let startup = Settings { cluster_seeds: vec![crate::redis_settings::SentinelSeed { host: "127.0.0.1".into(), port: 16401 }], ..settings };
        let mut fresh = Session::connect(&startup, "rustable-cluster-test-only").unwrap();
        assert_eq!(fresh.query(&redis::cmd("PING")).unwrap(), redis::Value::SimpleString("PONG".into()));
        for name in ["MONITOR", "SUBSCRIBE", "PSUBSCRIBE", "PUBSUB"] {
            let control = crate::redis_stream::Control::default();
            let mut command = redis::cmd(name);
            if name != "MONITOR" { command.arg(if name == "PUBSUB" { "CHANNELS" } else { "rustable:seed-fallback" }); }
            let mut reader = control.start(&startup, "rustable-cluster-test-only", "", &command).unwrap();
            let reply = redis::Parser::new().parse_value(&mut reader).unwrap();
            if name == "MONITOR" { assert_eq!(reply, redis::Value::Okay); }
            else { assert!(matches!(reply, redis::Value::Array(_))); }
            drop(reader);
            control.release();
        }
        // This establishes a transport through a surviving node, not replica failover.
    }
    #[test]
    #[ignore = "RUSTABLE_REDIS_CLUSTER_VECTOR_QA=1 scripts/test-redis-cluster.sh"]
    fn live_cluster_vectors_route_previews_and_pages_across_primaries() {
        use crate::redis_value::PageRequest;
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_VECTOR_QA").as_deref(), Ok("1"));
        let settings = Settings { host: "127.0.0.1".into(), port: 16400, tls: false, cluster: true, ..Default::default() };
        let mut seed = Session::raw_node(&Settings { cluster: false, ..settings.clone() }, "rustable-cluster-test-only").unwrap();
        let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(&mut seed).unwrap(), &settings.host, false).unwrap();
        let routes = topology.routes();
        let mut keys = Vec::new();
        let mut ports = std::collections::HashSet::new();
        for index in 0..1000 {
            let key = format!("rustable:vector-cluster:{}:{index}", std::process::id()).into_bytes();
            let slot = crate::redis_cluster::command_slot(redis::cmd("VCARD").arg(&key)).unwrap().unwrap();
            let endpoint = routes[slot as usize].as_ref().unwrap();
            assert_eq!(endpoint.host, "127.0.0.1");
            assert!([16400, 16401, 16402].contains(&endpoint.port));
            if !ports.insert(endpoint.port) { continue; }
            let mut owner = Session::raw_node(&Settings { port: endpoint.port, cluster: false, ..settings.clone() }, "rustable-cluster-test-only").unwrap();
            let mut setup = redis::pipe();
            for index in 0..205 { setup.cmd("VADD").arg(&key).arg("VALUES").arg(3).arg(1).arg(0).arg(0).arg(format!("member:{index:04}")); }
            setup.query::<Vec<redis::Value>>(&mut owner).unwrap();
            keys.push(key);
            if ports.len() == 3 { break; }
        }
        assert_eq!(ports.len(), 3);
        let mut session = Session::connect(&settings, "rustable-cluster-test-only").unwrap();
        for _ in 0..2 {
            let previews = session.key_rows(&keys).unwrap();
            assert_eq!(previews.len(), 3);
            for (row, key) in previews.iter().zip(&keys) {
                assert_eq!(&row.key, key);
                assert_eq!(row.kind, "vectorset");
                assert_eq!(row.value, redis::Value::Int(205));
                let similar = session.diagnostics(&crate::redis_diagnostics::Request::VectorSimilar { key: key.clone(), element: b"member:0000".to_vec() }).unwrap().vector_matches.unwrap();
                assert_eq!(similar.rows.len(), 20);

                let detail = session.diagnostics(&crate::redis_diagnostics::Request::VectorElement { key: key.clone(), element: b"member:0000".to_vec() }).unwrap().vector_element.unwrap();
                assert_eq!(detail.components, ["1", "0", "0"]);

                let first = session.inspect_view(key).unwrap();
                assert_eq!(first.page.display.rows.len(), 200);
                let second = session.value_page(&PageRequest { key: key.clone(), kind: "vectorset".into(), cursor: first.page.next.unwrap(), pattern: b"*".to_vec() }).unwrap();
                assert_eq!(second.page.display.rows.len(), 5);
                assert_eq!(second.page.display.rows[0].cells[0], "member:0200");
                assert!(second.page.next.is_none());
            }
        }
        for key in &keys { session.delete(key, false).unwrap(); }
    }
    #[test]
    #[ignore = "RUSTABLE_REDIS_CLUSTER_MODULE_QA=1 scripts/test-redis-cluster.sh"]
    fn live_cluster_timeseries_routes_previews_pages_and_ranges_across_primaries() {
        use crate::redis_value::{Cursor, PageRequest};
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_MODULE_QA").as_deref(), Ok("1"));
        let settings = Settings { host: "127.0.0.1".into(), port: 16400, tls: false, cluster: true, ..Default::default() };
        let mut seed = Session::raw_node(&Settings { cluster: false, ..settings.clone() }, "rustable-cluster-test-only").unwrap();
        let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(&mut seed).unwrap(), &settings.host, false).unwrap();
        let routes = topology.routes();
        let mut keys = Vec::new();
        let mut ports = std::collections::HashSet::new();
        for index in 0..1000 {
            let key = format!("rustable:ts-cluster:{}:{index}", std::process::id()).into_bytes();
            let slot = crate::redis_cluster::command_slot(redis::cmd("TS.GET").arg(&key)).unwrap().unwrap();
            let endpoint = routes[slot as usize].as_ref().unwrap();
            assert_eq!(endpoint.host, "127.0.0.1");
            assert!([16400, 16401, 16402].contains(&endpoint.port));
            if !ports.insert(endpoint.port) { continue; }
            let mut owner = Session::raw_node(&Settings { port: endpoint.port, cluster: false, ..settings.clone() }, "rustable-cluster-test-only").unwrap();
            let mut setup = redis::pipe();
            for timestamp in 0..205 { setup.cmd("TS.ADD").arg(&key).arg(timestamp).arg(timestamp as f64 / 10.0); }
            setup.query::<Vec<redis::Value>>(&mut owner).unwrap();
            keys.push(key);
            if ports.len() == 3 { break; }
        }
        assert_eq!(ports.len(), 3);
        let mut session = Session::connect(&settings, "rustable-cluster-test-only").unwrap();
        for _ in 0..2 {
            let previews = session.key_rows(&keys).unwrap();
            assert_eq!(previews.len(), 3);
            for (row, key) in previews.iter().zip(&keys) {
                assert_eq!(&row.key, key);
                assert_eq!(row.kind, "TSDB-TYPE");
                assert!(matches!(&row.value, redis::Value::Array(sample) if sample[0] == redis::Value::Int(204)));
                let first = session.inspect_view(key).unwrap();
                assert_eq!(first.page.display.rows.len(), 200);
                let info = session.diagnostics(&crate::redis_diagnostics::Request::TimeSeries(key.clone())).unwrap();
                assert!(info.summary.contains(&("Total samples".into(), "205".into())));
                let second = session.value_page(&PageRequest { key: key.clone(), kind: "TSDB-TYPE".into(), cursor: first.page.next.unwrap(), pattern: b"*".to_vec() }).unwrap();
                assert_eq!(second.page.display.rows.len(), 5);
                assert_eq!(second.page.display.rows[0].cells[0], "200");
                assert!(second.page.next.is_none());
                let ranged = session.value_page(&PageRequest { key: key.clone(), kind: "TSDB-TYPE".into(), cursor: Cursor::Timestamp { start: 50, end: Some(100) }, pattern: b"*".to_vec() }).unwrap();
                assert_eq!(ranged.page.display.rows.len(), 51);
                assert_eq!(ranged.page.display.rows.last().unwrap().cells[0], "100");
                assert!(ranged.page.next.is_none());
            }
        }
        for key in &keys { session.delete(key, false).unwrap(); }
    }
    #[test]
    #[ignore = "requires disposable Redis 8.2 fixture on RUSTABLE_REDIS_JSON_PORT"]
    fn live_timeseries_pagination_retains_samples_and_detects_type_changes() {
        use crate::redis_value::{Cursor, PageRequest};
        let port: u16 = std::env::var("RUSTABLE_REDIS_JSON_PORT").expect("Use scripts/test-redis.sh").parse().unwrap();
        assert_eq!(port, 16381);
        let mut session = Session::connect(&Settings { port, ..settings() }, "rustable-redis-test-only").unwrap();
        let key = format!("rustable:timeseries:{}", std::process::id());
        session.query(redis::cmd("DEL").arg(&key)).unwrap();
        let compacted = format!("{key}:rollup");
        session.query(redis::cmd("DEL").arg(&compacted)).unwrap();
        let mut create = redis::cmd("TS.CREATE");
        create.arg(&key).arg("RETENTION").arg(10000).arg("LABELS");
        for index in 0..70 { create.arg(format!("label-{index}")).arg("x".repeat(400)); }
        session.query(&create).unwrap();
        session.query(redis::cmd("TS.CREATE").arg(&compacted)).unwrap();
        session.query(redis::cmd("TS.CREATERULE").arg(&key).arg(&compacted).arg("AGGREGATION").arg("avg").arg(1000)).unwrap();
        let mut setup = redis::pipe();
        for index in 0..435 { setup.cmd("TS.ADD").arg(&key).arg(index * 10).arg(index as f64 / 10.0); }
        setup.query::<Vec<redis::Value>>(session.connection.as_mut().unwrap()).unwrap();
        let info = session.diagnostics(&crate::redis_diagnostics::Request::TimeSeries(key.as_bytes().to_vec())).unwrap();
        assert!(info.summary.contains(&("Total samples".into(), "435".into())));
        assert!(info.summary.contains(&("Retention (ms)".into(), "10000".into())));
        assert_eq!(info.rows.iter().filter(|row| row[0] == "Label").count(), 64);
        assert!(info.rows.iter().any(|row| row[0] == "Rule" && row[1] == compacted && row[2].contains("1000 ms")));
        assert!(info.footer.contains("Omitted: 6 labels"));
        assert!(info.footer.contains("64 shortened fields"));
        assert!(info.rows.iter().all(|row| row.iter().all(|cell| cell.len() <= 1024)));
        session.query(redis::cmd("DEL").arg(&compacted)).unwrap();
        let mut page = session.inspect_view(key.as_bytes()).unwrap();
        assert_eq!(page.snapshot.kind, "TSDB-TYPE");
        assert_eq!(page.page.display.headers, vec!["TIMESTAMP (ms)", "VALUE"]);
        let mut timestamps = Vec::new();
        loop {
            assert!(page.page.display.rows.len() <= 200);
            timestamps.extend(page.page.display.rows.iter().map(|row| row.cells[0].parse::<u64>().unwrap()));
            let Some(cursor) = page.page.next else { break; };
            page = session.value_page(&PageRequest { key: key.as_bytes().to_vec(), kind: "TSDB-TYPE".into(), cursor, pattern: b"*".to_vec() }).unwrap();
        }
        assert_eq!(timestamps, (0..435).map(|index| index * 10).collect::<Vec<_>>());
        let mut request = PageRequest { key: key.as_bytes().to_vec(), kind: "TSDB-TYPE".into(), cursor: Cursor::Timestamp { start: 1001, end: Some(4000) }, pattern: b"*".to_vec() };
        let mut ranged = Vec::new();
        loop {
            let page = session.value_page(&request).unwrap();
            ranged.extend(page.page.display.rows.iter().map(|row| row.cells[0].parse::<u64>().unwrap()));
            let Some(next) = page.page.next else { break; };
            assert!(matches!(next, Cursor::Timestamp { end: Some(4000), .. }));
            request.cursor = next;
        }
        assert_eq!(ranged, (101..=400).map(|index| index * 10).collect::<Vec<_>>());
        let rows = session.key_rows(&[key.as_bytes().to_vec()]).unwrap();
        assert!(matches!(&rows[0].value, redis::Value::Array(sample) if sample.first() == Some(&redis::Value::Int(4340))));
        session.query(redis::cmd("DEL").arg(&key)).unwrap();
        session.query(redis::cmd("TS.CREATE").arg(&key)).unwrap();
        let empty = session.inspect_view(key.as_bytes()).unwrap();
        assert!(empty.page.display.rows.is_empty());
        assert!(empty.page.next.is_none());
        session.query(redis::cmd("DEL").arg(&key)).unwrap();
        session.query(redis::cmd("SET").arg(&key).arg("changed type")).unwrap();
        assert!(session.value_page(&PageRequest { key: key.as_bytes().to_vec(), kind: "TSDB-TYPE".into(), cursor: Cursor::Timestamp { start: 0, end: None }, pattern: b"*".to_vec() }).is_err());
        session.query(redis::cmd("DEL").arg(&key)).unwrap();
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
    #[ignore = "requires scripts/test-redis-mtls.sh disposable fixture"]
    fn live_mutual_tls_client_identity() {
        assert_eq!(std::env::var("RUSTABLE_REDIS_MTLS_QA").as_deref(), Ok("1"));
        let directory = std::path::PathBuf::from(std::env::var_os("RUSTABLE_REDIS_MTLS_CERTS").unwrap());
        let settings = Settings {
            host: "127.0.0.1".into(),
            port: std::env::var("RUSTABLE_REDIS_MTLS_PORT").unwrap().parse().unwrap(),
            tls: true,
            root_certificate: Some(directory.join("ca.crt")),
            client_certificate: Some(directory.join("client.crt")),
            client_key: Some(directory.join("client.key")),
            ..Default::default()
        };
        // A forwarded endpoint changes only the dial address, never TLS identity.
        for (host, succeeds) in [("127.0.0.1", true), ("localhost", false)] {
            let info = redis::ConnectionAddr::TcpTls { host: host.into(), port: 1, insecure: false, tls_params: None }
                .into_connection_info().unwrap()
                .set_redis_settings(redis::RedisConnectionInfo::default().set_password("rustable-mtls-test-only"));
            let client = redis::Client::build_with_tls(info, redis::TlsCertificates {
                root_cert: Some(std::fs::read(directory.join("ca.crt")).unwrap()),
                client_tls: Some(redis::ClientTlsConfig {
                    client_cert: std::fs::read(directory.join("client.crt")).unwrap(),
                    client_key: std::fs::read(directory.join("client.key")).unwrap(),
                }),
            }).unwrap();
            let result = client.get_connection_via(std::net::SocketAddr::from(([127,0,0,1], settings.port)), settings.connect_timeout);
            assert_eq!(result.is_ok(), succeeds, "forwarded TLS identity {host}");
        }
        let mut session = Session::connect(&settings, "rustable-mtls-test-only").unwrap();
        for _ in 0..2 {
            assert_eq!(session.query(&redis::cmd("PING")).unwrap(), redis::Value::SimpleString("PONG".into()));
        }
        let control = crate::redis_stream::Control::default();
        let mut stream = control.connect(&settings).unwrap();
        use std::io::{Read, Write};
        stream.write_all(&redis::cmd("AUTH").arg("rustable-mtls-test-only").get_packed_command()).unwrap();
        let mut okay = [0; 5];
        stream.read_exact(&mut okay).unwrap();
        assert_eq!(&okay, b"+OK\r\n");
        control.stop();
        drop(stream);
        // Node and raw constructors must also preserve the client identity.
        assert!(Session::cluster_node(&settings, "rustable-mtls-test-only").is_ok());
        assert!(Session::raw_node(&settings, "rustable-mtls-test-only").is_ok());
        for variant in 0..6 {
            let mut invalid = settings.clone();
            match variant {
                0 => { invalid.client_certificate = None; invalid.client_key = None; }
                1 => { invalid.client_certificate = Some(directory.join("untrusted.crt")); invalid.client_key = Some(directory.join("untrusted.key")); }
                2 => invalid.host = "localhost".into(),
                3 => invalid.root_certificate = Some(directory.join("other-ca.crt")),
                4 => invalid.client_key = Some(directory.join("untrusted.key")),
                _ => invalid.client_key = Some(directory.join("missing.key")),
            }
            assert!(crate::redis_stream::Control::default().connect(&invalid).is_err(), "invalid streaming identity variant {variant} connected");
            assert!(Session::connect(&invalid, "rustable-mtls-test-only").is_err(), "invalid identity variant {variant} connected");
        }
        let mut oversized = settings.clone();
        let file = directory.join("oversized.pem");
        std::fs::write(&file, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        oversized.client_key = Some(file);
        assert!(Session::connect(&oversized, "rustable-mtls-test-only").err().unwrap().contains("1 MiB"));
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
            let rows = session.key_rows(&[key.as_bytes().to_vec()]).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].kind, kind);
            assert_eq!(rows[0].ttl_ms, -1);
            assert_ne!(rows[0].value, redis::Value::Nil);
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

#[cfg(test)]
#[path = "redis_browser_tests.rs"]
mod browser_tests;
