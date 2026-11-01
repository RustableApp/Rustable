//! One-shot, read-only connection diagnostics. Never runs on the render thread.
use crate::connection_profiles::ConnectionKind;
use std::error::Error;
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
            run(connection, password, &stop, &emit);
            emit(Event::Finished);
        });
        Self { receiver, cancel }
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

fn run(connection: ConnectionKind, password: String, cancel: &AtomicBool, emit: &impl Fn(Event)) {
    use ProbeField::*;
    match connection {
        ConnectionKind::SQLite { path } => {
            emit(Event::Running(vec![File]));
            let result = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .and_then(|conn| {
                conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
            });
            match result {
                Ok(_) => emit(Event::Success(vec![File])),
                Err(error) => emit(Event::Failed(vec![File], error.to_string())),
            }
        }
        ConnectionKind::Postgres(profile) => {
            let config = profile.with_password(password);
            let ssh_fields = vec![SshHost, SshPort, SshUser, SshIdentity];
            let tunnel = if let Some(ssh) = &config.ssh {
                emit(Event::Running(ssh_fields.clone()));
                match crate::ssh_tunnel::Tunnel::start(ssh, cancel) {
                    Ok(tunnel) => {
                        emit(Event::Success(ssh_fields));
                        Some(tunnel)
                    }
                    Err(error) => {
                        emit(Event::Failed(ssh_fields, error));
                        return;
                    }
                }
            } else {
                None
            };
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let mut fields = vec![Host, Port, Database, User, Password];
            if config.tls {
                fields.push(Tls);
            }
            emit(Event::Running(fields.clone()));
            let mut connection =
                crate::db::probe_client_config(&config, tunnel.as_ref().map(|t| t.local_port()));
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    emit(Event::Failed(fields, error.to_string()));
                    return;
                }
            };
            let result = runtime.block_on(async {
                if config.tls {
                    connection.ssl_mode(postgres::config::SslMode::Require);
                    let tls = match native_tls::TlsConnector::new() {
                        Ok(tls) => tls,
                        Err(error) => {
                            emit(Event::Failed(vec![Tls], error.to_string()));
                            return Err(ProbeError::Stopped);
                        }
                    };
                    connect_and_check(
                        &connection,
                        postgres_native_tls::MakeTlsConnector::new(tls),
                        cancel,
                        &fields,
                        emit,
                        std::time::Duration::from_secs(10),
                    )
                    .await
                } else {
                    connection.ssl_mode(postgres::config::SslMode::Disable);
                    connect_and_check(
                        &connection,
                        postgres::NoTls,
                        cancel,
                        &fields,
                        emit,
                        std::time::Duration::from_secs(10),
                    )
                    .await
                }
            });
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match result {
                Ok(()) | Err(ProbeError::Stopped) => {}
                Err(ProbeError::TimedOut) => {
                    emit(Event::Failed(fields, "Connection test timed out".into()));
                }
                Err(ProbeError::Postgres(error)) => {
                    if let Some(db) = error.as_db_error() {
                        let mut succeeded = vec![Host, Port];
                        if config.tls {
                            succeeded.push(Tls);
                        }
                        emit(Event::Success(succeeded));
                        emit(Event::Failed(
                            sqlstate_fields(db.code().code()),
                            describe_error(&error),
                        ));
                    } else {
                        let mut source: Option<&(dyn Error + 'static)> = Some(&error);
                        let mut tls_failure = false;
                        let mut network_failure = false;
                        while let Some(error) = source {
                            tls_failure |= error.is::<native_tls::Error>();
                            network_failure |= error.is::<std::io::Error>();
                            source = error.source();
                        }
                        let failed = if tls_failure {
                            vec![Tls]
                        } else if network_failure {
                            vec![Host, Port]
                        } else {
                            fields
                        };
                        emit(Event::Failed(failed, describe_error(&error)));
                    }
                }
            }
        }
    }
}
enum ProbeError {
    Postgres(postgres::Error),
    Stopped,
    TimedOut,
}

async fn interruptible<T>(
    future: impl std::future::Future<Output = Result<T, postgres::Error>>,
    cancel: &AtomicBool,
    timeout: std::time::Duration,
) -> Result<T, ProbeError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut future = std::pin::pin!(future);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ProbeError::Stopped);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(ProbeError::TimedOut);
        }
        let slice = (deadline - now).min(std::time::Duration::from_millis(20));
        if let Ok(result) = tokio::time::timeout(slice, future.as_mut()).await {
            return result.map_err(ProbeError::Postgres);
        }
    }
}

