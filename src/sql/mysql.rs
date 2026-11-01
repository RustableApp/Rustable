//! MySQL/MariaDB client-side statement boundaries. DELIMITER is a client directive,
//! never part of SQL sent to the server. The session must supply its SQL mode.
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Mode {
    pub no_backslash_escapes: bool,
    pub ansi_quotes: bool,
}

pub fn ranges(sql: &str, mode: Mode, strict: bool) -> Result<Vec<Range<usize>>, String> {
    scan(sql, mode, strict, false).map(|(ranges, _)| ranges)
}

pub fn editor_ranges(sql: &str, mode: Mode) -> Vec<Range<usize>> {
    scan(sql, mode, false, true).map(|(ranges, _)| ranges).unwrap_or_default()
}

/// Preserve the client delimiter active at the beginning of an editor slice.
/// Without this context, running a routine alone would split its body at `;`.
pub fn execution_text(sql: &str, range: Range<usize>, mode: Mode) -> Result<String, String> {
    let prefix = sql.get(..range.start).ok_or("Invalid SQL selection")?;
    let selected = sql.get(range).ok_or("Invalid SQL selection")?;
    let (_, delimiter) = scan(prefix, mode, false, false)?;
    if delimiter == ";" {
        Ok(selected.trim().to_owned())
    } else {
        Ok(format!("DELIMITER {delimiter}\n{}", selected.trim()))
    }
}

fn scan(sql: &str, mode: Mode, strict: bool, include_delimiter: bool) -> Result<(Vec<Range<usize>>, &str), String> {
    scan_from(sql, mode, strict, include_delimiter, ";", false).map(|(ranges, delimiter, _)| (ranges, delimiter)).map_err(|error| error.message)
}

/// Scan only the next statement so a preceding SET can change lexical modes.
pub fn next_statement(sql: &str, mode: Mode, delimiter: &str) -> Result<(Option<Range<usize>>, usize, String), String> {
    scan_from(sql, mode, true, false, delimiter, true)
        .map(|(ranges, delimiter, consumed)| (ranges.into_iter().next(), consumed, delimiter.to_owned())).map_err(|error| error.message)
}

#[derive(Debug)]
pub struct LexicalError {
    pub range: Range<usize>,
    pub message: String,
}

/// Bounded editor-only lexical check, using the same scanner as execution.
/// Offsets are bytes. This deliberately does not claim full SQL validation.
pub fn lexical_error(sql: &str, mode: Mode) -> Option<LexicalError> {
    if sql.len() > 64 * 1024 { return None; }
    scan_from(sql, mode, true, false, ";", false).err()
}

