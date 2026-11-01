//! Isolated, asynchronous LSP processes. SQL is never executed by this transport.
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct ServerSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub root: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub directory: Option<tempfile::TempDir>,
    pub initial_requests: Vec<(String, Value)>,
}
#[derive(Clone, Debug)]
pub struct Document {
    pub id: u64,
    pub version: u64,
    pub text: String,
    pub cursor: usize,
}
#[derive(Debug)]
pub enum Event {
    Ready,
    Diagnostics {
        id: u64,
        version: u64,
        items: Vec<Value>,
    },
    Completions {
        id: u64,
        version: u64,
        items: Vec<Value>,
    },
    Error(String),
}
pub struct Worker {
    pub rx: mpsc::Receiver<Event>,
    pending: Arc<Mutex<Option<Document>>>,
    stop: Arc<AtomicBool>,
    wake: thread::Thread,
}
impl Worker {
    pub fn start(
        ctx: egui_context::Context,
        factory: impl FnOnce() -> Result<ServerSpec, String> + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(4);
        let pending = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (pending.clone(), stop.clone());
        let handle = thread::spawn(move || {
            let emit = |event| {
                let _ = tx.try_send(event);
                ctx.request_repaint();
            };
            if let Err(error) = factory().and_then(|spec| run(spec, p, s, &emit)) {
                emit(Event::Error(error));
            }
        });
        Self {
            rx,
            pending,
            stop,
            wake: handle.thread().clone(),
        }
    }
    pub fn submit(&self, document: Document) {
        *self.pending.lock().unwrap() = Some(document);
        self.wake.unpark();
    }
}
// egui is re-exported by eframe, avoiding a second GUI dependency.
mod egui_context {
    pub use eframe::egui::Context;
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake.unpark();
    }
}