async fn connect_and_check<T>(
    config: &tokio_postgres::Config,
    tls: T,
    cancel: &AtomicBool,
    fields: &[ProbeField],
    emit: &impl Fn(Event),
    timeout: std::time::Duration,
) -> Result<(), ProbeError>
where
    T: tokio_postgres::tls::MakeTlsConnect<tokio_postgres::Socket>,
    T::Stream: Send + 'static,
    T::TlsConnect: Send,
    <T::TlsConnect as tokio_postgres::tls::TlsConnect<tokio_postgres::Socket>>::Future: Send,
{
    let (client, connection) = interruptible(config.connect(tls), cancel, timeout).await?;
    let task = tokio::spawn(connection);
    emit(Event::Success(
        fields
            .iter()
            .copied()
            .filter(|field| *field != ProbeField::Database)
            .collect(),
    ));
    let result = interruptible(client.simple_query("SELECT 1"), cancel, timeout).await;
    // Also release a stalled socket on cancellation/timeout, rather than leaving
    // its connection task waiting for the remote server to respond.
    task.abort();
    let _ = task.await;
    match result {
        Ok(_) => {
            emit(Event::Success(vec![ProbeField::Database]));
            Ok(())
        }
        Err(ProbeError::Postgres(error)) => {
            emit(Event::Failed(
                vec![ProbeField::Database],
                describe_error(&error),
            ));
            Err(ProbeError::Stopped)
        }
        Err(ProbeError::TimedOut) => {
            emit(Event::Failed(
                vec![ProbeField::Database],
                "Database readiness check timed out".into(),
            ));
            Err(ProbeError::Stopped)
        }
        Err(error) => Err(error),
    }
}

