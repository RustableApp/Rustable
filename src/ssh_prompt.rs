//! OpenSSH askpass bridge. Secrets travel over a private local socket, never argv,
//! environment variables, preferences, or logs.
#[cfg(unix)]
mod native {
    use std::{
        io::{Read, Write},
        os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
        path::Path,
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };
    use zeroize::Zeroizing;

    struct Prompt {
        label: String,
        secret: Zeroizing<String>,
        reply: mpsc::Sender<Option<Zeroizing<String>>>,
        stopped: Arc<AtomicBool>,
    }
    static CONTEXT: OnceLock<Mutex<Option<eframe::egui::Context>>> = OnceLock::new();
    static PROMPTS: OnceLock<Mutex<std::collections::VecDeque<Prompt>>> = OnceLock::new();
    fn prompts() -> &'static Mutex<std::collections::VecDeque<Prompt>> {
        PROMPTS.get_or_init(Default::default)
    }

    pub struct Bridge {
        stopped: Arc<AtomicBool>,
        pub waiting: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Bridge {
        pub fn cancelled(&self) -> bool {
            self.stopped.load(Ordering::Relaxed)
        }

        pub fn start(path: &Path, host: &str) -> Result<Self, String> {
            let listener =
                UnixListener::bind(path).map_err(|e| format!("Create SSH prompt socket: {e}"))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let stopped = Arc::new(AtomicBool::new(false));
            let waiting = Arc::new(AtomicBool::new(false));
            let stop = stopped.clone();
            let active = waiting.clone();
            let host = host.to_owned();
            let thread = std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(25));
                        continue;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let mut bytes = Vec::new();
                    if Read::by_ref(&mut stream)
                        .take(4096)
                        .read_to_end(&mut bytes)
                        .is_err()
                    {
                        continue;
                    }
                    let label = String::from_utf8_lossy(&bytes).into_owned();
                    // Never answer host verification or unrelated askpass questions.
                    if !label.to_ascii_lowercase().contains("passphrase") {
                        continue;
                    }
                    let (reply, receive) = mpsc::channel();
                    active.store(true, Ordering::Relaxed);
                    prompts().lock().unwrap().push_back(Prompt {
                        label: format!("{host}\n{label}"),
                        secret: Zeroizing::new(String::new()),
                        reply,
                        stopped: stop.clone(),
                    });
                    if let Some(ctx) = CONTEXT.get().and_then(|slot| slot.lock().ok()?.clone()) {
                        ctx.request_repaint();
                        ctx.request_repaint_of(eframe::egui::ViewportId::ROOT);
                    }
                    while !stop.load(Ordering::Relaxed) {
                        match receive.recv_timeout(Duration::from_millis(50)) {
                            Ok(Some(secret)) => {
                                let _ = stream.write_all(secret.as_bytes());
                                break;
                            }
                            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                    active.store(false, Ordering::Relaxed);
                }
            });
            Ok(Self {
                stopped,
                waiting,
                thread: Some(thread),
            })
        }
    }
    impl Drop for Bridge {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    pub fn helper() -> Option<i32> {
        let path = std::env::var_os("RUSTABLE_SSH_PROMPT_SOCKET")?;
        let result = (|| -> std::io::Result<()> {
            let mut stream = UnixStream::connect(path)?;
            let label = std::env::args().nth(1).unwrap_or_default();
            stream.write_all(label.as_bytes())?;
            stream.shutdown(std::net::Shutdown::Write)?;
            let mut secret = Zeroizing::new(Vec::new());
            stream.take(65536).read_to_end(&mut secret)?;
            if secret.is_empty() {
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            std::io::stdout().write_all(&secret)?;
            Ok(())
        })();
        Some(if result.is_ok() { 0 } else { 1 })
    }
    pub fn show(ctx: &eframe::egui::Context) {
        use eframe::egui;
        *CONTEXT.get_or_init(Default::default).lock().unwrap() = Some(ctx.clone());
        let mut queue = prompts().lock().unwrap();
        queue.retain(|p| !p.stopped.load(Ordering::Relaxed));
        let Some(prompt) = queue.front_mut() else {
            return;
        };
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("ssh-key-passphrase")).show(ctx, |ui| {
            ui.set_width(420.0_f32.min((ctx.content_rect().width() - 40.0).max(200.0)));
            ui.heading("Unlock SSH key");
            ui.label(&prompt.label);
            let response = ui.add(
                egui::TextEdit::singleline(&mut *prompt.secret)
                    .password(true)
                    .hint_text("Key passphrase")
                    .desired_width(f32::INFINITY),
            );
            if !ctx.memory(|memory| memory.has_focus(response.id)) && prompt.secret.is_empty() {
                response.request_focus();
            }
            ui.weak("Used for this connection only. The key stays encrypted.");
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
                if ui
                    .add_enabled(!prompt.secret.is_empty(), egui::Button::new("Unlock"))
                    .clicked()
                    || (!prompt.secret.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    answer = Some(true);
                }
            });
        });
        if modal.should_close() {
            answer = Some(false);
        }
        if let Some(unlock) = answer {
            let prompt = queue.pop_front().unwrap();
            if !unlock {
                prompt.stopped.store(true, Ordering::Relaxed);
            }
            let _ = prompt.reply.send(unlock.then_some(prompt.secret));
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{process::Command, time::Instant};
        static SERIAL: Mutex<()> = Mutex::new(());

        fn respond(answer: Option<&str>) {
            let start = Instant::now();
            loop {
                if let Some(prompt) = prompts().lock().unwrap().pop_front() {
                    if answer.is_none() {
                        prompt.stopped.store(true, Ordering::Relaxed);
                    }
                    prompt
                        .reply
                        .send(answer.map(|s| Zeroizing::new(s.to_owned())))
                        .unwrap();
                    return;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(5),
                    "SSH prompt did not arrive"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn private_bridge_returns_secret_and_cancel_closes_channel() {
            let _guard = SERIAL.lock().unwrap();
            for answer in [Some("a passphrase with spaces"), None] {
                let dir = tempfile::tempdir().unwrap();
                let socket = dir.path().join("askpass");
                let bridge = Bridge::start(&socket, "test-host").unwrap();
                assert_eq!(
                    std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                let thread = std::thread::spawn(move || {
                    let mut stream = UnixStream::connect(socket).unwrap();
                    stream
                        .write_all(b"Enter passphrase for key '/tmp/key':")
                        .unwrap();
                    stream.shutdown(std::net::Shutdown::Write).unwrap();
                    let mut reply = String::new();
                    stream.read_to_string(&mut reply).unwrap();
                    reply
                });
                respond(answer);
                assert_eq!(thread.join().unwrap(), answer.unwrap_or_default());
                assert_eq!(bridge.cancelled(), answer.is_none());
            }
        }

        #[test]
        #[ignore = "requires cargo build --bin rustable and system ssh-keygen"]
        fn openssh_decrypts_encrypted_key_through_app_helper() {
            let _guard = SERIAL.lock().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let key = dir.path().join("encrypted-key");
            assert!(
                Command::new("/usr/bin/ssh-keygen")
                    .args(["-q", "-t", "ed25519", "-N", "fixture passphrase", "-f"])
                    .arg(&key)
                    .status()
                    .unwrap()
                    .success()
            );
            let helper = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("rustable");
            assert!(helper.exists(), "Build the app binary first");
            for (attempt, answer) in [Some("wrong passphrase"), Some("fixture passphrase"), None]
                .into_iter()
                .enumerate()
            {
                let socket = dir.path().join(format!("askpass-{attempt}"));
                let _bridge = Bridge::start(&socket, "test-host").unwrap();
                let child = Command::new("/usr/bin/ssh-keygen")
                    .args(["-y", "-f"])
                    .arg(&key)
                    .env("SSH_ASKPASS", &helper)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("DISPLAY", "rustable:0")
                    .env("RUSTABLE_SSH_PROMPT_SOCKET", &socket)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                respond(answer);
                let output = child.wait_with_output().unwrap();
                assert_eq!(output.status.success(), attempt == 1);
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout).starts_with("ssh-ed25519 "),
                    attempt == 1
                );
                assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture passphrase"));
            }
        }
    }
}
#[cfg(unix)]
pub use native::*;

#[cfg(not(unix))]
pub fn helper() -> Option<i32> {
    None
}
#[cfg(not(unix))]
pub fn show(_: &eframe::egui::Context) {}
