//! Bounded, validated Redis Cluster discovery. Discovery does not contact announced nodes.
use redis::Value;
use std::collections::{BTreeMap, HashSet};
const MAX_NODES: usize = 1000;
const SLOTS: usize = 16384;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Endpoint {
    pub host: String,
    pub port: u16,
}
impl Endpoint {
    pub fn new(host: String, port: i64) -> Result<Self, String> {
        if host.is_empty()
            || host.len() > 253
            || host == "?"
            || host.chars().any(|c| {
                c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@' | '#' | '?')
            })
            || !(1..=65535).contains(&port)
        {
            return Err("Invalid or unannounced Redis endpoint".into());
        }
        // IPv6 is accepted as a bare address, never a URI or host:port string.
        if host.contains(':') && host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err("Invalid Redis IPv6 endpoint".into());
        }
        Ok(Self {
            host,
            port: port as u16,
        })
    }
    pub fn label(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}
#[derive(Debug)]
struct Node {
    id: String,
    endpoint: Option<Endpoint>,
    primary: bool,
    health: String,
}
#[derive(Debug)]
struct Shard {
    ranges: Vec<(u16, u16)>,
    nodes: Vec<Node>,
}
#[derive(Debug)]
pub(crate) struct Cluster {
    shards: Vec<Shard>,
    covered: usize,
}
fn text(value: &Value) -> Result<String, String> {
    let text = match value {
        Value::BulkString(v) => std::str::from_utf8(v).map_err(|_| "Invalid topology text")?,
        Value::SimpleString(v) => v,
        _ => return Err("Expected topology text".into()),
    };
    if text.len() > 1024 || text.chars().any(char::is_control) {
        return Err("Invalid topology text".into());
    }
    Ok(text.to_owned())
}
fn array(value: Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Array(v) => Ok(v),
        _ => Err("Expected topology array".into()),
    }
}
fn fields(value: Value) -> Result<BTreeMap<String, Value>, String> {
    let pairs = match value {
        Value::Map(pairs) => pairs,
        Value::Array(values) if values.len() <= 64 && values.len().is_multiple_of(2) => {
            let mut values = values.into_iter();
            let mut pairs = Vec::new();
            while let Some(key) = values.next() {
                pairs.push((key, values.next().unwrap()));
            }
            pairs
        }
        _ => return Err("Invalid topology fields".into()),
    };
    if pairs.len() > 32 {
        return Err("Too many topology fields".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in pairs {
        if result.insert(text(&key)?, value).is_some() {
            return Err("Duplicate topology field".into());
        }
    }
    Ok(result)
}
fn take(fields: &mut BTreeMap<String, Value>, key: &str) -> Result<Value, String> {
    fields
        .remove(key)
        .ok_or_else(|| format!("Missing topology field: {key}"))
}
fn integer(value: Value) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(n),
        _ => Err("Expected topology integer".into()),
    }
}
impl Cluster {
    pub fn decode(value: Value, source_host: &str, tls: bool) -> Result<Self, String> {
        let values = array(value)?;
        if values.is_empty() || values.len() > MAX_NODES {
            return Err("Cluster must contain 1–1,000 shards".into());
        }
        let mut shards = Vec::new();
        let mut occupied = [false; SLOTS];
        let mut ids = HashSet::new();
        let mut endpoints = HashSet::new();
        for value in values {
            let mut shard_fields = fields(value)?;
            let slots = array(take(&mut shard_fields, "slots")?)?;
            if !slots.len().is_multiple_of(2) || slots.len() > SLOTS * 2 {
                return Err("Invalid cluster slot ranges".into());
            }
            let mut ranges = Vec::new();
            for pair in slots.as_chunks::<2>().0 {
                let start = integer(pair[0].clone())?;
                let end = integer(pair[1].clone())?;
                if start < 0 || end < start || end >= SLOTS as i64 {
                    return Err("Invalid cluster slot range".into());
                }
                for slot in &mut occupied[start as usize..=end as usize] {
                    if *slot {
                        return Err("Overlapping cluster slot ownership".into());
                    }
                    *slot = true;
                }
                ranges.push((start as u16, end as u16));
            }
            let values = array(take(&mut shard_fields, "nodes")?)?;
            if values.is_empty() || ids.len() + values.len() > MAX_NODES {
                return Err("Cluster exceeds 1,000 nodes or contains an empty shard".into());
            }
            let mut nodes = Vec::new();
            for value in values {
                let mut f = fields(value)?;
                let id = text(&take(&mut f, "id")?)?;
                if id.len() != 40
                    || !id.bytes().all(|b| b.is_ascii_hexdigit())
                    || !ids.insert(id.clone())
                {
                    return Err("Invalid or duplicate cluster node ID".into());
                }
                let role = text(&take(&mut f, "role")?)?;
                if !matches!(role.as_str(), "master" | "replica") {
                    return Err("Invalid cluster node role".into());
                }
                let health = text(&take(&mut f, "health")?)?;
                if !matches!(health.as_str(), "online" | "failed" | "loading") {
                    return Err("Invalid cluster node health".into());
                }
                let host = match take(&mut f, "endpoint")? {
                    Value::Nil => source_host.to_owned(),
                    value => {
                        let h = text(&value)?;
                        if h.is_empty() {
                            source_host.to_owned()
                        } else {
                            h
                        }
                    }
                };
                let port = f
                    .remove(if tls { "tls-port" } else { "port" })
                    .map(integer)
                    .transpose()?;
                let endpoint = match port {
                    Some(port) if host != "?" && port != 0 => Some(Endpoint::new(host, port)?),
                    _ => None,
                };
                if let Some(endpoint) = &endpoint
                    && !endpoints.insert(endpoint.clone())
                {
                    return Err("Duplicate cluster endpoint".into());
                }
                nodes.push(Node {
                    id,
                    endpoint,
                    primary: role == "master",
                    health,
                });
            }
            if nodes.iter().filter(|n| n.primary).count() != 1 {
                return Err("Each shard must identify exactly one primary".into());
            }
            shards.push(Shard { ranges, nodes });
        }
        Ok(Self {
            shards,
            covered: occupied.iter().filter(|s| **s).count(),
        })
    }
    pub fn routes(&self) -> Vec<Option<Endpoint>> {
        let mut routes=vec![None;SLOTS];
        for shard in &self.shards {
            let primary=shard.nodes.iter().find(|n|n.primary && n.health=="online");
            let endpoint=primary.and_then(|n|n.endpoint.clone());
            for &(start,end) in &shard.ranges {
                routes[start as usize..=end as usize].fill(endpoint.clone());
            }
        }
        routes
    }
    /// Discovery candidates only; replicas never become key routes here.
    pub fn reconnect_nodes(&self) -> Vec<Endpoint> {
        let mut candidates = Vec::new();
        let mut roles = [0usize; 2];
        // Reserve room for both roles; large clusters must not crowd out replicas.
        // Within each pass prefer online nodes, round-robin across shards.
        for balanced in [true, false] {
            for online in [true, false] {
                for depth in 0..MAX_NODES {
                    let mut found = false;
                    for shard in &self.shards {
                        if let Some(node) = shard.nodes.get(depth) {
                            found = true;
                            let role = usize::from(node.primary);
                            if (node.health == "online") == online && (!balanced || roles[role] < 4)
                                && let Some(endpoint) = &node.endpoint
                                && !candidates.contains(endpoint)
                            {
                                candidates.push(endpoint.clone());
                                roles[role] += 1;
                                if candidates.len() == 8 { return candidates; }
                            }
                        }
                    }
                    if !found { break; }
                }
            }
        }
        candidates
    }
    pub fn report(&self) -> crate::redis_diagnostics::Report {
        let count: usize = self.shards.iter().map(|s| s.nodes.len()).sum();
        let unavailable = self
            .shards
            .iter()
            .flat_map(|s| &s.nodes)
            .filter(|n| n.health != "online" || n.endpoint.is_none())
            .count();
        let mut rows = Vec::with_capacity(count);
        for shard in &self.shards {
            let mut slots = shard
                .ranges
                .iter()
                .take(16)
                .map(|(a, b)| format!("{a}–{b}"))
                .collect::<Vec<_>>()
                .join(", ");
            if shard.ranges.len() > 16 {
                slots.push_str(&format!(" (+{} ranges)", shard.ranges.len() - 16));
            }
            for node in &shard.nodes {
                rows.push([
                    node.endpoint
                        .as_ref()
                        .map(Endpoint::label)
                        .unwrap_or_else(|| "Unavailable endpoint".into()),
                    format!(
                        "{} · {}",
                        if node.primary { "Primary" } else { "Replica" },
                        node.health
                    ),
                    format!("Slots {slots} · {}", node.id),
                ]);
            }
        }
        rows.sort_by(|a, b| a[0].cmp(&b[0]));
        crate::redis_diagnostics::Report {
            namespaces: vec![],
            summary:vec![("Shards".into(),self.shards.len().to_string()),("Nodes".into(),count.to_string()),("Slots assigned".into(),format!("{} / {SLOTS}",self.covered)),("Unavailable nodes".into(),unavailable.to_string())],
            rows,analysis:false,topology:true,module_key:None,vector_element:None,vector_matches:None,
            footer:"Topology reported by the connected node. Announced endpoints have not been contacted. Key browsing currently covers the connected node only.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn string(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }
    fn map(items: Vec<(&str, Value)>) -> Value {
        Value::Array(
            items
                .into_iter()
                .flat_map(|(k, v)| [string(k), v])
                .collect(),
        )
    }
    fn node(endpoint: Value, tls: bool) -> Value {
        map(vec![
            ("id", string(&"a".repeat(40))),
            ("endpoint", endpoint),
            (if tls { "tls-port" } else { "port" }, Value::Int(6379)),
            ("role", string("master")),
            ("health", string("online")),
        ])
    }
    fn shard(slots: &[i64], node: Value) -> Value {
        map(vec![
            (
                "slots",
                Value::Array(slots.iter().map(|n| Value::Int(*n)).collect()),
            ),
            ("nodes", Value::Array(vec![node])),
        ])
    }
    #[test]
    fn reconnect_candidates_include_replicas_without_routing_reads_to_them() {
        let cluster = Cluster { covered: 5, shards: (0..5).map(|i| Shard {
            ranges: vec![(i, i)],
            nodes: [true, false].into_iter().map(|primary| Node {
                id: format!("{i:040x}"), primary, health: "online".into(),
                endpoint: Some(Endpoint { host: "fixture.example".into(), port: 16000 + i + if primary { 0 } else { 10 } }),
            }).collect(),
        }).collect() };
        let candidates = cluster.reconnect_nodes();
        assert_eq!(candidates.iter().map(|e| e.port).collect::<Vec<_>>(), vec![16000, 16001, 16002, 16003, 16010, 16011, 16012, 16013]);
        assert!(cluster.routes().iter().flatten().all(|e| e.port < 16010));
        let mut unavailable = cluster;
        for shard in &mut unavailable.shards { shard.nodes[0].health = "failed".into(); }
        assert_eq!(unavailable.reconnect_nodes().len(), 8);
        assert!(unavailable.reconnect_nodes()[..4].iter().all(|e| e.port >= 16010));
        assert!(unavailable.routes().iter().all(Option::is_none));
        unavailable.shards[0].nodes[1].endpoint = None;
        assert!(!unavailable.reconnect_nodes().iter().any(|e| e.port == 16010));
    }
    #[test]
    fn discovery_validates_ranges_and_preserves_announced_tls_identity() {
        let value = Value::Array(vec![shard(
            &[0, 16383],
            node(string("redis.example"), true),
        )]);
        let cluster = Cluster::decode(value, "seed.example", true).unwrap();
        assert_eq!(cluster.covered, SLOTS);
        assert_eq!(
            cluster.shards[0].nodes[0].endpoint.as_ref().unwrap().host,
            "redis.example"
        );
        assert_eq!(cluster.report().rows.len(), 1);
        for slots in [&[-1, 2][..], &[0, 16384], &[2, 1], &[0], &[0, 10, 10, 20]] {
            assert!(
                Cluster::decode(
                    Value::Array(vec![shard(slots, node(string("localhost"), false))]),
                    "seed",
                    false
                )
                .is_err()
            );
        }
    }
    #[test]
    fn unknown_endpoints_do_not_downgrade_tls_or_invent_hosts() {
        for host in [Value::Nil, string("")] {
            let cluster = Cluster::decode(
                Value::Array(vec![shard(&[0, 1], node(host, true))]),
                "seed.example",
                true,
            )
            .unwrap();
            assert_eq!(
                cluster.shards[0].nodes[0].endpoint.as_ref().unwrap().host,
                "seed.example"
            );
        }
        for (host, tls) in [(string("?"), false), (string("localhost"), true)] {
            let cluster = Cluster::decode(
                Value::Array(vec![shard(&[0, 1], node(host, false))]),
                "seed",
                tls,
            )
            .unwrap();
            assert!(cluster.shards[0].nodes[0].endpoint.is_none());
        }
        for host in [
            "https://host",
            "user@host",
            "host:6379",
            "bad\nname",
            "[::1]",
            "?",
        ] {
            assert!(Endpoint::new(host.into(), 6379).is_err());
        }
        assert_eq!(
            Endpoint::new("::1".into(), 6379).unwrap().label(),
            "[::1]:6379"
        );
    }
    #[test]
    fn duplicate_nodes_fields_and_unbounded_topologies_are_rejected() {
        let a = shard(&[0, 1], node(string("localhost"), false));
        let b = shard(&[2, 3], node(string("localhost"), false));
        assert!(Cluster::decode(Value::Array(vec![a, b]), "seed", false).is_err());
        assert!(fields(map(vec![("x", Value::Nil), ("x", Value::Nil)])).is_err());
        assert!(Cluster::decode(Value::Array(vec![Value::Nil; 1001]), "seed", false).is_err());
        assert!(Cluster::decode(Value::Array(vec![]), "seed", false).is_err());
    }
    #[test]
    fn repeated_discovery_uses_one_exchange_without_contacting_advertised_nodes() {
        use std::{io::Write, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut parser = redis::Parser::new();
            loop {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                socket
                    .write_all(if args[0] == b"PING" {
                        b"+PONG\r\n"
                    } else {
                        b"+OK\r\n"
                    })
                    .unwrap();
                if args[0] == b"SELECT" {
                    break;
                }
            }
            for _ in 0..2 {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                assert_eq!(args, vec![b"CLUSTER".to_vec(), b"SHARDS".to_vec()]);
                let response = format!(
                    "*1\r\n*4\r\n$5\r\nslots\r\n*2\r\n:0\r\n:16383\r\n$5\r\nnodes\r\n*1\r\n*10\r\n$2\r\nid\r\n$40\r\n{}\r\n$8\r\nendpoint\r\n$17\r\nunreachable.testX\r\n$4\r\nport\r\n:6379\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$6\r\nhealth\r\n$6\r\nonline\r\n",
                    "a".repeat(40)
                );
                socket.write_all(response.as_bytes()).unwrap();
            }
        });
        let mut session = crate::redis_session::Session::connect(
            &crate::redis_settings::Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "",
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                session
                    .diagnostics(&crate::redis_diagnostics::Request::Cluster)
                    .unwrap()
                    .rows
                    .len(),
                1
            );
        }
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "manual topology CPU workload"]
    fn redis_topology_frame_benchmark() {
        let report = crate::redis_diagnostics::Report {
            namespaces: vec![],
            summary: vec![
                ("Shards".into(), "500".into()),
                ("Nodes".into(), "1000".into()),
                ("Slots assigned".into(), "16384 / 16384".into()),
                ("Unavailable nodes".into(), "0".into()),
            ],
            rows: (0..1000)
                .map(|i| {
                    [
                        format!("redis-{i:04}.example.test:6379"),
                        "Primary · online".into(),
                        format!("Slots 0–16383 · {i:040x}"),
                    ]
                })
                .collect(),
            analysis: false,
            topology: true,
            module_key: None,
            vector_element: None,
            vector_matches: None,
            footer: "Topology snapshot".into(),
        };
        let ctx = eframe::egui::Context::default();
        let mut samples = Vec::new();
        for frame in 0..180 {
            let start = std::time::Instant::now();
            let out = ctx.run(
                eframe::egui::RawInput {
                    screen_rect: Some(eframe::egui::Rect::from_min_size(
                        eframe::egui::Pos2::ZERO,
                        eframe::egui::vec2(1200.0, 760.0),
                    )),
                    time: Some(frame as f64 / 120.0),
                    ..Default::default()
                },
                |ctx| {
                    eframe::egui::Window::new("Cluster topology")
                        .default_width(800.0)
                        .show(ctx, |ui| {
                            let _ = report.show(ui);
                        });
                },
            );
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            if frame >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report("redis/topology", &samples);
    }
    #[test]
    #[ignore = "disposable cluster fixture via scripts/test-redis-cluster.sh"]
    fn live_cluster_discovery_reports_three_primaries_and_all_slots() {
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_CLUSTER_QA").as_deref(),
            Ok("1")
        );
        let settings = crate::redis_settings::Settings {
            host: "127.0.0.1".into(),
            port: 16400,
            tls: false,
            ..Default::default()
        };
        let mut session =
            crate::redis_session::Session::connect(&settings, "rustable-cluster-test-only")
                .unwrap();
        for _ in 0..2 {
            let report = session
                .diagnostics(&crate::redis_diagnostics::Request::Cluster)
                .unwrap();
            assert_eq!(report.rows.len(), 3);
            assert!(
                report
                    .summary
                    .contains(&("Slots assigned".into(), "16384 / 16384".into()))
            );
            assert!(report.rows.iter().all(|r| r[1] == "Primary · online"));
        }
    }
}
