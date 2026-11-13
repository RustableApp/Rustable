//! Local protocol fault injection and disposable QA server checks.
use super::*;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};

#[test]
fn stalled_cancel_tls_negotiation_does_not_block_watch_cleanup() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent, received) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut session, _) = listener.accept().unwrap();
        session.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut length = [0; 4];
        session.read_exact(&mut length).unwrap();
        let length = u32::from_be_bytes(length) as usize;
        assert!((8..4096).contains(&length));
        let mut startup = vec![0; length - 4];
        session.read_exact(&mut startup).unwrap();
        // AuthenticationOk, BackendKeyData, ReadyForQuery. No database needed.
        session.write_all(b"R\0\0\0\x08\0\0\0\0K\0\0\0\x0c\0\0\0\x01\0\0\0\x02Z\0\0\0\x05I").unwrap();
        let (mut cancellation, _) = listener.accept().unwrap();
        cancellation.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut ssl_request = [0; 8];
        cancellation.read_exact(&mut ssl_request).unwrap();
        assert_eq!(ssl_request, [0, 0, 0, 8, 4, 210, 22, 47]);
        sent.send(()).unwrap();
        // Deliberately never answer SSLRequest. Timeout must drop this socket.
        assert_eq!(cancellation.read(&mut [0]).unwrap(), 0);
    });
    let mut config = ::postgres::Config::new();
    config.host("127.0.0.1").port(port).user("fixture").connect_timeout(Duration::from_secs(5));
    let client = config.connect(NoTls).unwrap();
    let watch = CancelWatch::start(
        client.cancel_token(),
        Arc::new(AtomicBool::new(true)),
        Some(MakeTlsConnector::new(native_tls::TlsConnector::new().unwrap())),
    );
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    drop(watch);
    assert!(started.elapsed() < Duration::from_secs(4));
    server.join().unwrap();
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
fn lost_commit_acknowledgement_reports_uncertainty_without_replay() {
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let armed = Arc::new(AtomicBool::new(false));
    let cut = Arc::new(AtomicBool::new(false));
    let proxy_armed = armed.clone();
    let proxy_cut = cut.clone();
    let proxy = std::thread::spawn(move || {
        let (mut downstream, _) = listener.accept().unwrap();
        let mut upstream = TcpStream::connect("127.0.0.1:54329").unwrap();
        upstream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut input = downstream.try_clone().unwrap();
        let mut output = upstream.try_clone().unwrap();
        let forward = std::thread::spawn(move || {
            let _ = std::io::copy(&mut input, &mut output);
            let _ = output.shutdown(Shutdown::Both);
        });
        // Backend messages are tagged length-prefixed frames, including startup
        // authentication. With TLS disabled no encrypted handshake is involved.
        loop {
            let mut header = [0u8; 5];
            if upstream.read_exact(&mut header).is_err() {
                break;
            }
            let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
            assert!((4..=16 * 1024 * 1024).contains(&length));
            let mut body = vec![0; length - 4];
            upstream.read_exact(&mut body).unwrap();
            if proxy_armed.load(Ordering::Acquire) && header[0] == b'C' && body == b"COMMIT\0" {
                // The server has committed, but the client never receives its
                // CommandComplete or ReadyForQuery. This is not a server error.
                proxy_cut.store(true, Ordering::Release);
                break;
            }
            if downstream
                .write_all(&header)
                .and_then(|()| downstream.write_all(&body))
                .is_err()
            {
                break;
            }
        }
        let _ = downstream.shutdown(Shutdown::Both);
        let _ = upstream.shutdown(Shutdown::Both);
        forward.join().unwrap();
    });
    let config = crate::connection::PostgresConfig {
        host: "127.0.0.1".into(),
        port,
        database: "rustable_qa".into(),
        password: "rustable_qa_only".into(),
        tls: crate::connection::PostgresTlsMode::Disable,
        ..Default::default()
    };
    let mut observer = Client::connect(
        "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only",
        NoTls,
    )
    .unwrap();
    let table = format!("commit_ack_qa_{}", std::process::id());
    observer
        .batch_execute(&format!("CREATE TABLE {table}(value integer)"))
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
    let begin = driver.execute(
        &format!("BEGIN; INSERT INTO {table} VALUES(42)"),
        false,
        &cancel,
    );
    assert!(begin.error.is_none(), "{:?}", begin.error);
    assert_eq!(driver.transaction_state(), TransactionState::Active);
    let count = |client: &mut Client| {
        client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .unwrap()
            .get::<_, i64>(0)
    };
    assert_eq!(count(&mut observer), 0);
    armed.store(true, Ordering::Release);
    let report = driver.execute(
        &format!("COMMIT; INSERT INTO {table} VALUES(43)"),
        false,
        &cancel,
    );
    proxy.join().unwrap();
    assert!(cut.load(Ordering::Acquire));
    // Check the server outcome independently before inspecting the UI report.
    let committed_rows = count(&mut observer);
    observer
        .batch_execute(&format!("DROP TABLE {table}"))
        .unwrap();
    assert_eq!(
        committed_rows, 1,
        "committed once, never automatically replayed"
    );
    assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
    assert!(report.grids.is_empty());
    let error = report.error.unwrap();
    assert!(error.contains("Statement 1"), "{error}");
    assert!(error.contains("outcome is unknown"), "{error}");
    assert!(error.contains("Verify"), "{error}");
    assert!(
        !error.contains("may need ROLLBACK"),
        "dead session cannot roll back"
    );
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
fn next_operation_reconnects_a_lost_session_without_replaying_work() {
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let config = crate::connection::PostgresConfig {
        host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
        user: "postgres".into(), password: "rustable_qa_only".into(), tls: crate::connection::PostgresTlsMode::Disable,
        ssh: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let mut observer = PostgresDriver::open(&config, cancel.clone()).unwrap();
    let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
    driver.objects().unwrap();
    let selected = driver.schemas.selected.clone();
    let original_pid: i32 = driver.client.query_one("SELECT pg_backend_pid()", &[]).unwrap().get(0);
    // Healthy sessions, including explicit transactions, must stay intact.
    driver.client.batch_execute("BEGIN; CREATE TEMP TABLE reconnect_session(value int)").unwrap();
    driver.ensure_connected().unwrap();
    assert_eq!(driver.client.query_one("SELECT pg_backend_pid()", &[]).unwrap().get::<_, i32>(0), original_pid);
    driver.client.batch_execute("ROLLBACK").unwrap();
    observer.client.query_one("SELECT pg_terminate_backend($1)", &[&original_pid]).unwrap();
    // The first failed operation reports its error. Nothing automatically retries.
    assert!(driver.execute("SELECT 1", true, &cancel).error.is_some());
    assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
    let mut connection: Option<Box<dyn Driver>> = Some(Box::new(driver));
    let result = super::super::handle(&mut connection, Command::Query { sql: "SELECT 42".into(), safe: true }, &cancel).unwrap();
    let Payload::Query { report, schemas, .. } = result else { panic!("expected query result") };
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(schemas.selected, selected);
    assert_eq!(connection.as_mut().unwrap().transaction_state(), TransactionState::Idle);
    // A lost session also recovers when the next action is a catalog reload.
    let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
    driver.objects().unwrap();
    let pid: i32 = driver.client.query_one("SELECT pg_backend_pid()", &[]).unwrap().get(0);
    observer.client.query_one("SELECT pg_terminate_backend($1)", &[&pid]).unwrap();
    assert!(driver.objects().is_err());
    assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
    connection = Some(Box::new(driver));
    assert!(super::super::handle(&mut connection, Command::RefreshSchema, &cancel).is_ok());
    assert_eq!(connection.as_mut().unwrap().transaction_state(), TransactionState::Idle);
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL QA server on port 54329"]
fn typed_query_closes_statement_in_execution_exchange() {
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let mut client = Client::connect(
        "host=127.0.0.1 port=54329 dbname=rustable_qa user=postgres password=rustable_qa_only", NoTls,
    ).unwrap();
    for _ in 0..3 {
        let rows = client.query_typed("SELECT 42::int", &[]).unwrap();
        assert_eq!(rows[0].get::<_, i32>(0), 42);
        assert_eq!(client.transaction_status(), Some(b'I'), "no queued cleanup Sync");
    }
    assert!(client.query_typed("SELECT 1 / 0", &[]).is_err());
    client.check_connection().unwrap();
    let rows = client.query_typed("SELECT count(*) FROM pg_prepared_statements WHERE name LIKE 'rt%'", &[]).unwrap();
    // This query closes its own statement before execution too.
    assert_eq!(rows[0].get::<_, i64>(0), 0);
    assert_eq!(client.transaction_status(), Some(b'I'));
}

#[test]
#[ignore = "disposable issue-8 PostgreSQL TLS fixture with a long-lived, untrusted, wrong-host certificate"]
fn explicit_tls_modes_accept_encrypt_only_and_reject_untrusted_identity() {
    use crate::connection::PostgresTlsMode;
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_TLS_QA").as_deref(), Ok("1"));
    let mut config = crate::connection::PostgresConfig {
        host: "127.0.0.1".into(), port: 54329, database: "rustable_qa".into(),
        user: "postgres".into(), password: "rustable_qa_only".into(),
        tls: PostgresTlsMode::Require, ssh: None,
    };
    for mode in [PostgresTlsMode::Require, PostgresTlsMode::Prefer] {
        config.tls = mode;
        let mut client = super::client_config(&config, None)
            .connect(postgres_native_tls::MakeTlsConnector::new(mode.connector().unwrap())).unwrap();
        assert!(client.query_one("SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()", &[]).unwrap().get::<_, bool>(0));
        client.simple_query("SELECT 42").unwrap();
        let mut driver = PostgresDriver::open(&config, Arc::new(AtomicBool::new(false))).unwrap();
        assert!(driver.client.query_one("SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()", &[]).unwrap().get::<_, bool>(0));
    }
    for mode in [PostgresTlsMode::VerifyCa, PostgresTlsMode::VerifyFull] {
        config.tls = mode;
        assert!(super::client_config(&config, None)
            .connect(postgres_native_tls::MakeTlsConnector::new(mode.connector().unwrap())).is_err());
    }
    config.tls = PostgresTlsMode::Disable;
    let mut client = super::client_config(&config, None).connect(postgres::NoTls).unwrap();
    assert!(!client.query_one("SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()", &[]).unwrap().get::<_, bool>(0));
}

#[test]
fn required_tls_modes_never_send_startup_after_server_refuses_encryption() {
    use crate::connection::PostgresTlsMode;
    for mode in [PostgresTlsMode::Require, PostgresTlsMode::VerifyCa, PostgresTlsMode::VerifyFull] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = [0; 8];
            socket.read_exact(&mut request).unwrap();
            assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47]);
            socket.write_all(b"N").unwrap();
            assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0, "must close without sending credentials");
        });
        let config = crate::connection::PostgresConfig { host: "127.0.0.1".into(), port, tls: mode, ..Default::default() };
        assert!(client_config(&config, None).connect(MakeTlsConnector::new(mode.connector().unwrap())).is_err());
        server.join().unwrap();
    }
}
