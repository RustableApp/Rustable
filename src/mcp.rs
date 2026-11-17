//! Read-only Model Context Protocol server over stdio (`rustable mcp`).
//!
//! JSON-RPC messages are newline-delimited on stdin/stdout; diagnostics go to
//! stderr only. The reader thread answers protocol requests immediately and
//! queues tool calls for one worker thread that owns all database sessions, so
//! `notifications/cancelled` and the per-call deadline can interrupt a running
//! query through the drivers' existing cancellation paths.
mod catalog;
mod tools;

use serde_json::{Value as Json, json};
use std::{
    collections::HashSet,
    io::{BufRead, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Largest accepted JSON-RPC message; bounds memory for a single request.
const MAX_MESSAGE: u64 = 4 * 1024 * 1024;
const DEFAULT_TIMEOUT: u64 = 60;

const INSTRUCTIONS: &str = "Read-only access to the SQL connections saved in the Rustable desktop app. \
Start with list_connections, then list_tables and describe_table. Use read_table to sample or page \
through a table, and query for read-only SQL with LIMIT. The database server rejects writes, DDL and \
transaction changes.";

const USAGE: &str = "Usage: rustable mcp [--connection NAME]... [--timeout SECONDS] [--settings PATH]

Runs a read-only Model Context Protocol server on stdin/stdout using the SQL
connections saved in Rustable. Saved passwords are read from the system
credential store; SSH keys that need a passphrase must be loaded in ssh-agent.

Options:
  --connection NAME  Expose only this saved connection (name or id). Repeatable.
                     By default every SQLite, PostgreSQL, MySQL and MariaDB
                     connection is available.
  --timeout SECONDS  Cancel a tool call that runs longer than this (default 60).
  --settings PATH    Read saved connections from this app.ron instead of the
                     default Rustable storage location.
";

struct Options {
    allow: Vec<String>,
    timeout: Duration,
    settings: Option<PathBuf>,
}

fn options(args: Vec<String>) -> Result<Option<Options>, String> {
    let mut options = Options { allow: Vec::new(), timeout: Duration::from_secs(DEFAULT_TIMEOUT), settings: None };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_owned(), Some(value.to_owned())),
            _ => (arg, None),
        };
        let mut value = || inline.clone().or_else(|| args.next()).ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "-h" | "--help" => return Ok(None),
            "--connection" => options.allow.push(value()?),
            "--timeout" => {
                let seconds: u64 = value()?.parse().map_err(|_| "--timeout must be a whole number of seconds".to_owned())?;
                if !(1..=3600).contains(&seconds) {
                    return Err("--timeout must be between 1 and 3600 seconds".into());
                }
                options.timeout = Duration::from_secs(seconds);
            }
            "--settings" => options.settings = Some(value()?.into()),
            other => return Err(format!("Unknown option {other}")),
        }
    }
    Ok(Some(options))
}

#[derive(Clone)]
struct Output(Arc<Mutex<std::io::Stdout>>);
impl Output {
    fn send(&self, message: &Json) {
        let mut line = message.to_string();
        line.push('\n');
        let mut stdout = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if stdout.write_all(line.as_bytes()).and_then(|()| stdout.flush()).is_err() {
            // The client closed its end; nothing more can be delivered.
            std::process::exit(0);
        }
    }
    fn result(&self, id: &Json, result: Json) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
    fn error(&self, id: &Json, code: i64, message: &str) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    }
}

/// Request ids are strings or integers; compare them by their JSON text.
fn key(id: &Json) -> String {
    id.to_string()
}

struct Call {
    id: Json,
    name: String,
    arguments: Json,
}

#[derive(Default)]
struct Pending {
    queued: HashSet<String>,
    cancelled: HashSet<String>,
    running: Option<String>,
}

pub(crate) fn run(args: Vec<String>) -> i32 {
    let options = match options(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return 0;
        }
        Err(error) => {
            eprintln!("rustable mcp: {error}\n\n{USAGE}");
            return 2;
        }
    };
    crate::ssh_prompt::refuse_prompts();
    let settings = options.settings.clone().or_else(catalog::Catalog::default_path);
    let catalog = catalog::Catalog::new(settings, options.allow.clone());
    let output = Output(Arc::new(Mutex::new(std::io::stdout())));
    let cancel = Arc::new(AtomicBool::new(false));
    let pending = Arc::new(Mutex::new(Pending::default()));
    let (calls, queue) = mpsc::sync_channel::<Call>(32);

    let worker = {
        let output = output.clone();
        let cancel = cancel.clone();
        let pending = pending.clone();
        let timeout = options.timeout;
        std::thread::Builder::new()
            .name("rustable-mcp".into())
            .spawn(move || {
                let mut tools = tools::Tools::new(catalog, cancel.clone());
                for call in queue {
                    let id = key(&call.id);
                    {
                        let mut state = pending.lock().unwrap();
                        state.queued.remove(&id);
                        if state.cancelled.remove(&id) {
                            continue;
                        }
                        state.running = Some(id.clone());
                    }
                    cancel.store(false, Ordering::Relaxed);
                    let expired = Arc::new(AtomicBool::new(false));
                    let deadline = deadline(timeout, cancel.clone(), expired.clone());
                    let outcome = tools.call(&call.name, &call.arguments);
                    drop(deadline);
                    cancel.store(false, Ordering::Relaxed);
                    let cancelled = {
                        let mut state = pending.lock().unwrap();
                        state.running = None;
                        state.cancelled.remove(&id)
                    };
                    if cancelled {
                        continue; // The client abandoned this request.
                    }
                    let outcome = if expired.load(Ordering::Relaxed) {
                        Err(format!(
                            "Cancelled after the {}-second limit. Narrow the request (add WHERE/LIMIT) or start the server with a larger --timeout.",
                            timeout.as_secs()
                        ))
                    } else {
                        outcome
                    };
                    output.result(&call.id, tool_result(outcome));
                }
            })
            .expect("MCP worker thread")
    };

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = match (&mut input).take(MAX_MESSAGE).read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                eprintln!("rustable mcp: could not read stdin: {error}");
                break;
            }
        };
        if read as u64 == MAX_MESSAGE && line.last() != Some(&b'\n') {
            // Discard the remainder of an oversized message.
            let mut discard = Vec::new();
            while (&mut input).take(MAX_MESSAGE).read_until(b'\n', &mut discard).is_ok_and(|n| n > 0)
                && discard.last() != Some(&b'\n')
            {
                discard.clear();
            }
            output.error(&Json::Null, -32600, "Message exceeds the 4 MiB limit");
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message: Json = match serde_json::from_slice(&line) {
            Ok(message) => message,
            Err(_) => {
                output.error(&Json::Null, -32700, "Parse error");
                continue;
            }
        };
        handle(message, &output, &calls, &pending, &cancel);
    }
    // The client disconnected: stop any running work and let the worker exit.
    drop(calls);
    cancel.store(true, Ordering::Relaxed);
    let _ = worker.join();
    0
}

