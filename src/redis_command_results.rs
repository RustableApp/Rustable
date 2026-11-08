//! Bounded display results; execute sequentially and stop after the first error.
#[cfg(target_arch = "wasm32")]
use crate::redis_demo as redis;
use std::time::Duration;

#[derive(Debug)]
pub(crate) enum Status {
    Completed,
    Failed,
    NotRun,
}
#[derive(Debug)]
pub(crate) struct Entry {
    pub command: String,
    pub status: Status,
    pub elapsed: Duration,
    pub output: String,
}
#[derive(Debug, Default)]
pub(crate) struct Report {
    pub entries: Vec<Entry>,
}
impl Report {
    pub fn text(&self) -> String {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                format!(
                    "{}. {} · {} · {:.2} ms\n{}\n",
                    i + 1,
                    entry.command,
                    entry.status.label(),
                    entry.elapsed.as_secs_f64() * 1000.0,
                    entry.output
                )
            })
            .collect()
    }
}
impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::NotRun => "Not run",
        }
    }
}
pub(crate) fn execute(
    commands: &[Vec<Vec<u8>>],
    mut query: impl FnMut(&[Vec<u8>]) -> Result<redis::Value, String>,
) -> Report {
    let mut report = Report::default();
    let mut failed = false;
    let mut remaining = 256 * 1024;
    for args in commands {
        let mut command = String::new();
        for arg in args {
            if command.len() >= 256 {
                command.push('…');
                break;
            }
            if !command.is_empty() {
                command.push(' ');
            }
            let bytes = &arg[..arg.len().min(256)];
            if !bytes.is_empty() && bytes.iter().all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\')) {
                command.push_str(std::str::from_utf8(bytes).unwrap());
            } else {
                command.push_str(&crate::redis_commands::quote(bytes));
            }
            if arg.len() > bytes.len() { command.push('…'); }
        }
        let mut elapsed = Duration::ZERO;
        let (status, mut output) = if failed {
            (
                Status::NotRun,
                "Stopped after an earlier error; no command was sent.".into(),
            )
        } else {
            let start = web_time::Instant::now();
            let result = query(args);
            elapsed = start.elapsed();
            match result {
                Ok(value) => (
                    Status::Completed,
                    crate::redis_workspace::render_values(&[value]),
                ),
                Err(error) => {
                    failed = true;
                    (
                        Status::Failed,
                        format!("{error}\nEarlier completed commands are not rolled back."),
                    )
                }
            }
        };
        // Keep failure/skip explanations even when successful output exhausts its budget.
        if matches!(status, Status::Completed) {
            let mut end = output.len().min(remaining);
            while !output.is_char_boundary(end) {
                end -= 1;
            }
            if end < output.len() {
                output.truncate(end);
                output.push_str("\n[batch output limit reached]");
            }
            remaining = remaining.saturating_sub(end);
        } else if output.len() > 8192 {
            let mut end = 8192;
            while !output.is_char_boundary(end) {
                end -= 1;
            }
            output.truncate(end);
            output.push_str("… [truncated]");
        }
        report.entries.push(Entry {
            command,
            status,
            elapsed,
            output,
        });
    }
    report
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_binary_arguments_remain_distinguishable_in_headers() {
        let commands = crate::redis_commands::parse(r#"GET ""
GET "a b"
GET \x00"#).unwrap();
        let report = execute(&commands, |_| Ok(redis::Value::Nil));
        assert_eq!(report.entries[0].command, "GET \"\"");
        assert!(report.entries[1].command.contains("\"a b\""));
        assert!(report.entries[2].command.contains("\\x00"));
    }
    #[test]
    fn retains_completed_output_and_never_executes_after_error() {
        let commands =
            crate::redis_commands::parse("GET first\nGET wrongtype\nINCR never").unwrap();
        let mut calls = 0;
        let report = execute(&commands, |_| {
            calls += 1;
            if calls == 1 {
                Ok(redis::Value::Int(42))
            } else {
                Err("WRONGTYPE".into())
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(report.entries.len(), 3);
        assert!(report.entries[0].output.contains("42"));
        assert!(matches!(report.entries[1].status, Status::Failed));
        assert!(matches!(report.entries[2].status, Status::NotRun));
        assert!(report.text().contains("not rolled back"));
    }
    #[test]
    fn display_budget_is_shared_between_commands() {
        let commands = crate::redis_commands::parse(&"GET key\n".repeat(100)).unwrap();
        let report = execute(&commands, |_| {
            Ok(redis::Value::BulkString(vec![b'a'; 65536]))
        });
        assert_eq!(report.entries.len(), 100);
        assert!(report.text().len() < 300 * 1024);
        assert!(
            report
                .entries
                .last()
                .unwrap()
                .output
                .contains("limit reached")
        );
    }
}
