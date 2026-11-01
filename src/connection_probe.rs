//! One-shot, read-only connection diagnostics. Never runs on the render thread.
use crate::connection_profiles::ConnectionKind;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProbeField {
    File,
    SshHost,
    SshPort,
    SshUser,
    SshIdentity,
    Host,
    Port,
    Database,
    User,
    Password,
    Tls,
}
#[derive(Clone, Debug)]
pub enum Event {
    Running(Vec<ProbeField>),
    Success(Vec<ProbeField>),
    Failed(Vec<ProbeField>, String),
    Finished,
}
pub struct Probe {
    receiver: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Probe {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(connection: ConnectionKind, password: String, ctx: eframe::egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let emit = |event| {
                if !stop.load(Ordering::Relaxed) {
                    let _ = sender.send(event);
                    ctx.request_repaint();
                }
            };
            native::run(connection, password, &stop, &emit);
            emit(Event::Finished);
        });
        Self { receiver, cancel }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn start(_connection: ConnectionKind, _password: String, ctx: eframe::egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(Event::Failed(vec![], "Connection testing requires the desktop app.".into()));
        let _ = sender.send(Event::Finished);
        ctx.request_repaint();
        Self { receiver, cancel: Arc::new(AtomicBool::new(false)) }
    }
    pub fn poll(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