fn scan_from<'a>(sql: &'a str, mode: Mode, strict: bool, include_delimiter: bool, initial: &'a str, one: bool) -> Result<(Vec<Range<usize>>, &'a str, usize), LexicalError> {
    let bytes = sql.as_bytes();
    let (mut i, mut start, mut code) = (0, 0, false);
    let mut delimiter = initial;
    let mut result = Vec::new();
    while i < bytes.len() {
        // Directives are recognized only at the beginning of a logical line,
        // outside statements/quotes/comments. Keep custom delimiters byte-exact.
        if !code && (i == 0 || bytes[i - 1] == b'\n') {
            let line_end = sql[i..].find('\n').map_or(sql.len(), |n| i + n);
            let line = sql[i..line_end].trim();
            if line.get(..9).is_some_and(|s| s.eq_ignore_ascii_case("delimiter"))
                && line.as_bytes().get(9).is_none_or(u8::is_ascii_whitespace) {
                let value = line[9..].trim();
                if value.is_empty() || value.bytes().any(|b| b.is_ascii_whitespace() || b == 0) {
                    return Err(LexicalError { range: i..line_end, message: "DELIMITER needs one nonempty delimiter without whitespace.".into() });
                }
                delimiter = value;
                i = (line_end + 1).min(bytes.len());
                start = i;
                continue;
            }
        }
        if bytes[i..].starts_with(delimiter.as_bytes()) {
            if code { result.push(start..i + if include_delimiter { delimiter.len() } else { 0 }); }
            i += delimiter.len();
            if one && code { return Ok((result, delimiter, i)); }
            start = i;
            code = false;
            continue;
        }
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                code = true;
                let quote_start = i;
                let quote = bytes[i];
                let escapes = !mode.no_backslash_escapes && quote != b'`'
                    && !(quote == b'"' && mode.ansi_quotes);
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if escapes && bytes[i] == b'\\' { i = (i + 2).min(bytes.len()); continue; }
                    if bytes[i] == quote {
                        i += 1;
                        if bytes.get(i) == Some(&quote) { i += 1; continue; }
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed && strict { return Err(LexicalError { range: quote_start..sql.len(), message: "Unterminated MySQL quote".into() }); }
            }
            b'#' => { while i < bytes.len() && bytes[i] != b'\n' { i += 1; } }
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && bytes.get(i + 2).is_some_and(|b| b.is_ascii_whitespace() || b.is_ascii_control()) => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // Executable comments must survive even with no surrounding SQL.
                // MariaDB also recognizes /*M! ... */.
                code |= bytes.get(i + 2) == Some(&b'!') || bytes.get(i + 2..i + 4) == Some(b"M!");
                if let Some(end) = sql[i + 2..].find("*/") { i += end + 4; }
                else if strict { return Err(LexicalError { range: i..sql.len(), message: "Unterminated MySQL comment".into() }); }
                else { i = bytes.len(); }
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => { code = true; i += 1; }
        }
    }
    if code { result.push(start..sql.len()); }
    Ok((result, delimiter, sql.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parts(sql: &str, mode: Mode) -> Vec<&str> {
        ranges(sql, mode, true).unwrap().into_iter().map(|r| sql[r].trim()).collect()
    }
    #[test]
    fn lexical_diagnostics_point_to_original_source_bytes() {
        let mode = Mode::default();
        for (sql, start, message) in [
            ("SELECT '日本'; /* unfinished", "SELECT '日本'; ".len(), "comment"),
            ("SELECT `unfinished", 7, "quote"),
            ("  DELIMITER  ", 0, "DELIMITER"),
        ] {
            let error = lexical_error(sql, mode).unwrap();
            assert_eq!(error.range, start..sql.len());
            assert!(error.message.contains(message));
            assert!(sql.get(error.range).is_some());
        }
        assert!(lexical_error("SELECT 4--2; # comment\nSELECT 3", mode).is_none());
        assert!(lexical_error("/*M! SELECT 1 */", mode).is_none());
        assert!(lexical_error("SELECT \"name\\\"", crate::sql::mysql::Mode { ansi_quotes: true, ..mode }).is_none());
    }

    #[test]
    fn quotes_comments_and_arithmetic_have_mysql_boundaries() {
        let sql = "SELECT 'it\\'s;ok', `a``;b`, \"x;y\"; # ;ignored\nSELECT 4--2; -- ;ignored\nSELECT '日本';";
        let actual = parts(sql, Mode::default());
        assert_eq!(actual.len(), 3);
        assert!(actual[1].ends_with("SELECT 4--2"));
        assert!(actual[2].ends_with("SELECT '日本'"));
        assert!(parts("# nothing;\n-- comment;\n/* ordinary; */", Mode::default()).is_empty());
        assert_eq!(parts("/*!80000 SET @x=1 */; /*M! SET @y=2 */;", Mode::default()).len(), 2);
    }
    #[test]
    fn custom_delimiter_preserves_routine_body_and_omits_directives() {
        let sql = "  DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT ';'; SELECT 2; END$$\nDELIMITER ;\nSELECT 3;";
        assert_eq!(parts(sql, Mode::default()), vec!["CREATE PROCEDURE p() BEGIN SELECT ';'; SELECT 2; END", "SELECT 3"]);
        assert!(ranges("DELIMITER\nSELECT 1;", Mode::default(), true).is_err());
    }
    #[test]
    fn session_sql_modes_change_quote_escaping() {
        let sql = "SELECT 'a\\'; SELECT 2;";
        assert!(ranges(sql, Mode::default(), true).is_err());
        assert_eq!(parts(sql, Mode { no_backslash_escapes: true, ansi_quotes: false }).len(), 2);
        let sql = "SELECT \"a\\\"; SELECT 2;";
        assert!(ranges(sql, Mode::default(), true).is_err());
        assert_eq!(parts(sql, Mode { ansi_quotes: true, no_backslash_escapes: false }).len(), 2);
    }
    #[test]
    fn editor_keeps_unfinished_tail_but_execution_rejects_it() {
        for sql in ["SELECT 1; SELECT 'unfinished", "SELECT 1; /* unfinished"] {
            assert!(ranges(sql, Mode::default(), true).is_err());
            assert!(ranges(sql, Mode::default(), false).is_ok());
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn incremental_scanning_carries_delimiters_and_uses_new_mode() {
        let sql = "DELIMITER $$\nSET sql_mode='NO_BACKSLASH_ESCAPES'$$\nSELECT 'a\\'$$\nDELIMITER ;\nSELECT 3;";
        let (first, consumed, delimiter) = next_statement(sql, Mode::default(), ";").unwrap();
        assert_eq!(&sql[first.unwrap()], "SET sql_mode='NO_BACKSLASH_ESCAPES'");
        assert_eq!(delimiter, "$$");
        let tail = &sql[consumed..];
        let (second, consumed, delimiter) = next_statement(tail, Mode { no_backslash_escapes: true, ansi_quotes: false }, &delimiter).unwrap();
        assert_eq!(tail[second.unwrap()].trim(), "SELECT 'a\\'");
        let tail = &tail[consumed..];
        let (third, _, delimiter) = next_statement(tail, Mode::default(), &delimiter).unwrap();
        assert_eq!(tail[third.unwrap()].trim(), "SELECT 3");
        assert_eq!(delimiter, ";");
    }

    #[test]
    fn isolated_routine_retains_delimiter_for_execution() {
        let sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT '日本'; SELECT 2; END$$\nDELIMITER ;\nSELECT 3;";
        let range = editor_ranges(sql, Mode::default()).remove(0);
        let extracted = execution_text(sql, range, Mode::default()).unwrap();
        let parts = ranges(&extracted, Mode::default(), true).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(&extracted[parts[0].clone()], "CREATE PROCEDURE p() BEGIN SELECT '日本'; SELECT 2; END");
        let range = editor_ranges(sql, Mode::default()).remove(1);
        assert_eq!(execution_text(sql, range, Mode::default()).unwrap(), "SELECT 3;");
    }

    #[test]
    fn selected_script_can_override_inherited_delimiter() {
        let sql = "DELIMITER //\nSELECT 1//\nDELIMITER ;\nSELECT 2; SELECT 3;";
        let start = sql.find("DELIMITER ;").unwrap();
        let extracted = execution_text(sql, start..sql.len(), Mode::default()).unwrap();
        let parts = ranges(&extracted, Mode::default(), true).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(extracted[parts[0].clone()].trim(), "SELECT 2");
        assert_eq!(extracted[parts[1].clone()].trim(), "SELECT 3");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod server_tests {
    use super::*;
    use ::mysql::{Conn, OptsBuilder, prelude::Queryable};

    fn fixture_options() -> OptsBuilder {
        let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT")
            .expect("set disposable server port").parse().unwrap();
        assert!([43316, 43317].contains(&port));
        OptsBuilder::default().ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(port).user(Some("rustable")).pass(Some("rustable-test-only"))
            .db_name(Some("rustable_test")).prefer_socket(false)
            .tcp_connect_timeout(Some(std::time::Duration::from_secs(2)))
            .read_timeout(Some(std::time::Duration::from_secs(3)))
            .write_timeout(Some(std::time::Duration::from_secs(3)))
            .max_allowed_packet(Some(32 * 1024 * 1024))
    }

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn cancelled_write_preserves_prior_transaction_and_timeout_is_unknown() {
        use std::time::{Duration, Instant};
        let options = fixture_options();
        let mut conn = Conn::new(options.clone()).unwrap();
        let mut control = Conn::new(options.clone()).unwrap();
        conn.query_drop("CREATE TEMPORARY TABLE rustable_cancel_probe (value INT) ENGINE=InnoDB").unwrap();
        conn.query_drop("INSERT INTO rustable_cancel_probe VALUES (7)").unwrap();
        conn.query_drop("START TRANSACTION").unwrap();
        conn.query_drop("UPDATE rustable_cancel_probe SET value=8").unwrap();
        let id = conn.connection_id();
        let (start_tx, start_rx) = std::sync::mpsc::channel();
        let cancellation = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            // Poll this user's own process, so KILL cannot run before the UPDATE.
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let sql: Option<String> = control.exec_first(
                    "SELECT INFO FROM information_schema.PROCESSLIST WHERE ID=?", (id,)).unwrap().flatten();
                if sql.as_deref().is_some_and(|sql| sql.contains("SLEEP(10)")) { break; }
                assert!(Instant::now() < deadline, "write did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            control.query_drop(format!("KILL QUERY {id}")).unwrap();
        });
        let started = Instant::now();
        start_tx.send(()).unwrap();
        let result = conn.query_drop("UPDATE rustable_cancel_probe SET value=IF(SLEEP(10)=0,99,100)");
        cancellation.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(matches!(result, Err(::mysql::Error::MySqlError(ref error)) if error.code == 1317), "{result:?}");
        assert_eq!(conn.transaction_status(), None);
        conn.ping().unwrap();
        assert_eq!(conn.transaction_status(), Some(true));
        assert_eq!(conn.query_first::<i64, _>("SELECT value FROM rustable_cancel_probe").unwrap(), Some(8));
        conn.query_drop("ROLLBACK").unwrap();
        assert_eq!(conn.query_first::<i64, _>("SELECT value FROM rustable_cancel_probe").unwrap(), Some(7));
        assert_eq!(conn.transaction_status(), Some(false));

        let mut stalled = Conn::new(options.read_timeout(Some(Duration::from_millis(100)))).unwrap();
        let started = Instant::now();
        assert!(stalled.query_drop("SELECT SLEEP(1)").is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(stalled.transaction_status(), None);
        // A timed-out stream must be discarded, not blindly reused for a write.
        drop(stalled);
    }

    /// Run separately for the disposable MySQL and MariaDB compose services.
    #[test]
    #[ignore = "requires disposable server: RUSTABLE_MYSQL_TEST_PORT=43316 or 43317"]
    fn live_server_routines_modes_and_transaction_status() {
        let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT")
            .expect("set disposable server port").parse().unwrap();
        assert!([43316, 43317].contains(&port), "use the isolated fixture ports");
        let options = OptsBuilder::default()
            .ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(port)
            .user(Some("rustable"))
            .pass(Some("rustable-test-only"))
            .db_name(Some("rustable_test"))
            .prefer_socket(false);
        let mut conn = Conn::new(options.clone()).unwrap();
        let version: String = conn.query_first("SELECT VERSION()").unwrap().unwrap();
        eprintln!("server: {version}");
        // The protocol connection is reused; no implicit BEGIN is needed for reads.
        let id: u64 = conn.query_first("SELECT CONNECTION_ID()").unwrap().unwrap();
        assert_eq!(conn.transaction_status(), Some(false));
        conn.query_drop("DROP PROCEDURE IF EXISTS rustable_delimiter_probe").unwrap();
        let script = "DELIMITER $$\nCREATE PROCEDURE rustable_delimiter_probe() BEGIN SELECT '日本' AS label; SELECT 9007199254740993 AS exact_integer; END$$\nDELIMITER ;\nSELECT 3;";
        let routine = editor_ranges(script, Mode::default()).remove(0);
        let prepared = execution_text(script, routine, Mode::default()).unwrap();
        for range in ranges(&prepared, Mode::default(), true).unwrap() {
            conn.query_drop(&prepared[range]).unwrap();
        }
        let mut results = conn.query_iter("CALL rustable_delimiter_probe()").unwrap();
        let first: Vec<::mysql::Row> = results.iter().unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(first[0].get::<String, _>(0).unwrap(), "日本");
        let second: Vec<::mysql::Row> = results.iter().unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(second[0].get::<u64, _>(0).unwrap(), 9007199254740993);
        drop(results); // Drain CALL's final status result before reusing connection.
        assert_eq!(conn.query_first::<u64, _>("SELECT CONNECTION_ID()").unwrap(), Some(id));
        conn.query_drop("SET SESSION sql_mode='ANSI_QUOTES,NO_BACKSLASH_ESCAPES'").unwrap();
        let mode = Mode { ansi_quotes: true, no_backslash_escapes: true };
        let sql = "SELECT 'a\\' AS value; SELECT 2 AS value;";
        let statements = ranges(sql, mode, true).unwrap();
        assert_eq!(statements.len(), 2);
        assert_eq!(conn.query_first::<String, _>(&sql[statements[0].clone()]).unwrap(), Some("a\\".into()));
        conn.query_drop("CREATE TEMPORARY TABLE rustable_tx_probe (value INT) ENGINE=InnoDB").unwrap();
        conn.query_drop("INSERT INTO rustable_tx_probe VALUES (7)").unwrap();
        conn.query_drop("START TRANSACTION").unwrap();
        assert_eq!(conn.transaction_status(), Some(true));
        conn.query_drop("UPDATE rustable_tx_probe SET value=8").unwrap();
        // ERR packets have no transaction flags. Do not mistake unknown for idle.
        assert!(conn.query_drop("SELECT * FROM rustable_missing_table_93847").is_err());
        assert_eq!(conn.transaction_status(), None);
        conn.ping().unwrap();
        assert_eq!(conn.transaction_status(), Some(true));
        conn.query_drop("ROLLBACK").unwrap();
        assert_eq!(conn.transaction_status(), Some(false));
        assert_eq!(conn.query_first::<i64, _>("SELECT value FROM rustable_tx_probe").unwrap(), Some(7));
        conn.query_drop("START TRANSACTION").unwrap();
        conn.query_drop("UPDATE rustable_tx_probe SET value=9").unwrap();
        // Regular DDL implicitly commits, unlike CREATE TEMPORARY TABLE.
        conn.query_drop("DROP PROCEDURE rustable_delimiter_probe").unwrap();
        assert_eq!(conn.transaction_status(), Some(false));
        conn.query_drop("ROLLBACK").unwrap();
        assert_eq!(conn.query_first::<i64, _>("SELECT value FROM rustable_tx_probe").unwrap(), Some(9));
        let mut killer = Conn::new(options).unwrap();
        killer.query_drop(format!("KILL CONNECTION {id}")).unwrap();
        assert!(conn.query_drop("SELECT 1").is_err());
        assert_eq!(conn.transaction_status(), None);
    }
}

// Tokenizer-only adapter: retain MySQL token categories while honoring the
// session's quote rules. Parsing/execution still use the shared SQL scanner.
#[derive(Debug)]
pub(crate) struct Lexer(pub Mode);
impl sqlparser::dialect::Dialect for Lexer {
    fn dialect(&self) -> std::any::TypeId { std::any::TypeId::of::<sqlparser::dialect::MySqlDialect>() }
    fn is_identifier_start(&self, ch: char) -> bool { sqlparser::dialect::MySqlDialect {}.is_identifier_start(ch) }
    fn is_identifier_part(&self, ch: char) -> bool { sqlparser::dialect::MySqlDialect {}.is_identifier_part(ch) }
    fn is_delimited_identifier_start(&self, ch: char) -> bool { ch == '`' || (self.0.ansi_quotes && ch == '"') }
    fn supports_string_literal_backslash_escape(&self) -> bool { !self.0.no_backslash_escapes }
    fn ignores_wildcard_escapes(&self) -> bool { true }
    fn supports_numeric_prefix(&self) -> bool { true }
    fn supports_multiline_comment_hints(&self) -> bool { true }
    fn requires_single_line_comment_whitespace(&self) -> bool { true }
}
