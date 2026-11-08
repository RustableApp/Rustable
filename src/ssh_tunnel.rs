//! Rust-owned OpenSSH transport. No shell and no installation are involved.
//!
//! `start` belongs on a connection worker, never the render thread. The returned
//! lease owns a private, non-reused master and its loopback listener. Dropping the
//! lease closes all forwarded connections. Cancelling database queries should use
//! the database's cancellation protocol, not destroy this lease.
use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const SSH: &str = "/usr/bin/ssh";
const START_TIMEOUT: Duration = Duration::from_secs(20);
const ERROR_LIMIT: usize = 16 * 1024;

/// Native OpenSSH configuration supplies aliases, jump hosts and authentication.
/// An explicit key is added before configured identities; it is not exclusive.
/// Encrypted keys use the agent or an ephemeral GUI passphrase prompt.
#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    /// Zero inherits the port from OpenSSH configuration.
    pub port: u16,
    pub user: String,
    pub identity_file: Option<PathBuf>,
    /// None uses OpenSSH's standard user/global known_hosts files. Unknown and
    /// changed hosts always fail closed; enrollment is deliberately separate.
    pub known_hosts_file: Option<PathBuf>,
    /// None uses native user/system configuration; fixtures supply an isolated file.
    pub config_file: Option<PathBuf>,
    /// Resolved by the SSH server, permitting database hosts on its private LAN.
    pub remote_host: String,
    pub remote_port: u16,
}

