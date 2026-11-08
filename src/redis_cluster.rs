//! Explicit cluster redirects; transport failures never replay a submitted command.
use crate::{redis_settings::Settings, redis_topology::Endpoint};
use redis::ConnectionLike;
use std::collections::{HashMap, VecDeque};
const MAX_CONNECTIONS: usize = 64;
// Matches the largest bounded caller: loaded-key memory diagnostics.
const MAX_READ_COMMANDS: usize = 1000;
pub(crate) struct Router {
    settings: Settings,
    password: String,
    seed: Endpoint,
    discovery_nodes: Vec<Endpoint>,
    configured_nodes: Vec<Endpoint>,
    routes: Vec<Option<Endpoint>>,
    connections: HashMap<Endpoint, redis::Connection>,
    recent: VecDeque<Endpoint>,
    stopped: bool,
    scan: Option<Scan>,
    scan_token: u64,
}
pub(crate) enum WriteState<T> {
    NotAttempted,
    Unknown,
    Confirmed(T),
}
pub(crate) struct WriteOutcome<T> {
    pub key: Vec<u8>,
    pub state: WriteState<T>,
    pub message: String,
}
pub(crate) struct WriteReport<T> {
    pub outcomes: Vec<WriteOutcome<T>>,
    pub stopped: Option<String>,
}
struct Scan {
    nodes: VecDeque<(Endpoint, u64)>,
    pattern: Vec<u8>,
    kind: String,
    token: u64,
}
type ReadGroup = (Endpoint, Vec<usize>, Vec<u8>);
type Rejected = (usize, Endpoint, redis::RedisError);
impl Router {
    pub fn new(
        settings: Settings,
        password: String,
        connection: redis::Connection,
        routes: Vec<Option<Endpoint>>,
    ) -> Self {
        let seed = Endpoint {
            host: settings.host.clone(),
            port: settings.port,
        };
        Self {
            settings,
            password,
            seed: seed.clone(),
            discovery_nodes: Vec::new(),
            configured_nodes: Vec::new(),
            routes,
            connections: HashMap::from([(seed.clone(), connection)]),
            recent: VecDeque::from([seed]),
            stopped: false,
            scan: None,
            scan_token: 0,
        }
    }
    pub fn restore_timeouts(&mut self, settings: &Settings) -> Result<(), String> {
        for connection in self.connections.values() {
            connection.set_read_timeout(Some(settings.read_timeout)).map_err(|e| e.to_string())?;
            connection.set_write_timeout(Some(settings.read_timeout)).map_err(|e| e.to_string())?;
        }
        self.settings.connect_timeout = settings.connect_timeout;
        self.settings.read_timeout = settings.read_timeout;
        Ok(())
    }
    pub fn remember_seeds(&mut self, settings: &Settings) {
        self.configured_nodes = std::iter::once(Endpoint { host: settings.host.clone(), port: settings.port })
            .chain(settings.cluster_seeds.iter().map(|seed| Endpoint { host: seed.host.clone(), port: seed.port }))
            .take(8).collect();
    }
    pub fn remember_discovery(&mut self, topology: &crate::redis_topology::Cluster) {
        self.discovery_nodes = topology.reconnect_nodes();
    }
    pub fn is_connected(&self) -> bool {
        !self.stopped
    }
    /// Rebuild transport/topology only. The failed user operation is never retained.
    pub fn reconnect(&mut self) -> Result<Endpoint, String> {
        use std::time::{Duration, Instant};
        self.stopped = true;
        self.connections.clear();
        self.recent.clear();
        self.scan = None;
        let mut candidates = vec![self.seed.clone()];
        for endpoint in self.configured_nodes.iter().chain(&self.discovery_nodes).chain(self.routes.iter().flatten()) {
            if !candidates.contains(endpoint) { candidates.push(endpoint.clone()); }
            if candidates.len() == 8 { break; }
        }
        let deadline = Instant::now() + self.settings.connect_timeout;
        let mut last = String::from("No Cluster node available");
        for (index, endpoint) in candidates.iter().enumerate() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { break; }
            // Reserve time for alternate discovered nodes if one address stalls.
            let budget = (remaining / (candidates.len() - index) as u32).min(Duration::from_millis(500));
            let settings = Settings { host: endpoint.host.clone(), port: endpoint.port,
                connect_timeout: budget, read_timeout: budget, ..self.settings.clone() };
            let attempt = (|| {
                let mut connection = crate::redis_session::Session::raw_node(&settings, &self.password)?;
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() { return Err("Cluster reconnect deadline reached".into()); }
                let timeout = remaining.min(budget);
                connection.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
                connection.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
                // Discovery itself verifies command access; no PING preflight.
                let value = redis::cmd("CLUSTER").arg("SHARDS").query::<redis::Value>(&mut connection).map_err(|e| e.to_string())?;
                let topology = crate::redis_topology::Cluster::decode(value, &endpoint.host, settings.tls)?;
                if Instant::now() >= deadline { return Err("Cluster reconnect deadline reached".into()); }
                connection.set_read_timeout(Some(self.settings.read_timeout)).map_err(|e| e.to_string())?;
                connection.set_write_timeout(Some(self.settings.read_timeout)).map_err(|e| e.to_string())?;
                Ok((connection, topology))
            })();
            match attempt {
                Ok((connection, topology)) => {
                    self.seed = endpoint.clone();
                    self.routes = topology.routes();
                    self.remember_discovery(&topology);
                    self.connections.insert(endpoint.clone(), connection);
                    self.recent.push_back(endpoint.clone());
                    self.stopped = false;
                    return Ok(endpoint.clone());
                }
                Err(error) => last = error,
            }
        }
        Err(format!("Cluster reconnect failed across up to {} known nodes: {last}. No operation was replayed.", candidates.len()))
    }
    fn connection(&mut self, endpoint: &Endpoint) -> Result<&mut redis::Connection, String> {
        if self.stopped {
            return Err("Cluster connection lost. Reconnect before continuing; submitted writes were not retried.".into());
        }
        if !self.connections.contains_key(endpoint) {
            let mut settings = self.settings.clone();
            settings.host = endpoint.host.clone();
            settings.port = endpoint.port;
            let connection =
                crate::redis_session::Session::cluster_node(&settings, &self.password)?;
            if self.connections.len() >= MAX_CONNECTIONS
                && let Some(old) = self.recent.pop_front()
            {
                self.connections.remove(&old);
            }
            self.connections.insert(endpoint.clone(), connection);
        }
        self.recent.retain(|e| e != endpoint);
        self.recent.push_back(endpoint.clone());
        Ok(self.connections.get_mut(endpoint).unwrap())
    }
    fn prepare(&mut self, endpoints: &[Endpoint]) -> Result<(), String> {
        if self.stopped {
            return Err("Cluster connection lost. Reconnect before continuing.".into());
        }
        let mut missing = Vec::new();
        for endpoint in endpoints {
            if self.connections.contains_key(endpoint) {
                self.recent.retain(|e| e != endpoint);
                self.recent.push_back(endpoint.clone());
            } else {
                missing.push(endpoint.clone());
            }
        }
        // At most eight cold handshakes run concurrently on the owning worker;
        // no operation is sent until all needed connections have authenticated.
        for batch in missing.chunks(8) {
            let results = std::thread::scope(|scope| {
                let workers: Vec<_> = batch
                    .iter()
                    .map(|endpoint| {
                        let mut settings = self.settings.clone();
                        settings.host = endpoint.host.clone();
                        settings.port = endpoint.port;
                        let password = &self.password;
                        scope.spawn(move || {
                            crate::redis_session::Session::cluster_node(&settings, password)
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|worker| {
                        worker
                            .join()
                            .unwrap_or_else(|_| Err("Cluster connection worker stopped".into()))
                    })
                    .collect::<Vec<_>>()
            });
            for (endpoint, result) in batch.iter().zip(results) {
                let connection = result?;
                if self.connections.len() >= MAX_CONNECTIONS
                    && let Some(old) = self.recent.pop_front()
                {
                    self.connections.remove(&old);
                }
                self.connections.insert(endpoint.clone(), connection);
                self.recent.push_back(endpoint.clone());
            }
        }
        Ok(())
    }
    pub fn scan(
        &mut self,
        token: u64,
        pattern: &[u8],
        count: usize,
        kind: &str,
    ) -> Result<crate::redis_session::KeyPage, String> {
        let mut scan = if token == 0 {
            self.scan = None;
            let mut nodes: Vec<_> = self
                .routes
                .iter()
                .filter_map(Option::as_ref)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .cloned()
                .collect();
            nodes.sort_by(|a, b| a.host.cmp(&b.host).then(a.port.cmp(&b.port)));
            if nodes.is_empty() {
                return Err(
                    "Cluster has no available primaries; reconnect to refresh topology.".into(),
                );
            }
            if self.routes.iter().any(Option::is_none) {
                return Err(
                    "Cluster slot coverage is incomplete; cannot claim a complete scan.".into(),
                );
            }
            Scan {
                nodes: nodes.into_iter().map(|endpoint| (endpoint, 0)).collect(),
                pattern: pattern.to_vec(),
                kind: kind.to_owned(),
                token: 0,
            }
        } else {
            let scan = self
                .scan
                .take()
                .ok_or("Cluster scan expired; restart the scan.")?;
            if scan.token != token || scan.pattern != pattern || scan.kind != kind {
                return Err("Cluster scan changed; restart the scan.".into());
            }
            scan
        };
        let batch: Vec<_> = scan
            .nodes
            .drain(..scan.nodes.len().min(MAX_CONNECTIONS))
            .collect();
        let count = count.clamp(1, 1000).div_ceil(batch.len());
        let groups: Vec<_> = batch
            .iter()
            .enumerate()
            .map(|(index, (endpoint, cursor))| {
                let mut command = redis::cmd("SCAN");
                command
                    .arg(*cursor)
                    .arg("MATCH")
                    .arg(pattern)
                    .arg("COUNT")
                    .arg(count);
                if !kind.is_empty() {
                    command.arg("TYPE").arg(kind);
                }
                (endpoint.clone(), vec![index], command.get_packed_command())
            })
            .collect();
        let (values, rejected) = self.exchange(&groups, batch.len())?;
        if let Some((_, _, error)) = rejected.first() {
            return Err(server_error(error));
        }
        let mut keys = Vec::new();
        for ((endpoint, _), value) in batch.into_iter().zip(values) {
            let (cursor, mut page): (u64, Vec<Vec<u8>>) =
                redis::from_redis_value(value).map_err(|e| e.to_string())?;
            if keys.len() + page.len() > 10000 {
                return Err("Cluster scan page exceeds 10,000 keys; narrow the filter.".into());
            }
            keys.append(&mut page);
            if cursor != 0 {
                scan.nodes.push_back((endpoint, cursor));
            }
        }
        if scan.nodes.is_empty() {
            self.scan = None;
            return Ok(crate::redis_session::KeyPage { cursor: 0, keys });
        }
        self.scan_token = self
            .scan_token
            .checked_add(1)
            .ok_or("Cluster scan token exhausted; reconnect.")?;
        scan.token = self.scan_token;
        self.scan = Some(scan);
        Ok(crate::redis_session::KeyPage {
            cursor: self.scan_token,
            keys,
        })
    }
    /// One bounded fan-out of independent per-key scripts. Never replay an
    /// unconfirmed write; retain confirmed replies from other live nodes.
    pub fn import_batch(
        &mut self,
        entries: &[crate::redis_archive::Entry],
        collision: crate::redis_archive::Collision,
        expiry: crate::redis_archive::Expiry,
    ) -> crate::redis_archive::ImportReport {
        use crate::redis_archive::*;
        let keys: Vec<_> = entries.iter().map(|entry| entry.key.as_slice()).collect();
        let commands: Vec<_> = entries
            .iter()
            .map(|entry| {
                let mut command = redis::cmd("EVAL");
                command
                    .arg(IMPORT_SCRIPT)
                    .arg(1)
                    .arg(&entry.key)
                    .arg(if collision == Collision::Replace {
                        "replace"
                    } else {
                        "skip"
                    })
                    .arg(if expiry == Expiry::Persistent {
                        "persistent"
                    } else {
                        "preserve"
                    })
                    .arg(entry.expires_at_ms.unwrap_or(-1))
                    .arg(&entry.dump);
                command
            })
            .collect();
        let result = self.write_batch(&keys, &commands, 16, |value| {
            let mut outcomes = import_reply(value, 1)?;
            Ok(outcomes.remove(0))
        });
        ImportReport {
            outcomes: result
                .outcomes
                .into_iter()
                .map(|row| {
                    let (state, message) = match row.state {
                        WriteState::Confirmed((state, message)) => (state, message),
                        WriteState::Unknown => (State::Unknown, row.message),
                        WriteState::NotAttempted => (State::NotAttempted, row.message),
                    };
                    Outcome {
                        key: row.key,
                        state,
                        message,
                    }
                })
                .collect(),
            stopped: result.stopped,
        }
    }
    pub fn bulk(
        &mut self,
        mutation: &crate::redis_mutation::Mutation,
        read_only: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<WriteReport<bool>, String> {
        use crate::redis_mutation::Action;
        mutation.validate(read_only)?;
        if !matches!(mutation.action, Action::DeleteKeys | Action::ExpireKeys) {
            return Err("Only delete and expiry support bulk outcomes.".into());
        }
        let mut report = WriteReport {
            outcomes: Vec::with_capacity(mutation.keys.len()),
            stopped: None,
        };
        for batch in mutation.keys.chunks(16) {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                report.stopped = Some("Cancelled; remaining keys were not attempted.".into());
                break;
            }
            let keys: Vec<_> = batch.iter().map(Vec::as_slice).collect();
            let commands: Vec<_> = batch
                .iter()
                .map(|key| {
                    let mut command = redis::cmd(if mutation.action == Action::DeleteKeys {
                        "UNLINK"
                    } else if mutation.ttl_ms.is_some() {
                        "PEXPIRE"
                    } else {
                        "PERSIST"
                    });
                    command.arg(key);
                    if mutation.action == Action::ExpireKeys
                        && let Some(ttl) = mutation.ttl_ms
                    {
                        command.arg(ttl);
                    }
                    command
                })
                .collect();
            let part = self.write_batch(&keys, &commands, 16, |value| match value {
                redis::Value::Int(0) => Ok(false),
                redis::Value::Int(1) => Ok(true),
                _ => Err("Invalid bulk mutation response; inspect before retrying.".into()),
            });
            report.outcomes.extend(part.outcomes);
            if part.stopped.is_some() {
                report.stopped = part.stopped;
                break;
            }
        }
        report
            .outcomes
            .extend(
                mutation.keys[report.outcomes.len()..]
                    .iter()
                    .map(|key| WriteOutcome {
                        key: key.clone(),
                        state: WriteState::NotAttempted,
                        message: String::new(),
                    }),
            );
        Ok(report)
    }
    fn write_batch<T>(
        &mut self,
        keys: &[&[u8]],
        commands: &[redis::Cmd],
        limit: usize,
        decode: impl Fn(redis::Value) -> Result<T, String>,
    ) -> WriteReport<T> {
        let mut report = WriteReport {
            outcomes: keys
                .iter()
                .map(|key| WriteOutcome {
                    key: key.to_vec(),
                    state: WriteState::NotAttempted,
                    message: String::new(),
                })
                .collect(),
            stopped: None,
        };
        if keys.is_empty() || keys.len() > limit || commands.len() != keys.len() {
            report.stopped = Some(format!("Cluster write requires 1–{limit} keys per batch."));
            return report;
        }
        let mut groups: Vec<ReadGroup> = Vec::new();
        for (index, (key, command)) in keys.iter().zip(commands).enumerate() {
            let endpoint = self.routes[hash_slot(key) as usize]
                .clone()
                .unwrap_or_else(|| self.seed.clone());
            let group = match groups.iter().position(|g| g.0 == endpoint) {
                Some(i) => i,
                None => {
                    groups.push((endpoint, Vec::new(), Vec::new()));
                    groups.len() - 1
                }
            };
            groups[group].1.push(index);
            command.write_packed_command(&mut groups[group].2);
        }
        if groups.len() > MAX_CONNECTIONS {
            report.stopped = Some("Write batch exceeds 64 nodes; narrow the selection.".into());
            return report;
        }
        if let Err(error) = self.prepare(&groups.iter().map(|g| g.0.clone()).collect::<Vec<_>>()) {
            report.stopped = Some(error);
            return report; // Authentication failed before any write was submitted.
        }
        let mut sent = 0;
        let mut transport_lost = false;
        for (endpoint, indexes, bytes) in &groups {
            // send can fail after writing some bytes, so this whole node batch is
            // unconfirmed until individual replies prove otherwise.
            for &index in indexes {
                report.outcomes[index].state = WriteState::Unknown;
                report.outcomes[index].message =
                    "Submitted outcome is unconfirmed. Inspect before retrying.".into();
            }
            if let Err(error) = self
                .connections
                .get_mut(endpoint)
                .unwrap()
                .send_packed_command(bytes)
            {
                report.stopped = Some(format!(
                    "Cluster write transport lost: {error}. Nothing was replayed."
                ));
                transport_lost = true;
                break;
            }
            sent += 1;
        }
        let mut redirects = Vec::new();
        for (endpoint, indexes, _) in groups.iter().take(sent) {
            for &index in indexes {
                let connection = self.connections.get_mut(endpoint).unwrap();
                match connection
                    .recv_response()
                    .and_then(redis::Value::extract_error)
                {
                    Ok(value) => match decode(value) {
                        Ok(value) => {
                            report.outcomes[index].state = WriteState::Confirmed(value);
                            report.outcomes[index].message.clear();
                        }
                        Err(error) => {
                            report.stopped.get_or_insert(error);
                        }
                    },
                    Err(error)
                        if error.is_io_error()
                            || error.kind() == redis::ErrorKind::Parse
                            || !connection.is_open() =>
                    {
                        report.stopped.get_or_insert_with(|| {
                            format!("Cluster write transport lost: {error}. Nothing was replayed.")
                        });
                        transport_lost = true;
                        break; // Continue draining other nodes, keeping their results.
                    }
                    Err(error) if matches!(error.code(), Some("MOVED" | "ASK")) => {
                        report.outcomes[index].state = WriteState::NotAttempted;
                        report.outcomes[index].message =
                            "Server rejected this attempt with a redirect.".into();
                        redirects.push((index, endpoint.clone(), error));
                    }
                    Err(error) => {
                        report.stopped.get_or_insert_with(|| server_error(&error));
                    }
                }
            }
        }
        if transport_lost {
            self.stopped = true;
            self.connections.clear();
            self.recent.clear();
        }
        // Only explicit rejections may be retried, after all initial replies
        // are drained and only when the entire transport remains healthy.
        if report.stopped.is_none() {
            for (index, source, error) in redirects {
                let slot = Some(hash_slot(keys[index]));
                let target = self.redirect(&error, &source, slot);
                let (endpoint, asking) = match target {
                    Ok(target) => target,
                    Err(error) => {
                        report.stopped = Some(error);
                        break;
                    }
                };
                report.outcomes[index].state = WriteState::Unknown;
                report.outcomes[index].message =
                    "Redirected outcome is unconfirmed. Inspect before retrying.".into();
                match self
                    .query_at(&commands[index], slot, endpoint, asking)
                    .and_then(&decode)
                {
                    Ok(value) => {
                        report.outcomes[index].state = WriteState::Confirmed(value);
                        report.outcomes[index].message.clear();
                    }
                    Err(error) => {
                        report.stopped = Some(error);
                        break;
                    }
                }
            }
        }
        report
    }
    pub fn read_pipeline(
        &mut self,
        pipeline: &redis::Pipeline,
    ) -> Result<Vec<redis::Value>, String> {
        if pipeline.is_transaction() {
            return Err("Cluster preview cannot execute transactions".into());
        }
        let commands: Vec<_> = pipeline.cmd_iter().collect();
        if commands.len() > MAX_READ_COMMANDS {
            return Err("Cluster read batch exceeds 1,000 commands".into());
        }
        let mut groups: Vec<(Endpoint, Vec<usize>, Vec<u8>)> = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            let args: Vec<Vec<u8>> = command
                .args_iter()
                .map(|arg| match arg {
                    redis::Arg::Simple(value) => Ok(value.to_vec()),
                    _ => Err("Unsupported preview cursor"),
                })
                .collect::<Result<_, _>>()?;
            // Only the two fixed archive readers may bypass the editor's
            // conservative script guard. EVAL_RO also enforces reads server-side.
            let archive_read = args.len() == 4
                && args[0] == b"EVAL_RO"
                && args[2] == b"1"
                && (args[1] == crate::redis_archive::REVIEW_SCRIPT.as_bytes()
                    || (args[1] == crate::redis_archive::EXPORT_SCRIPT.as_bytes()
                        && commands.len() <= 16));
            let vector_read = args.len() == 5 && args[0] == b"EVAL_RO" && args[1] == crate::redis_vectors::PAGE_SCRIPT.as_bytes() && args[2] == b"1";
            if !archive_read && !vector_read {
                crate::redis_commands::validate(&args, true)?;
            }
            let endpoint = command_slot(command)?
                .and_then(|slot| self.routes[slot as usize].clone())
                .unwrap_or_else(|| self.seed.clone());
            let group = if let Some(index) = groups.iter().position(|g| g.0 == endpoint) {
                index
            } else {
                groups.push((endpoint, Vec::new(), Vec::new()));
                groups.len() - 1
            };
            groups[group].1.push(index);
            command.write_packed_command(&mut groups[group].2);
        }
        let (mut values, rejected) = self.exchange(&groups, commands.len())?;
        // Every outstanding response was drained before redirect work starts.
        for (index, source, error) in rejected {
            let command = commands[index];
            let slot = command_slot(command)?;
            let (endpoint, asking) = self.redirect(&error, &source, slot)?;
            values[index] = self.query_at(command, slot, endpoint, asking)?;
        }
        Ok(values)
    }
    fn exchange(
        &mut self,
        groups: &[ReadGroup],
        size: usize,
    ) -> Result<(Vec<redis::Value>, Vec<Rejected>), String> {
        let mut values = vec![redis::Value::Nil; size];
        let mut rejected = Vec::new();
        // A wave is completely drained before the next prepare may evict sockets.
        // Existing <=64-node reads keep their single fan-out exchange.
        for groups in groups.chunks(MAX_CONNECTIONS) {
            // Finish cold connections before sending any operation, then fan out all
            // node batches before awaiting replies. Warm reads take one exchange.
            self.prepare(
                &groups
                    .iter()
                    .map(|group| group.0.clone())
                    .collect::<Vec<_>>(),
            )?;
            for (endpoint, _, bytes) in groups {
                if let Err(error) = self
                    .connections
                    .get_mut(endpoint)
                    .unwrap()
                    .send_packed_command(bytes)
                {
                    self.stopped = true;
                    self.connections.clear();
                    self.recent.clear();
                    return Err(format!(
                        "Cluster preview transport lost: {error}; reconnect before continuing."
                    ));
                }
            }

            for (endpoint, indexes, _) in groups {
                for &index in indexes {
                    let connection = self.connections.get_mut(endpoint).unwrap();
                    match connection
                        .recv_response()
                        .and_then(redis::Value::extract_error)
                    {
                        Ok(value) => values[index] = value,
                        Err(error)
                            if error.is_io_error()
                                || error.kind() == redis::ErrorKind::Parse
                                || !connection.is_open() =>
                        {
                            self.stopped = true;
                            self.connections.clear();
                            self.recent.clear();
                            return Err(format!(
                                "Cluster preview transport lost: {error}; reconnect before continuing."
                            ));
                        }
                        Err(error) => rejected.push((index, endpoint.clone(), error)),
                    }
                }
            }
        }
        Ok((values, rejected))
    }
    fn redirect(
        &mut self,
        error: &redis::RedisError,
        source: &Endpoint,
        slot: Option<u16>,
    ) -> Result<(Endpoint, bool), String> {
        if !matches!(error.code(), Some("MOVED" | "ASK")) {
            return Err(server_error(error));
        }
        let (address, redirect_slot) = error.redirect_node().ok_or("Invalid cluster redirect")?;
        if redirect_slot >= 16384 || slot.is_some_and(|slot| slot != redirect_slot) {
            return Err("Cluster redirect changed the command's hash slot".into());
        }
        let target = parse_redirect(address, &source.host)?;
        let asking = error.code() == Some("ASK");
        if !asking {
            self.routes[redirect_slot as usize] = Some(target.clone());
            self.scan = None;
        }
        Ok((target, asking))
    }
    pub fn query(&mut self, command: &redis::Cmd) -> Result<redis::Value, String> {
        let slot = command_slot(command)?;
        let endpoint = slot
            .and_then(|s| self.routes[s as usize].clone())
            .unwrap_or_else(|| self.seed.clone());
        self.query_at(command, slot, endpoint, false)
    }
    pub fn query_bounded(&mut self, command: &redis::Cmd, limit: usize) -> Result<redis::Value, String> {
        if !matches!(command.args_iter().next(), Some(redis::Arg::Simple(name)) if name.eq_ignore_ascii_case(b"VSIM")) { return Err("Bounded cluster queries currently support VSIM only.".into()); }
        let slot = command_slot(command)?;
        let endpoint = slot.and_then(|s| self.routes[s as usize].clone()).unwrap_or_else(|| self.seed.clone());
        self.query_at_limited(command, slot, endpoint, false, Some(limit))
    }
    fn query_at(&mut self, command: &redis::Cmd, slot: Option<u16>, endpoint: Endpoint, asking: bool) -> Result<redis::Value, String> {
        self.query_at_limited(command, slot, endpoint, asking, None)
    }
    fn query_at_limited(
        &mut self,
        command: &redis::Cmd,
        slot: Option<u16>,
        mut endpoint: Endpoint,
        mut asking: bool,
        limit: Option<usize>,
    ) -> Result<redis::Value, String> {
        for _ in 0..5 {
            let conn = self.connection(&endpoint)?;
            let result = if asking {
                // ASKING and the rejected command are sent together, not as two RTTs.
                let mut pipe = redis::pipe();
                pipe.cmd("ASKING").add_command(command.clone());
                (if let Some(limit) = limit { conn.req_packed_commands_bounded(&pipe.get_packed_pipeline(), &[1024, limit]) } else { conn.req_packed_commands(&pipe.get_packed_pipeline(), 0, 2) })
                    .and_then(|mut values| {
                        if values.len() != 2 {
                            return Err(redis::RedisError::from((
                                redis::ErrorKind::Parse,
                                "Invalid ASK response count",
                            )));
                        }
                        let command_reply = values.pop().unwrap().extract_error();
                        let asking_reply = values.pop().unwrap().extract_error();
                        // A rejected ASKING does not prevent the following command
                        // from executing if ownership changed. Keep its actual reply.
                        match (command_reply, asking_reply) {
                            (Ok(value), _) => Ok(value),
                            (Err(error), Ok(_)) => Err(error),
                            (Err(_), Err(error)) => Err(error),
                        }
                    })
            } else if let Some(limit) = limit {
                conn.req_packed_commands_bounded(&command.get_packed_command(), &[limit]).and_then(|mut values| values.remove(0).extract_error())
            } else {
                command.query::<redis::Value>(conn)
            };
            match result {
                Ok(value) => return Ok(value),
                Err(error)
                    if error.is_io_error()
                        || error.kind() == redis::ErrorKind::Parse
                        || !conn.is_open() =>
                {
                    self.stopped = true;
                    self.connections.clear();
                    self.recent.clear();
                    if limit.is_some() { return Err(format!("Cluster search connection closed: {error}. Reconnect before retrying. Nothing was replayed.")); }
                    return Err(format!(
                        "Cluster connection lost: {error}. A submitted write may have completed; verify before reconnecting. Nothing was replayed."
                    ));
                }
                Err(error) if matches!(error.code(), Some("MOVED" | "ASK")) => {
                    (endpoint, asking) = self.redirect(&error, &endpoint, slot)?;
                }
                Err(error) => return Err(server_error(&error)),
            }
        }
        Err(
            "Cluster redirected the command too many times; refresh or reconnect before retrying."
                .into(),
        )
    }
}
fn server_error(error: &redis::RedisError) -> String {
    match error.code() {
        Some(code) => format!("{code}: {error}"),
        None => error.to_string(),
    }
}
fn parse_redirect(address: &str, current_host: &str) -> Result<Endpoint, String> {
    let (host, port) = address
        .rsplit_once(':')
        .ok_or("Invalid cluster redirect address")?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let host = if host.is_empty() { current_host } else { host };
    Endpoint::new(
        host.into(),
        port.parse().map_err(|_| "Invalid cluster redirect port")?,
    )
}
pub(crate) fn crosses_slots(keys: &[Vec<u8>]) -> bool {
    keys.first().is_some_and(|first| {
        keys.iter()
            .skip(1)
            .any(|key| hash_slot(key) != hash_slot(first))
    })
}
fn hash_slot(key: &[u8]) -> u16 {
    let key = if let Some(start) = key.iter().position(|b| *b == b'{') {
        match key[start + 1..].iter().position(|b| *b == b'}') {
            Some(end) if end > 0 => &key[start + 1..start + 1 + end],
            _ => key,
        }
    } else {
        key
    };
    let mut crc = 0u16;
    for byte in key {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc % 16384
}
pub(crate) fn command_slot(command: &redis::Cmd) -> Result<Option<u16>, String> {
    let args: Vec<&[u8]> = command
        .args_iter()
        .map(|a| match a {
            redis::Arg::Simple(v) => Ok(v),
            _ => Err("Unsupported cursor argument"),
        })
        .collect::<Result<_, _>>()?;
    let name = args.first().ok_or("Empty Redis command")?;
    let name = std::str::from_utf8(name)
        .map_err(|_| "Invalid command name")?
        .to_ascii_uppercase();
    let keys: &[&[u8]] = match name.as_str() {
        "EVAL" | "EVAL_RO" | "EVALSHA" | "EVALSHA_RO" => {
            let count = std::str::from_utf8(args.get(2).ok_or("Missing script key count")?)
                .ok()
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or("Invalid script key count")?;
            if count > args.len().saturating_sub(3) {
                return Err("Missing script keys".into());
            }
            &args[3..3 + count]
        }
        "MSET" | "MSETNX" => {
            if args.len() < 3 || args.len().is_multiple_of(2) {
                return Err("MSET requires key/value pairs".into());
            }
            let slot = hash_slot(args[1]);
            if args[1..]
                .iter()
                .step_by(2)
                .any(|key| hash_slot(key) != slot)
            {
                return Err(
                    "CROSSSLOT: keys must share a hash slot; the operation was not split or sent."
                        .into(),
                );
            }
            return Ok(Some(slot));
        }
        "RENAME" | "RENAMENX" | "SMOVE" | "RPOPLPUSH" | "LMOVE" => {
            args.get(1..3).ok_or("Missing source/destination keys")?
        }
        "DEL" | "UNLINK" | "EXISTS" | "MGET" | "TOUCH" => &args[1..],
        "INCR" | "INCRBY" | "INCRBYFLOAT" | "DECR" | "DECRBY" | "APPEND" | "GETSET" | "GETDEL"
        | "GETEX" | "SETNX" | "SETEX" | "PSETEX" | "SETRANGE" | "GETBIT" | "SETBIT"
        | "BITCOUNT" | "BITPOS" | "HEXISTS" | "HSTRLEN" | "HINCRBY" | "HINCRBYFLOAT" | "HMGET"
        | "HMSET" | "HKEYS" | "HVALS" | "HRANDFIELD" | "SISMEMBER" | "SMISMEMBER"
        | "SRANDMEMBER" | "SPOP" | "LINDEX" | "LINSERT" | "LREM" | "LTRIM" | "LPOS" | "ZSCORE"
        | "ZMSCORE" | "ZCOUNT" | "ZLEXCOUNT" | "ZRANK" | "ZREVRANK" | "ZREVRANGE"
        | "ZRANGEBYSCORE" | "ZREVRANGEBYSCORE" | "ZPOPMAX" | "ZPOPMIN" | "ZINCRBY"
        | "ZRANDMEMBER" | "XREVRANGE" | "GET" | "SET" | "GETRANGE" | "TYPE" | "TTL" | "PTTL"
        | "EXPIRE" | "PEXPIRE" | "PERSIST" | "PEXPIRETIME" | "DUMP" | "RESTORE" | "STRLEN"
        | "HSCAN" | "HGET" | "HGETALL" | "HSET" | "HDEL" | "HLEN" | "SSCAN" | "SADD" | "SREM"
        | "SCARD" | "SMEMBERS" | "LRANGE" | "LLEN" | "LPUSH" | "RPUSH" | "LSET" | "LPOP"
        | "RPOP" | "ZRANGE" | "ZSCAN" | "ZCARD" | "ZADD" | "ZREM" | "XRANGE" | "XLEN" | "XADD"
        | "VSIM" | "VCARD" | "VRANGE" | "TS.RANGE" | "TS.REVRANGE" | "TS.GET" | "TS.INFO" | "XDEL" | "JSON.GET" | "JSON.SET" | "JSON.TYPE" => {
            args.get(1..2).ok_or("Missing Redis key")?
        }
        "MEMORY"
            if args
                .get(1)
                .is_some_and(|s| s.eq_ignore_ascii_case(b"USAGE")) =>
        {
            args.get(2..3).ok_or("Missing Redis key")?
        }
        "FLUSHALL" | "FLUSHDB" | "SHUTDOWN" | "SELECT" => {
            return Err(
                "This server-wide command is not available through cluster routing.".into(),
            );
        }
        _ => return Ok(None), // Unknown commands rely on the server's explicit redirect, never guessed key positions.
    };
    let mut slot = None;
    for key in keys {
        let next = hash_slot(key);
        if slot.is_some_and(|s| s != next) {
            return Err(
                "CROSSSLOT: keys must share a hash slot; the operation was not split or sent."
                    .into(),
            );
        }
        slot = Some(next);
    }
    Ok(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        time::Duration,
    };
    fn read(socket: &mut TcpStream, parser: &mut redis::Parser) -> Vec<Vec<u8>> {
        redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap()
    }
    fn handshake(socket: &mut TcpStream, parser: &mut redis::Parser) {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        loop {
            let command = read(socket, parser);
            socket
                .write_all(if command[0] == b"PING" {
                    b"+PONG\r\n"
                } else {
                    b"+OK\r\n"
                })
                .unwrap();
            if command[0] == b"PING" {
                break;
            }
        }
    }
    fn router(port: u16) -> Router {
        let settings = Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            cluster: true,
            ..Default::default()
        };
        let connection = crate::redis_session::Session::cluster_node(&settings, "").unwrap();
        Router::new(settings, String::new(), connection, vec![None; 16384])
    }
    #[test]
    fn binary_hash_tags_and_cross_slot_guards() {
        assert_eq!(hash_slot(b"123456789"), 0x31c3);
        assert_eq!(hash_slot(b"foo{bar}zap"), hash_slot(b"bar"));
        assert_eq!(hash_slot(b"foo{{bar}}zap"), hash_slot(b"{bar"));
        assert_ne!(hash_slot(b"foo{}{bar}"), hash_slot(b"bar"));
        assert_eq!(hash_slot(b"a{\0\xff}b"), hash_slot(b"\0\xff"));
        assert!(
            command_slot(redis::cmd("DEL").arg("a").arg("b"))
                .unwrap_err()
                .contains("CROSSSLOT")
        );
        assert_eq!(
            command_slot(
                redis::cmd("EVAL")
                    .arg("return 1")
                    .arg(2)
                    .arg("{a}:x")
                    .arg("{a}:y")
            )
            .unwrap(),
            Some(hash_slot(b"a"))
        );
        assert!(command_slot(redis::cmd("EVAL").arg("return 1").arg(9)).is_err());
        assert_eq!(parse_redirect(":6380", "host").unwrap().host, "host");
        assert_eq!(parse_redirect("[::1]:6380", "host").unwrap().host, "::1");
        assert!(parse_redirect("attacker/path:6380", "host").is_err());
    }
    #[test]
    fn bounded_similarity_overflow_stops_router_without_replay() {
        use std::io::Read;
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = source.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = source.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
            socket.write_all(b"$1000000\r\n").unwrap();
            let _ = socket.write_all(&[b'x';256]);
            let mut byte = [0];
            match socket.read(&mut byte) { Ok(0) => {}, Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe) => {}, result => panic!("unexpected request after overflow: {result:?}") }
        });
        let mut router = router(port);
        let command = redis::cmd("VSIM").arg("key").arg("ELE").arg("source").to_owned();
        assert!(router.query_bounded(&command, 128).unwrap_err().contains("byte limit"));
        assert!(router.stopped);
        assert!(router.connections.is_empty());
        assert!(router.query_bounded(&command, 128).is_err());
        peer.join().unwrap();
    }
    #[test]
    fn bounded_similarity_redirects_preserve_exchange_and_route_semantics() {
        for (asking, denied_asking) in [(false, false), (true, false), (true, true)] {
            let source = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = source.local_addr().unwrap().port();
            let target = TcpListener::bind("127.0.0.1:0").unwrap();
            let target_port = target.local_addr().unwrap().port();
            let slot = hash_slot(b"key");
            let source_peer = std::thread::spawn(move || {
                let (mut socket, _) = source.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
                write!(
                    socket,
                    "-{} {slot} 127.0.0.1:{target_port}\r\n",
                    if asking { "ASK" } else { "MOVED" }
                )
                .unwrap();
                if asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"GET");
                    socket.write_all(b"$5\r\nvalue\r\n").unwrap();
                }
            });
            let target_peer = std::thread::spawn(move || {
                let (mut socket, _) = target.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                if asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"ASKING");
                }
                // The second command must arrive before ASKING receives a reply.
                assert_eq!(read(&mut socket, &mut parser)[0], b"VSIM");
                socket
                    .write_all(if denied_asking {
                        b"-NOPERM ASKING denied\r\n*0\r\n"
                    } else if asking {
                        b"+OK\r\n*0\r\n"
                    } else {
                        b"*0\r\n"
                    })
                    .unwrap();
                if !asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"GET");
                    socket.write_all(b"$5\r\nvalue\r\n").unwrap();
                }
            });
            let mut router = router(port);
            router
                .query_bounded(redis::cmd("VSIM").arg("key").arg("ELE").arg("value").arg("COUNT").arg(20), 1024)
                .unwrap();
            assert_eq!(
                router.query(redis::cmd("GET").arg("key")).unwrap(),
                redis::Value::BulkString(b"value".to_vec())
            );
            assert_eq!(router.routes[slot as usize].is_some(), !asking);
            source_peer.join().unwrap();
            target_peer.join().unwrap();
        }
    }
    #[test]
    fn moved_updates_the_route_and_ask_is_pipelined_without_changing_it() {
        for (asking, denied_asking) in [(false, false), (true, false), (true, true)] {
            let source = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = source.local_addr().unwrap().port();
            let target = TcpListener::bind("127.0.0.1:0").unwrap();
            let target_port = target.local_addr().unwrap().port();
            let slot = hash_slot(b"key");
            let source_peer = std::thread::spawn(move || {
                let (mut socket, _) = source.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                assert_eq!(read(&mut socket, &mut parser)[0], b"SET");
                write!(
                    socket,
                    "-{} {slot} 127.0.0.1:{target_port}\r\n",
                    if asking { "ASK" } else { "MOVED" }
                )
                .unwrap();
                if asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"GET");
                    socket.write_all(b"$5\r\nvalue\r\n").unwrap();
                }
            });
            let target_peer = std::thread::spawn(move || {
                let (mut socket, _) = target.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                if asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"ASKING");
                }
                // The second command must arrive before ASKING receives a reply.
                assert_eq!(read(&mut socket, &mut parser)[0], b"SET");
                socket
                    .write_all(if denied_asking {
                        b"-NOPERM ASKING denied\r\n+OK\r\n"
                    } else if asking {
                        b"+OK\r\n+OK\r\n"
                    } else {
                        b"+OK\r\n"
                    })
                    .unwrap();
                if !asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"GET");
                    socket.write_all(b"$5\r\nvalue\r\n").unwrap();
                }
            });
            let mut router = router(port);
            router
                .query(redis::cmd("SET").arg("key").arg("value"))
                .unwrap();
            assert_eq!(
                router.query(redis::cmd("GET").arg("key")).unwrap(),
                redis::Value::BulkString(b"value".to_vec())
            );
            assert_eq!(router.routes[slot as usize].is_some(), !asking);
            source_peer.join().unwrap();
            target_peer.join().unwrap();
        }
    }
    #[test]
    fn read_batches_fan_out_before_waiting_and_preserve_order() {
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = source.local_addr().unwrap().port();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let (a_tx, a_rx) = std::sync::mpsc::channel();
        let (b_tx, b_rx) = std::sync::mpsc::channel();
        let peers = [
            (source, a_tx, b_rx, b"a".to_vec()),
            (target, b_tx, a_rx, b"b".to_vec()),
        ]
        .map(|(listener, tx, rx, key)| {
            std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                for _ in 0..2 {
                    let command = read(&mut socket, &mut parser);
                    assert_eq!(command[0], b"GET");
                    assert_eq!(command[1], key);
                    tx.send(()).unwrap();
                    rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    socket.write_all(b"$1\r\n").unwrap();
                    socket.write_all(&key).unwrap();
                    socket.write_all(b"\r\n").unwrap();
                }
            })
        });
        let mut router = router(port);
        router.routes[hash_slot(b"a") as usize] = Some(router.seed.clone());
        router.routes[hash_slot(b"b") as usize] = Some(Endpoint {
            host: "127.0.0.1".into(),
            port: target_port,
        });
        let mut pipe = redis::pipe();
        pipe.cmd("GET").arg("b").cmd("GET").arg("a");
        for _ in 0..2 {
            assert_eq!(
                router.read_pipeline(&pipe).unwrap(),
                vec![
                    redis::Value::BulkString(b"b".to_vec()),
                    redis::Value::BulkString(b"a".to_vec())
                ]
            );
        }
        for peer in peers {
            peer.join().unwrap();
        }
    }
    #[test]
    fn preview_errors_drain_remaining_replies_before_next_command() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            assert_eq!(read(&mut socket, &mut parser)[0], b"TYPE");
            assert_eq!(read(&mut socket, &mut parser)[0], b"PTTL");
            socket.write_all(b"-NOPERM denied\r\n:42\r\n").unwrap();
            assert_eq!(read(&mut socket, &mut parser)[0], b"GET");
            socket.write_all(b"$2\r\nok\r\n").unwrap();
        });
        let mut router = router(port);
        let mut pipe = redis::pipe();
        pipe.cmd("TYPE").arg("a").cmd("PTTL").arg("a");
        assert!(router.read_pipeline(&pipe).unwrap_err().contains("NOPERM"));
        assert_eq!(
            router.query(redis::cmd("GET").arg("a")).unwrap(),
            redis::Value::BulkString(b"ok".to_vec())
        );
        peer.join().unwrap();
    }
    #[test]
    fn import_retries_only_explicit_redirects_after_draining_the_batch() {
        use crate::redis_archive::*;
        for asking in [false, true] {
            let source = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = source.local_addr().unwrap().port();
            let target = TcpListener::bind("127.0.0.1:0").unwrap();
            let target_port = target.local_addr().unwrap().port();
            let slot = hash_slot(b"a");
            let source_peer = std::thread::spawn(move || {
                let (mut socket, _) = source.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                for key in [b"a", b"b"] {
                    let command = read(&mut socket, &mut parser);
                    assert_eq!(command[0], b"EVAL");
                    assert_eq!(command[3], key);
                }
                write!(
                    socket,
                    "-{} {slot} 127.0.0.1:{target_port}\r\n",
                    if asking { "ASK" } else { "MOVED" }
                )
                .unwrap();
                socket
                    .write_all(b"*1\r\n*2\r\n$6\r\nexists\r\n$0\r\n\r\n")
                    .unwrap();
            });
            let target_peer = std::thread::spawn(move || {
                let (mut socket, _) = target.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                if asking {
                    assert_eq!(read(&mut socket, &mut parser)[0], b"ASKING");
                }
                let command = read(&mut socket, &mut parser);
                assert_eq!(command[0], b"EVAL");
                assert_eq!(command[3], b"a");
                // ASKING and EVAL must both arrive before any response.
                if asking {
                    socket.write_all(b"+OK\r\n").unwrap();
                }
                socket
                    .write_all(b"*1\r\n*2\r\n$8\r\nrestored\r\n$0\r\n\r\n")
                    .unwrap();
            });
            let mut router = router(port);
            let entries: Vec<_> = [b"a", b"b"]
                .into_iter()
                .map(|key| Entry {
                    key: key.to_vec(),
                    kind: "string".into(),
                    expires_at_ms: None,
                    dump: vec![1],
                })
                .collect();
            let report = router.import_batch(&entries, Collision::Skip, Expiry::Preserve);
            assert!(report.stopped.is_none(), "{:?}", report.stopped);
            assert_eq!(report.outcomes[0].state, State::Restored);
            assert_eq!(report.outcomes[1].state, State::Exists);
            assert_eq!(router.routes[slot as usize].is_some(), !asking);
            source_peer.join().unwrap();
            target_peer.join().unwrap();
        }
    }
    #[test]
    fn import_preparation_failure_leaves_all_keys_unattempted() {
        use crate::redis_archive::*;
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = source.local_addr().unwrap().port();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = source.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            // Even the healthy node receives no EVAL when target setup fails.
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let failing = std::thread::spawn(move || {
            let (socket, _) = target.accept().unwrap();
            drop(socket); // Deterministic failure during connection setup.
        });
        let mut router = router(port);
        router.routes[hash_slot(b"b") as usize] = Some(Endpoint {
            host: "127.0.0.1".into(),
            port: target_port,
        });
        let entries: Vec<_> = [b"a", b"b"]
            .into_iter()
            .map(|key| Entry {
                key: key.to_vec(),
                kind: "string".into(),
                expires_at_ms: None,
                dump: vec![1],
            })
            .collect();
        let report = router.import_batch(&entries, Collision::Skip, Expiry::Preserve);
        assert!(report.stopped.is_some());
        assert!(
            report
                .outcomes
                .iter()
                .all(|o| o.state == State::NotAttempted)
        );
        assert_eq!(
            router.query(&redis::cmd("PING")).unwrap(),
            redis::Value::SimpleString("PONG".into())
        );
        peer.join().unwrap();
        failing.join().unwrap();
    }
    #[test]
    fn import_loss_preserves_other_nodes_and_never_replays() {
        use crate::redis_archive::*;
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = source.local_addr().unwrap().port();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_port = target.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        let source_peer = std::thread::spawn(move || {
            let (mut socket, _) = source.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            for key in [b"a", b"b"] {
                let command = read(&mut socket, &mut parser);
                assert_eq!(command[0], b"EVAL");
                assert_eq!(command[1], IMPORT_SCRIPT.as_bytes());
                assert_eq!(command[3], key);
            }
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            socket
                .write_all(b"*1\r\n*2\r\n$8\r\nrestored\r\n$0\r\n\r\n")
                .unwrap();
            // Second write may have completed; close without its reply.
        });
        let target_peer = std::thread::spawn(move || {
            let (mut socket, _) = target.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            let command = read(&mut socket, &mut parser);
            assert_eq!(command[0], b"EVAL");
            assert_eq!(command[3], b"c");
            tx.send(()).unwrap();
            socket
                .write_all(b"*1\r\n*2\r\n$8\r\nrestored\r\n$0\r\n\r\n")
                .unwrap();
        });
        let mut router = router(port);
        for key in [b"a", b"b"] {
            router.routes[hash_slot(key) as usize] = Some(router.seed.clone());
        }
        router.routes[hash_slot(b"c") as usize] = Some(Endpoint {
            host: "127.0.0.1".into(),
            port: target_port,
        });
        let entries: Vec<_> = [b"a", b"b", b"c"]
            .into_iter()
            .map(|key| Entry {
                key: key.to_vec(),
                kind: "string".into(),
                expires_at_ms: None,
                dump: vec![1],
            })
            .collect();
        let report = router.import_batch(&entries, Collision::Skip, Expiry::Preserve);
        assert!(report.stopped.is_some());
        assert_eq!(
            report.outcomes.iter().map(|o| o.state).collect::<Vec<_>>(),
            vec![State::Restored, State::Unknown, State::Restored]
        );
        assert!(!router.is_connected());
        let retry = router.import_batch(&entries, Collision::Skip, Expiry::Preserve);
        assert!(
            retry
                .outcomes
                .iter()
                .all(|o| o.state == State::NotAttempted)
        );
        assert!(retry.stopped.is_some());
        source_peer.join().unwrap();
        target_peer.join().unwrap();
    }
    #[test]
    fn archive_reads_pipeline_and_only_admit_bounded_fixed_scripts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            for _ in 0..2 {
                for i in 0..16 {
                    let command = read(&mut socket, &mut parser);
                    assert_eq!(command[0], b"EVAL_RO");
                    assert_eq!(command[1], crate::redis_archive::EXPORT_SCRIPT.as_bytes());
                    assert_eq!(command[2], b"1");
                    assert_eq!(command[3], format!("archive:{i}").as_bytes());
                }
                // Replies deliberately wait for the entire batch.
                for i in 0..16 {
                    write!(socket, ":{i}\r\n").unwrap();
                }
            }
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let mut router = router(port);
        let mut pipe = redis::pipe();
        for i in 0..16 {
            pipe.cmd("EVAL_RO")
                .arg(crate::redis_archive::EXPORT_SCRIPT)
                .arg(1)
                .arg(format!("archive:{i}"));
        }
        for _ in 0..2 {
            assert_eq!(
                router.read_pipeline(&pipe).unwrap(),
                (0..16).map(redis::Value::Int).collect::<Vec<_>>()
            );
        }
        pipe.cmd("EVAL_RO")
            .arg(crate::redis_archive::EXPORT_SCRIPT)
            .arg(1)
            .arg("too-many");
        assert!(router.read_pipeline(&pipe).is_err());
        let mut unknown = redis::pipe();
        unknown.cmd("EVAL_RO").arg("return 1").arg(1).arg("key");
        assert!(router.read_pipeline(&unknown).is_err());
        let mut write = redis::pipe();
        write
            .cmd("EVAL")
            .arg(crate::redis_archive::EXPORT_SCRIPT)
            .arg(1)
            .arg("key");
        assert!(router.read_pipeline(&write).is_err());
        assert_eq!(
            router.query(&redis::cmd("PING")).unwrap(),
            redis::Value::SimpleString("PONG".into())
        );
        peer.join().unwrap();
    }
    #[test]
    fn large_read_batches_drain_bounded_waves_and_preserve_order() {
        for fail_first_wave in [false, true] {
        let mut unopened = None;
        let listeners: Vec<_> = (0..65).map(|_| TcpListener::bind("127.0.0.1:0").unwrap()).collect();
        let endpoints: Vec<_> = listeners.iter().map(|listener| Endpoint { host: "127.0.0.1".into(), port: listener.local_addr().unwrap().port() }).collect();
        let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
        let mut releases = Vec::new();
        let mut peers = Vec::new();
        for (index, listener) in listeners.into_iter().enumerate() {
            if fail_first_wave && index == 64 { unopened = Some(listener); continue; }
            let (tx, rx) = std::sync::mpsc::channel();
            releases.push(tx);
            let arrived = arrived_tx.clone();
            peers.push(std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut parser = redis::Parser::new();
                handshake(&mut socket, &mut parser);
                assert_eq!(read(&mut socket, &mut parser), vec![b"GET".to_vec(), index.to_string().into_bytes()]);
                arrived.send(index).unwrap();
                rx.recv_timeout(Duration::from_secs(10)).unwrap();
                if !(fail_first_wave && index == 0) {
                    let reply = write!(socket, ":{index}\r\n");
                    if !fail_first_wave { reply.unwrap(); }
                }
            }));
        }
        let coordinator = std::thread::spawn(move || {
            let mut arrived = Vec::new();
            for _ in 0..64 { arrived.push(arrived_rx.recv_timeout(Duration::from_secs(10)).unwrap()); }
            arrived.sort_unstable();
            assert_eq!(arrived, (0..64).collect::<Vec<_>>());
            // One simulated response delay for the entire first wave, not per node.
            std::thread::sleep(Duration::from_millis(25));
            for tx in &releases[..64] { tx.send(()).unwrap(); }
            if fail_first_wave { return; }
            assert_eq!(arrived_rx.recv_timeout(Duration::from_secs(10)).unwrap(), 64);
            std::thread::sleep(Duration::from_millis(25));
            releases[64].send(()).unwrap();
        });
        let mut router = router(endpoints[0].port);
        let groups: Vec<ReadGroup> = endpoints.into_iter().enumerate().map(|(index, endpoint)| (endpoint, vec![64 - index], redis::cmd("GET").arg(index.to_string()).get_packed_command())).collect();
        let start = std::time::Instant::now();
        let result = router.exchange(&groups, 65);
        if fail_first_wave {
            assert!(result.unwrap_err().contains("transport lost"));
            assert!(!router.is_connected());
            let listener = unopened.unwrap();
            listener.set_nonblocking(true).unwrap();
            assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        } else {
        let (values, rejected) = result.unwrap();
        eprintln!("65-node cold read, two 25ms response delays: {:?}", start.elapsed());
        assert_eq!(values, (0..65).rev().map(redis::Value::Int).collect::<Vec<_>>());
        assert!(rejected.is_empty());
        assert!(router.connections.len() <= MAX_CONNECTIONS);
        assert!(router.is_connected());
        }
        coordinator.join().unwrap();
        for peer in peers { peer.join().unwrap(); }
        }
    }
    #[test]
    fn full_diagnostics_batch_is_pipelined_and_oversize_sends_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            // Withhold all replies until the complete batch arrives. A sequential
            // per-key implementation cannot pass this protocol check.
            for _ in 0..2 {
                for i in 0..MAX_READ_COMMANDS {
                    assert_eq!(
                        read(&mut socket, &mut parser),
                        vec![
                            b"MEMORY".to_vec(),
                            b"USAGE".to_vec(),
                            format!("sample:{i}").into_bytes(),
                            b"SAMPLES".to_vec(),
                            b"5".to_vec(),
                        ]
                    );
                }
                for i in 0..MAX_READ_COMMANDS {
                    write!(socket, ":{i}\r\n").unwrap();
                }
            }
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let mut router = router(port);
        let mut pipe = redis::pipe();
        for i in 0..MAX_READ_COMMANDS {
            pipe.cmd("MEMORY")
                .arg("USAGE")
                .arg(format!("sample:{i}"))
                .arg("SAMPLES")
                .arg(5);
        }
        for _ in 0..2 {
            let values = router.read_pipeline(&pipe).unwrap();
            assert_eq!(
                values,
                (0..MAX_READ_COMMANDS)
                    .map(|i| redis::Value::Int(i as i64))
                    .collect::<Vec<_>>()
            );
        }
        pipe.cmd("MEMORY").arg("USAGE").arg("must-not-send");
        assert!(router.read_pipeline(&pipe).unwrap_err().contains("1,000"));
        assert_eq!(
            router.query(&redis::cmd("PING")).unwrap(),
            redis::Value::SimpleString("PONG".into())
        );
        peer.join().unwrap();
    }
    #[test]
    fn lost_write_reply_stops_all_requests_without_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            handshake(&mut socket, &mut parser);
            assert_eq!(read(&mut socket, &mut parser)[0], b"INCR");
            // Commit could have happened. Close without its reply.
        });
        let mut router = router(port);
        assert!(
            router
                .query(redis::cmd("INCR").arg("counter"))
                .unwrap_err()
                .contains("may have completed")
        );
        assert!(!router.is_connected());
        assert!(
            router
                .query(redis::cmd("INCR").arg("counter"))
                .unwrap_err()
                .contains("Reconnect")
        );
        peer.join().unwrap();
    }
    #[test]
    fn stalled_native_tls_handshake_obeys_connection_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (_socket, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let start = std::time::Instant::now();
        let settings = Settings {
            host: "127.0.0.1".into(),
            port,
            tls: true,
            cluster: true,
            connect_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        assert!(crate::redis_session::Session::cluster_node(&settings, "").is_err());
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "TLS ignored connect deadline: {:?}",
            start.elapsed()
        );
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "disposable TLS fixtures via scripts/test-redis.sh"]
    fn discovered_nodes_preserve_tls_identity_and_never_fall_back_to_plaintext() {
        let ca = std::env::var_os("RUSTABLE_REDIS_TEST_CA").expect("disposable fixture CA");
        for (host, port) in [("localhost", 16380), ("127.0.0.1", 16379)] {
            let settings = Settings {
                host: "127.0.0.1".into(),
                port: 16380,
                cluster: true,
                tls: true,
                root_certificate: Some(ca.clone().into()),
                connect_timeout: Duration::from_millis(500),
                read_timeout: Duration::from_millis(500),
                ..Default::default()
            };
            let connection =
                crate::redis_session::Session::cluster_node(&settings, "rustable-redis-test-only")
                    .unwrap();
            let mut router = Router::new(
                settings,
                "rustable-redis-test-only".into(),
                connection,
                vec![None; 16384],
            );
            router.routes[hash_slot(b"tls:key") as usize] = Some(Endpoint {
                host: host.into(),
                port,
            });
            let error = router
                .query(redis::cmd("SET").arg("tls:key").arg("must-not-write"))
                .unwrap_err();
            assert!(error.contains("Redis connection"), "{error}");
            assert!(!error.contains("rustable-redis-test-only"));
        }
    }
    #[test]
    #[ignore = "disposable mutual TLS Cluster via scripts/test-redis-cluster-mtls.sh"]
    fn live_mutual_tls_routes_preserve_identity_on_every_primary() {
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_MTLS_QA").as_deref(), Ok("1"));
        let certs = std::path::PathBuf::from(std::env::var_os("RUSTABLE_REDIS_MTLS_CERTS").unwrap());
        let settings = Settings {
            host: "127.0.0.1".into(), port: 16430, cluster: true, tls: true,
            root_certificate: Some(certs.join("ca.crt")),
            client_certificate: Some(certs.join("client.crt")),
            client_key: Some(certs.join("client.key")),
            ..Default::default()
        };
        let password = "rustable-mtls-test-only";
        let mut seed = crate::redis_session::Session::raw_node(&settings, password).unwrap();
        let topology = crate::redis_topology::Cluster::decode(
            redis::cmd("CLUSTER").arg("SHARDS").query(&mut seed).unwrap(), "127.0.0.1", true,
        ).unwrap();
        let routes = topology.routes();
        let keys: Vec<_> = (16430..=16432).map(|port| {
            (0..10000).map(|i| format!("mtls:routing:{i}")).find(|key|
                routes[hash_slot(key.as_bytes()) as usize].as_ref().is_some_and(|endpoint| endpoint.port == port)
            ).expect("key on each primary")
        }).collect();
        // Empty routing forces MOVED discovery rather than pre-populating owners.
        let connection = crate::redis_session::Session::cluster_node(&settings, password).unwrap();
        let mut router = Router::new(settings.clone(), password.into(), connection, vec![None; 16384]);
        for key in &keys {
            assert_eq!(router.query(redis::cmd("INCR").arg(key)).unwrap(), redis::Value::Int(1));
            assert_eq!(router.query(redis::cmd("INCR").arg(key)).unwrap(), redis::Value::Int(2));
        }
        for key in &keys {
            assert_eq!(router.query(redis::cmd("GET").arg(key)).unwrap(), redis::Value::BulkString(b"2".to_vec()));
            router.query(redis::cmd("DEL").arg(key)).unwrap();
        }
        // The seed is valid, but newly discovered connections must still require
        // the identity and the configured server hostname. No plaintext fallback.
        for fault in 0..3 {
            let connection = crate::redis_session::Session::cluster_node(&settings, password).unwrap();
            let mut invalid = settings.clone();
            if fault == 0 { invalid.client_certificate = None; invalid.client_key = None; }
            if fault == 1 {
                invalid.client_certificate = Some(certs.join("untrusted.crt"));
                invalid.client_key = Some(certs.join("untrusted.key"));
            }
            let mut router = Router::new(invalid, password.into(), connection, vec![None; 16384]);
            router.routes[hash_slot(keys[1].as_bytes()) as usize] = Some(Endpoint {
                host: if fault == 2 { "localhost" } else { "127.0.0.1" }.into(), port: 16431,
            });
            assert!(router.query(redis::cmd("INCR").arg(&keys[1])).is_err(), "invalid discovered identity {fault}");
        }
        let mut valid = crate::redis_session::Session::connect(&settings, password).unwrap();
        assert_eq!(valid.execute(&format!("GET {}", keys[1]), true).unwrap(), vec![redis::Value::Nil]);
    }

    #[test]
    #[ignore = "disposable cluster fixture via scripts/test-redis-cluster.sh"]
    fn live_resharding_follows_ask_then_caches_moved_without_duplicate_writes() {
        assert_eq!(std::env::var("RUSTABLE_REDIS_CLUSTER_QA").as_deref(), Ok("1"));
        let password = "rustable-cluster-test-only";
        let settings = Settings { host: "127.0.0.1".into(), port: 16400, tls: false, cluster: true, ..Default::default() };
        let mut nodes: Vec<_> = (16400..=16402).map(|port| crate::redis_session::Session::raw_node(&Settings { port, ..settings.clone() }, password).unwrap()).collect();
        let ids: Vec<String> = nodes.iter_mut().map(|node| redis::cmd("CLUSTER").arg("MYID").query(node).unwrap()).collect();
        let topology = crate::redis_topology::Cluster::decode(redis::cmd("CLUSTER").arg("SHARDS").query(&mut nodes[0]).unwrap(), "127.0.0.1", false).unwrap();
        let routes = topology.routes();
        assert!(routes.iter().flatten().all(|e| e.host == "127.0.0.1" && (16400..=16402).contains(&e.port)));
        let (key, slot) = (0..10000).find_map(|i| {
            let key = format!("rustable:reshard:{i}").into_bytes();
            let slot = hash_slot(&key);
            if routes[slot as usize].as_ref()?.port != 16400 { return None; }
            let count: u64 = redis::cmd("CLUSTER").arg("COUNTKEYSINSLOT").arg(slot).query(&mut nodes[0]).unwrap();
            (count == 0).then_some((key, slot))
        }).expect("empty slot owned by original primary");
        let source = routes[slot as usize].clone().unwrap();
        let target = Endpoint { host: "127.0.0.1".into(), port: 16401 };
        let connection = crate::redis_session::Session::cluster_node(&settings, password).unwrap();
        let mut router = Router::new(settings, password.into(), connection, routes);
        router.query(redis::cmd("SET").arg(&key).arg(0).arg("PX").arg(90000)).unwrap();
        let staying = format!("{{{}}}:staying", String::from_utf8_lossy(&key)).into_bytes();
        assert_eq!(hash_slot(&staying), slot);
        router.query(redis::cmd("SET").arg(&staying).arg("stays on source")).unwrap();
        redis::cmd("CLUSTER").arg("SETSLOT").arg(slot).arg("IMPORTING").arg(&ids[0]).query::<()>(&mut nodes[1]).unwrap();
        redis::cmd("CLUSTER").arg("SETSLOT").arg(slot).arg("MIGRATING").arg(&ids[1]).query::<()>(&mut nodes[0]).unwrap();
        redis::cmd("MIGRATE").arg("127.0.0.1").arg(16401).arg(&key).arg(0).arg(2000).arg("AUTH").arg(password).query::<()>(&mut nodes[0]).unwrap();
        let ask = redis::cmd("GET").arg(&key).query::<redis::Value>(&mut nodes[0]).unwrap_err();
        assert_eq!(ask.code(), Some("ASK"));
        assert_eq!(router.query(redis::cmd("INCR").arg(&key)).unwrap(), redis::Value::Int(1));
        assert_eq!(router.routes[slot as usize], Some(source.clone()), "ASK must not replace the owner");
        let values = router.read_pipeline(redis::pipe().cmd("GET").arg(&key).cmd("GET").arg(&staying).cmd("PTTL").arg(&key)).unwrap();
        assert_eq!(values[0], redis::Value::BulkString(b"1".to_vec()));
        assert_eq!(values[1], redis::Value::BulkString(b"stays on source".to_vec()));
        assert!(matches!(values[2], redis::Value::Int(ttl) if ttl > 0 && ttl <= 90000));
        assert_eq!(router.query(redis::cmd("DEL").arg(&staying)).unwrap(), redis::Value::Int(1));
        for index in [1, 0, 2] {
            redis::cmd("CLUSTER").arg("SETSLOT").arg(slot).arg("NODE").arg(&ids[1]).query::<()>(&mut nodes[index]).unwrap();
        }
        let moved = redis::cmd("GET").arg(&key).query::<redis::Value>(&mut nodes[0]).unwrap_err();
        assert_eq!(moved.code(), Some("MOVED"));
        assert_eq!(router.query(redis::cmd("INCR").arg(&key)).unwrap(), redis::Value::Int(2));
        assert_eq!(router.routes[slot as usize], Some(target));
        assert_eq!(router.query(redis::cmd("INCR").arg(&key)).unwrap(), redis::Value::Int(3));
        assert_eq!(router.query(redis::cmd("DEL").arg(&key)).unwrap(), redis::Value::Int(1));
        // Restore the empty slot so later fixture cases retain the original layout.
        for index in [0, 1, 2] {
            redis::cmd("CLUSTER").arg("SETSLOT").arg(slot).arg("NODE").arg(&ids[0]).query::<()>(&mut nodes[index]).unwrap();
        }
    }
    #[test]
    #[ignore = "disposable cluster fixture via scripts/test-redis-cluster.sh"]
    fn live_routed_writes_and_fanned_out_previews() {
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_CLUSTER_QA").as_deref(),
            Ok("1")
        );
        let settings = Settings {
            host: "127.0.0.1".into(),
            port: 16400,
            tls: false,
            cluster: true,
            ..Default::default()
        };
        let mut session =
            crate::redis_session::Session::connect(&settings, "rustable-cluster-test-only")
                .unwrap();
        let keys: Vec<_> = (0..30)
            .map(|i| format!("routing:{i}").into_bytes())
            .collect();
        for key in &keys {
            session
                .execute(
                    &format!("SET {} value", String::from_utf8_lossy(key)),
                    false,
                )
                .unwrap();
        }
        for _ in 0..2 {
            let rows = session.key_rows(&keys).unwrap();
            assert_eq!(rows.len(), 30);
            assert!(
                rows.iter().all(|r| r.kind == "string"
                    && r.value == redis::Value::BulkString(b"value".to_vec()))
            );
            for key in &keys {
                assert_eq!(session.inspect(key).unwrap().key, *key);
            }
        }
        assert!(
            !session
                .diagnostics(&crate::redis_diagnostics::Request::Server)
                .unwrap()
                .summary
                .is_empty()
        );
        // The maximum diagnostics selection crosses all three primaries,
        // includes vanished keys, and works both cold and warm.
        let sampled: Vec<_> = (0..1000)
            .map(|i| format!("routing:{i}").into_bytes())
            .collect();
        for _ in 0..2 {
            let report = session
                .diagnostics(&crate::redis_diagnostics::Request::Keys(sampled.clone()))
                .unwrap();
            assert_eq!(report.summary[0].1, "30");
            assert_eq!(report.summary[2].1, "970");
            assert_eq!(report.rows.len(), 30);
            assert!(
                report
                    .rows
                    .iter()
                    .all(|row| row[1].parse::<u64>().unwrap() > 0)
            );
        }
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut export_keys = keys.clone();
        export_keys.push(b"routing:absent".to_vec());
        let exported = session.export_archive(&export_keys, 0, &cancel).unwrap();
        assert_eq!(exported.archive.entries.len(), 30);
        assert_eq!(exported.skipped.len(), 1);
        assert_eq!(exported.skipped[0].0, b"routing:absent");
        for (entry, key) in exported.archive.entries.iter().zip(&keys) {
            assert_eq!(&entry.key, key);
            assert_eq!(entry.kind, "string");
            assert_eq!(entry.expires_at_ms, None);
            assert!(!entry.dump.is_empty());
        }
        let mut archive =
            crate::redis_archive::Archive::decode(&exported.archive.encode().unwrap()).unwrap();
        archive.entries[0].key = b"routing:absent".to_vec();
        archive.entries[1].expires_at_ms = Some(1);
        for _ in 0..2 {
            let review = session.review_archive(&archive, 0).unwrap();
            assert_eq!(review.targets.len(), 30);
            assert_eq!(review.targets[0].kind, "none");
            assert!(review.targets[1..].iter().all(|t| t.kind == "string"));
            assert_eq!(review.existing, 29);
            assert_eq!(review.expired, 1);
        }
        assert!(session.review_archive(&archive, 1).is_err());
        assert!(session.export_archive(&keys, 1, &cancel).is_err());
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert!(
            session
                .export_archive(&keys, 0, &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        let first = session
            .scan_filtered(0, b"routing:*", 1000, "string")
            .unwrap();
        assert_eq!(first.cursor, 0);
        assert_eq!(
            first
                .keys
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            keys.iter().cloned().collect()
        );
        assert!(
            session
                .execute(
                    "SET routing:must-not-write value\nDEL routing:0 routing:1",
                    false
                )
                .unwrap_err()
                .contains("CROSSSLOT")
        );
        assert_eq!(
            session
                .execute("EXISTS routing:must-not-write", true)
                .unwrap()[0],
            redis::Value::Int(0)
        );
        let mut cursor = 0;
        let mut found = std::collections::HashSet::new();
        for _ in 0..1000 {
            let page = session
                .scan_filtered(cursor, b"routing:*", 1, "string")
                .unwrap();
            found.extend(page.keys);
            cursor = page.cursor;
            if cursor == 0 {
                break;
            }
        }
        assert_eq!(cursor, 0);
        assert_eq!(found, keys.into_iter().collect());
        session
            .execute("SET {rename}:source original", false)
            .unwrap();
        let mut rename = crate::redis_mutation::Mutation {
            action: crate::redis_mutation::Action::Rename,
            keys: vec![b"{rename}:source".to_vec()],
            kind: "string".into(),
            field: b"another-slot".to_vec(),
            value: Vec::new(),
            expected: Vec::new(),
            length: 0,
            ttl_ms: None,
        };
        assert!(
            session
                .mutate(&rename, false)
                .unwrap_err()
                .contains("CROSSSLOT")
        );
        assert_eq!(
            session.execute("GET {rename}:source", true).unwrap()[0],
            redis::Value::BulkString(b"original".to_vec())
        );
        rename.field = b"{rename}:destination".to_vec();
        assert_eq!(session.mutate(&rename, false).unwrap(), 1);
        assert_eq!(
            session.execute("GET {rename}:destination", true).unwrap()[0],
            redis::Value::BulkString(b"original".to_vec())
        );
        assert_eq!(
            session.execute("EXISTS {rename}:source", true).unwrap()[0],
            redis::Value::Int(0)
        );

        use crate::redis_archive::{Collision, Expiry, State};
        assert!(
            session
                .import_archive(&archive, Collision::Skip, Expiry::Preserve, true, &cancel)
                .is_err()
        );
        let cancelled = session
            .import_archive(&archive, Collision::Skip, Expiry::Preserve, false, &cancel)
            .unwrap();
        assert!(
            cancelled
                .outcomes
                .iter()
                .all(|o| o.state == State::NotAttempted)
        );
        cancel.store(false, std::sync::atomic::Ordering::Release);
        archive.entries[0].key = b"archive:binary\0\xff".to_vec();
        let report = session
            .import_archive(&archive, Collision::Skip, Expiry::Preserve, false, &cancel)
            .unwrap();
        assert!(report.stopped.is_none(), "{:?}", report.stopped);
        assert_eq!(report.outcomes[0].state, State::Restored);
        assert_eq!(report.outcomes[1].state, State::Expired);
        assert!(
            report.outcomes[2..]
                .iter()
                .all(|o| o.state == State::Exists)
        );
        let report = session
            .import_archive(
                &archive,
                Collision::Replace,
                Expiry::Persistent,
                false,
                &cancel,
            )
            .unwrap();
        assert!(report.stopped.is_none());
        assert!(report.outcomes.iter().all(|o| o.state == State::Restored));
        for entry in &archive.entries {
            let snapshot = session.inspect(&entry.key).unwrap();
            assert_eq!(snapshot.value, redis::Value::BulkString(b"value".to_vec()));
            assert_eq!(snapshot.ttl_ms, -1);
        }
        archive.entries[0].dump = b"invalid-dump".to_vec();
        let report = session
            .import_archive(
                &archive,
                Collision::Replace,
                Expiry::Persistent,
                false,
                &cancel,
            )
            .unwrap();
        assert!(report.stopped.is_none());
        assert_eq!(report.outcomes[0].state, State::Error);
        assert!(
            report.outcomes[1..]
                .iter()
                .all(|o| o.state == State::Restored)
        );
        assert_eq!(
            session.inspect(&archive.entries[0].key).unwrap().value,
            redis::Value::BulkString(b"value".to_vec())
        );

        let mut bulk = crate::redis_mutation::Mutation {
            action: crate::redis_mutation::Action::ExpireKeys,
            keys: archive.entries.iter().map(|e| e.key.clone()).collect(),
            kind: String::new(),
            field: vec![],
            value: vec![],
            expected: vec![],
            length: 0,
            ttl_ms: Some(60000),
        };
        assert!(session.bulk_mutate(&bulk, true, &cancel).is_err());
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert!(
            session
                .bulk_mutate(&bulk, false, &cancel)
                .unwrap()
                .outcomes
                .iter()
                .all(|r| matches!(r.state, WriteState::NotAttempted))
        );
        cancel.store(false, std::sync::atomic::Ordering::Release);
        for ttl in [Some(60000), None] {
            bulk.ttl_ms = ttl;
            let report = session.bulk_mutate(&bulk, false, &cancel).unwrap();
            assert!(report.stopped.is_none());
            assert!(
                report
                    .outcomes
                    .iter()
                    .all(|r| matches!(r.state, WriteState::Confirmed(true)))
            );
        }
        let report = session.bulk_mutate(&bulk, false, &cancel).unwrap();
        assert!(
            report
                .outcomes
                .iter()
                .all(|r| matches!(r.state, WriteState::Confirmed(false)))
        );
        bulk.action = crate::redis_mutation::Action::DeleteKeys;
        let report = session.bulk_mutate(&bulk, false, &cancel).unwrap();
        assert!(report.stopped.is_none());
        assert!(
            report
                .outcomes
                .iter()
                .all(|r| matches!(r.state, WriteState::Confirmed(true)))
        );
        let report = session.bulk_mutate(&bulk, false, &cancel).unwrap();
        assert!(
            report
                .outcomes
                .iter()
                .all(|r| matches!(r.state, WriteState::Confirmed(false)))
        );
        assert!(
            session
                .execute("DEL routing:0 routing:1", false)
                .unwrap_err()
                .contains("CROSSSLOT")
        );
    }
}
