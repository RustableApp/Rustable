//! Versioned, bounded Redis DUMP archives. No commands are read from files.
use ring::digest;
use std::{
    io::{Read, Write},
    path::Path,
};
const MAGIC: &[u8; 8] = b"RSTRDIS1";
pub(crate) const MAX_FILE: usize = 64 * 1024 * 1024;
pub(crate) const MAX_KEYS: usize = 1000;
pub(crate) const MAX_DUMP: usize = 1024 * 1024;
#[derive(Debug, PartialEq)]
pub(crate) struct Entry {
    pub key: Vec<u8>,
    pub kind: String,
    pub expires_at_ms: Option<i64>,
    pub dump: Vec<u8>,
}
#[derive(Debug, PartialEq)]
pub(crate) struct Archive {
    pub created_at_ms: i64,
    pub database: u32,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Collision {
    #[default]
    Skip,
    Replace,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Expiry {
    #[default]
    Preserve,
    Persistent,
}
impl Archive {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=9_007_199_254_740_991).contains(&self.created_at_ms)
            || self.entries.is_empty()
            || self.entries.len() > MAX_KEYS
        {
            return Err("An archive must contain 1–1,000 keys and a valid creation time.".into());
        }
        let mut names = std::collections::HashSet::new();
        let mut size = 24usize;
        for entry in &self.entries {
            if entry.key.len() > 4096
                || entry.kind.is_empty()
                || entry.kind.len() > 128
                || entry.kind.chars().any(char::is_control)
                || entry.dump.is_empty()
                || entry.dump.len() > MAX_DUMP
                || entry
                    .expires_at_ms
                    .is_some_and(|n| !(1..=9_007_199_254_740_991).contains(&n))
            {
                return Err(
                    "Archive entry exceeds a name/type/value limit or has invalid expiry.".into(),
                );
            }
            if !names.insert(entry.key.as_slice()) {
                return Err("Archive contains duplicate key names.".into());
            }
            size = size
                .checked_add(18 + entry.key.len() + entry.kind.len() + entry.dump.len())
                .ok_or("Archive length overflow")?;
            if size > MAX_FILE - 32 {
                return Err("Archive exceeds 64 MiB.".into());
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let capacity = 56
            + self
                .entries
                .iter()
                .map(|e| 18 + e.key.len() + e.kind.len() + e.dump.len())
                .sum::<usize>();
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend(MAGIC);
        bytes.extend(self.created_at_ms.to_le_bytes());
        bytes.extend(self.database.to_le_bytes());
        bytes.extend((self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            bytes.extend((entry.key.len() as u32).to_le_bytes());
            bytes.extend((entry.kind.len() as u16).to_le_bytes());
            bytes.extend(entry.expires_at_ms.unwrap_or(-1).to_le_bytes());
            bytes.extend((entry.dump.len() as u32).to_le_bytes());
            bytes.extend(&entry.key);
            bytes.extend(entry.kind.as_bytes());
            bytes.extend(&entry.dump);
        }
        let checksum = digest::digest(&digest::SHA256, &bytes);
        bytes.extend(checksum.as_ref());
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FILE || bytes.len() < 56 {
            return Err("Invalid archive size (maximum 64 MiB).".into());
        }
        let body = &bytes[..bytes.len() - 32];
        if digest::digest(&digest::SHA256, body).as_ref() != &bytes[bytes.len() - 32..] {
            return Err("Archive checksum does not match; the file is damaged.".into());
        }
        let mut cursor = Cursor {
            bytes: body,
            offset: 0,
        };
        if cursor.take(8)? != MAGIC {
            return Err("Unsupported Redis archive format/version.".into());
        }
        let created_at_ms = i64::from_le_bytes(cursor.take(8)?.try_into().unwrap());
        let database = u32::from_le_bytes(cursor.take(4)?.try_into().unwrap());
        let count = u32::from_le_bytes(cursor.take(4)?.try_into().unwrap()) as usize;
        if count == 0 || count > MAX_KEYS {
            return Err("Archive key count exceeds 1–1,000.".into());
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let key_len = u32::from_le_bytes(cursor.take(4)?.try_into().unwrap()) as usize;
            let kind_len = u16::from_le_bytes(cursor.take(2)?.try_into().unwrap()) as usize;
            let expiry = i64::from_le_bytes(cursor.take(8)?.try_into().unwrap());
            let dump_len = u32::from_le_bytes(cursor.take(4)?.try_into().unwrap()) as usize;
            if key_len > 4096 || kind_len > 128 || dump_len > MAX_DUMP {
                return Err("Archive entry exceeds a name/type/value limit.".into());
            }
            let key = cursor.take(key_len)?.to_vec();
            let kind = std::str::from_utf8(cursor.take(kind_len)?)
                .map_err(|_| "Archive type is not UTF-8")?
                .to_owned();
            let dump = cursor.take(dump_len)?.to_vec();
            entries.push(Entry {
                key,
                kind,
                expires_at_ms: if expiry == -1 { None } else { Some(expiry) },
                dump,
            });
        }
        if cursor.offset != body.len() {
            return Err("Unexpected trailing archive data.".into());
        }
        let archive = Self {
            created_at_ms,
            database,
            entries,
        };
        archive.validate()?;
        Ok(archive)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Choose a regular Redis archive file.".into());
        }
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::decode(&bytes)
    }
    pub fn save(&self, path: &Path, replace: bool) -> Result<(), String> {
        let bytes = self.encode()?;
        let resolved = crate::script_file::destination(path)?;
        let path = resolved.as_path();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        if replace {
            file.persist(path).map_err(|e| e.error.to_string())?;
        } else {
            file.persist_noclobber(path)
                .map_err(|e| e.error.to_string())?;
        }
        Ok(())
    }
}
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or("Archive length overflow")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or("Truncated Redis archive")?;
        self.offset = end;
        Ok(value)
    }
}

