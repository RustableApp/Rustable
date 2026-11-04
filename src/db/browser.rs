//! Browser transport for the same database commands and SQLite driver as desktop.
use super::*;
use bincode::Options;
use std::{cell::{Cell, RefCell}, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};

use super::transport::codec;

#[wasm_bindgen(module = "/src/db/browser_cancel.js")]
extern "C" {
    fn database_cancelled() -> bool;
}
thread_local! {
    static CONNECTION: RefCell<Option<Box<dyn Driver>>> = RefCell::new(None);
    static DEADLINE: Cell<Option<web_time::Instant>> = const { Cell::new(None) };
}

pub(super) fn should_interrupt() -> bool {
    database_cancelled() || DEADLINE.with(|deadline| deadline.get().is_some_and(|t| web_time::Instant::now() >= t))
}

/// Called only inside the browser database worker, never from a rendering frame.
pub(crate) fn execute(bytes: &[u8]) -> Vec<u8> {
    let command = codec().deserialize::<Command>(bytes).map_err(|e| e.to_string());
    let (result, state) = CONNECTION.with(|slot| {
        let mut connection = slot.borrow_mut();
        DEADLINE.set(Some(web_time::Instant::now() + Duration::from_secs(2)));
        let result = command.and_then(|command| {
            validate_command(&command)?;
            match &command {
                Command::Open(None) | Command::OpenBrowserDatabase { .. } | Command::Query { .. } | Command::Browse(_)
                | Command::Definition(_) | Command::TableDetails(_) | Command::RefreshSchema | Command::SelectSchema(_) | Command::ListDatabases
                | Command::Plan(_) | Command::Apply(_) | Command::ImportTarget(_)
                | Command::DownloadTable { .. } | Command::PreviewCsv { .. } | Command::ImportCsv { .. } => {}
                _ => return Err("This operation needs a browser file adapter or the desktop app.".into()),
            }
            handle(&mut connection, command, &Arc::new(AtomicBool::new(false)))
        });
        DEADLINE.set(None);
        let state = connection.as_mut().map_or(TransactionState::Idle, |driver| driver.transaction_state());
        (result, state)
    });
    codec().serialize(&(result, state)).unwrap_or_else(|error| {
        codec().serialize(&(Err::<Payload, _>(format!("Browser result exceeds its transport limit: {error}")), state)).expect("bounded error message")
    })
}

fn validate_command(command: &Command) -> Result<(), String> {
    if let Command::Query { sql, .. } = command && sql.len() > 8 * 1024 * 1024 {
        return Err("Browser SQL runs are limited to 8 MiB. Select a smaller script.".into());
    }
    Ok(())
}

pub struct Worker {
    worker: Result<web_sys::Worker, String>,
    pub rx: Receiver<Result<Payload, String>>,
    transaction: Arc<AtomicU8>,
    cancellation: Option<js_sys::Int32Array>,
    failed: Rc<Cell<bool>>,
    started: Rc<Cell<Option<web_time::Instant>>>,
    replies: mpsc::Sender<Result<Payload, String>>,
    ctx: eframe::egui::Context,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}