const MAX_FRAME: usize = 2 * 1024 * 1024;
fn read_frame(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut length = None;
    let mut total = 0;
    loop {
        let mut line = String::new();
        if reader
            .by_ref()
            .take(8193)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("Language server closed its output".into());
        }
        total += line.len();
        if total > 8192 {
            return Err("Oversized language server header".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length
        .filter(|n| *n <= MAX_FRAME)
        .ok_or("Invalid language server message length")?;
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
fn send(writer: &mut impl Write, value: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    write!(writer, "Content-Length: {}\r\n\r\n", bytes.len())
        .and_then(|_| writer.write_all(&bytes))
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}
pub fn position(text: &str, cursor: usize) -> Value {
    let (mut line, mut character) = (0, 0);
    for ch in text.chars().take(cursor) {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16();
        }
    }
    json!({"line":line,"character":character})
}
fn uri(path: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
fn run(
    spec: ServerSpec,
    pending: Arc<Mutex<Option<Document>>>,
    stop: Arc<AtomicBool>,
    emit: &impl Fn(Event),
) -> Result<(), String> {
    // No inherited DB credentials, agent sockets, or per-user configuration environment.
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start language server: {e}"))?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    // Supervisor owns process lifetime, so even a blocked pipe write cannot block shutdown.
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    let stopping = stop.clone();
    let directory = spec.directory;
    thread::spawn(move || {
        // Retain private configuration until the supervised process has exited.
        let _directory = directory;
        while !stopping.load(Ordering::Relaxed) && !done.load(Ordering::Relaxed) {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
        let _ = child.kill();
        let _ = child.wait();
    });
    struct Finish(Arc<AtomicBool>);
    impl Drop for Finish {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let _finish = Finish(finished);
    let (tx, rx) = mpsc::sync_channel(2);
    let owner = thread::current();
    thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let result = read_frame(&mut reader);
            let failed = result.is_err();
            let disconnected = tx.send(result).is_err();
            owner.unpark();
            if disconnected || failed {
                break;
            }
        }
    });
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":uri(&spec.root),"capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"completion":{"completionItem":{"snippetSupport":false}},"publishDiagnostics":{"versionSupport":true}}}}}),
    )?;
    let started = Instant::now();
    let mut ready = false;
    let mut next_id = 2u64;
    let mut init_left = 0usize;
    let mut current: Option<(Document, String, u64)> = None;
    while !stop.load(Ordering::Relaxed) {
        if !ready && started.elapsed() > Duration::from_secs(20) {
            return Err("Language server initialization timed out".into());
        }
        match rx.try_recv() {
            Ok(Ok(message)) => {
                if let (Some(id), Some(method)) = (
                    message.get("id"),
                    message.get("method").and_then(Value::as_str),
                ) {
                    let result = if method == "workspace/configuration" {
                        json!(vec![
                            Value::Null;
                            message["params"]["items"]
                                .as_array()
                                .map_or(0, Vec::len)
                        ])
                    } else {
                        Value::Null
                    };
                    send(&mut input, json!({"jsonrpc":"2.0","id":id,"result":result}))?;
                } else if message["id"] == 1 {
                    if let Some(error) = message.get("error") {
                        return Err(format!("Language server initialization failed: {error}"));
                    }
                    send(
                        &mut input,
                        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
                    )?;
                    for (method, params) in &spec.initial_requests {
                        send(
                            &mut input,
                            json!({"jsonrpc":"2.0","id":next_id,"method":method,"params":params}),
                        )?;
                        next_id += 1;
                        init_left += 1;
                    }
                    if init_left == 0 {
                        ready = true;
                        emit(Event::Ready);
                    }
                } else if !ready && message.get("id").is_some() {
                    if let Some(error) = message.get("error") {
                        return Err(format!("Language server configuration failed: {error}"));
                    }
                    init_left = init_left.saturating_sub(1);
                    if init_left == 0 {
                        ready = true;
                        emit(Event::Ready);
                    }
                } else if let Some((doc, url, request)) = &current {
                    if message["method"] == "textDocument/publishDiagnostics"
                        && message["params"]["uri"] == *url
                    {
                        emit(Event::Diagnostics {
                            id: doc.id,
                            version: doc.version,
                            items: message["params"]["diagnostics"]
                                .as_array()
                                .map(|items| items.iter().take(500).cloned().collect())
                                .unwrap_or_default(),
                        });
                    } else if message["id"] == *request {
                        let result = &message["result"];
                        emit(Event::Completions {
                            id: doc.id,
                            version: doc.version,
                            items: result
                                .as_array()
                                .or_else(|| result["items"].as_array())
                                .map(|items| items.iter().take(200).cloned().collect())
                                .unwrap_or_default(),
                        });
                    }
                }
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("Language server stopped".into());
            }
            Err(mpsc::TryRecvError::Empty) => {
                // Tokens prevent a lost wake between checking the queue and
                // parking. Only idle timeout checks poll; edits and responses
                // wake this worker immediately without render-thread I/O.
                thread::park_timeout(Duration::from_millis(25));
            }
        }
        // Drop the slot lock before pipe I/O: submitting on the UI thread must
        // never wait for a slow or blocked server stdin.
        let next_document = if ready {
            pending.lock().unwrap().take()
        } else {
            None
        };
        if let Some(doc) = next_document {
            if let Some((_, url, old_request)) = current.take() {
                send(
                    &mut input,
                    json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":old_request}}),
                )?;
                send(
                    &mut input,
                    json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":url}}}),
                )?;
            }
            if doc.text.len() > 1024 * 1024 {
                emit(Event::Diagnostics {
                    id: doc.id,
                    version: doc.version,
                    items: Vec::new(),
                });
                emit(Event::Completions {
                    id: doc.id,
                    version: doc.version,
                    items: Vec::new(),
                });
                emit(Event::Error(
                    "Language assistance paused for SQL larger than 1 MiB".into(),
                ));
                continue;
            }
            let url = uri(&spec
                .root
                .join(format!("query-{}-{}-{next_id}.sql", doc.id, doc.version)));
            send(
                &mut input,
                json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":url,"languageId":"sql","version":1,"text":doc.text}}}),
            )?;
            let request = next_id;
            next_id += 1;
            send(
                &mut input,
                json!({"jsonrpc":"2.0","id":request,"method":"textDocument/completion","params":{"textDocument":{"uri":url},"position":position(&doc.text,doc.cursor),"context":{"triggerKind":1}}}),
            )?;
            current = Some((doc, url, request));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_position() {
        assert_eq!(position("a😀\nx😀", 5), json!({"line":1,"character":3}));
    }
    #[test]
    fn bounded_frames() {
        assert!(read_frame(&mut &b"Content-Length: 99999999\r\n\r\n"[..]).is_err());
        let mut bytes = Vec::new();
        send(&mut bytes, json!({"x":"😀"})).unwrap();
        assert_eq!(
            read_frame(&mut bytes.as_slice()).unwrap(),
            json!({"x":"😀"})
        );
    }
}

/// Disconnected upstream PostgreSQL parser/linter. No database credentials enter this process.
pub fn postgres_spec(program: PathBuf) -> Result<ServerSpec, String> {
    let directory = tempfile::Builder::new()
        .prefix("rustable-pgls-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let root = directory.path().to_path_buf();
    let config = json!({"db":{"disableConnection":true},"typecheck":{"enabled":false}});
    let path = root.join("postgres-language-server.jsonc");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).map_err(|e| e.to_string())?;
    Ok(ServerSpec {
        program,
        args: vec![path.to_string_lossy().into_owned()],
        root,
        env: vec![],
        directory: Some(directory),
        initial_requests: vec![(
            "pgls/set_configuration_overrides".into(),
            json!({"overrides":config}),
        )],
    })
}
/// Upstream completion engine fed only the catalog already loaded by Rustable.
/// This helper has no database connection, credentials, row values or executable DDL.
pub fn postgres_completion_spec(
    program: PathBuf,
    objects: &[crate::db::Object],
) -> Result<ServerSpec, String> {
    let catalog = postgres_completion_catalog(objects)?;
    let directory = tempfile::Builder::new()
        .prefix("rustable-pg-completion-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let root = directory.path().to_path_buf();
    let path = root.join("catalog.json");
    std::fs::write(&path, catalog).map_err(|e| e.to_string())?;
    Ok(ServerSpec {
        program,
        args: vec![
            "--completion-catalog".into(),
            path.to_string_lossy().into_owned(),
        ],
        root,
        env: vec![],
        directory: Some(directory),
        initial_requests: vec![],
    })
}

fn postgres_completion_catalog(objects: &[crate::db::Object]) -> Result<Vec<u8>, String> {
    #[derive(serde::Serialize)]
    struct Column<'a> {
        name: &'a str,
        data_type: &'a str,
        not_null: bool,
    }
    #[derive(serde::Serialize)]
    struct Table<'a> {
        schema: &'a str,
        name: &'a str,
        columns: Vec<Column<'a>>,
    }
    #[derive(serde::Serialize)]
    struct Catalog<'a> {
        tables: Vec<Table<'a>>,
    }
    let mut tables = Vec::new();
    let mut bytes = 0usize;
    let mut columns = 0usize;
    for object in objects
        .iter()
        .filter(|object| matches!(object.kind.as_str(), "table" | "view"))
    {
        if object.metadata_error.is_some() {
            continue;
        }
        let Some(schema) = object.schema.as_deref() else {
            continue;
        };
        let Some(name) = object.display_name.as_deref() else {
            continue;
        };
        bytes = bytes
            .saturating_add(schema.len())
            .saturating_add(name.len());
        columns = columns.saturating_add(object.columns.len());
        for column in &object.columns {
            bytes = bytes
                .saturating_add(column.name.len())
                .saturating_add(column.data_type.len());
        }
        if tables.len() >= 10_000 || columns > 100_000 || bytes > 2 * 1024 * 1024 {
            return Err("PostgreSQL completion catalog exceeds the supported size; local suggestions remain available.".into());
        }
        tables.push(Table {
            schema,
            name,
            columns: object
                .columns
                .iter()
                .map(|column| Column {
                    name: &column.name,
                    data_type: &column.data_type,
                    not_null: column.not_null,
                })
                .collect(),
        });
    }
    let encoded = serde_json::to_vec(&Catalog { tables }).map_err(|e| e.to_string())?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err("PostgreSQL completion catalog exceeds 8 MiB.".into());
    }
    Ok(encoded)
}

