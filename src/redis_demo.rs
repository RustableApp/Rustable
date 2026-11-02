//! Bounded, disposable Redis-compatible demo data, implemented in Rust/WASM.
//! This is not a Redis server: unsupported commands fail explicitly.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use crate::redis_session::{KeyPage, KeyRow, Snapshot};
#[cfg(not(target_arch = "wasm32"))]
pub use redis::Value;
use std::collections::BTreeMap;
use web_time::Instant;
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Value {
    BulkString(Vec<u8>),
    Array(Vec<Value>),
    Set(Vec<Value>),
    Map(Vec<(Value, Value)>),
    SimpleString(String),
    Nil,
    Okay,
    Int(i64),
    Double(f64),
    Boolean(bool),
    Unsupported,
}
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct KeyPage {
    pub cursor: u64,
    pub keys: Vec<Vec<u8>>,
}
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct KeyRow {
    pub key: Vec<u8>,
    pub kind: String,
    pub ttl_ms: i64,
    pub value: Value,
}
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub key: Vec<u8>,
    pub kind: String,
    pub ttl_ms: i64,
    pub length: Option<u64>,
    pub value: Value,
    pub truncated: bool,
}
#[derive(Clone)]
struct Entry {
    kind: &'static str,
    value: Value,
    expires: Option<Instant>,
}
pub struct Session {
    databases: Vec<BTreeMap<Vec<u8>, Entry>>,
    database: usize,
}
const HELP: &str = "In-memory Redis demo (Rust/WASM), not a Redis server. Data resets on page reload. Supported: PING, ECHO, GET, SET, MGET, DEL, EXISTS, TYPE, TTL, PTTL, EXPIRE, PERSIST, DBSIZE, HGET, HGETALL, HLEN, LRANGE, LLEN, SMEMBERS, SCARD, ZRANGE WITHSCORES, ZCARD, INFO. Use the database picker for db0–db15. Disable read-only mode to edit demo data.";
fn bulk(s: impl AsRef<[u8]>) -> Value {
    Value::BulkString(s.as_ref().to_vec())
}
impl Session {
    pub fn new() -> Self {
        let mut s = Self {
            databases: vec![BTreeMap::new(); 16],
            database: 0,
        };
        for (key, kind, value) in [
            (
                "welcome",
                "string",
                bulk("Hello from Rust + WASM! Try GET welcome, or INFO for demo commands."),
            ),
            (
                "cache:homepage",
                "string",
                bulk(r#"{"title":"Rustable","visitors":1284,"cached":true}"#),
            ),
            (
                "user:42",
                "hash",
                Value::Array(vec![
                    bulk("name"),
                    bulk("Avery Chen"),
                    bulk("plan"),
                    bulk("Pro"),
                    bulk("country"),
                    bulk("Canada"),
                ]),
            ),
            (
                "queue:emails",
                "list",
                Value::Array(vec![
                    bulk("welcome:42"),
                    bulk("receipt:108"),
                    bulk("weekly-digest"),
                ]),
            ),
            (
                "tags:popular",
                "set",
                Value::Array(vec![
                    bulk("rust"),
                    bulk("sqlite"),
                    bulk("redis"),
                    bulk("wasm"),
                ]),
            ),
            (
                "leaderboard",
                "zset",
                Value::Array(vec![
                    bulk("avery"),
                    bulk("125"),
                    bulk("sofia"),
                    bulk("280"),
                    bulk("liam"),
                    bulk("420"),
                ]),
            ),
            (
                "session:temporary",
                "string",
                bulk("This key expires after five minutes."),
            ),
        ] {
            s.databases[0].insert(
                key.as_bytes().to_vec(),
                Entry {
                    kind,
                    value,
                    expires: None,
                },
            );
        }
        s.databases[0]
            .get_mut(b"session:temporary".as_slice())
            .unwrap()
            .expires = Instant::now().checked_add(std::time::Duration::from_secs(300));
        s.databases[1].insert(
            b"environment".to_vec(),
            Entry {
                kind: "string",
                value: bulk("A separate logical database. Changes here do not affect db0."),
                expires: None,
            },
        );
        s
    }
    pub fn select(&mut self, database: u32) -> Result<(), String> {
        if database >= 16 {
            return Err("Demo databases range from 0 to 15.".into());
        }
        self.database = database as usize;
        Ok(())
    }
    fn purge(&mut self) {
        let now = Instant::now();
        self.databases[self.database].retain(|_, e| e.expires.is_none_or(|at| at > now));
    }
    fn entry(&mut self, key: &[u8]) -> Option<&Entry> {
        self.purge();
        self.databases[self.database].get(key)
    }
    pub fn inspect(&mut self, key: &[u8]) -> Result<Snapshot, String> {
        let entry = self.entry(key);
        let (kind, value, ttl_ms, length) = match entry {
            None => ("none", Value::Nil, -2, 0),
            Some(e) => (
                e.kind,
                e.value.clone(),
                e.expires.map_or(-1, |at| {
                    at.saturating_duration_since(Instant::now()).as_millis() as i64
                }),
                match &e.value {
                    Value::BulkString(v) => v.len(),
                    Value::Array(v) => {
                        v.len()
                            / if matches!(e.kind, "hash" | "zset") {
                                2
                            } else {
                                1
                            }
                    }
                    _ => 0,
                },
            ),
        };
        Ok(Snapshot {
            key: key.to_vec(),
            kind: kind.into(),
            value,
            ttl_ms,
            length: Some(length as u64),
            truncated: false,
        })
    }
    pub fn scan(&mut self, cursor: u64, pattern: &[u8], count: u32) -> Result<KeyPage, String> {
        if pattern.len() > 1024 {
            return Err("Demo search patterns are limited to 1 KiB.".into());
        }
        self.purge();
        let start = usize::try_from(cursor).map_err(|_| "Invalid cursor")?;
        let keys: Vec<_> = self.databases[self.database]
            .keys()
            .filter(|k| glob(pattern, k))
            .cloned()
            .collect();
        let end = start
            .saturating_add(count.clamp(1, 1000) as usize)
            .min(keys.len());
        Ok(KeyPage {
            cursor: if end < keys.len() { end as u64 } else { 0 },
            keys: keys.get(start..end).unwrap_or_default().to_vec(),
        })
    }
    pub fn key_rows(&mut self, keys: &[Vec<u8>]) -> Result<Vec<KeyRow>, String> {
        keys.iter()
            .map(|key| {
                self.inspect(key).map(|s| KeyRow {
                    key: s.key,
                    kind: s.kind,
                    ttl_ms: s.ttl_ms,
                    value: s.value,
                })
            })
            .collect()
    }
    fn writable(safe: bool) -> Result<(), String> {
        if safe {
            Err("Read-only mode is enabled.".into())
        } else {
            Ok(())
        }
    }
    pub fn replace_string(
        &mut self,
        key: &[u8],
        original: &[u8],
        value: &[u8],
        safe: bool,
    ) -> Result<(), String> {
        Self::writable(safe)?;
        if value.len() > 65536 {
            return Err("Demo values are limited to 64 KiB.".into());
        }
        let e = self.entry(key).ok_or("Key no longer exists.")?;
        if !matches!(&e.value, Value::BulkString(bytes) if bytes == original) {
            return Err("Value changed; refresh before saving.".into());
        }
        self.check_capacity(key, value)?;
        self.databases[self.database].get_mut(key).unwrap().value = bulk(value);
        Ok(())
    }
    pub fn delete(&mut self, key: &[u8], safe: bool) -> Result<bool, String> {
        Self::writable(safe)?;
        self.purge();
        Ok(self.databases[self.database].remove(key).is_some())
    }
    pub fn expire(&mut self, key: &[u8], ttl: Option<u64>, safe: bool) -> Result<bool, String> {
        Self::writable(safe)?;
        self.purge();
        let expires = ttl
            .map(|seconds| {
                Instant::now()
                    .checked_add(std::time::Duration::from_secs(seconds))
                    .ok_or("Expiry is too large.")
            })
            .transpose()?;
        let Some(e) = self.databases[self.database].get_mut(key) else {
            return Ok(false);
        };
        let changed = expires.is_some() || e.expires.is_some();
        e.expires = expires;
        Ok(changed)
    }
    pub fn execute(&mut self, text: &str, safe: bool) -> Result<Vec<Value>, String> {
        let commands = crate::redis_commands::parse(text)?;
        for args in &commands {
            crate::redis_commands::validate(args, safe)?;
            Self::validate_demo(args)?;
        }
        commands.iter().map(|args| self.command(args)).collect()
    }
    fn validate_demo(a: &[Vec<u8>]) -> Result<(), String> {
        let n = String::from_utf8_lossy(&a[0]).to_ascii_uppercase();
        let valid = match n.as_str() {
            "PING" => (1..=2).contains(&a.len()),
            "INFO" | "DBSIZE" => a.len() == 1,
            "ECHO" | "GET" | "TYPE" | "TTL" | "PTTL" | "PERSIST" | "HGETALL" | "HLEN" | "LLEN"
            | "SMEMBERS" | "SCARD" | "ZCARD" => a.len() == 2,
            "SET" | "HGET" | "EXPIRE" => a.len() == 3,
            "MGET" | "DEL" | "EXISTS" => a.len() >= 2,
            "LRANGE" => a.len() == 4,
            "ZRANGE" => a.len() == 4 || (a.len() == 5 && a[4].eq_ignore_ascii_case(b"WITHSCORES")),
            _ => {
                return Err(format!(
                    "{n} is not implemented by this in-memory demo. Run INFO for supported commands. The desktop app connects to a real Redis server."
                ));
            }
        };
        if !valid {
            return Err(format!(
                "Wrong arguments or unsupported options for {n}. Run INFO for demo commands."
            ));
        }
        if a.iter().any(|a| a.len() > 65536) {
            return Err("Demo arguments are limited to 64 KiB.".into());
        }
        Ok(())
    }
    fn check_capacity(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        if key.len() > 1024 {
            return Err("Demo keys are limited to 1 KiB.".into());
        }
        let bytes: usize = self
            .databases
            .iter()
            .flat_map(|db| db.iter())
            .map(|(k, e)| {
                k.len()
                    + match &e.value {
                        Value::BulkString(v) => v.len(),
                        Value::Array(v) => v
                            .iter()
                            .map(|v| {
                                if let Value::BulkString(b) = v {
                                    b.len()
                                } else {
                                    0
                                }
                            })
                            .sum(),
                        _ => 0,
                    }
            })
            .sum();
        if bytes + key.len() + value.len() > 4 * 1024 * 1024 {
            return Err(
                "Demo memory limit reached (4 MiB). Delete some keys or reload the page.".into(),
            );
        }
        Ok(())
    }
    fn command(&mut self, a: &[Vec<u8>]) -> Result<Value, String> {
        let n = String::from_utf8_lossy(&a[0]).to_ascii_uppercase();
        match n.as_str() {
            "PING" => return Ok(a.get(1).map_or(Value::SimpleString("PONG".into()), bulk)),
            "ECHO" => return Ok(bulk(&a[1])),
            "INFO" => return Ok(bulk(HELP)),
            "DBSIZE" => {
                self.purge();
                return Ok(Value::Int(self.databases[self.database].len() as i64));
            }
            "SET" => {
                self.purge();
                self.check_capacity(&a[1], &a[2])?;
                if !self.databases[self.database].contains_key(&a[1])
                    && self.databases[self.database].len() >= 256
                {
                    return Err("Demo limit: 256 keys per database.".into());
                }
                self.databases[self.database].insert(
                    a[1].clone(),
                    Entry {
                        kind: "string",
                        value: bulk(&a[2]),
                        expires: None,
                    },
                );
                return Ok(Value::Okay);
            }
            "DEL" => {
                let mut count = 0;
                for key in &a[1..] {
                    count += i64::from(self.delete(key, false)?);
                }
                return Ok(Value::Int(count));
            }
            "EXISTS" => {
                let count = a[1..]
                    .iter()
                    .filter(|key| self.entry(key).is_some())
                    .count();
                return Ok(Value::Int(count as i64));
            }
            "MGET" => {
                let mut out = vec![];
                for key in &a[1..] {
                    out.push(
                        self.entry(key)
                            .filter(|e| e.kind == "string")
                            .map_or(Value::Nil, |e| e.value.clone()),
                    );
                }
                return Ok(Value::Array(out));
            }
            "EXPIRE" => {
                let seconds = integer(&a[2])?;
                let changed = if seconds <= 0 {
                    self.delete(&a[1], false)?
                } else {
                    self.expire(&a[1], Some(seconds as u64), false)?
                };
                return Ok(Value::Int(i64::from(changed)));
            }
            "PERSIST" => {
                return self
                    .expire(&a[1], None, false)
                    .map(|v| Value::Int(i64::from(v)));
            }
            _ => {}
        }
        let s = self.inspect(&a[1])?;
        match n.as_str() {
            "TYPE" => Ok(Value::SimpleString(s.kind)),
            "TTL" | "PTTL" => Ok(Value::Int(if n == "TTL" && s.ttl_ms >= 0 {
                s.ttl_ms / 1000
            } else {
                s.ttl_ms
            })),
            _ => {
                let expected = match n.as_str() {
                    "GET" => "string",
                    "HGET" | "HGETALL" | "HLEN" => "hash",
                    "LRANGE" | "LLEN" => "list",
                    "SMEMBERS" | "SCARD" => "set",
                    _ => "zset",
                };
                if s.kind != "none" && s.kind != expected {
                    return Err(
                        "WRONGTYPE Operation against a key holding the wrong kind of value".into(),
                    );
                }
                match n.as_str() {
                    "GET" => Ok(s.value),
                    "HLEN" | "LLEN" | "SCARD" | "ZCARD" => {
                        Ok(Value::Int(s.length.unwrap_or(0) as i64))
                    }
                    "HGET" => {
                        let Value::Array(v) = s.value else {
                            return Ok(Value::Nil);
                        };
                        Ok(v.as_chunks::<2>().0.iter()
                            .find(|v| matches!(&v[0], Value::BulkString(k) if k == &a[2]))
                            .map_or(Value::Nil, |v| v[1].clone()))
                    }
                    "HGETALL" | "SMEMBERS" => Ok(if s.kind == "none" {
                        Value::Array(vec![])
                    } else {
                        s.value
                    }),
                    "LRANGE" | "ZRANGE" => {
                        let start = integer(&a[2])?;
                        let end = integer(&a[3])?;
                        let Value::Array(v) = s.value else {
                            return Ok(Value::Array(vec![]));
                        };
                        let stride = if expected == "zset" { 2 } else { 1 };
                        let len = (v.len() / stride) as i64;
                        let start = if start < 0 {
                            len.saturating_add(start)
                        } else {
                            start
                        }
                        .max(0);
                        let end = if end < 0 {
                            len.saturating_add(end)
                        } else {
                            end
                        }
                        .min(len - 1);
                        let mut out = vec![];
                        if start <= end {
                            for i in start..=end {
                                out.push(v[i as usize * stride].clone());
                                if stride == 2 && a.len() == 5 {
                                    out.push(v[i as usize * stride + 1].clone());
                                }
                            }
                        }
                        Ok(Value::Array(out))
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}
fn integer(bytes: &[u8]) -> Result<i64, String> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "Expected an integer in range.".into())
}
// Iterative wildcard matching avoids recursive backtracking and preserves binary keys.
fn glob(pattern: &[u8], key: &[u8]) -> bool {
    let (mut p, mut k, mut star, mut retry) = (0, 0, None, 0);
    while k < key.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == key[k]) {
            p += 1;
            k += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = k;
        } else if let Some(s) = star {
            p = s + 1;
            retry += 1;
            k = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_commands_and_types() {
        let mut s = Session::new();
        assert!(
            matches!(s.execute("PING", true).unwrap()[0], Value::SimpleString(ref v) if v == "PONG")
        );
        assert!(s.execute("SET a b", true).is_err());
        s.execute("SET a b", false).unwrap();
        assert!(
            matches!(s.execute("GET a", true).unwrap()[0], Value::BulkString(ref v) if v == b"b")
        );
        assert!(
            s.execute("GET user:42", true)
                .unwrap_err()
                .contains("WRONGTYPE")
        );
        assert!(
            matches!(s.execute("HGET user:42 name", true).unwrap()[0], Value::BulkString(ref v) if v == b"Avery Chen")
        );
        assert!(s.execute("SET untouched value\nFLUSHALL", false).is_err());
        assert_eq!(s.inspect(b"untouched").unwrap().kind, "none");
    }
    #[test]
    fn expiry_selection_and_optimistic_edits() {
        let mut s = Session::new();
        s.execute("SET a first", false).unwrap();
        assert!(s.replace_string(b"a", b"stale", b"next", false).is_err());
        s.replace_string(b"a", b"first", b"next", false).unwrap();
        s.select(1).unwrap();
        assert_eq!(s.inspect(b"a").unwrap().kind, "none");
        assert!(s.select(16).is_err());
        s.select(0).unwrap();
        assert_eq!(s.inspect(b"a").unwrap().kind, "string");
        s.expire(b"a", Some(0), false).unwrap();
        assert_eq!(s.inspect(b"a").unwrap().ttl_ms, -2);
    }
    #[test]
    fn binary_values_and_demo_limits() {
        let mut s = Session::new();
        s.execute(r"SET binary \x00\xff", false).unwrap();
        assert!(
            matches!(s.inspect(b"binary").unwrap().value, Value::BulkString(ref v) if v == &[0, 255])
        );
        assert!(s.scan(0, &vec![b'*'; 1025], 200).is_err());
        assert!(
            s.execute(&format!("SET {} value", "k".repeat(1025)), false)
                .is_err()
        );
        for i in 0..248 {
            s.execute(&format!("SET key:{i} value"), false)
                .unwrap();
        }
        assert!(s.execute("SET overflow value", false).is_err());
        s.execute("DEL binary", false).unwrap();
        s.execute("SET available value", false).unwrap();
    }
    #[test]
    fn scan_and_ranges() {
        let mut s = Session::new();
        let page = s.scan(0, b"cache:*", 200).unwrap();
        assert_eq!(page.keys, vec![b"cache:homepage".to_vec()]);
        assert_eq!(s.key_rows(&page.keys).unwrap()[0].kind, "string");
        assert!(
            matches!(s.execute("LRANGE queue:emails -1 -1", true).unwrap()[0], Value::Array(ref v) if v.len() == 1)
        );
        assert!(
            matches!(s.execute("ZRANGE leaderboard 0 -1 WITHSCORES", true).unwrap()[0], Value::Array(ref v) if v.len() == 6)
        );
        assert!(s.execute("SET a b EX 10", false).is_err());
        assert!(s.execute("EXPIRE welcome nope", false).is_err());
    }
}
