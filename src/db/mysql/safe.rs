//! Read-only editor batches for MySQL and MariaDB. Syntax checks exclude
//! implicit-commit operations; the server enforces read-only table access.
use crate::sql::mysql::{self, Mode};
use sqlparser::{ast::Statement, parser::Parser, tokenizer::{Token, Tokenizer}};

/// Returns true for explicit transaction recovery (sent without our wrapper).
pub(super) fn validate(sql: &str, mode: Mode) -> Result<bool, String> {
    let refusal = "Safe mode only allows read queries and explicit COMMIT/ROLLBACK. Disable Safe mode to run this statement.";
    if !mysql::executable_comments(sql, mode)?.is_empty() {
        return Err("Executable MySQL/MariaDB comments are not allowed in Safe mode. Write the read query directly instead.".into());
    }
    let dialect = mysql::Lexer(mode);
    let statements = Parser::parse_sql(&dialect, sql)
        .map_err(|_| "Could not verify this statement as a read query in Safe mode.".to_owned())?;
    let [statement] = statements.as_slice() else { return Err(refusal.into()); };
    if matches!(statement, Statement::Commit { chain: false, modifier: None, .. }
        | Statement::Rollback { chain: false, savepoint: None }) {
        return Ok(true);
    }
    fn read(statement: &Statement) -> bool {
        match statement {
            Statement::Query(_) => true,
            Statement::Explain { statement, .. } => read(statement),
            // SHOW has many AST variants; only parsed SHOW statements qualify.
            other => other.to_string().starts_with("SHOW "),
        }
    }
    if !read(statement) { return Err(refusal.into()); }
    // SELECT INTO can write files or session variables. Also refuse nested DML
    // (including data-modifying CTEs) even if a future server supports it.
    let tokens = Tokenizer::new(&dialect, sql).tokenize().map_err(|error| error.to_string())?;
    if tokens.iter().any(|token| matches!(token, Token::Word(word)
        if word.quote_style.is_none() && matches!(word.value.to_ascii_uppercase().as_str(),
            "INTO" | "INSERT" | "UPDATE" | "DELETE" | "REPLACE"))) {
        return Err(refusal.into());
    }
    Ok(false)
}

