//! Experimental MCP transport into a live desktop workspaces.
//! All IO runs off the UI thread. Commands expire before admission; execution
//! returns immediately and clients inspect the same results the user sees.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

pub(crate) const MAX_MESSAGE: u64 = 128 * 1024;
pub(crate) struct Request {
    pub name: String,
    pub arguments: Value,
    pub expires: Instant,
    pub reply: mpsc::SyncSender<Result<Value, String>>,
}
pub(crate) struct Bridge {
    pub requests: Option<mpsc::Receiver<Request>>,
    pub id: u64,
    pub demo: bool,
    pub enabled: bool,
    pub activity: String,
    pub owned: std::collections::HashSet<u64>,
}

impl Bridge {
    pub(crate) fn next_id() -> u64 {
        use ring::rand::SecureRandom;
        static NEXT: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
        NEXT.get_or_init(|| {
            let mut bytes = [0u8; 8];
            ring::rand::SystemRandom::new()
                .fill(&mut bytes)
                .expect("workspace ID entropy");
            // A different process must not reuse stale IDs, and JSON clients
            // must preserve integers exactly (stay below JavaScript's 2^53).
            std::sync::atomic::AtomicU64::new(u64::from_le_bytes(bytes) & ((1u64 << 52) - 1))
        })
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    pub(crate) fn window() -> Self {
        Self {
            requests: None,
            id: Self::next_id(),
            demo: false,
            enabled: false,
            activity: "AI control is off for this connection".into(),
            owned: Default::default(),
        }
    }
    pub(crate) fn revoke(&mut self) {
        self.enabled = false;
        self.id = Self::next_id();
    }
}

pub(crate) fn definitions() -> Value {
    let tools = [
        (
            "list_workspaces",
            "List only active SQL workspaces explicitly enabled by the user. Open a connection in Rustable and click Enable AI control if this is empty. Workspace IDs change after takeover or connection/schema changes; never substitute a different workspace automatically.",
            json!({}),
        ),
        (
            "workspace",
            "Inspect the selected live connection, visible query tabs and catalog (up to 200 objects).",
            json!({}),
        ),
        (
            "open_query",
            "Open a new AI-owned SQL tab. Leaves existing user tabs intact. SQL is not executed.",
            json!({"title":{"type":"string"},"sql":{"type":"string"}}),
        ),
        (
            "replace_query",
            "Replace SQL in an AI-owned tab only if expected_sql still matches. Never overwrite user edits.",
            json!({"tab_id":{"type":"integer"},"expected_sql":{"type":"string"},"sql":{"type":"string"}}),
        ),
        (
            "run_query",
            "Run an AI-owned tab through the visible editor, preserving Safe mode and confirmation dialogs. Requires expected_sql. Returns immediately; inspect_query reports completion. Use LIMIT.",
            json!({"tab_id":{"type":"integer"},"expected_sql":{"type":"string"}}),
        ),
        (
            "inspect_query",
            "Inspect an AI-owned tab: SQL, running/error state and a bounded preview of the selected result (20 rows, 30 columns). Does not fetch spilled rows.",
            json!({"tab_id":{"type":"integer"}}),
        ),
        (
            "cancel_query",
            "Cancel the currently running AI query through the normal database cancellation path.",
            json!({}),
        ),
    ];
    Value::Array(tools.into_iter().map(|(name, description, mut properties)| {
        if name != "list_workspaces" { properties["workspace_id"] = json!({"type":"integer","description":"Exact workspace ID returned by list_workspaces. Never guess or silently switch targets."}); }
        let required: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect())
}

fn message(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    let count = reader
        .take(MAX_MESSAGE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count as u64 > MAX_MESSAGE {
        return Err("Request exceeds 128 KiB".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}

#[cfg(unix)]
pub(crate) fn listen(path: &std::path::Path, ctx: eframe::egui::Context) -> Result<Bridge, String> {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let parent = path
        .parent()
        .ok_or("Socket needs a private parent directory")?;
    let permissions = std::fs::metadata(parent)
        .map_err(|e| e.to_string())?
        .permissions();
    if permissions.mode() & 0o077 != 0 {
        return Err("Socket directory must be private (mode 0700); use mktemp -d.".into());
    }
    let listener =
        UnixListener::bind(path).map_err(|e| format!("Cannot bind {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let (send, requests) = mpsc::sync_channel::<Request>(8);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            let outcome = (|| {
                let value = message(&mut BufReader::new(&mut stream))?.ok_or("Missing request")?;
                let name = value["name"]
                    .as_str()
                    .ok_or("Missing tool name")?
                    .to_owned();
                let arguments = value["arguments"].clone();
                let (reply, receive) = mpsc::sync_channel(1);
                send.try_send(Request {
                    name,
                    arguments,
                    expires: Instant::now() + Duration::from_secs(3),
                    reply,
                })
                .map_err(|_| "Workspace unavailable or queue full")?;
                ctx.request_repaint();
                receive
                    .recv_timeout(Duration::from_secs(4))
                    .map_err(|_| "Workspace did not respond")?
            })();
            let result = match outcome {
                Ok(value) => json!({"ok":value}),
                Err(error) => json!({"error":error}),
            };
            let encoded = result.to_string();
            if encoded.len() as u64 > MAX_MESSAGE - 1024 {
                let _ = writeln!(
                    stream,
                    "{}",
                    json!({"error":"Response exceeds the 128 KiB preview budget; narrow the workspace or query."})
                );
            } else {
                let _ = writeln!(stream, "{encoded}");
            }
        }
    });
    Ok(Bridge {
        requests: Some(requests),
        id: Bridge::next_id(),
        demo: true,
        enabled: true,
        activity: "Ready for your AI client".into(),
        owned: Default::default(),
    })
}

/// Bound the final MCP envelope too: embedding JSON in text adds escaping.
fn result_line(id: &Value, result: Value) -> String {
    let line = json!({"jsonrpc":"2.0","id":id,"result":result}).to_string();
    if line.len() as u64 <= MAX_MESSAGE {
        return line;
    }
    json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":"Response exceeds the 128 KiB preview budget; narrow the request."}]}}).to_string()
}

/// Stdio MCP adapter. Each tool request opens a short-lived local connection.
#[cfg(unix)]
pub fn client(path: &std::path::Path) -> i32 {
    use std::os::unix::net::UnixStream;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let request = match message(&mut input) {
            Ok(Some(value)) => value,
            Ok(None) => return 0,
            Err(error) => {
                eprintln!("Matrix MCP: {error}");
                return 1;
            }
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        if !(id.is_i64() || id.is_u64() || id.as_str().is_some_and(|id| id.len() <= 256)) {
            let error = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Request ID must be an integer or a string of at most 256 bytes"}});
            if writeln!(output, "{error}")
                .and_then(|_| output.flush())
                .is_err()
            {
                return 0;
            }
            continue;
        }

        let result = match request["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"rustable-matrix","version":env!("CARGO_PKG_VERSION")},"instructions":"You control explicitly enabled SQL workspaces in Rustable. Start with list_workspaces and use the exact workspace_id on every action. Open named query tabs, then run and inspect them. Query tools return immediately; wait before polling. The user can take over at any time. Never claim a query succeeded until inspect_query confirms it. Never automatically replay a write with an unknown commit outcome. Never choose a different workspace when a target disappears. Credentials stay in Rustable. Safe mode and user confirmations govern writes; inspect the workspace before executing."})
            }
            Some("ping") => json!({}),
            Some("tools/list") => json!({"tools":definitions()}),
            Some("tools/call") => {
                let result = (|| -> Result<Value, String> {
                    let mut stream = UnixStream::connect(path)
                        .map_err(|e| format!("Open Rustable with Matrix enabled first: {e}"))?;
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .map_err(|e| e.to_string())?;
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .map_err(|e| e.to_string())?;
                    writeln!(stream, "{}", request["params"]).map_err(|e| e.to_string())?;
                    let response =
                        message(&mut BufReader::new(stream))?.ok_or("Workspace disconnected")?;
                    if let Some(error) = response["error"].as_str() {
                        return Err(error.into());
                    }
                    Ok(response["ok"].clone())
                })();
                match result {
                    Ok(value) => {
                        json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                    }
                    Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
                }
            }
            _ => {
                let error = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unknown method"}});
                if writeln!(output, "{error}")
                    .and_then(|_| output.flush())
                    .is_err()
                {
                    return 0;
                }
                continue;
            }
        };
        if writeln!(output, "{}", result_line(id, result))
            .and_then(|_| output.flush())
            .is_err()
        {
            return 0;
        }
    }
}

/// Capture the manager banner once in an explicit fixture run.
#[cfg(feature = "qa")]
pub(crate) fn capture_launcher(ctx: &eframe::egui::Context) {
    let Some(prefix) = std::env::var_os("RUSTABLE_MATRIX_CAPTURE") else {
        return;
    };
    if std::env::var_os("RUSTABLE_MATRIX_FIXTURE").is_none() {
        return;
    }
    let key = eframe::egui::Id::new("matrix-launcher-capture");
    let stage = ctx.data(|d| d.get_temp::<u8>(key).unwrap_or(0));
    if stage == 0 {
        ctx.data_mut(|d| d.insert_temp(key, 1u8));
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Screenshot(Default::default()));
        ctx.request_repaint();
    } else if stage == 1 {
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                eframe::egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            ctx.data_mut(|d| d.insert_temp(key, 2u8));
            std::thread::spawn(move || {
                let mut bytes =
                    format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                for pixel in &image.pixels {
                    bytes.extend_from_slice(&pixel.to_array()[..3]);
                }
                std::fs::write(
                    format!("{}-manager.ppm", std::path::PathBuf::from(prefix).display()),
                    bytes,
                )
                .expect("write manager capture");
            });
        }
    }
}

