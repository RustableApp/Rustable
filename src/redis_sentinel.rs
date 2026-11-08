//! Bounded Sentinel discovery. Returned transports have passed ROLE verification.
use crate::{redis_session::Session, redis_settings::Settings};
use std::time::{Duration, Instant};

pub(crate) fn connect(
    settings: &Settings,
    password: &str,
    sentinel_password: &str,
) -> Result<Session, String> {
    with_primary(
        settings,
        sentinel_password,
        || false,
        |target_settings, deadline| {
            let mut primary = Session::raw_node(target_settings, password)?;
            set_timeout(&primary, remaining(deadline)?)?;
            // Authentication already completed. ROLE and SELECT are independent
            // session checks, sent together before this transport is exposed.
            let replies = redis::pipe()
                .cmd("ROLE")
                .cmd("SELECT")
                .arg(settings.database)
                .query::<Vec<redis::Value>>(&mut primary)
                .map_err(|error| error.to_string())?;
            if replies.len() != 2 || !is_primary(&replies[0]) {
                return Ok(None);
            }
            if !matches!(&replies[1], redis::Value::Okay) {
                return Err("Redis did not confirm database selection.".into());
            }
            remaining(deadline)?;
            set_timeout(&primary, settings.read_timeout)?;
            Ok(Some(Session::verified_primary(primary, target_settings)))
        },
    )
}

