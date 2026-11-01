//! Fault injection against disposable local MySQL/MariaDB servers only.
use super::*;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};

fn packet(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let length = usize::from(header[0]) | (usize::from(header[1]) << 8) | (usize::from(header[2]) << 16);
    if length > 1024 * 1024 {
        return Err(std::io::Error::other("fault fixture packet exceeds 1 MiB"));
    }
    let mut bytes = vec![0; length + 4];
    bytes[..4].copy_from_slice(&header);
    stream.read_exact(&mut bytes[4..])?;
    Ok(bytes)
}

#[test]
#[ignore = "requires disposable MySQL/MariaDB fixture"]
fn lost_mysql_commit_reply_reports_unknown_outcome_without_replay() {
    lost_transaction_reply(true);
}

#[test]
#[ignore = "requires disposable MySQL/MariaDB fixture"]
fn lost_mysql_rollback_reply_disconnects_without_claiming_cleanup_succeeded() {
    lost_transaction_reply(false);
}

fn lost_transaction_reply(commit: bool) {
    let table = if commit { "lost_commit_probe" } else { "lost_rollback_probe" };
    let command = if commit { b"COMMIT".as_slice() } else { b"ROLLBACK".as_slice() };
    let target_port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
    assert!([43316, 43317].contains(&target_port));
    let config = MysqlConfig { host:"127.0.0.1".into(), port:target_port, user:"rustable".into(), password:"rustable-test-only".into(), database:"rustable_test".into(), tls:false, read_timeout:Duration::from_secs(5), ..Default::default() };
    let mut observer = Conn::new(OptsBuilder::from(&config)).unwrap();
    observer.query_drop(format!("DROP TABLE IF EXISTS {table}")).unwrap();
    observer.query_drop(format!("CREATE TABLE {table}(id INT AUTO_INCREMENT PRIMARY KEY, label VARCHAR(20)) ENGINE=InnoDB")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let armed = Arc::new(AtomicBool::new(false));
    let (sent, received) = std::sync::mpsc::channel();
    let proxy = std::thread::spawn(move || {
        let (mut downstream, _) = listener.accept().unwrap();
        let mut upstream = TcpStream::connect(("127.0.0.1", target_port)).unwrap();
        upstream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        downstream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut input = downstream.try_clone().unwrap();
        let mut output = upstream.try_clone().unwrap();
        let request_armed = armed.clone();
        let requests = std::thread::spawn(move || {
            while let Ok(bytes) = packet(&mut input) {
                // COM_QUERY, followed by the exact command emitted by apply.
                if bytes.get(4) == Some(&3) && bytes.get(5..) == Some(command) {
                    request_armed.store(true, Ordering::Release);
                }
                if output.write_all(&bytes).is_err() { break; }
            }
            let _ = output.shutdown(Shutdown::Both);
        });
        while let Ok(bytes) = packet(&mut upstream) {
            if armed.load(Ordering::Acquire) {
                // Consume the real server reply, but never let the client see it.
                let _ = sent.send(bytes);
                break;
            }
            if downstream.write_all(&bytes).is_err() { break; }
        }
        let _ = downstream.shutdown(Shutdown::Both);
        let _ = upstream.shutdown(Shutdown::Both);
        requests.join().unwrap();
    });
    let mut proxy_config = config.clone(); proxy_config.port = port;
    let mut driver = MysqlDriver::connect(&proxy_config).unwrap();
    // Non-strict truncation forces apply to roll back after the insert executes.
    if !commit { driver.conn().unwrap().query_drop("SET sql_mode=''").unwrap(); }
    let label = if commit { "saved once" } else { "this value is longer than twenty characters" };
    let plan = driver.plan(&[Change::InsertRow { id:1, table:table.into(), fields:vec![("label".into(),Value::Text(label.into()))] }]).unwrap();
    let error = driver.apply(&plan).unwrap_err();
    assert!(error.contains(if commit { "Commit outcome is unknown" } else { "Rollback could not be confirmed" }), "{error}");
    assert_eq!(driver.transaction_state(), TransactionState::Disconnected);
    let reply = received.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(reply.get(4), Some(&0), "server actually acknowledged the transaction command with an OK packet");
    proxy.join().unwrap();
    let rows: Vec<String> = observer.query(format!("SELECT label FROM {table}")).unwrap();
    assert_eq!(rows, if commit { vec!["saved once"] } else { vec![] });
    assert!(driver.apply(&plan).unwrap_err().contains("disconnected"), "disconnected session cannot replay the batch");
    let count: Option<u64> = observer.query_first(format!("SELECT COUNT(*) FROM {table}")).unwrap();
    assert_eq!(count,Some(if commit { 1 } else { 0 }));
    observer.query_drop(format!("DROP TABLE {table}")).unwrap();
}