impl Worker {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let replies = tx.clone();
        let context = ctx.clone();
        let failed = Rc::new(Cell::new(false));
        let started = Rc::new(Cell::new(None));
        let message_failed = failed.clone();
        let message_started = started.clone();
        let error_failed = failed.clone();
        let error_started = started.clone();
        let transaction = Arc::new(AtomicU8::new(0));
        let status = transaction.clone();
        let error_status = transaction.clone();
        let repaint = ctx.clone();
        let errors = tx.clone();
        let message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            if message_failed.get() { return; }
            message_started.set(None);
            let result = super::transport::message_bytes(event.data()).and_then(|bytes|
                codec().deserialize::<(Result<Payload, String>, TransactionState)>(&bytes).map_err(|error| error.to_string()));
            let reply = match result {
                Ok((reply, state)) => { status.store(state as u8, Ordering::Release); reply }
                Err(error) => {
                    message_failed.set(true);
                    status.store(TransactionState::Disconnected as u8, Ordering::Release);
                    Err(format!("Invalid database worker response: {error}. Open the demo again to start a fresh database."))
                },
            };
            let _ = tx.send(reply);
            repaint.request_repaint();
        }) as Box<dyn FnMut(_)>);
        let error = Closure::wrap(Box::new(move |event: web_sys::ErrorEvent| {
            if error_failed.replace(true) { return; }
            error_started.set(None);
            error_status.store(TransactionState::Disconnected as u8, Ordering::Release);
            let _ = errors.send(Err(format!("Browser database worker failed: {}. Open the demo again to start a fresh database; changes in the failed database cannot be recovered.", event.message())));
            ctx.request_repaint();
        }) as Box<dyn FnMut(_)>);
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        let worker = web_sys::Worker::new_with_options("./database-worker.js", &options).map_err(|e| format!("Could not start database worker: {}", crate::browser_error(e)));
        let mut cancellation = None;
        if let Ok(worker) = &worker {
            worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error.as_ref().unchecked_ref()));
            // Shared cancellation does not wait for the busy worker's event loop.
            if js_sys::Reflect::get(&js_sys::global(), &"crossOriginIsolated".into()).ok().and_then(|v| v.as_bool()) == Some(true) {
                let buffer = js_sys::SharedArrayBuffer::new(4);
                let array = js_sys::Int32Array::new(&buffer);
                let message = js_sys::Object::new();
                if js_sys::Reflect::set(&message, &"cancel".into(), &buffer).is_ok() && worker.post_message(&message).is_ok() {
                    cancellation = Some(array);
                }
            }
        }
        if worker.is_err() { failed.set(true); transaction.store(TransactionState::Disconnected as u8, Ordering::Release); }
        Self { worker, rx, transaction, cancellation, failed, started, replies, ctx: context, _message: message, _error: error }
    }
    pub fn send(&self, command: Command) -> Result<(), String> {
        if self.failed.get() { return Err("The browser database worker is disconnected. Open the demo again to start a fresh database.".into()); }
        if self.started.get().is_some() { return Err("A browser database operation is already running.".into()); }
        validate_command(&command)?;
        if let Some(flag) = &self.cancellation { let _ = js_sys::Atomics::store(flag, 0, 0); }
        let bytes = codec().serialize(&command).map_err(|e| format!("Operation is not available in this browser session: {e}"))?;
        self.worker.as_ref().map_err(Clone::clone)?.post_message(&js_sys::Uint8Array::from(bytes.as_slice())).map_err(|e| format!("Could not send database command: {}", crate::browser_error(e)))?;
        self.started.set(Some(web_time::Instant::now()));
        self.ctx.request_repaint_after(Duration::from_secs(10));
        Ok(())
    }
    pub fn restart_if_failed(&mut self) {
        if self.failed.get() { *self = Self::new(self.ctx.clone()); }
    }
    pub fn check_health(&self) {
        if self.failed.get() {
            if let Ok(worker) = &self.worker { worker.terminate(); }
            return;
        }
        if self.started.get().is_some_and(|start| start.elapsed() >= Duration::from_secs(10)) {
            self.failed.set(true);
            self.started.set(None);
            self.transaction.store(TransactionState::Disconnected as u8, Ordering::Release);
            if let Ok(worker) = &self.worker { worker.terminate(); }
            let _ = self.replies.send(Err("The browser database worker stopped responding and was terminated. Open the demo again to start a fresh database; its previous contents are lost.".into()));
            self.ctx.request_repaint();
        }
    }
    pub fn transaction_state(&self) -> TransactionState {
        match self.transaction.load(Ordering::Acquire) { 1 => TransactionState::Active, 2 => TransactionState::Failed, 3 => TransactionState::Disconnected, _ => TransactionState::Idle }
    }
    pub fn progress(&self) -> progress::Snapshot { progress::Snapshot::default() }
    pub fn cancel(&self) {
        if let Some(flag) = &self.cancellation { let _ = js_sys::Atomics::store(flag, 0, 1); }
    }
}
impl Drop for Worker {
    fn drop(&mut self) { if let Ok(worker) = &self.worker { worker.terminate(); } }
}