/// Resolve and validate the actual transport supplied by the caller. `None`
/// means a stale replica; errors never cause user writes to be replayed.
pub(crate) fn with_primary<T>(
    settings: &Settings,
    sentinel_password: &str,
    cancelled: impl Fn() -> bool,
    mut open: impl FnMut(&Settings, Instant) -> Result<Option<T>, String>,
) -> Result<T, String> {
    settings.validate()?;
    let sentinel = settings
        .sentinel
        .as_ref()
        .ok_or("Missing Sentinel configuration")?;
    let deadline = Instant::now() + settings.connect_timeout;
    let mut unknown = 0;
    let mut known = false;
    let mut last_error = String::new();
    for round in 0..2 {
        let mut stale = false;
        for seed in &sentinel.seeds {
            if cancelled() {
                return Err("Stopped".into());
            }
            let budget = remaining(deadline)?;
            let seed_settings = Settings {
                host: seed.host.clone(),
                port: seed.port,
                username: sentinel.username.clone(),
                ssh: settings.ssh.clone(),
                tls: sentinel.tls,
                root_certificate: sentinel.root_certificate.clone(),
                client_certificate: sentinel.client_certificate.clone(),
                client_key: sentinel.client_key.clone(),
                connect_timeout: budget.min(Duration::from_millis(500)),
                read_timeout: budget.min(Duration::from_millis(500)),
                ..Default::default()
            };
            let mut connection = match Session::raw_node(&seed_settings, sentinel_password) {
                Ok(connection) => connection,
                Err(error) => {
                    last_error = error;
                    continue;
                }
            };
            set_timeout(
                &connection,
                remaining(deadline)?.min(Duration::from_millis(500)),
            )?;
            let value = match redis::cmd("SENTINEL")
                .arg("get-master-addr-by-name")
                .arg(&sentinel.service)
                .query::<redis::Value>(&mut connection)
            {
                Ok(value) => value,
                Err(error) => {
                    last_error = error.to_string();
                    continue;
                }
            };
            if value == redis::Value::Nil {
                unknown += 1;
                continue;
            }
            let (host, port) = match target(value) {
                Ok(target) => target,
                Err(error) => {
                    last_error = error;
                    continue;
                }
            };
            known = true;
            let budget = remaining(deadline)?;
            let target_settings = Settings {
                host,
                port,
                sentinel: None,
                cluster: false,
                connect_timeout: budget,
                read_timeout: budget,
                ..settings.clone()
            };
            if cancelled() {
                return Err("Stopped".into());
            }
            match open(&target_settings, deadline) {
                Ok(Some(primary)) => {
                    if cancelled() {
                        return Err("Stopped".into());
                    }
                    return Ok(primary);
                }
                Ok(None) => {
                    stale = true;
                    last_error = "Sentinel returned a node that is not currently a primary.".into();
                }
                Err(error) => last_error = error,
            }
        }
        if round == 0 && stale {
            std::thread::sleep(remaining(deadline)?.min(Duration::from_millis(100)));
        } else {
            break;
        }
    }
    if unknown > 0 && !known {
        Err("Contacted Sentinels do not know this service name.".into())
    } else if known {
        Err(format!("Sentinel primary connection failed: {last_error}"))
    } else {
        Err(format!("Sentinel discovery unavailable: {last_error}"))
    }
}
pub(crate) fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| "Sentinel discovery deadline exceeded; connection was not exposed.".into())
}
fn set_timeout(connection: &redis::Connection, timeout: Duration) -> Result<(), String> {
    connection
        .set_read_timeout(Some(timeout))
        .and_then(|_| connection.set_write_timeout(Some(timeout)))
        .map_err(|e| e.to_string())
}
fn target(value: redis::Value) -> Result<(String, u16), String> {
    let (host, port): (String, u16) =
        redis::from_redis_value(value).map_err(|_| "Invalid Sentinel primary address")?;
    if host.trim().is_empty()
        || host.len() > 1024
        || host
            .bytes()
            .any(|b| b.is_ascii_control() || b == b'/' || b == b' ')
        || port == 0
    {
        return Err("Invalid Sentinel primary address".into());
    }
    Ok((host, port))
}
pub(crate) fn is_primary(value: &redis::Value) -> bool {
    matches!(value, redis::Value::Array(parts) if parts.len() == 3 && parts.first().is_some_and(|role| matches!(role, redis::Value::BulkString(role) if role == b"master")) && matches!(parts[1], redis::Value::Int(n) if n >= 0) && matches!(parts[2], redis::Value::Array(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redis_settings::{Sentinel, SentinelSeed};
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
    };
    fn read(socket: &mut TcpStream, parser: &mut redis::Parser) -> Vec<Vec<u8>> {
        redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap()
    }
    fn authenticated_command(
        socket: &mut TcpStream,
        password: &[u8],
    ) -> (redis::Parser, Vec<Vec<u8>>) {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        let mut authenticated = false;
        loop {
            let command = read(socket, &mut parser);
            match command[0].as_slice() {
                b"AUTH" => {
                    assert_eq!(command.last().unwrap(), password);
                    authenticated = true;
                }
                b"CLIENT" => {
                    assert_eq!(command[1], b"SETINFO");
                }
                _ => {
                    assert!(authenticated);
                    return (parser, command);
                }
            }
            socket.write_all(b"+OK\r\n").unwrap();
        }
    }
    #[test]
    fn separate_auth_and_role_select_share_one_check_exchange_without_ping() {
        let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
        let seed_port = sentinel.local_addr().unwrap().port();
        let data = TcpListener::bind("127.0.0.1:0").unwrap();
        let data_port = data.local_addr().unwrap().port();
        let seed = std::thread::spawn(move || {
            let (mut socket, _) = sentinel.accept().unwrap();
            let (_, command) = authenticated_command(&mut socket, b"sentinel-secret");
            assert_eq!(
                command,
                vec![
                    b"SENTINEL".to_vec(),
                    b"get-master-addr-by-name".to_vec(),
                    b"service".to_vec()
                ]
            );
            write!(
                socket,
                "*2\r\n$9\r\n127.0.0.1\r\n${}\r\n{data_port}\r\n",
                data_port.to_string().len()
            )
            .unwrap();
        });
        let primary = std::thread::spawn(move || {
            let (mut socket, _) = data.accept().unwrap();
            let (mut parser, command) = authenticated_command(&mut socket, b"data-secret");
            assert_eq!(command[0], b"ROLE");
            assert_eq!(
                read(&mut socket, &mut parser),
                vec![b"SELECT".to_vec(), b"3".to_vec()]
            );
            // Both checks arrived before either response was sent.
            socket
                .write_all(b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n+OK\r\n")
                .unwrap();
            assert_eq!(
                read(&mut socket, &mut parser),
                vec![b"GET".to_vec(), b"probe".to_vec()]
            );
            socket.write_all(b"$2\r\nok\r\n").unwrap();
        });
        let settings = Settings {
            database: 3,
            tls: false,
            sentinel: Some(Sentinel {
                service: "service".into(),
                seeds: vec![SentinelSeed {
                    host: "127.0.0.1".into(),
                    port: seed_port,
                }],
                tls: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut session =
            Session::connect_with_sentinel(&settings, "data-secret", "sentinel-secret").unwrap();
        assert_eq!(
            session.execute("GET probe", true).unwrap(),
            vec![redis::Value::BulkString(b"ok".to_vec())]
        );
        seed.join().unwrap();
        primary.join().unwrap();
    }
    #[test]
    fn stale_primary_is_rechecked_but_never_exposed_for_data_commands() {
        use std::io::Read;
        let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
        let seed_port = sentinel.local_addr().unwrap().port();
        let replica = TcpListener::bind("127.0.0.1:0").unwrap();
        let replica_port = replica.local_addr().unwrap().port();
        let seed = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = sentinel.accept().unwrap();
                let (_, command) = authenticated_command(&mut socket, b"sentinel-secret");
                assert_eq!(command[0], b"SENTINEL");
                write!(
                    socket,
                    "*2\r\n$9\r\n127.0.0.1\r\n${}\r\n{replica_port}\r\n",
                    replica_port.to_string().len()
                )
                .unwrap();
            }
        });
        let data = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = replica.accept().unwrap();
                let (mut parser, command) = authenticated_command(&mut socket, b"data-secret");
                assert_eq!(command[0], b"ROLE");
                assert_eq!(read(&mut socket, &mut parser)[0], b"SELECT");
                socket.write_all(b"*5\r\n$5\r\nslave\r\n$9\r\n127.0.0.1\r\n:6379\r\n$9\r\nconnected\r\n:0\r\n+OK\r\n").unwrap();
                assert_eq!(
                    socket.read(&mut [0u8; 1]).unwrap(),
                    0,
                    "Replica received a data command"
                );
            }
        });
        let settings = Settings {
            tls: false,
            sentinel: Some(Sentinel {
                service: "service".into(),
                seeds: vec![SentinelSeed {
                    host: "127.0.0.1".into(),
                    port: seed_port,
                }],
                tls: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let error = Session::connect_with_sentinel(&settings, "data-secret", "sentinel-secret")
            .err()
            .unwrap();
        assert!(error.contains("not currently a primary"), "{error}");
        seed.join().unwrap();
        data.join().unwrap();
    }
    #[test]
    fn lost_write_reply_and_readonly_demotion_close_without_replaying() {
        use std::io::Read;
        for demoted in [false, true] {
            let data = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = data.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = data.accept().unwrap();
                let (mut parser, command) = authenticated_command(&mut socket, b"data-secret");
                assert_eq!(command[0], b"ROLE");
                assert_eq!(read(&mut socket, &mut parser)[0], b"SELECT");
                socket
                    .write_all(b"*3\r\n$6\r\nmaster\r\n:0\r\n*0\r\n+OK\r\n")
                    .unwrap();
                assert_eq!(
                    read(&mut socket, &mut parser),
                    vec![b"INCR".to_vec(), b"counter".to_vec()]
                );
                if demoted {
                    socket
                        .write_all(b"-READONLY You cannot write to a replica\r\n")
                        .unwrap();
                    let mut byte = [0];
                    assert_eq!(
                        socket.read(&mut byte).unwrap(),
                        0,
                        "demoted connection must close without another command"
                    );
                }
                // With no reply, the write might have committed. Do not replay it.
            });
            let settings = Settings {
                host: "127.0.0.1".into(),
                port,
                tls: false,
                ..Default::default()
            };
            let mut connection = Session::raw_node(&settings, "data-secret").unwrap();
            let _: Vec<redis::Value> = redis::pipe()
                .cmd("ROLE")
                .cmd("SELECT")
                .arg(0)
                .query(&mut connection)
                .unwrap();
            let mut session = Session::verified_primary(connection, &settings);
            let report = session.execute_report("INCR counter\nINCR later", false).unwrap();
            assert!(matches!(report.entries[0].status, crate::redis_command_results::Status::Failed));
            assert!(matches!(report.entries[1].status, crate::redis_command_results::Status::NotRun));
            let error = &report.entries[0].output;
            assert!(
                error.contains(if demoted {
                    "not replayed"
                } else {
                    "may have completed"
                }),
                "{error}"
            );
            assert!(!session.is_connected());
            assert!(
                session
                    .execute("INCR counter", false)
                    .unwrap_err()
                    .contains("connection closed")
            );
            server.join().unwrap();
        }
    }
    #[test]
    fn streaming_checks_role_and_batches_auth_role_and_operation() {
        use std::io::BufRead;
        for operation in ["MONITOR", "SUBSCRIBE", "PUBSUB"] {
            for valid in [true, false] {
                let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
                let data = TcpListener::bind("127.0.0.1:0").unwrap();
                let seed_port = sentinel.local_addr().unwrap().port();
                let data_port = data.local_addr().unwrap().port();
                let attempts = if valid { 1 } else { 2 };
                let seed = std::thread::spawn(move || {
                    for _ in 0..attempts {
                        let (mut socket, _) = sentinel.accept().unwrap();
                        let (_, command) = authenticated_command(&mut socket, b"discovery-secret");
                        assert_eq!(command[0], b"SENTINEL");
                        write!(
                            socket,
                            "*2\r\n$9\r\n127.0.0.1\r\n${}\r\n{data_port}\r\n",
                            data_port.to_string().len()
                        )
                        .unwrap();
                    }
                });
                let primary = std::thread::spawn(move || {
                    for _ in 0..attempts {
                        let (mut socket, _) = data.accept().unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut parser = redis::Parser::new();
                        assert_eq!(
                            read(&mut socket, &mut parser),
                            vec![b"AUTH".to_vec(), b"data-secret".to_vec()]
                        );
                        assert_eq!(read(&mut socket, &mut parser), vec![b"ROLE".to_vec()]);
                        assert_eq!(read(&mut socket, &mut parser)[0], operation.as_bytes());
                        // All three requests must arrive before any response.
                        socket.write_all(b"+OK\r\n").unwrap();
                        if valid {
                            socket.write_all(b"*3\r\n$6\r\nmaster\r\n:0\r\n*1\r\n*3\r\n$9\r\n127.0.0.1\r\n$4\r\n6380\r\n$1\r\n0\r\n+ACK\r\n").unwrap();
                        } else {
                            socket.write_all(b"*1\r\n$5\r\nslave\r\n+ACK\r\n").unwrap();
                        }
                    }
                });
                let settings = Settings {
                    tls: false,
                    sentinel: Some(Sentinel {
                        service: "service".into(),
                        tls: false,
                        seeds: vec![SentinelSeed {
                            host: "127.0.0.1".into(),
                            port: seed_port,
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                let control = crate::redis_stream::Control::default();
                let mut command = redis::cmd(operation);
                if operation == "SUBSCRIBE" {
                    command.arg("events");
                }
                if operation == "PUBSUB" {
                    command.arg("CHANNELS");
                }
                let result = control.start(&settings, "data-secret", "discovery-secret", &command);
                if valid {
                    let mut reader = result.unwrap();
                    let mut ack = String::new();
                    reader.read_line(&mut ack).unwrap();
                    assert_eq!(ack, "+ACK\r\n");
                } else {
                    assert!(result.err().unwrap().contains("not currently a primary"));
                }
                seed.join().unwrap();
                primary.join().unwrap();
            }
        }
    }
    #[test]
    fn malformed_discovery_and_nonprimary_roles_are_rejected() {
        assert!(target(redis::Value::Nil).is_err());
        assert!(
            target(redis::Value::Array(vec![
                redis::Value::BulkString(b"host/path".to_vec()),
                redis::Value::Int(6379)
            ]))
            .is_err()
        );
        assert!(!is_primary(&redis::Value::Array(vec![
            redis::Value::BulkString(b"master".to_vec())
        ])));
        assert!(!is_primary(&redis::Value::Array(vec![
            redis::Value::BulkString(b"slave".to_vec()),
            redis::Value::Int(0),
            redis::Value::Array(vec![])
        ])));
    }
    #[test]
    #[ignore = "disposable TLS fixture via scripts/test-redis-sentinel-tls.sh"]
    fn tls_independent_trust_scopes_are_enforced_for_sessions_and_streams() {
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_SENTINEL_TLS_QA").as_deref(),
            Ok("1")
        );
        let sentinel_ca =
            std::path::PathBuf::from(std::env::var_os("RUSTABLE_SENTINEL_TLS_CA").unwrap());
        let data_ca =
            std::path::PathBuf::from(std::env::var_os("RUSTABLE_SENTINEL_DATA_TLS_CA").unwrap());
        let certs = std::path::PathBuf::from(std::env::var_os("RUSTABLE_SENTINEL_TLS_CERTS").unwrap());
        let settings = Settings {
            client_certificate: Some(certs.join("data-client.crt")),
            client_key: Some(certs.join("data-client.key")),
            root_certificate: Some(data_ca.clone()),
            sentinel: Some(Sentinel {
                client_certificate: Some(certs.join("sentinel-client.crt")),
                client_key: Some(certs.join("sentinel-client.key")),
                service: "rustable-tls-qa".into(),
                root_certificate: Some(sentinel_ca.clone()),
                seeds: vec![SentinelSeed {
                    host: "127.0.0.1".into(),
                    port: 26420,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        for fault in 0..7 {
            let mut candidate = settings.clone();
            match fault {
                1 => candidate.root_certificate = Some(sentinel_ca.clone()),
                2 => candidate.sentinel.as_mut().unwrap().root_certificate = Some(data_ca.clone()),
                3 => candidate.sentinel.as_mut().unwrap().seeds[0].host = "localhost".into(),
                4 => candidate.root_certificate = None,
                5 => { candidate.client_certificate = None; candidate.client_key = None; }
                6 => { let discovery = candidate.sentinel.as_mut().unwrap(); discovery.client_certificate = None; discovery.client_key = None; }
                _ => {}
            }
            let session = connect(
                &candidate,
                "rustable-data-test-only",
                "rustable-sentinel-test-only",
            );
            assert_eq!(session.is_ok(), fault == 0, "session trust case {fault}");
            if let Ok(mut session) = session {
                assert_eq!(session.execute("PING", true).unwrap(), vec![redis::Value::SimpleString("PONG".into())]);
            }
            for command in [redis::cmd("MONITOR"), {
                let mut command = redis::cmd("SUBSCRIBE");
                command.arg("tls-proof");
                command
            }] {
                let control = crate::redis_stream::Control::default();
                let stream = control.start(
                    &candidate,
                    "rustable-data-test-only",
                    "rustable-sentinel-test-only",
                    &command,
                );
                assert_eq!(stream.is_ok(), fault == 0, "stream trust case {fault}");
                if let Ok(mut stream) = stream {
                    let reply = redis::Parser::new().parse_value(&mut stream).unwrap();
                    match command.args_iter().next().unwrap() {
                        redis::Arg::Simple(b"MONITOR") => assert_eq!(reply, redis::Value::Okay),
                        _ => assert_eq!(reply, redis::Value::Array(vec![
                            redis::Value::BulkString(b"subscribe".to_vec()),
                            redis::Value::BulkString(b"tls-proof".to_vec()),
                            redis::Value::Int(1),
                        ])),
                    }
                }
                control.stop();
            }
        }
    }
    #[test]
    #[ignore = "disposable Sentinel fixture via scripts/test-redis-sentinel.sh"]
    fn live_streams_start_on_the_discovered_primary() {
        use std::io::BufRead;
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_SENTINEL_QA").as_deref(),
            Ok("1")
        );
        let settings = Settings {
            tls: false,
            sentinel: Some(Sentinel {
                service: "rustable-qa".into(),
                tls: false,
                seeds: vec![SentinelSeed {
                    host: "127.0.0.1".into(),
                    port: 26410,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut session = connect(
            &settings,
            "rustable-data-test-only",
            "rustable-sentinel-test-only",
        )
        .unwrap();
        let monitor = crate::redis_stream::Control::default();
        let mut capture = monitor
            .start(
                &settings,
                "rustable-data-test-only",
                "rustable-sentinel-test-only",
                &redis::cmd("MONITOR"),
            )
            .unwrap();
        let mut ack = String::new();
        capture.read_line(&mut ack).unwrap();
        assert_eq!(ack, "+OK\r\n");
        let subscription = crate::redis_stream::Control::default();
        let mut command = redis::cmd("SUBSCRIBE");
        command.arg("rustable-sentinel-stream-proof");
        let mut messages = subscription
            .start(
                &settings,
                "rustable-data-test-only",
                "rustable-sentinel-test-only",
                &command,
            )
            .unwrap();
        let mut decoder = crate::redis_pubsub_frame::Decoder::default();
        assert!(
            matches!(decoder.next(&mut messages).unwrap(), redis::Value::Array(values) if values[0] == redis::Value::BulkString(b"subscribe".to_vec()))
        );
        session
            .execute("PUBLISH rustable-sentinel-stream-proof verified", false)
            .unwrap();
        assert!(
            matches!(decoder.next(&mut messages).unwrap(), redis::Value::Array(values) if values[0] == redis::Value::BulkString(b"message".to_vec()) && values[2] == redis::Value::BulkString(b"verified".to_vec()))
        );
        let mut seen = false;
        for _ in 0..32 {
            let mut line = String::new();
            capture.read_line(&mut line).unwrap();
            if line.contains("PUBLISH") && line.contains("rustable-sentinel-stream-proof") {
                seen = true;
                break;
            }
        }
        assert!(seen);
        monitor.stop();
        subscription.stop();
    }
    #[test]
    #[ignore = "disposable Sentinel fixture via scripts/test-redis-sentinel.sh"]
    fn live_probe_uses_the_independent_discovery_password() {
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_SENTINEL_QA").as_deref(),
            Ok("1")
        );
        let settings = Settings {
            tls: false,
            database: 3,
            sentinel: Some(Sentinel {
                service: "rustable-qa".into(),
                tls: false,
                seeds: vec![SentinelSeed {
                    host: "127.0.0.1".into(),
                    port: 26410,
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        for valid in [true, false] {
            let probe = crate::connection_probe::Probe::start_with_sentinel(
                crate::connection_profiles::ConnectionKind::Redis(Box::new(settings.clone())),
                "rustable-data-test-only".into(),
                if valid {
                    "rustable-sentinel-test-only"
                } else {
                    "wrong-discovery-password"
                }
                .into(),
                eframe::egui::Context::default(),
            );
            let deadline = Instant::now() + Duration::from_secs(6);
            let (mut succeeded, mut failed) = (false, false);
            loop {
                if let Some(event) = probe.poll() {
                    match event {
                        crate::connection_probe::Event::Success(fields) => {
                            succeeded |=
                                fields.contains(&crate::connection_probe::ProbeField::Database)
                        }
                        crate::connection_probe::Event::Failed(_, _) => failed = true,
                        crate::connection_probe::Event::Finished => break,
                        _ => {}
                    }
                }
                assert!(Instant::now() < deadline, "Probe did not finish");
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(succeeded, valid);
            assert_eq!(failed, !valid);
        }
    }
    #[test]
    #[ignore = "disposable Sentinel fixture via scripts/test-redis-sentinel.sh"]
    fn live_discovery_reauthenticates_and_selects_the_current_primary() {
        assert_eq!(
            std::env::var("RUSTABLE_REDIS_SENTINEL_QA").as_deref(),
            Ok("1")
        );
        let expected = std::env::var("RUSTABLE_SENTINEL_EXPECTED_PRIMARY").unwrap();
        assert!(["16410", "16411", "16412"].contains(&expected.as_str()));
        let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable_port = unavailable.local_addr().unwrap().port();
        let rejected_seed = std::thread::spawn(move || {
            for _ in 0..5 {
                drop(unavailable.accept().unwrap().0);
            }
        });
        let settings = Settings {
            database: 3,
            tls: false,
            sentinel: Some(Sentinel {
                service: "rustable-qa".into(),
                tls: false,
                seeds: vec![
                    SentinelSeed {
                        host: "127.0.0.1".into(),
                        port: unavailable_port,
                    },
                    SentinelSeed {
                        host: "127.0.0.1".into(),
                        port: 26410,
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        for _ in 0..2 {
            let mut session = Session::connect_with_sentinel(
                &settings,
                "rustable-data-test-only",
                "rustable-sentinel-test-only",
            )
            .unwrap();
            let info = session.execute("INFO server", true).unwrap();
            let redis::Value::BulkString(bytes) = &info[0] else {
                panic!("INFO response");
            };
            assert!(String::from_utf8_lossy(bytes).contains(&format!("tcp_port:{expected}\r\n")));
            session
                .execute("SET sentinel:client-proof value", false)
                .unwrap();
            assert_eq!(
                session.execute("GET sentinel:client-proof", true).unwrap(),
                vec![redis::Value::BulkString(b"value".to_vec())]
            );
        }
        let mut unknown = settings.clone();
        unknown.sentinel.as_mut().unwrap().service = "missing-service".into();
        assert!(
            Session::connect_with_sentinel(
                &unknown,
                "rustable-data-test-only",
                "rustable-sentinel-test-only"
            )
            .err()
            .unwrap()
            .contains("service name")
        );
        assert!(
            Session::connect_with_sentinel(
                &settings,
                "wrong-data-password",
                "rustable-sentinel-test-only"
            )
            .is_err()
        );
        assert!(
            Session::connect_with_sentinel(
                &settings,
                "rustable-data-test-only",
                "wrong-sentinel-password"
            )
            .is_err()
        );
        rejected_seed.join().unwrap();
    }
}