pub(super) fn batch(sql: &str, mode: Mode) -> String {
    // A custom DELIMITER may leave a semicolon before trailing comments.
    let terminated = Tokenizer::new(&mysql::Lexer(mode), sql).tokenize()
        .is_ok_and(|tokens| tokens.iter().rev().find(|t| !matches!(t, Token::Whitespace(_)))
            == Some(&Token::SemiColon));
    format!("START TRANSACTION READ ONLY;\n{sql}\n{}ROLLBACK AND NO CHAIN NO RELEASE",
        if terminated { "" } else { "; " })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fail_closed_without_mistaking_strings_for_commands() {
        for mode in [Mode::default(), Mode { ansi_quotes: true, no_backslash_escapes: true }] {
            for sql in ["SELECT 1", "SELECT 'DROP TABLE t'", "SELECT '/*M! DROP TABLE t */'",
                "WITH t AS (SELECT 1) SELECT * FROM t", "SHOW TABLES", "SHOW CREATE TABLE t", "EXPLAIN SELECT 1",
                "SELECT 1 -- comment", "SELECT 1 # /*M! DROP TABLE t */"] {
                assert_eq!(validate(sql, mode), Ok(false), "{sql}");
            }
            for sql in ["DELETE FROM t WHERE id=1", "CREATE TEMPORARY TABLE t(i INT)",
                "SET autocommit=1", "CALL f()", "START TRANSACTION", "SELECT 1; DROP TABLE t",
                "SELECT 1 INTO OUTFILE '/tmp/safe-test'", "SELECT 1 INTO @x",
                "/*! SELECT 1 */", "/*M! DROP TABLE t */", "SELECT 1 /*M! INTO OUTFILE '/tmp/x' */",
                "EXPLAIN ANALYZE DELETE FROM t", "COMMIT AND CHAIN"] {
                assert!(validate(sql, mode).is_err(), "{sql}");
            }
            for sql in ["COMMIT", "ROLLBACK"] { assert_eq!(validate(sql, mode), Ok(true)); }
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::super::*;
    fn config() -> MysqlConfig {
        let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        MysqlConfig { host: "127.0.0.1".into(), port, user: "rustable".into(),
            password: "rustable-test-only".into(), database: "rustable_test".into(),
            tls: false, ..Default::default() }
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn safe_mode_preserves_data_transactions_and_reuses_connection() {
        let config = config();
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let conn = driver.conn().unwrap();
        let version: String = conn.query_first("SELECT VERSION()").unwrap().unwrap();
        eprintln!("Safe mode fixture: {version}");
        conn.query_drop("DROP TABLE IF EXISTS safe_mode_probe").unwrap();
        conn.query_drop("CREATE TABLE safe_mode_probe(id INT PRIMARY KEY, n INT) ENGINE=InnoDB").unwrap();
        conn.query_drop("INSERT INTO safe_mode_probe VALUES(1,10)").unwrap();
        conn.query_drop("DROP FUNCTION IF EXISTS safe_mode_write").unwrap();
        conn.query_drop("CREATE FUNCTION safe_mode_write() RETURNS INT DETERMINISTIC MODIFIES SQL DATA BEGIN UPDATE safe_mode_probe SET n=20; RETURN 1; END").unwrap();
        for sql in ["SELECT * FROM safe_mode_probe", "SELECT 1 WHERE FALSE", "SHOW TABLES",
            "SHOW CREATE TABLE safe_mode_probe", "EXPLAIN SELECT * FROM safe_mode_probe",
            "SELECT 1 -- trailing comment", "DELIMITER $$\nSELECT 1; -- comment\n$$", "SELECT '/*M! DROP TABLE safe_mode_probe */'"] {
            let result = driver.execute(sql, true, &cancel);
            assert!(result.error.is_none(), "{sql}: {:?}", result.error);
            assert_eq!(result.grids.len(), 1, "{sql}: transaction wrappers must stay hidden");
            assert_eq!(driver.connection.as_ref().unwrap().transaction_status(), Some(false));
        }
        for sql in ["DELETE FROM safe_mode_probe", "UPDATE safe_mode_probe SET n=30 WHERE id=1",
            "DROP TABLE safe_mode_probe", "/*M! DROP TABLE safe_mode_probe */",
            "/*! DROP TABLE safe_mode_probe */", "SELECT 1 INTO OUTFILE '/tmp/rustable-safe-mode'",
            "SELECT safe_mode_write()", "SELECT * FROM missing_safe_mode_table"] {
            let result = driver.execute(sql, true, &cancel);
            assert!(result.error.is_some(), "{sql}");
            if sql == "SELECT safe_mode_write()" {
                assert!(result.error.as_ref().unwrap().contains("1792"), "server must enforce READ ONLY: {:?}", result.error);
            }
            assert_eq!(driver.connection.as_ref().unwrap().transaction_status(), Some(false), "{sql}");
            assert!(driver.execute("SELECT 1", true, &cancel).error.is_none(), "{sql}");
        }
        assert_eq!(driver.conn().unwrap().query_first::<i32,_>("SELECT n FROM safe_mode_probe").unwrap(), Some(10));
        // Safe mode must not implicitly commit a pre-existing write transaction.
        driver.conn().unwrap().query_drop("START TRANSACTION; UPDATE safe_mode_probe SET n=99").unwrap();
        assert!(driver.execute("SELECT 1", true, &cancel).error.unwrap().contains("existing transaction"));
        assert_eq!(driver.connection.as_ref().unwrap().transaction_status(), Some(true));
        assert!(driver.execute("ROLLBACK", true, &cancel).error.is_none());
        assert_eq!(driver.conn().unwrap().query_first::<i32,_>("SELECT n FROM safe_mode_probe").unwrap(), Some(10));
        for settings in ["SET autocommit=0", "SET completion_type=1", "SET completion_type=2",
            "SET sql_mode='ANSI_QUOTES,NO_BACKSLASH_ESCAPES'"] {
            assert!(driver.execute(settings, false, &cancel).error.is_none());
            let result = driver.execute("SELECT 42 AS value", true, &cancel);
            assert!(result.error.is_none(), "{settings}: {:?}", result.error);
            assert_eq!(driver.connection.as_ref().unwrap().transaction_status(), Some(false));
        }
        // Safe mode can be disabled without leaving a sticky read-only session.
        assert!(driver.execute("SET completion_type=0; SET autocommit=1; UPDATE safe_mode_probe SET n=11 WHERE id=1", false, &cancel).error.is_none());
        let report = driver.execute(&"SELECT 1;".repeat(128), true, &cancel);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids.len(), 128);
        driver.conn().unwrap().query_drop("DROP FUNCTION safe_mode_write; DROP TABLE safe_mode_probe").unwrap();
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn safe_mode_cancellation_cleans_up_owned_transaction() {
        let mut driver = MysqlDriver::connect(&config()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let task = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            signal.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = driver.execute("SELECT SLEEP(10), 1", true, &cancel);
        task.join().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(result.error.is_some(), "{result:?}");
        assert_eq!(driver.connection.as_ref().unwrap().transaction_status(), Some(false));
        cancel.store(false, Ordering::Relaxed);
        assert!(driver.execute("SELECT 42", true, &cancel).error.is_none());
    }
}

#[cfg(test)]
mod latency_tests {
    use super::super::*;
    use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, sync::atomic::AtomicUsize, time::Instant};

    // Delay arrival in both directions, allowing bytes already in flight to
    // overlap. Counting protocol commands catches hidden setup/cleanup/pings.
    fn forward(mut source: TcpStream, mut target: TcpStream, commands: Option<Arc<AtomicUsize>>) {
        let shutdown = source.try_clone().unwrap();
        let (send, receive) = std::sync::mpsc::sync_channel(64);
        let reader = std::thread::spawn(move || {
            let mut bytes = [0u8; 65536];
            let mut packet = Vec::new();
            while let Ok(length) = source.read(&mut bytes) {
                if length == 0 { break; }
                if let Some(commands) = &commands {
                    packet.extend_from_slice(&bytes[..length]);
                    while packet.len() >= 4 {
                        let length = usize::from(packet[0]) | usize::from(packet[1]) << 8 | usize::from(packet[2]) << 16;
                        if packet.len() < length + 4 { break; }
                        if packet[3] == 0 { commands.fetch_add(1, Ordering::Relaxed); }
                        packet.drain(..length + 4);
                    }
                }
                if send.send((Instant::now() + Duration::from_millis(15), bytes[..length].to_vec())).is_err() { break; }
            }
        });
        while let Ok((deadline, bytes)) = receive.recv() {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            if target.write_all(&bytes).is_err() { break; }
        }
        drop(receive);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
        let _ = target.shutdown(std::net::Shutdown::Both);
        reader.join().unwrap();
    }

    #[test]
    #[ignore = "manual disposable MySQL/MariaDB WAN latency benchmark"]
    fn safe_mode_one_exchange_cold_and_warm() {
        let fixture: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&fixture));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let count = Arc::new(AtomicUsize::new(0));
        let commands = count.clone();
        let proxy = std::thread::spawn(move || {
            let (downstream, _) = listener.accept().unwrap();
            let upstream = TcpStream::connect(("127.0.0.1", fixture)).unwrap();
            downstream.set_nodelay(true).unwrap();
            upstream.set_nodelay(true).unwrap();
            let reverse_source = upstream.try_clone().unwrap();
            let reverse_target = downstream.try_clone().unwrap();
            let reverse = std::thread::spawn(move || forward(reverse_source, reverse_target, None));
            forward(downstream, upstream, Some(commands));
            reverse.join().unwrap();
        });
        let config = MysqlConfig { host: "127.0.0.1".into(), port, user: "rustable".into(),
            password: "rustable-test-only".into(), database: "rustable_test".into(),
            tls: false, ..Default::default() };
        let mut driver = MysqlDriver::connect(&config).unwrap();
        let cancel = AtomicBool::new(false);
        let mut samples = [Vec::new(), Vec::new()];
        for iteration in 0..12 {
            for (index, safe) in [false, true].into_iter().enumerate() {
                let before = count.load(Ordering::Relaxed);
                let start = Instant::now();
                let result = driver.execute("SELECT 42", safe, &cancel);
                assert!(result.error.is_none(), "{:?}", result.error);
                assert_eq!(driver.transaction_state(), TransactionState::Idle);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(count.load(Ordering::Relaxed) - before, 1, "safe={safe}, iteration={iteration}");
                assert_eq!(result.grids.len(), 1);
                assert_eq!(result.grids[0].rows, [vec![Value::Integer(42)]]);
                samples[index].push(elapsed);
            }
        }
        let before = count.load(Ordering::Relaxed);
        let failed_started = Instant::now();
        assert!(driver.execute("SELECT * FROM missing_safe_mode_table", true, &cancel).error.is_some());
        eprintln!("port={fixture} failed safe query including rollback={:.2}ms", failed_started.elapsed().as_secs_f64()*1000.0);
        assert_eq!(driver.transaction_state(), TransactionState::Idle);
        assert_eq!(count.load(Ordering::Relaxed) - before, 2, "only failed batches need a cleanup exchange");
        let before = count.load(Ordering::Relaxed);
        assert!(driver.execute("DROP TABLE safe_mode_probe", true, &cancel).error.is_some());
        assert_eq!(count.load(Ordering::Relaxed) - before, 0);
        for (name, values) in ["normal", "safe"].into_iter().zip(&samples) {
            let mut sorted = values.clone(); sorted.sort_by(f64::total_cmp);
            eprintln!("port={fixture} {name}: first={:.2}ms median={:.2}ms max={:.2}ms, 30ms simulated RTT, n={}", values[0], sorted[sorted.len()/2], sorted.last().unwrap(), values.len());
        }
        assert!(samples[1].iter().sum::<f64>() / 12.0 < samples[0].iter().sum::<f64>() / 12.0 + 15.0,
            "safe mode must not add another network RTT");
        drop(driver);
        proxy.join().unwrap();
    }
}