pub struct Tunnel {
    child: Child,
    _directory: tempfile::TempDir,
    local_port: u16,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl Tunnel {
    pub fn start(config: &Config, cancel: &AtomicBool) -> Result<Self, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("SSH connection cancelled".into());
        }
        if cfg!(windows) {
            return Err("Built-in SSH tunnels are not available on Windows yet. Use a direct connection or an external SSH tunnel.".into());
        }
        config.validate()?;
        let directory = tempfile::Builder::new()
            .prefix("rustable-ssh-")
            .tempdir()
            .map_err(|e| format!("Create private SSH control directory: {e}"))?;
        let control = directory.path().join("control");
        // Keep well below both macOS and Linux sockaddr_un limits. Private
        // control paths avoid reusing or terminating a user's own SSH session.
        if control.as_os_str().len() > 100 {
            return Err("Temporary directory path is too long for an SSH control socket".into());
        }
        let reserved = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Reserve SSH forwarding port: {e}"))?;
        let local_port = reserved.local_addr().map_err(|e| e.to_string())?.port();
        let mut command = command(config, &control);
        #[cfg(unix)]
        let prompt = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
            let socket = directory.path().join("askpass");
            let bridge = crate::ssh_prompt::Bridge::start(&socket, &config.host)?;
            command.env("SSH_ASKPASS", std::env::current_exe().map_err(|e| e.to_string())?)
                .env("SSH_ASKPASS_REQUIRE", "force").env("RUSTABLE_SSH_PROMPT_SOCKET", socket)
                .env("DISPLAY", "rustable:0");
            bridge
        };
        // OpenSSH cannot inherit this listener. If another process takes the
        // released port, ExitOnForwardFailure aborts startup rather than returning
        // a tunnel pointed at the wrong listener.
        let mut child = command
            .spawn()
            .map_err(|e| format!("Start system OpenSSH ({SSH}): {e}"))?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let capture = stderr.clone();
        let mut reader = child
            .stderr
            .take()
            .ok_or("OpenSSH error pipe unavailable")?;
        let stderr_thread = std::thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                if let Ok(mut output) = capture.lock() {
                    let remaining = ERROR_LIMIT.saturating_sub(output.len());
                    output.extend_from_slice(&buffer[..count.min(remaining)]);
                }
                // Continue draining even once retention is full, so the process
                // cannot stall on a full error pipe.
            }
        });
        let mut tunnel = Self {
            child,
            _directory: directory,
            local_port,
            stderr,
            stderr_thread: Some(stderr_thread),
        };
        #[cfg(unix)]
        let mut started = Instant::now();
        #[cfg(not(unix))]
        let started = Instant::now();
        loop {
            #[cfg(unix)]
            {
                if prompt.cancelled() { return Err("SSH key unlock cancelled".into()); }
                if prompt.waiting.load(Ordering::Relaxed) { started = Instant::now(); }
            }
            if cancel.load(Ordering::Relaxed) {
                return Err("SSH connection cancelled".into());
            }
            if let Some(status) = tunnel
                .child
                .try_wait()
                .map_err(|e| format!("Check SSH process: {e}"))?
            {
                tunnel.join_reader();
                return Err(format!(
                    "SSH connection failed ({status}): {}",
                    tunnel.diagnostics()
                ));
            }
            if control.exists()
                && check_master(&control, &config.host)?
                && tunnel
                    .child
                    .try_wait()
                    .map_err(|e| e.to_string())?
                    .is_none()
            {
                // ClearAllForwardings on the master excludes inherited listeners.
                // Add just our loopback forward over its private control socket.
                drop(reserved);
                install_forward(config, &control, local_port, cancel)?;
                return Ok(tunnel);
            }
            if started.elapsed() >= START_TIMEOUT {
                return Err(format!(
                    "SSH connection timed out. {}",
                    tunnel.diagnostics()
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }
    #[cfg(all(test, unix))]
    pub fn local_host(&self) -> &'static str {
        "127.0.0.1"
    }

    /// Detect a disconnected gateway before dispatching further database work.
    pub fn check_alive(&mut self) -> Result<(), String> {
        if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
            self.join_reader();
            Err(format!(
                "SSH tunnel closed ({status}): {}",
                self.diagnostics()
            ))
        } else {
            Ok(())
        }
    }

    pub fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.join_reader();
    }

    fn join_reader(&mut self) {
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
    fn diagnostics(&self) -> String {
        self.stderr
            .lock()
            .map(|s| String::from_utf8_lossy(&s).trim().to_owned())
            .unwrap_or_default()
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.close(); /* directory drops after the child stops */
    }
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        validate_host(&self.host)?;
        validate_host(&self.remote_host)?;
        if self.remote_port == 0 {
            return Err("Destination port must be nonzero".into());
        }
        if self
            .user
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("Enter an SSH user without whitespace or control characters".into());
        }
        for (name, path) in [
            ("private key", &self.identity_file),
            ("known_hosts", &self.known_hosts_file),
        ] {
            if let Some(path) = path {
                if !path.is_absolute() {
                    return Err(format!("SSH {name} path must be absolute"));
                }
                if !path.is_file() {
                    return Err(format!("SSH {name} file does not exist"));
                }
                if path.to_str().is_none_or(|s| s.contains(['\n', '\r', '\0'])) {
                    return Err(format!("Invalid SSH {name} path"));
                }
            }
        }
        Ok(())
    }
}
fn validate_host(host: &str) -> Result<(), String> {
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(
            "Enter a hostname or IP address without URL syntax, whitespace, or shell characters"
                .into(),
        );
    }
    Ok(())
}
fn forward_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}
fn command(config: &Config, control: &Path) -> Command {
    let mut command = Command::new(SSH);
    if let Some(path) = &config.config_file {
        command.arg("-F").arg(path);
    }
    command.args(["-N", "-T", "-M", "-S"]).arg(control).args([
        "-o",
        "ControlMaster=yes",
        "-o",
        "ClearAllForwardings=yes",
        "-o",
        "PermitLocalCommand=no",
        "-o",
        "Tunnel=no",
        "-o",
        "ForkAfterAuthentication=no",
        "-o",
        "RemoteCommand=none",
        "-o",
        "ControlPersist=no",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "BatchMode=no",
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        "PreferredAuthentications=publickey",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ConnectionAttempts=1",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ForwardX11=no",
        "-o",
        "RequestTTY=no",
    ]);
    if let Some(path) = &config.identity_file {
        // -i checks the unexpanded path before token expansion, preventing
        // literal percent signs. IdentityFile performs expansion exactly once.
        let path = escape_tokens(path)
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        command
            .arg("-o")
            .arg(format!("IdentityFile=\"{path}\""))
            .args(["-o", "IdentitiesOnly=yes"]);
    }
    if let Some(path) = &config.known_hosts_file {
        // OpenSSH parses this option as a list, so quote literal spaces and
        // backslashes within a single file path; percent tokens are escaped.
        let path = escape_tokens(path)
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        command
            .arg("-o")
            .arg(format!("UserKnownHostsFile=\"{path}\""));
    }
    if config.port != 0 {
        command.arg("-p").arg(config.port.to_string());
    }
    if !config.user.is_empty() {
        command.arg("-l").arg(&config.user);
    }
    command
        .arg("--")
        .arg(&config.host)
        .env("SSH_ASKPASS_REQUIRE", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}
fn forward_command(config: &Config, control: &Path, port: u16) -> Command {
    let mut command = Command::new(SSH);
    command
        .args(["-F", "/dev/null", "-S"])
        .arg(control)
        .args([
            "-O",
            "forward",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ClearAllForwardings=no",
            "-L",
        ])
        .arg(format!(
            "127.0.0.1:{port}:{}:{}",
            forward_host(&config.remote_host),
            config.remote_port
        ))
        .arg("--")
        .arg(&config.host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn install_forward(
    config: &Config,
    control: &Path,
    port: u16,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("SSH forwarding cancelled".into());
    }
    let mut child = forward_command(config, control, port)
        .spawn()
        .map_err(|e| format!("Install SSH forward: {e}"))?;
    let started = Instant::now();
    loop {
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Check SSH forward: {error}"));
            }
        };
        if let Some(status) = status {
            if status.success() {
                return Ok(());
            }
            let mut error = String::new();
            if let Some(stderr) = child.stderr.take() {
                let _ = stderr.take(ERROR_LIMIT as u64).read_to_string(&mut error);
            }
            return Err(format!("Install SSH forward: {}", error.trim()));
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("SSH forwarding cancelled or timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn escape_tokens(path: &Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}
fn check_master(control: &Path, host: &str) -> Result<bool, String> {
    let mut child = Command::new(SSH)
        .args(["-F", "/dev/null", "-S"])
        .arg(control)
        .args(["-O", "check", "--", host])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Check SSH forwarding readiness: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.success());
        }
        if start.elapsed() > Duration::from_millis(300) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            host: "127.0.0.1".into(),
            port: 22,
            user: "qa".into(),
            identity_file: None,
            known_hosts_file: None,
            config_file: if cfg!(unix) { Some("/dev/null".into()) } else { None },
            remote_host: "db.internal".into(),
            remote_port: 5432,
        }
    }
    #[cfg(unix)]
    fn resolved(command: &Command) -> String {
        let output = Command::new(SSH)
            .arg("-G")
            .args(command.get_args())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn native_config_resolves_includes_aliases_and_explicit_overrides_without_network() {
        let directory = tempfile::tempdir().unwrap();
        let include = directory.path().join("included.conf");
        let config_path = directory.path().join("ssh_config");
        std::fs::write(&include, "Host qa-*\n HostName database.gateway.test\n User inherited-user\n Port 2222\n IdentityFile /tmp/configured-key\n ProxyJump bastion.example.test\n LocalForward 1234 localhost:4321\n RemoteForward 6789 localhost:9876\n DynamicForward 2345\n RemoteCommand unwanted-command\n PermitLocalCommand yes\n LocalCommand unwanted-command\n ForkAfterAuthentication yes\n Tunnel point-to-point\n").unwrap();
        std::fs::write(&config_path, format!("Include {}\n", include.display())).unwrap();
        let mut c = config();
        c.host = "qa-alias".into();
        c.port = 0;
        c.user.clear();
        c.config_file = Some(config_path);
        assert!(c.validate().is_ok());
        let output = resolved(&command(&c, Path::new("/tmp/private/control")));
        for expected in [
            "hostname database.gateway.test",
            "user inherited-user",
            "port 2222",
            "identityfile /tmp/configured-key",
            "proxyjump bastion.example.test",
            "clearallforwardings yes",
            "permitlocalcommand no",
            "forkafterauthentication no",
            "tunnel false",
            "stricthostkeychecking true",
        ] {
            assert!(
                output.lines().any(|line| line == expected),
                "Missing {expected}: {output}"
            );
        }
        assert!(!output.lines().any(|line| line.starts_with("localforward ")
            || line.starts_with("remoteforward ")
            || line.starts_with("dynamicforward ")));
        c.user = "override-user".into();
        c.port = 22;
        c.identity_file = Some("/tmp/selected-key".into());
        let output = resolved(&command(&c, Path::new("/tmp/private/control")));
        assert!(output.lines().any(|line| line == "user override-user"));
        assert!(output.lines().any(|line| line == "port 22"));
        let keys: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("identityfile "))
            .collect();
        assert_eq!(
            keys,
            [
                "identityfile /tmp/selected-key",
                "identityfile /tmp/configured-key"
            ]
        );
        let forward = forward_command(&c, Path::new("/tmp/private/control"), 12345);
        let args: Vec<_> = forward
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect();
        assert!(args.iter().any(|arg| arg == "ClearAllForwardings=no"));
        assert!(
            args.iter()
                .any(|arg| arg == "127.0.0.1:12345:db.internal:5432")
        );
        #[cfg(unix)]
        assert!(args.windows(2).any(|pair| pair == ["-F", "/dev/null"]));
    }

    #[test]
    #[ignore = "starts disposable loopback sshd using installed system tools"]
    #[cfg(unix)]
    fn config_alias_live_forward_and_drop_cleanup() {
        use std::io::Write;
        use std::net::TcpStream;
        struct Server(Child);
        impl Drop for Server {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let directory = tempfile::Builder::new()
            .prefix("rustable-ssh-qa-")
            .tempdir_in("/tmp")
            .unwrap();
        let dir = directory.path();
        for name in ["host", "client"] {
            let output = Command::new("/usr/bin/ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(dir.join(name))
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
        let ssh_port = reserved.local_addr().unwrap().port();
        let user = String::from_utf8(
            Command::new("/usr/bin/id")
                .arg("-un")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let sshd_config = dir.join("sshd_config");
        std::fs::write(&sshd_config, format!("ListenAddress 127.0.0.1\nPort {ssh_port}\nHostKey {}\nAuthorizedKeysFile {}\nPidFile {}\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nAllowUsers {user}\nAllowTcpForwarding yes\n", dir.join("host").display(), dir.join("client.pub").display(), dir.join("sshd.pid").display())).unwrap();
        let log = std::fs::File::create(dir.join("sshd.log")).unwrap();
        drop(reserved);
        let mut server = Server(
            Command::new("/usr/sbin/sshd")
                .args(["-D", "-e", "-f"])
                .arg(&sshd_config)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        let start = Instant::now();
        while TcpStream::connect((Ipv4Addr::LOCALHOST, ssh_port)).is_err() {
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "sshd: {}",
                std::fs::read_to_string(dir.join("sshd.log")).unwrap()
            );
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(20));
        }
        let echo = TcpListener::bind("127.0.0.1:0").unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        echo.set_nonblocking(true).unwrap();
        let echo_thread = std::thread::spawn(move || {
            let start = Instant::now();
            loop {
                if let Ok((mut stream, _)) = echo.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut bytes = [0; 4];
                    stream.read_exact(&mut bytes).unwrap();
                    stream.write_all(&bytes).unwrap();
                    return;
                }
                assert!(start.elapsed() < Duration::from_secs(25));
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let client_config = dir.join("config");
        std::fs::write(&client_config, format!("Host fixture-alias\n HostName 127.0.0.1\n Port {ssh_port}\n User {user}\n IdentityFile {}\n LocalForward 12345 localhost:1\n", dir.join("client").display())).unwrap();
        let known = dir.join("known_hosts");
        std::fs::write(
            &known,
            format!(
                "[127.0.0.1]:{ssh_port} {}",
                std::fs::read_to_string(dir.join("host.pub")).unwrap()
            ),
        )
        .unwrap();
        let c = Config {
            host: "fixture-alias".into(),
            port: 0,
            user: String::new(),
            identity_file: None,
            config_file: Some(client_config),
            known_hosts_file: Some(known),
            remote_host: "127.0.0.1".into(),
            remote_port: echo_port,
        };
        // Never enroll or overwrite the user's host keys during verification.
        let rejected_hosts = dir.join("rejected_hosts");
        let mut rejected = c.clone();
        rejected.known_hosts_file = Some(rejected_hosts.clone());
        for contents in [String::new(), format!("[127.0.0.1]:{ssh_port} {}", std::fs::read_to_string(dir.join("client.pub")).unwrap())] {
            std::fs::write(&rejected_hosts, contents).unwrap();
            assert!(Tunnel::start(&rejected, &AtomicBool::new(false)).is_err(), "unverified SSH host connected");
        }
        let mut tunnel = Tunnel::start(&c, &AtomicBool::new(false)).unwrap();
        let local_port = tunnel.local_port();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, local_port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(b"ping").unwrap();
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ping");
        echo_thread.join().unwrap();
        tunnel.check_alive().unwrap();
        let control_dir = tunnel._directory.path().to_owned();
        drop(stream);
        drop(tunnel);
        assert!(!control_dir.exists());
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, local_port)).is_err());

        // A Redis connection owns its tunnel, including when moved out of a
        // session into a topology router. No caller-side lease is needed.
        let redis_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut redis_config = c.clone();
        redis_config.remote_port = redis_listener.local_addr().unwrap().port();
        let responder = std::thread::spawn(move || {
            let (mut socket, _) = redis_listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut parser = redis::Parser::new();
            loop {
                let command: Vec<Vec<u8>> = redis::from_redis_value(parser.parse_value(&mut socket).unwrap()).unwrap();
                match command[0].as_slice() {
                    b"CLIENT" => socket.write_all(b"+OK\r\n").unwrap(),
                    b"PING" => { socket.write_all(b"+PONG\r\n").unwrap(); break; }
                    _ => panic!("unexpected Redis setup command"),
                }
            }
        });
        let tunnel = Tunnel::start(&redis_config, &AtomicBool::new(false)).unwrap();
        let port = tunnel.local_port();
        let directory = tunnel._directory.path().to_owned();
        let client = redis::Client::open("redis://unresolvable.invalid:1").unwrap();
        let mut connection = client.get_connection_via(
            (Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(5),
        ).unwrap();
        connection.retain_transport(Box::new(tunnel));
        assert!(directory.exists());
        assert_eq!(redis::cmd("PING").query::<String>(&mut connection).unwrap(), "PONG");
        responder.join().unwrap();
        drop(connection);
        assert!(!directory.exists());
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());

        crate::redis_settings::with_ssh_test_files(
            c.known_hosts_file.clone().unwrap(), c.config_file.clone().unwrap(), || {
                for mode in 0..3 {
                    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                    let settings = crate::redis_settings::Settings {
                        host: "127.0.0.1".into(), port: listener.local_addr().unwrap().port(), tls: false,
                        ssh: Some(crate::connection_profiles::SshOptions { host: "fixture-alias".into(), ..Default::default() }),
                        ..Default::default()
                    };
                    let responder = std::thread::spawn(move || {
                        listener.set_nonblocking(true).unwrap();
                        let started = Instant::now();
                        let mut socket = loop {
                            if let Ok((socket, _)) = listener.accept() { break socket; }
                            assert!(started.elapsed() < Duration::from_secs(10));
                            std::thread::sleep(Duration::from_millis(10));
                        };
                        socket.set_nonblocking(false).unwrap();
                        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let mut parser = redis::Parser::new();
                        let mut operations = 0;
                        while let Ok(value) = parser.parse_value(&mut socket) {
                            let command: Vec<Vec<u8>> = redis::from_redis_value(value).unwrap();
                            match command[0].as_slice() {
                                b"CLIENT" | b"SELECT" => socket.write_all(b"+OK\r\n").unwrap(),
                                b"PING" => { operations += 1; socket.write_all(b"+PONG\r\n").unwrap(); }
                                b"MONITOR" => { operations += 1; socket.write_all(b"+OK\r\n").unwrap(); }
                                _ => panic!("unexpected tunneled operation"),
                            }
                        }
                        operations
                    });
                    match mode {
                        0 => {
                            let mut session = crate::redis_session::Session::connect(&settings, "").unwrap();
                            assert_eq!(session.execute("PING", true).unwrap(), vec![redis::Value::SimpleString("PONG".into())]);
                        }
                        1 => {
                            let mut node = crate::redis_session::Session::cluster_node(&settings, "").unwrap();
                            assert_eq!(redis::cmd("PING").query::<String>(&mut node).unwrap(), "PONG");
                        }
                        _ => {
                            let control = crate::redis_stream::Control::default();
                            let mut stream = control.start(&settings, "", "", &redis::cmd("MONITOR")).unwrap();
                            assert_eq!(redis::Parser::new().parse_value(&mut stream).unwrap(), redis::Value::Okay);
                            control.stop();
                        }
                    }
                    assert!(responder.join().unwrap() >= 1);
                }
            },
        );
    }

    #[test]
    fn arguments_keep_host_validation_and_forwarding_private() {
        let config = config();
        let command = command(&config, Path::new("/tmp/private/control"));
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for arg in [
            "StrictHostKeyChecking=yes",
            "BatchMode=no",
            "ExitOnForwardFailure=yes",
            "ControlPersist=no",
            "ForwardAgent=no",
            "ClearAllForwardings=yes",
        ] {
            assert!(args.contains(&arg.to_owned()));
        }
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), "127.0.0.1");
        assert!(
            !args
                .iter()
                .any(|a| a.contains("accept-new") || a.contains("StrictHostKeyChecking=no"))
        );
    }
    #[test]
    fn paths_are_literal_and_host_syntax_cannot_inject_options() {
        for host in [
            "-oProxyCommand=evil",
            "a b",
            "a\nb",
            "$(evil)",
            "host:22",
            "user@host",
        ] {
            assert!(validate_host(host).is_err());
        }
        assert!(validate_host("::1").is_ok());
        assert_eq!(forward_host("::1"), "[::1]");
        assert_eq!(
            escape_tokens(Path::new("/tmp/key %h file")),
            "/tmp/key %%h file"
        );
        let mut c = config();
        c.identity_file = Some(PathBuf::from("relative"));
        assert!(c.validate().is_err());
    }
    #[test]
    fn cancelled_start_never_spawns_a_process() {
        let cancelled = AtomicBool::new(true);
        assert!(
            Tunnel::start(&config(), &cancelled)
                .err()
                .unwrap()
                .contains("cancelled")
        );
    }
    #[test]
    #[cfg(unix)]
    fn close_and_drop_reap_process_and_remove_private_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut tunnel = Tunnel {
            child,
            _directory: directory,
            local_port: 12345,
            stderr: Arc::new(Mutex::new(Vec::new())),
            stderr_thread: None,
        };
        assert!(tunnel.check_alive().is_ok());
        assert_eq!(tunnel.local_host(), "127.0.0.1");
        assert_eq!(tunnel.local_port(), 12345);
        tunnel.close();
        assert!(tunnel.child.try_wait().unwrap().is_some());
        tunnel.close();
        drop(tunnel);
        assert!(!path.exists());
    }
    #[test]
    #[cfg(unix)]
    fn failed_local_connection_reports_error_without_external_access() {
        // A bound non-listening-for-SSH port fails locally without contacting an
        // external server; use a closed ephemeral port to fail immediately.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut c = config();
        c.port = port;
        let error = Tunnel::start(&c, &AtomicBool::new(false))
            .err()
            .expect("closed local SSH endpoint must fail");
        assert!(error.contains("SSH connection failed"), "{error}");
    }
}

#[cfg(all(test, windows))]
#[test]
fn windows_reports_unsupported_transport_without_spawning() {
    let config = Config { host: "localhost".into(), port: 22, user: String::new(), identity_file: None, known_hosts_file: None, config_file: None, remote_host: "localhost".into(), remote_port: 5432 };
    assert!(Tunnel::start(&config, &AtomicBool::new(false)).err().unwrap().contains("not available on Windows"));
}