/// Each record is captured atomically. At most sixteen 1 MiB payloads per reply.
/// DUMP may still require substantial server CPU for a large value; this cap
/// bounds client/wire retention, not Redis's temporary serialization allocation.
pub(crate) const EXPORT_SCRIPT: &str = r#"
local clock=redis.call('TIME');local now=clock[1]*1000+math.floor(clock[2]/1000)
local result={now}
for _,key in ipairs(KEYS) do
 local kind=redis.call('TYPE',key).ok
 if kind=='none' then table.insert(result,{'missing','',-1,''})
 else
  local value=redis.pcall('DUMP',key)
  if type(value)=='table' and value.err then table.insert(result,{'error',kind,-1,string.sub(value.err,1,4096)})
  elseif not value then table.insert(result,{'missing','',-1,''})
  elseif #value>1048576 then table.insert(result,{'oversize',kind,-1,''})
  else
   local expiry=redis.call('PEXPIRETIME',key)
   if expiry>9007199254740991 then table.insert(result,{'expiry-range',kind,-1,''})
   else table.insert(result,{'ok',kind,expiry,value}) end
  end
 end
end
return result
"#;
/// Redis validates DUMP version/checksum before replacement. Every error is
/// collected per key; earlier successful entries are not claimed to roll back.
pub(crate) const IMPORT_SCRIPT: &str = r#"
local clock=redis.call('TIME');local now=clock[1]*1000+math.floor(clock[2]/1000)
local result={}
for i,key in ipairs(KEYS) do
 local expires=tonumber(ARGV[i*2+1]);local value=ARGV[i*2+2]
 if ARGV[2]=='preserve' and expires>0 and expires<=now then table.insert(result,{'expired','Deadline already passed; existing key unchanged.'})
 else
  local ttl='0';if ARGV[2]=='preserve' and expires>0 then ttl=ARGV[i*2+1] end
  local response
  if ARGV[1]=='replace' then response=redis.pcall('RESTORE',key,ttl,value,'ABSTTL','REPLACE')
  else response=redis.pcall('RESTORE',key,ttl,value,'ABSTTL') end
  if type(response)=='table' and response.err then
   if string.sub(response.err,1,7)=='BUSYKEY' then table.insert(result,{'exists','Existing key left unchanged.'})
   else table.insert(result,{'error',string.sub(response.err,1,4096)}) end
  else table.insert(result,{'restored',''}) end
 end