/// Native rendering QA: capture the completed scripted tour without OS automation.
#[cfg(feature = "qa")]
pub(crate) struct Capture {
    pub app: crate::Rustable,
    pub path: std::path::PathBuf,
    pub requested: bool,
    pub saved: Option<mpsc::Receiver<()>>,
}
#[cfg(feature = "qa")]
impl eframe::App for Capture {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
        if std::env::var_os("RUSTABLE_MATRIX_FIXTURE").is_some() {
            self.app.matrix_fixture_capture(ctx);
            return;
        }
        if self
            .saved
            .as_ref()
            .is_some_and(|done| done.try_recv().is_ok())
        {
            self.saved = None;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                eframe::egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let path = self.path.clone();
            let (done, saved) = mpsc::channel();
            self.saved = Some(saved);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let mut bytes =
                    format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                for pixel in &image.pixels {
                    bytes.extend_from_slice(&pixel.to_array()[..3]);
                }
                std::fs::write(path, bytes).expect("write Matrix preview");
                let _ = done.send(());
                ctx.request_repaint();
            });
        }
        if !self.requested && self.app.matrix_capture_ready() {
            self.requested = true;
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Screenshot(Default::default()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_final_mcp_envelope_after_escaping() {
        let text = "\\\"".repeat(50_000);
        let line = result_line(
            &json!(1),
            json!({"content":[{"type":"text","text":text}],"isError":false}),
        );
        assert!(line.len() as u64 <= MAX_MESSAGE);
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["result"]["isError"], true);
    }
    #[test]
    fn bounds_incoming_messages() {
        let mut too_large = std::io::Cursor::new(vec![b' '; MAX_MESSAGE as usize + 1]);
        assert!(message(&mut too_large).unwrap_err().contains("128 KiB"));
        assert_eq!(
            message(&mut std::io::Cursor::new(b"{\"id\":1}\n")).unwrap(),
            Some(json!({"id":1}))
        );
    }
}