fn sqlstate_fields(code: &str) -> Vec<ProbeField> {
    use ProbeField::*;
    match code {
        "28P01" | "28000" => vec![User, Password],
        "3D000" => vec![Database],
        // Server failures can be unrelated to any single input (capacity, shutdown, etc.).
        _ => vec![Host, Port, Database, User, Password],
    }
}
fn describe_error(error: &postgres::Error) -> String {
    if let Some(db) = error.as_db_error() {
        return format!("{} (SQLSTATE {})", db.message(), db.code().code());
    }
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let detail = cause.to_string();
        if !text.contains(&detail) {
            text.push_str(": ");
            text.push_str(&detail);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires opt-in disposable local PostgreSQL QA fixture"]
    fn local_postgres_probe_reports_auth_database_and_success() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let mut profile = crate::connection_profiles::PostgresProfile {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            tls: false,
            ssh: None,
        };
        let collect = |profile, password: &str| {
            let events = std::cell::RefCell::new(Vec::new());
            run(
                ConnectionKind::Postgres(profile),
                password.into(),
                &AtomicBool::new(false),
                &|event| events.borrow_mut().push(event),
            );
            events.into_inner()
        };
        let events = collect(profile.clone(), "deliberately_incorrect_qa_password");
        assert!(
            matches!(events.last(), Some(Event::Failed(fields, message)) if fields == &[ProbeField::User, ProbeField::Password] && message.contains("28P01")),
            "{events:?}"
        );
        profile.database = "rustable_missing_probe_database".into();
        let events = collect(profile.clone(), "rustable_qa_only");
        assert!(
            matches!(events.last(), Some(Event::Failed(fields, message)) if fields == &[ProbeField::Database] && message.contains("3D000")),
            "{events:?}"
        );
        profile.database = "rustable_qa".into();
        let events = collect(profile, "rustable_qa_only");
        assert!(
            matches!(events.last(), Some(Event::Success(fields)) if fields.contains(&ProbeField::Database)),
            "{events:?}"
        );
    }
    #[test]
    fn cancelling_stalled_handshake_and_readiness_query_releases_socket() {
        use std::io::{Read, Write};
        use std::time::Duration;
        for authenticated in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let profile = crate::connection_profiles::PostgresProfile {
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
                tls: false,
                ..Default::default()
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let (finished, completion) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                run(
                    ConnectionKind::Postgres(profile),
                    String::new(),
                    &stop,
                    &|_| {},
                );
                finished.send(()).unwrap();
            });
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut length = [0; 4];
            socket.read_exact(&mut length).unwrap();
            let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
            socket.read_exact(&mut startup).unwrap();
            if authenticated {
                // AuthenticationOk followed by ReadyForQuery, then deliberately
                // withhold the SELECT 1 response to model an unresponsive server.
                socket
                    .write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")
                    .unwrap();
                let mut tag = [0];
                socket.read_exact(&mut tag).unwrap();
                assert_eq!(tag, [b'Q']);
                socket.read_exact(&mut length).unwrap();
                let mut query = vec![0; u32::from_be_bytes(length) as usize - 4];
                socket.read_exact(&mut query).unwrap();
                assert_eq!(query, b"SELECT 1\0");
            }
            cancel.store(true, Ordering::Relaxed);
            completion
                .recv_timeout(Duration::from_secs(2))
                .expect("cancelled probe must finish promptly");
            worker.join().unwrap();
            let mut byte = [0];
            match socket.read(&mut byte) {
                Ok(0) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                result => panic!("probe left the socket open: {result:?}"),
            }
        }
    }

    #[test]
    fn readiness_timeout_preserves_authenticated_fields_and_closes_socket() {
        use std::io::{Read, Write};
        use std::time::Duration;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut length = [0; 4];
            socket.read_exact(&mut length).unwrap();
            let mut startup = vec![0; u32::from_be_bytes(length) as usize - 4];
            socket.read_exact(&mut startup).unwrap();
            socket
                .write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")
                .unwrap();
            let mut tag = [0];
            socket.read_exact(&mut tag).unwrap();
            assert_eq!(tag, [b'Q']);
            socket.read_exact(&mut length).unwrap();
            let mut query = vec![0; u32::from_be_bytes(length) as usize - 4];
            socket.read_exact(&mut query).unwrap();
            assert_eq!(query, b"SELECT 1\0");
            assert_eq!(
                socket.read(&mut tag).unwrap(),
                0,
                "timeout must close transport"
            );
        });
        let mut config = tokio_postgres::Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("qa")
            .ssl_mode(postgres::config::SslMode::Disable);
        let events = std::cell::RefCell::new(Vec::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(connect_and_check(
            &config,
            postgres::NoTls,
            &AtomicBool::new(false),
            &[
                ProbeField::Host,
                ProbeField::Port,
                ProbeField::User,
                ProbeField::Password,
                ProbeField::Database,
            ],
            &|event| events.borrow_mut().push(event),
            Duration::from_millis(200),
        ));
        assert!(matches!(result, Err(ProbeError::Stopped)));
        let events = events.into_inner();
        assert!(
            matches!(&events[0], Event::Success(fields) if fields.contains(&ProbeField::Password) && !fields.contains(&ProbeField::Database))
        );
        assert!(
            matches!(&events[1], Event::Failed(fields, message) if fields == &[ProbeField::Database] && message.contains("timed out"))
        );
        assert_eq!(events.len(), 2);
        server.join().unwrap();
    }

    #[test]
    fn sqlite_probe_is_read_only_and_never_creates_missing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.sqlite");
        let events = std::cell::RefCell::new(Vec::new());
        run(
            ConnectionKind::SQLite { path: path.clone() },
            String::new(),
            &AtomicBool::new(false),
            &|event| events.borrow_mut().push(event),
        );
        assert!(!path.exists());
        assert!(
            matches!(events.borrow().last(), Some(Event::Failed(fields, _)) if fields == &[ProbeField::File])
        );
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE untouched(id INTEGER)")
            .unwrap();
        events.borrow_mut().clear();
        run(
            ConnectionKind::SQLite { path },
            String::new(),
            &AtomicBool::new(false),
            &|event| events.borrow_mut().push(event),
        );
        assert!(matches!(events.borrow().last(), Some(Event::Success(_))));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM untouched", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn postgres_errors_do_not_blame_password_for_missing_database() {
        assert_eq!(
            sqlstate_fields("28P01"),
            vec![ProbeField::User, ProbeField::Password]
        );
        assert_eq!(sqlstate_fields("3D000"), vec![ProbeField::Database]);
        assert!(sqlstate_fields("53300").contains(&ProbeField::Host));
    }
    #[test]
    fn closed_loopback_port_marks_network_not_password() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let profile = crate::connection_profiles::PostgresProfile {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        };
        let events = std::cell::RefCell::new(Vec::new());
        run(
            ConnectionKind::Postgres(profile),
            String::new(),
            &AtomicBool::new(false),
            &|event| events.borrow_mut().push(event),
        );
        assert!(
            matches!(events.borrow().last(), Some(Event::Failed(fields, _)) if fields == &[ProbeField::Host, ProbeField::Port])
        );
    }
}