end
return result
"#;

#[derive(Debug)]
pub(crate) struct ExportReport {
    pub archive: Archive,
    pub skipped: Vec<(Vec<u8>, String)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum State {
    Restored,
    Exists,
    Expired,
    Error,
    Unknown,
    NotAttempted,
}
#[derive(Debug)]
pub(crate) struct Outcome {
    pub key: Vec<u8>,
    pub state: State,
    pub message: String,
}
#[derive(Debug)]
pub(crate) struct ImportReport {
    pub outcomes: Vec<Outcome>,
    pub stopped: Option<String>,
}
pub(crate) struct ExportBatch {
    pub timestamp: i64,
    pub entries: Vec<Entry>,
    pub skipped: Vec<(Vec<u8>, String)>,
}
pub(crate) fn export_reply(value: redis::Value, keys: &[Vec<u8>]) -> Result<ExportBatch, String> {
    let redis::Value::Array(mut rows) = value else {
        return Err("Invalid export response".into());
    };
    if rows.len() != keys.len() + 1 {
        return Err("Invalid export response length".into());
    }
    let redis::Value::Int(now) = rows.remove(0) else {
        return Err("Invalid export timestamp".into());
    };
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for (key, row) in keys.iter().zip(rows) {
        let (status, kind, expiry, payload): (String, String, i64, Vec<u8>) =
            redis::from_redis_value(row).map_err(|e| format!("Invalid export entry: {e}"))?;
        if status == "ok" {
            entries.push(Entry {
                key: key.clone(),
                kind,
                expires_at_ms: if expiry == -1 { None } else { Some(expiry) },
                dump: payload,
            });
        } else {
            let reason = match status.as_str() {
                "missing" => "Key no longer exists".into(),
                "oversize" => "Serialized value exceeds 1 MiB".into(),
                "expiry-range" => "Expiry exceeds the exact timestamp range".into(),
                "error" => {
                    String::from_utf8_lossy(&payload[..payload.len().min(4096)]).into_owned()
                }
                _ => return Err("Unknown export status".into()),
            };
            skipped.push((key.clone(), reason));
        }
    }
    Ok(ExportBatch {
        timestamp: now,
        entries,
        skipped,
    })
}
pub(crate) fn import_reply(
    value: redis::Value,
    count: usize,
) -> Result<Vec<(State, String)>, String> {
    let redis::Value::Array(rows) = value else {
        return Err("Invalid import response".into());
    };
    if rows.len() != count {
        return Err("Invalid import result count".into());
    }
    rows.into_iter()
        .map(|row| {
            let (state, mut message): (String, String) =
                redis::from_redis_value(row).map_err(|e| format!("Invalid import outcome: {e}"))?;
            let state = match state.as_str() {
                "restored" => State::Restored,
                "exists" => State::Exists,
                "expired" => State::Expired,
                "error" => State::Error,
                _ => return Err("Unknown import status".into()),
            };
            if message.len() > 4096 {
                let mut end = 4096;
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                message.truncate(end);
            }
            Ok((state, message))
        })
        .collect()
}

#[derive(Debug)]
pub(crate) struct Target {
    pub kind: String,
    pub expiry: String,
}
#[derive(Debug)]
pub(crate) struct Review {
    pub database: u32,
    pub server_now_ms: i64,
    pub targets: Vec<Target>,
    pub existing: usize,
    pub expired: usize,
}
pub(crate) const REVIEW_SCRIPT: &str = r#"
local clock=redis.call('TIME');local result={clock[1]*1000+math.floor(clock[2]/1000)}
for _,key in ipairs(KEYS) do
 local expiry=redis.call('PEXPIRETIME',key)
 if expiry>9007199254740991 then expiry=-3 end
 table.insert(result,{redis.call('TYPE',key).ok,expiry})
end
return result
"#;
pub(crate) fn expiry_label(expiry: Option<i64>) -> String {
    expiry.map_or_else(
        || "No expiry".into(),
        |n| {
            chrono::DateTime::from_timestamp_millis(n).map_or_else(
                || format!("{n} ms since epoch"),
                |d| d.format("%Y-%m-%d %H:%M UTC").to_string(),
            )
        },
    )
}
pub(crate) fn require_database(expected: u32, current: u32) -> Result<(), String> {
    if expected != current {
        Err("The target database changed. Review the current database before continuing.".into())
    } else {
        Ok(())
    }
}
pub(crate) fn review_reply(
    value: redis::Value,
    database: u32,
    count: usize,
) -> Result<Review, String> {
    let redis::Value::Array(mut rows) = value else {
        return Err("Invalid target review".into());
    };
    if rows.len() != count + 1 {
        return Err("Invalid target review length".into());
    }
    let redis::Value::Int(server_now_ms) = rows.remove(0) else {
        return Err("Invalid review timestamp".into());
    };
    if !(1..=9_007_199_254_740_991).contains(&server_now_ms) {
        return Err("Invalid review timestamp".into());
    }
    let mut targets = Vec::with_capacity(count);
    let mut existing = 0;
    for row in rows {
        let (kind, expiry): (String, i64) =
            redis::from_redis_value(row).map_err(|e| e.to_string())?;
        if kind.is_empty() || kind.len() > 128 || kind.chars().any(char::is_control) {
            return Err("Invalid target type".into());
        }
        if kind != "none" {
            existing += 1;
        }
        let expiry = match expiry {
            -1 => "No expiry".into(),
            -2 => "Missing".into(),
            -3 => "Outside exact timestamp range".into(),
            n if n > 0 => expiry_label(Some(n)),
            _ => return Err("Invalid target expiry".into()),
        };
        targets.push(Target { kind, expiry });
    }
    Ok(Review {
        database,
        server_now_ms,
        targets,
        existing,
        expired: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Archive {
        Archive {
            created_at_ms: 1700000000000,
            database: 3,
            entries: vec![
                Entry {
                    key: vec![0, 255, b' '],
                    kind: "hash".into(),
                    expires_at_ms: Some(1800000000000),
                    dump: vec![0, 255, 13, 10],
                },
                Entry {
                    key: vec![],
                    kind: "string".into(),
                    expires_at_ms: None,
                    dump: vec![1],
                },
            ],
        }
    }
    #[test]
    fn binary_names_expiry_and_payloads_round_trip() {
        let a = fixture();
        assert_eq!(Archive::decode(&a.encode().unwrap()).unwrap(), a);
    }
    #[test]
    fn corruption_truncation_duplicates_and_bounds_are_rejected() {
        let mut bytes = fixture().encode().unwrap();
        bytes[25] ^= 1;
        assert!(Archive::decode(&bytes).unwrap_err().contains("checksum"));
        for n in 0..bytes.len() {
            assert!(Archive::decode(&bytes[..n]).is_err());
        }
        let mut a = fixture();
        a.entries[1].key = a.entries[0].key.clone();
        assert!(a.encode().is_err());
        a = fixture();
        a.entries[0].dump = vec![0; MAX_DUMP + 1];
        assert!(a.encode().is_err());
        a = fixture();
        a.entries[0].expires_at_ms = Some(0);
        assert!(a.encode().is_err());
    }
    #[test]
    fn forged_lengths_and_trailing_data_are_rejected() {
        let mut bytes = fixture().encode().unwrap();
        bytes.truncate(bytes.len() - 32);
        bytes[38..42].copy_from_slice(&u32::MAX.to_le_bytes());
        let sum = digest::digest(&digest::SHA256, &bytes);
        bytes.extend(sum.as_ref());
        assert!(Archive::decode(&bytes).is_err());
        let mut bytes = fixture().encode().unwrap();
        bytes.truncate(bytes.len() - 32);
        bytes.push(0);
        let sum = digest::digest(&digest::SHA256, &bytes);
        bytes.extend(sum.as_ref());
        assert!(Archive::decode(&bytes).unwrap_err().contains("trailing"));
    }
    #[test]
    fn atomic_save_respects_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.rredis");
        fixture().save(&path, false).unwrap();
        assert_eq!(Archive::read(&path).unwrap(), fixture());
        let before = std::fs::read(&path).unwrap();
        assert!(fixture().save(&path, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut replacement = fixture();
        replacement.database = 4;
        replacement.save(&path, true).unwrap();
        assert_eq!(Archive::read(&path).unwrap().database, 4);
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::{redis_session::Session, redis_settings::Settings};
    use std::{net::TcpListener, sync::atomic::AtomicBool, time::Duration};
    fn setup_peer(stream: &mut std::net::TcpStream, parser: &mut redis::Parser) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        loop {
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut *stream).unwrap()).unwrap();
            stream
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
    }
    #[test]
    fn cancelled_and_read_only_imports_send_nothing_and_lost_replies_are_not_replayed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            setup_peer(&mut stream, &mut parser);
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            assert_eq!(args[0], b"EVAL");
            assert_eq!(args[2], b"16");
            assert_eq!(args[19], b"skip");
            assert_eq!(args[20], b"preserve");
            // Closing after accepting the batch simulates an unknown write outcome.
        });
        let mut session = Session::connect(
            &Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "",
        )
        .unwrap();
        let archive = Archive {
            created_at_ms: 1700000000000,
            database: 0,
            entries: (0..17)
                .map(|n| Entry {
                    key: format!("key:{n}").into_bytes(),
                    kind: "string".into(),
                    expires_at_ms: None,
                    dump: vec![1],
                })
                .collect(),
        };
        assert!(
            session
                .import_archive(
                    &archive,
                    Collision::Skip,
                    Expiry::Preserve,
                    true,
                    &AtomicBool::new(false)
                )
                .unwrap_err()
                .contains("Read-only")
        );
        let cancelled = session
            .import_archive(
                &archive,
                Collision::Skip,
                Expiry::Preserve,
                false,
                &AtomicBool::new(true),
            )
            .unwrap();
        assert!(
            cancelled
                .outcomes
                .iter()
                .all(|o| o.state == State::NotAttempted)
        );
        let report = session
            .import_archive(
                &archive,
                Collision::Skip,
                Expiry::Preserve,
                false,
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(
            report.outcomes[..16]
                .iter()
                .all(|o| o.state == State::Unknown)
        );
        assert_eq!(report.outcomes[16].state, State::NotAttempted);
        assert!(report.stopped.is_some());
        assert!(!session.is_connected());
        peer.join().unwrap();
    }
    #[test]
    fn export_and_import_each_use_one_exchange_per_batch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut parser = redis::Parser::new();
            setup_peer(&mut stream, &mut parser);
            for _ in 0..2 {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                assert_eq!(args[0], b"EVAL_RO");
                assert_eq!(args[2], b"1");
                assert_eq!(args[3], b"key\0\xff");
                stream.write_all(b"*2\r\n:1700000000000\r\n*4\r\n$2\r\nok\r\n$6\r\nstring\r\n:-1\r\n$3\r\n\0\xffx\r\n").unwrap();
                let review: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                assert_eq!(review[0], b"EVAL_RO");
                assert_eq!(review[1], REVIEW_SCRIPT.as_bytes());
                assert_eq!(review[2], b"1");
                stream
                    .write_all(b"*2\r\n:1700000000000\r\n*2\r\n$6\r\nstring\r\n:-1\r\n")
                    .unwrap();
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                assert_eq!(args[0], b"EVAL");
                assert_eq!(args[2], b"1");
                assert_eq!(args[3], b"key\0\xff");
                assert_eq!(args[4], b"replace");
                assert_eq!(args[5], b"persistent");
                assert_eq!(args[7], b"\0\xffx");
                stream
                    .write_all(b"*1\r\n*2\r\n$8\r\nrestored\r\n$0\r\n\r\n")
                    .unwrap();
            }
        });
        let mut session = Session::connect(
            &Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            },
            "",
        )
        .unwrap();
        let cancel = AtomicBool::new(false);
        for _ in 0..2 {
            let exported = session
                .export_archive(&[b"key\0\xff".to_vec()], 0, &cancel)
                .unwrap();
            assert_eq!(exported.archive.entries[0].dump, b"\0\xffx");
            let review = session.review_archive(&exported.archive, 0).unwrap();
            assert_eq!(review.existing, 1);
            assert_eq!(review.targets[0].kind, "string");
            let restored = session
                .import_archive(
                    &exported.archive,
                    Collision::Replace,
                    Expiry::Persistent,
                    false,
                    &cancel,
                )
                .unwrap();
            assert_eq!(restored.outcomes[0].state, State::Restored);
            assert_eq!(restored.outcomes[0].key, b"key\0\xff");
        }
        peer.join().unwrap();
    }
    #[test]
    #[ignore = "disposable Redis fixtures via scripts/test-redis.sh"]
    fn live_large_denied_and_module_archives() {
        let mut admin = redis::Client::open("redis://:rustable-redis-test-only@127.0.0.1:16379/9")
            .unwrap()
            .get_connection()
            .unwrap();
        let mut random = vec![0; MAX_DUMP + 100];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random).unwrap();
        redis::cmd("SET")
            .arg("archive:large")
            .arg(&random)
            .query::<()>(&mut admin)
            .unwrap();
        let settings = Settings {
            host: "127.0.0.1".into(),
            port: 16379,
            tls: false,
            database: 9,
            ..Default::default()
        };
        let mut session = Session::connect(&settings, "rustable-redis-test-only").unwrap();
        let cancel = AtomicBool::new(false);
        let report = session
            .export_archive(&[b"archive:large".to_vec()], 9, &cancel)
            .unwrap();
        assert!(report.archive.entries.is_empty());
        assert!(report.skipped[0].1.contains("1 MiB"));
        redis::cmd("ACL")
            .arg(&[
                "SETUSER",
                "archive-denied",
                "on",
                ">archive-test-only",
                "~*",
                "+@all",
                "-dump",
            ])
            .query::<()>(&mut admin)
            .unwrap();
        let mut denied_settings = settings.clone();
        denied_settings.username = "archive-denied".into();
        let mut denied = Session::connect(&denied_settings, "archive-test-only").unwrap();
        let report = denied
            .export_archive(&[b"archive:large".to_vec()], 9, &cancel)
            .unwrap();
        assert!(report.archive.entries.is_empty());
        assert_eq!(report.skipped.len(), 1);
        redis::cmd("ACL")
            .arg(&["DELUSER", "archive-denied"])
            .query::<i64>(&mut admin)
            .unwrap();
        let mut module_settings = settings;
        module_settings.port = 16381;
        let mut modules = Session::connect(&module_settings, "rustable-redis-test-only").unwrap();
        modules
            .execute("JSON.SET archive:json $ '{\"ok\":true}'", false)
            .unwrap();
        let mut exported = modules
            .export_archive(&[b"archive:json".to_vec()], 9, &cancel)
            .unwrap()
            .archive;
        exported.entries[0].key = b"archive:json-copy".to_vec();
        let result = modules
            .import_archive(
                &exported,
                Collision::Replace,
                Expiry::Preserve,
                false,
                &cancel,
            )
            .unwrap();
        assert_eq!(result.outcomes[0].state, State::Restored);
        let result = session
            .import_archive(
                &exported,
                Collision::Replace,
                Expiry::Preserve,
                false,
                &cancel,
            )
            .unwrap();
        assert_eq!(result.outcomes[0].state, State::Error);
        assert_eq!(
            session.execute("EXISTS archive:json-copy", true).unwrap()[0],
            redis::Value::Int(0)
        );
    }
    #[test]
    #[ignore = "disposable Redis fixtures via scripts/test-redis.sh"]
    fn live_round_trip_types_binary_expiry_collisions_and_partial_errors() {
        let mut session = Session::connect(
            &Settings {
                host: "127.0.0.1".into(),
                port: 16379,
                tls: false,
                database: 9,
                ..Default::default()
            },
            "rustable-redis-test-only",
        )
        .unwrap();
        session.execute("SET archive:string value\nHSET archive:hash name Ada plan Pro\nRPUSH archive:list one two\nSADD archive:set a b\nZADD archive:zset 1 one 2 two\nXADD archive:stream 1-0 field value\nSET archive:ttl temporary PX 60000\nSET \"archive:binary\\x00\\xff\" \"\\x00\\xff\\r\\n\"",false).unwrap();
        let keys = vec![
            b"archive:string".to_vec(),
            b"archive:hash".to_vec(),
            b"archive:list".to_vec(),
            b"archive:set".to_vec(),
            b"archive:zset".to_vec(),
            b"archive:stream".to_vec(),
            b"archive:ttl".to_vec(),
            b"archive:binary\0\xff".to_vec(),
            b"archive:missing".to_vec(),
        ];
        let cancel = AtomicBool::new(false);
        let exported = session.export_archive(&keys, 9, &cancel).unwrap();
        assert_eq!(exported.archive.entries.len(), 8);
        assert_eq!(exported.skipped.len(), 1);
        let missing = session
            .export_archive(&[b"archive:missing".to_vec()], 9, &cancel)
            .unwrap();
        assert!(missing.archive.entries.is_empty());
        assert_eq!(missing.skipped.len(), 1);
        let mut archive = Archive::decode(&exported.archive.encode().unwrap()).unwrap();
        for entry in &mut archive.entries {
            entry.key.splice(0..0, b"copy:".iter().copied());
        }
        session
            .execute("SET copy:archive:hash untouched", false)
            .unwrap();
        let review = session.review_archive(&archive, 9).unwrap();
        assert_eq!(review.existing, 1);
        assert_eq!(review.expired, 0);
        let report = session
            .import_archive(&archive, Collision::Skip, Expiry::Preserve, false, &cancel)
            .unwrap();
        assert_eq!(
            report
                .outcomes
                .iter()
                .filter(|r| r.state == State::Restored)
                .count(),
            7
        );
        assert_eq!(report.outcomes[1].state, State::Exists);
        assert_eq!(
            session.execute("GET copy:archive:hash", true).unwrap()[0],
            redis::Value::BulkString(b"untouched".to_vec())
        );
        let report = session
            .import_archive(
                &archive,
                Collision::Replace,
                Expiry::Preserve,
                false,
                &cancel,
            )
            .unwrap();
        assert!(report.outcomes.iter().all(|r| r.state == State::Restored));
        let restored = session
            .export_archive(
                &archive
                    .entries
                    .iter()
                    .map(|e| e.key.clone())
                    .collect::<Vec<_>>(),
                9,
                &cancel,
            )
            .unwrap();
        for (before, after) in archive.entries.iter().zip(&restored.archive.entries) {
            assert_eq!(before.dump, after.dump);
            assert_eq!(before.kind, after.kind);
            assert_eq!(before.expires_at_ms, after.expires_at_ms);
        }
        let expired = Entry {
            key: b"copy:expired".to_vec(),
            kind: "string".into(),
            expires_at_ms: Some(1),
            dump: archive.entries[0].dump.clone(),
        };
        let invalid = Entry {
            key: b"copy:invalid".to_vec(),
            kind: "string".into(),
            expires_at_ms: None,
            dump: b"not a valid DUMP".to_vec(),
        };
        archive.entries = vec![expired, invalid];
        session
            .execute(
                "SET copy:expired keep-expired\nSET copy:invalid keep-invalid",
                false,
            )
            .unwrap();
        let report = session
            .import_archive(
                &archive,
                Collision::Replace,
                Expiry::Preserve,
                false,
                &cancel,
            )
            .unwrap();
        assert_eq!(report.outcomes[0].state, State::Expired);
        assert_eq!(report.outcomes[1].state, State::Error);
        assert_eq!(
            session.execute("GET copy:expired", true).unwrap()[0],
            redis::Value::BulkString(b"keep-expired".to_vec())
        );
        assert_eq!(
            session.execute("GET copy:invalid", true).unwrap()[0],
            redis::Value::BulkString(b"keep-invalid".to_vec())
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
        assert_eq!(report.outcomes[0].state, State::Restored);
        assert_eq!(report.outcomes[1].state, State::Error);
        assert_eq!(
            session.execute("PTTL copy:expired", true).unwrap()[0],
            redis::Value::Int(-1)
        );
    }
}