fn handle(
    message: Json,
    output: &Output,
    calls: &mpsc::SyncSender<Call>,
    pending: &Mutex<Pending>,
    cancel: &AtomicBool,
) {
    let Some(method) = message.get("method").and_then(Json::as_str) else {
        if let Some(id) = message.get("id").filter(|_| message.get("result").is_none() && message.get("error").is_none()) {
            output.error(id, -32600, "Invalid request");
        }
        return; // Responses to server requests are never expected.
    };
    let params = message.get("params").cloned().unwrap_or(Json::Null);
    let Some(id) = message.get("id").cloned() else {
        if method == "notifications/cancelled"
            && let Some(request) = params.get("requestId")
        {
            let request = key(request);
            let mut state = pending.lock().unwrap();
            if state.running.as_deref() == Some(request.as_str()) {
                state.cancelled.insert(request);
                cancel.store(true, Ordering::Relaxed);
            } else if state.queued.contains(&request) {
                state.cancelled.insert(request);
            }
        }
        return;
    };
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Json::as_str);
            let version = requested
                .filter(|version| PROTOCOL_VERSIONS.contains(version))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            output.result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "rustable", "title": "Rustable", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }),
            );
        }
        "ping" => output.result(&id, json!({})),
        "tools/list" => output.result(&id, json!({"tools": tools::definitions()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Json::as_str) else {
                return output.error(&id, -32602, "Missing tool name");
            };
            if !tools::NAMES.contains(&name) {
                return output.error(&id, -32602, &format!("Unknown tool: {name}"));
            }
            let call = Call {
                id: id.clone(),
                name: name.to_owned(),
                arguments: params.get("arguments").cloned().unwrap_or_else(|| json!({})),
            };
            pending.lock().unwrap().queued.insert(key(&id));
            if calls.send(call).is_err() {
                output.error(&id, -32603, "The MCP worker stopped");
            }
        }
        _ => output.error(&id, -32601, &format!("Method not found: {method}")),
    }
}

/// Sets the shared cancellation flag if the call outlives `timeout`. Dropping
/// the returned sender ends the timer without effect.
fn deadline(timeout: Duration, cancel: Arc<AtomicBool>, expired: Arc<AtomicBool>) -> mpsc::Sender<()> {
    let (done, wait) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if wait.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
            expired.store(true, Ordering::Relaxed);
            cancel.store(true, Ordering::Relaxed);
        }
    });
    done
}

fn tool_result(outcome: Result<Json, String>) -> Json {
    match outcome {
        Ok(value) => {
            let failed = value.get("error").is_some();
            json!({"content": [{"type": "text", "text": value.to_string()}], "isError": failed})
        }
        Err(error) => json!({"content": [{"type": "text", "text": error}], "isError": true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options() {
        let parsed = options(vec!["--connection".into(), "A".into(), "--connection=B".into(), "--timeout".into(), "5".into()])
            .unwrap()
            .unwrap();
        assert_eq!(parsed.allow, ["A", "B"]);
        assert_eq!(parsed.timeout, Duration::from_secs(5));
        assert!(options(vec!["--help".into()]).unwrap().is_none());
        assert!(options(vec!["--timeout".into(), "0".into()]).is_err());
        assert!(options(vec!["--bogus".into()]).is_err());
        assert!(options(vec!["--connection".into()]).is_err());
    }

    #[test]
    fn deadline_cancels_only_after_timeout() {
        let cancel = Arc::new(AtomicBool::new(false));
        let expired = Arc::new(AtomicBool::new(false));
        drop(deadline(Duration::from_millis(20), cancel.clone(), expired.clone()));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!cancel.load(Ordering::Relaxed));
        let _timer = deadline(Duration::from_millis(10), cancel.clone(), expired.clone());
        std::thread::sleep(Duration::from_millis(80));
        assert!(cancel.load(Ordering::Relaxed) && expired.load(Ordering::Relaxed));
    }

    #[test]
    fn tool_errors_are_reported_in_band() {
        assert_eq!(tool_result(Err("nope".into()))["isError"], true);
        assert_eq!(tool_result(Ok(json!({"results": []})))["isError"], false);
        assert_eq!(tool_result(Ok(json!({"results": [], "error": "x"})))["isError"], true);
    }
}