pub fn discover_postgres_program() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = std::env::var_os("RUSTABLE_POSTGRES_LSP") {
        candidates.push(PathBuf::from(value));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("rustable-postgres-language-server"));
    }
    #[cfg(debug_assertions)]
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/pg-language-server/release/rustable-postgres-language-server"),
    );
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod live_postgres {
    use super::*;
    fn fixture() -> crate::db::Object {
        crate::db::Object {
            name: "\"public\".\"customers\"".into(),
            schema: Some("public".into()),
            display_name: Some("customers".into()),
            kind: "table".into(),
            ddl: "SECRET_DDL".into(),
            metadata_error: None,
            columns: vec![crate::db::Column {
                name: "customer_name".into(),
                data_type: "text".into(),
                not_null: true,
                default: "SECRET_DEFAULT".into(),
                pk: false,
                generated: false,
            }],
        }
    }

    #[test]
    fn completion_catalog_is_bounded_and_excludes_executable_metadata() {
        let object = fixture();
        let encoded = postgres_completion_catalog(std::slice::from_ref(&object)).unwrap();
        let text = String::from_utf8(encoded).unwrap();
        assert!(!text.contains("SECRET"));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["tables"][0]["name"], "customers");
        assert_eq!(value["tables"][0]["columns"][0]["name"], "customer_name");
        let mut bad = object;
        bad.metadata_error = Some("unavailable".into());
        assert_eq!(
            serde_json::from_slice::<Value>(&postgres_completion_catalog(&[bad]).unwrap()).unwrap()
                ["tables"],
            json!([])
        );
        let mut huge = fixture();
        huge.columns[0].name = "x".repeat(2 * 1024 * 1024);
        assert!(postgres_completion_catalog(&[huge]).is_err());
    }

    #[test]
    #[ignore = "requires Cargo-built PostgreSQL completion helper"]
    fn upstream_completion_uses_cached_schema_without_database() {
        let program = discover_postgres_program().expect("build private PostgreSQL helper");
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            postgres_completion_spec(program, &[fixture()])
        });
        let text = "SELECT c.cust FROM public.customers c;";
        worker.submit(Document {
            id: 71,
            version: 1,
            text: text.into(),
            cursor: 13,
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match worker.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Event::Completions {
                    id: 71,
                    version: 1,
                    items,
                }) => {
                    let item = items
                        .iter()
                        .find(|item| item["label"] == "customer_name")
                        .expect("schema column completion");
                    let edit = &item["textEdit"];
                    let range = crate::editor_language::range(text, &edit["range"])
                        .expect("valid replacement range");
                    let chars: Vec<_> = text.chars().collect();
                    let completed = chars[..range.start].iter().collect::<String>()
                        + edit["newText"].as_str().unwrap()
                        + &chars[range.end..].iter().collect::<String>();
                    assert_eq!(completed, "SELECT c.customer_name FROM public.customers c;");
                    return;
                }
                Ok(Event::Error(error)) => panic!("{error}"),
                _ => {}
            }
        }
        panic!("schema completion timed out");
    }

    #[test]
    #[ignore = "requires Cargo-built private PostgreSQL language server"]
    fn disconnected_parser_diagnostics() {
        let program =
            discover_postgres_program().expect("build tools/postgres-language-server first");
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            postgres_spec(program)
        });
        worker.submit(Document {
            id: 41,
            version: 1,
            text: "SELECT FROM ;".into(),
            cursor: 12,
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut ready = false;
        let mut diagnostics = false;
        let mut completions = false;
        while Instant::now() < deadline {
            match worker.rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Ready) => ready = true,
                Ok(Event::Error(error)) => panic!("{error}"),
                Ok(Event::Diagnostics {
                    id: 41,
                    version: 1,
                    items,
                }) if !items.is_empty() => {
                    eprintln!("PG diagnostics: {items:?}");
                    diagnostics = true;
                }
                Ok(Event::Completions {
                    id: 41, version: 1, ..
                }) => completions = true,
                _ => {}
            }
            if ready && diagnostics && completions {
                // A too-large replacement must retire the previous document,
                // clear its presentation without retaining stale diagnostics.
                worker.submit(Document {
                    id: 41,
                    version: 2,
                    text: "x".repeat(1024 * 1024 + 1),
                    cursor: 0,
                });
                let mut cleared = false;
                let limit = Instant::now() + Duration::from_secs(2);
                while Instant::now() < limit {
                    match worker.rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(Event::Diagnostics {
                            id: 41,
                            version: 2,
                            items,
                        }) if items.is_empty() => cleared = true,
                        Ok(Event::Error(error)) if error.contains("larger than 1 MiB") => {
                            assert!(cleared);
                            return;
                        }
                        _ => {}
                    }
                }
                panic!("oversized replacement was not retired");
            }
        }
        panic!("ready={ready} diagnostics={diagnostics} completions={completions}");
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn start_failure_is_reported_without_blocking_ui() {
        let worker = Worker::start(eframe::egui::Context::default(), || {
            Err("fixture missing server".into())
        });
        assert!(
            matches!(worker.rx.recv_timeout(Duration::from_secs(2)),Ok(Event::Error(error)) if error=="fixture missing server")
        );
    }
    #[test]
    fn pending_documents_coalesce_even_when_server_is_starting() {
        let (tx, rx) = mpsc::channel();
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            let _ = rx.recv_timeout(Duration::from_secs(2));
            Err("fixture complete".into())
        });
        for version in 0..100 {
            worker.submit(Document {
                id: 1,
                version,
                text: "SELECT 1".into(),
                cursor: version as usize,
            });
        }
        let snapshot = worker.pending.lock().unwrap().take().unwrap();
        assert_eq!(snapshot.version, 99);
        assert_eq!(snapshot.cursor, 99);
        tx.send(()).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn dropping_worker_kills_and_reaps_nonresponsive_server() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        let pid_path = root.join("pid");
        let worker = Worker::start(eframe::egui::Context::default(), move || {
            Ok(ServerSpec {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "echo $$ > pid; exec /bin/sleep 30".into()],
                root,
                env: vec![],
                directory: Some(directory),
                initial_requests: vec![],
            })
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_path)
                && !pid.trim().is_empty()
            {
                break pid.trim().to_string();
            }
            assert!(Instant::now() < deadline, "server never started");
            thread::sleep(Duration::from_millis(10));
        };
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(50));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let alive = Command::new("/bin/kill")
                .args(["-0", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            if !alive {
                break;
            }
            assert!(Instant::now() < deadline, "language server was not reaped");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !pid_path.exists(),
            "private config lifetime ended after child exit"
        );
    }
}
