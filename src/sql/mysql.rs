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
    let bytes = sql.as_bytes();
    let (mut i, mut start, mut code) = (0, 0, false);
    let mut delimiter = ";";
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
                    return Err("DELIMITER needs one nonempty delimiter without whitespace.".into());
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
            start = i;
            code = false;
            continue;
        }
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                code = true;
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
                if !closed && strict { return Err("Unterminated MySQL quote".into()); }
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
                else if strict { return Err("Unterminated MySQL comment".into()); }
                else { i = bytes.len(); }
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => { code = true; i += 1; }
        }
    }
    if code { result.push(start..sql.len()); }
    Ok((result, delimiter))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parts(sql: &str, mode: Mode) -> Vec<&str> {
        ranges(sql, mode, true).unwrap().into_iter().map(|r| sql[r].trim()).collect()
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
        let mut conn = Conn::new(options).unwrap();
        let version: String = conn.query_first("SELECT VERSION()").unwrap().unwrap();
        eprintln!("server: {version}");
        // The protocol connection is reused; no implicit BEGIN is needed for reads.
        let id: u64 = conn.query_first("SELECT CONNECTION_ID()").unwrap().unwrap();
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
        conn.query_drop("UPDATE rustable_tx_probe SET value=8").unwrap();
        conn.query_drop("ROLLBACK").unwrap();
        assert_eq!(conn.query_first::<i64, _>("SELECT value FROM rustable_tx_probe").unwrap(), Some(7));
        conn.query_drop("DROP PROCEDURE rustable_delimiter_probe").unwrap();
    }
}
