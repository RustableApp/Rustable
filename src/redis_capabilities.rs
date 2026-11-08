//! Advisory metadata for the checked node, refreshed explicitly and never persisted.
#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub(crate) struct Capabilities {
    pub version: Option<[u64; 3]>,
    pub commands: [Option<bool>; 4],
}
fn version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.split('.');
    Some([
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.split('-').next()?.parse().ok()?,
    ])
}
impl Capabilities {
    pub fn warning(&self, name: &str, since: &str) -> &'static str {
        let module = if name.starts_with("JSON.") {
            Some(0)
        } else if name.starts_with("FT.") {
            Some(1)
        } else if name.starts_with("TS.") {
            Some(2)
        } else if name == "VSIM" {
            Some(3)
        } else {
            None
        };
        if let Some(index) = module {
            if self.commands[index] == Some(false) {
                return [
                    "JSON.GET unavailable on checked node; JSON support may be limited",
                    "FT.SEARCH unavailable on checked node; Search support may be limited",
                    "TS.INFO unavailable on checked node; time-series support may be limited",
                    "VSIM unavailable on checked node",
                ][index];
            }
        } else if let (Some(actual), Some(required)) = (self.version, version(since))
            && actual < required {
                return "Requires a newer Redis version than the cached server version";
        }
        ""
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn decode(values: Vec<redis::Value>) -> Self {
        let mut result = Self::default();
        if let Some(redis::Value::BulkString(info)) = values.first()
            && let Ok(info) = std::str::from_utf8(info) {
                result.version = info.lines().find_map(|line| {
                    line.strip_prefix("redis_version:")
                        .and_then(|v| version(v.trim()))
                });
        }
        if let Some(redis::Value::Array(commands)) = values.get(1)
            && commands.len() == 4 {
                for (state, command) in result.commands.iter_mut().zip(commands) {
                    *state = match command {
                        redis::Value::Nil => Some(false),
                        redis::Value::Array(fields) if !fields.is_empty() => Some(true),
                        _ => None,
                    };
                }
        }
        result
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pipeline() -> redis::Pipeline {
    let mut pipe = redis::pipe();
    pipe.cmd("INFO")
        .arg("SERVER")
        .cmd("COMMAND")
        .arg("INFO")
        .arg("JSON.GET")
        .arg("FT.SEARCH")
        .arg("TS.INFO")
        .arg("VSIM");
    pipe
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_metadata_is_not_absence_and_versions_are_numeric() {
        let unknown = Capabilities::default();
        assert!(unknown.warning("JSON.GET", "").is_empty());
        let caps = Capabilities {
            version: Some([6, 2, 0]),
            commands: [Some(false), Some(true), None, Some(false)],
        };
        assert!(!caps.warning("LMPOP", "7.0.0").is_empty());
        assert!(caps.warning("GET", "1.0.0").is_empty());
        assert!(!caps.warning("JSON.SET", "").is_empty());
        assert!(caps.warning("FT.SEARCH", "").is_empty());
        assert!(caps.warning("TS.GET", "").is_empty());
        assert!(!caps.warning("VSIM", "").is_empty());
        assert_eq!(version("7.10.2-dev"), Some([7, 10, 2]));
        let decoded = Capabilities::decode(vec![
            redis::Value::BulkString(b"# Server\r\nredis_version:7.2.6\r\n".to_vec()),
            redis::Value::Nil,
        ]);
        assert_eq!(decoded.version, Some([7, 2, 6]));
        assert_eq!(decoded.commands, [None; 4]);
    }
}
