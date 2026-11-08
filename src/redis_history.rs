//! Bounded command text, scoped by saved connection identity rather than endpoint.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
pub(crate) const MAX_ENTRIES: usize = 100;
pub(crate) const MAX_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PROFILES: usize = 16;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct History {
    pub commands: VecDeque<String>,
    pub updated_ms: u64,
}
impl History {
    pub fn sanitize(&mut self) {
        self.commands.retain(|command| eligible(command));
        trim(&mut self.commands);
    }
}
pub(crate) fn eligible(text: &str) -> bool {
    if text.is_empty() || text.len() > 65536 {
        return false;
    }
    let Ok(commands) = crate::redis_commands::parse(text) else {
        return false;
    };
    !commands.is_empty()
        && commands.iter().all(|args| {
            // Never persist authentication or administrative credential commands,
            // including attempts rejected by the pooled editor's execution guard.
            let name = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
            !matches!(
                name.as_str(),
                "AUTH" | "HELLO" | "ACL" | "CONFIG" | "MIGRATE"
            ) && crate::redis_commands::validate(args, false).is_ok()
        })
}
pub(crate) fn trim(commands: &mut VecDeque<String>) {
    let mut bytes: usize = commands.iter().map(String::len).sum();
    while commands.len() > MAX_ENTRIES || bytes > MAX_BYTES {
        if let Some(command) = commands.pop_front() {
            bytes -= command.len();
        } else {
            break;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_history_keeps_recent_commands_and_excludes_credentials() {
        let mut history = History::default();
        for i in 0..120 {
            history.commands.push_back(format!("GET key:{i}"));
        }
        history.commands.push_back("AUTH user secret".into());
        history.commands.push_back("ACL SETUSER app >secret".into());
        history
            .commands
            .push_back("CONFIG SET requirepass secret".into());
        history.sanitize();
        assert_eq!(history.commands.len(), 100);
        assert_eq!(history.commands.front().unwrap(), "GET key:20");
        for _ in 0..10 {
            history
                .commands
                .push_back(format!("SET key {}", "a".repeat(60000)));
        }
        history.sanitize();
        assert!(history.commands.iter().map(String::len).sum::<usize>() <= MAX_BYTES);
        assert!(!eligible("GET \"unfinished"));
        assert!(eligible("GET key\nTTL key"));
    }
}
