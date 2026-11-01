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
fn account(id: ProfileId) -> String {
    format!("connection-{}-password", id.0)
}

pub trait Backend: Send + Sync + 'static {
    fn read(&self, account: &str) -> Result<Option<String>, String>;
    fn write(&self, account: &str, password: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}
#[cfg(not(test))]
struct Native;
#[cfg(not(test))]
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
struct Request {
    token: u64,
    id: ProfileId,
    operation: Operation,
}
pub struct Worker {
    requests: Sender<Request>,
    responses: Receiver<Response>,
    next_token: u64,
}
impl Worker {
    pub fn new() -> Self {
        #[cfg(not(test))]
        let backend = Arc::new(Native);
        #[cfg(test)]
        let backend = Arc::new(MemoryBackend::default());
        Self::with_backend(backend)
    }
    pub fn with_backend(backend: Arc<dyn Backend>) -> Self {
        let (requests, receiver) = mpsc::channel::<Request>();
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                let name = account(request.id);
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
    pub fn request(&mut self, id: ProfileId, operation: Operation) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        let token = self.next_token;
        let _ = self.requests.send(Request {
            token,
            id,
            operation,
        });
        token
    }
    pub fn poll(&self) -> Option<Response> {
        self.responses.try_recv().ok()
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct MemoryBackend {
    values: std::sync::Mutex<std::collections::HashMap<String, String>>,
}
#[cfg(test)]
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
