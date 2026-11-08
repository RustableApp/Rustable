//! Passwords live exclusively in the platform credential store or ephemeral memory.
//! A single worker serializes operations so a later delete cannot race an earlier save.
use crate::connection_profiles::ProfileId;
use std::sync::{
    Arc,
    mpsc::{self, Receiver, Sender},
};

pub const SERVICE: &str = if cfg!(feature = "qa") {
    "dev.rustable.qa"
} else if cfg!(debug_assertions) {
    "dev.rustable.development"
} else {
    "dev.rustable.app"
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Purpose {
    #[default]
    Database,
    RedisSentinel,
}
fn account(id: ProfileId, purpose: Purpose) -> String {
    match purpose {
        // Preserve existing keychain accounts; no migration or overwrite.
        Purpose::Database => format!("connection-{}-password", id.0),
        Purpose::RedisSentinel => format!("connection-{}-redis-sentinel-password", id.0),
    }
}

pub trait Backend: Send + Sync + 'static {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, password: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}
#[cfg(all(not(test), not(target_arch = "wasm32")))]
struct Native;
#[cfg(all(not(test), not(target_arch = "wasm32")))]
impl Backend for Native {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn write(&self, account: &str, password: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|entry| entry.set_password(password))
            .map_err(|e| e.to_string())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

pub enum Operation {
    Read,
    Write(String),
    Delete,
}
pub struct Response {
    pub token: u64,
    pub result: Result<Option<String>, String>,
}
#[cfg(not(target_arch = "wasm32"))]
struct Request {
    token: u64,
    id: ProfileId,
    purpose: Purpose,
    operation: Operation,
}
pub struct Worker {
    #[cfg(not(target_arch = "wasm32"))]
    requests: Sender<Request>,
    #[cfg(target_arch = "wasm32")]
    backend: Arc<dyn Backend>,
    #[cfg(target_arch = "wasm32")]
    results: Sender<Response>,
    responses: Receiver<Response>,
    next_token: u64,
}
impl Worker {
    pub fn new() -> Self {
        #[cfg(all(not(test), not(target_arch = "wasm32")))]
        let backend = Arc::new(Native);
        #[cfg(any(test, target_arch = "wasm32"))]
        let backend = Arc::new(MemoryBackend::default());
        Self::with_backend(backend)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        let (requests, receiver) = mpsc::channel::<Request>();
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let name = account(request.id, request.purpose);
                let result = match request.operation {
                    Operation::Read => backend.read(&name),
                    Operation::Write(password) => backend.write(&name, &password).map(|()| None),
                    Operation::Delete => backend.delete(&name).map(|()| None),
                };
                let _ = sender.send(Response {
                    token: request.token,
                    result,
                });
            }
        });
        Self {
            requests,
            responses,
            next_token: 0,
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        let (results, responses) = mpsc::channel();
        Self { backend, results, responses, next_token: 0 }
    }
    pub fn request(&mut self, id: ProfileId, operation: Operation) -> u64 {
        self.request_for(id, Purpose::Database, operation)
    }
    pub fn request_for(&mut self, id: ProfileId, purpose: Purpose, operation: Operation) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        let token = self.next_token;
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.requests.send(Request {
            token,
            id,
            purpose,
            operation,
        });
        #[cfg(target_arch = "wasm32")]
        {
            let name = account(id, purpose);
            let result = match operation {
                Operation::Read => self.backend.read(&name),
                Operation::Write(password) => self.backend.write(&name, &password).map(|()| None),
                Operation::Delete => self.backend.delete(&name).map(|()| None),
            };
            let _ = self.results.send(Response { token, result });
        }
        token
    }
    pub fn poll(&self) -> Option<Response> {
        self.responses.try_recv().ok()
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
#[derive(Default)]
pub struct MemoryBackend {
    values: std::sync::Mutex<std::collections::HashMap<String, String>>,
}
#[cfg(any(test, target_arch = "wasm32"))]
impl Backend for MemoryBackend {
    fn read(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.values.lock().unwrap().get(account).cloned())
    }
    fn write(&self, account: &str, password: &str) -> Result<(), String> {
        self.values
            .lock()
            .unwrap()
            .insert(account.to_owned(), password.to_owned());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.values.lock().unwrap().remove(account);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn result(worker: &Worker) -> Response {
        let start = std::time::Instant::now();
        loop {
            if let Some(response) = worker.poll() {
                return response;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn sentinel_and_database_passwords_are_independent_and_keep_legacy_accounts() {
        let backend = Arc::new(MemoryBackend::default());
        backend.write("connection-7-password", "legacy-data").unwrap();
        let mut worker = Worker::with_backend(backend.clone());
        let save = worker.request_for(ProfileId(7), Purpose::RedisSentinel, Operation::Write("discovery-only".into()));
        let read = worker.request(ProfileId(7), Operation::Read);
        assert_eq!(result(&worker).token, save);
        let response = result(&worker);
        assert_eq!(response.token, read);
        assert_eq!(response.result.unwrap().as_deref(), Some("legacy-data"));
        worker.request(ProfileId(7), Operation::Delete);
        worker.request_for(ProfileId(7), Purpose::RedisSentinel, Operation::Read);
        assert!(result(&worker).result.is_ok());
        assert_eq!(result(&worker).result.unwrap().as_deref(), Some("discovery-only"));
        // A later scoped delete is ordered after an earlier save in the same worker.
        worker.request_for(ProfileId(7), Purpose::RedisSentinel, Operation::Write("replacement".into()));
        worker.request_for(ProfileId(7), Purpose::RedisSentinel, Operation::Delete);
        for _ in 0..2 { assert!(result(&worker).result.is_ok()); }
        assert_eq!(backend.read("connection-7-password").unwrap(), None);
        assert_eq!(backend.read("connection-7-redis-sentinel-password").unwrap(), None);
    }
    #[test]
    fn serial_worker_keeps_profiles_isolated_and_delete_follows_pending_save() {
        let mut worker = Worker::new();
        worker.request(
            ProfileId(1),
            Operation::Write("only-in-secure-store".into()),
        );
        worker.request(ProfileId(2), Operation::Write("other".into()));
        worker.request(ProfileId(1), Operation::Delete);
        for _ in 0..3 {
            assert!(result(&worker).result.is_ok());
        }
        worker.request(ProfileId(1), Operation::Read);
        assert_eq!(result(&worker).result.unwrap(), None);
        worker.request(ProfileId(2), Operation::Read);
        assert_eq!(result(&worker).result.unwrap().as_deref(), Some("other"));
        assert!(SERVICE.starts_with("dev.rustable."));
    }
}
