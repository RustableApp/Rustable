//! Dialect-aware statement boundaries shared by editor and execution.
use std::{ffi::CString, ops::Range};
pub mod destructive;
pub mod mysql;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Dialect {
    #[default]
    Sqlite,
    Postgres,
    Mysql(mysql::Mode),
}
impl Dialect {
    pub fn for_engine(engine: &str) -> Self {
        if engine.eq_ignore_ascii_case("postgresql") {
            Self::Postgres
        } else if engine.eq_ignore_ascii_case("mysql") || engine.eq_ignore_ascii_case("mariadb") {
            Self::Mysql(mysql::Mode::default())
        } else {
            Self::Sqlite
        }
    }
}

/// Strict PostgreSQL boundaries, including terminators. Incomplete quotes/comments
/// reject the whole script before execution; editor scanning retains the unfinished tail.
pub fn postgres_statements(sql: &str) -> Result<Vec<Range<usize>>, String> {
    postgres_ranges(sql, true)
}

fn postgres_ranges(sql: &str, strict: bool) -> Result<Vec<Range<usize>>, String> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut start = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                let escape = quote == b'\''
                    && i > 0
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && (i < 2 || !pg_ident(bytes[i - 2]));
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if escape && bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        if bytes.get(i) == Some(&quote) {
                            i += 1;
                            continue;
                        }
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed && strict {
                    return Err("Unterminated SQL quote".into());
                }
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => match pg_comment_end(bytes, i) {
                Some(end) => i = end,
                None if strict => return Err("Unterminated SQL comment".into()),
                None => i = bytes.len(),
            },
            b'$' if i == 0 || !pg_ident(bytes[i - 1]) => {
                let mut end = i + 1;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || bytes[end] == b'_'
                        || bytes[end] >= 128)
                {
                    end += 1;
                }
                if bytes.get(end) == Some(&b'$') && (end == i + 1 || !bytes[i + 1].is_ascii_digit())
                {
                    let tag = &sql[i..=end];
                    match sql[end + 1..].find(tag) {
                        Some(close) => i = end + 1 + close + tag.len(),
                        None if strict => return Err("Unterminated SQL dollar quote".into()),
                        None => i = bytes.len(),
                    }
                } else {
                    i += 1;
                }
            }
            b';' => {
                if pg_has_code(&sql[start..i]) {
                    out.push(start..i + 1);
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if pg_has_code(&sql[start..]) {
        out.push(start..sql.len());
    }
    Ok(out)
}
fn pg_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 128
}
fn pg_comment_end(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 1;
    i += 2;
    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes.get(i..i + 2) == Some(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(i);
            }
        } else {
            i += 1;
        }
    }
    None
}
fn pg_has_code(mut s: &str) -> bool {
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if s.starts_with("--") {
            s = s.split_once('\n').map(|(_, tail)| tail).unwrap_or_default();
        } else if s.starts_with("/*") {
            s = pg_comment_end(s.as_bytes(), 0)
                .map(|end| &s[end..])
                .unwrap_or_default();
        } else {
            return !s.is_empty();
        }
    }
}

/// SQLite's own completeness parser understands quoted semicolons and trigger bodies.
/// Ranges use UTF-8 byte indices; the editor's cursor uses Unicode character indices.
pub fn statements(sql: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, byte) in sql.bytes().enumerate() {
        if byte != b';' {
            continue;
        }
        if let Ok(candidate) = CString::new(&sql[start..=index]) {
            // SAFETY: candidate is a live, NUL-terminated C string. sqlite3_complete
            // only reads it and does not require a connection or mutate database state.
            if unsafe { rusqlite::ffi::sqlite3_complete(candidate.as_ptr()) } != 0 {
                if has_code(&sql[start..=index]) {
                    ranges.push(start..index + 1);
                }
                start = index + 1;
            }
        }
    }
    if has_code(&sql[start..]) {
        ranges.push(start..sql.len());
    }
    ranges
}

fn has_code(mut text: &str) -> bool {
    loop {
        text = text.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if text.starts_with("--") {
            text = text.find('\n').map(|n| &text[n + 1..]).unwrap_or_default();
        } else if text.starts_with("/*") {
            text = text.find("*/").map(|n| &text[n + 2..]).unwrap_or_default();
        } else {
            return !text.is_empty();
        }
    }
}

pub fn char_to_byte(sql: &str, character: usize) -> usize {
    sql.char_indices()
        .nth(character)
        .map(|(b, _)| b)
        .unwrap_or(sql.len())
}

#[cfg(test)]
pub fn execution_range(sql: &str, selection: Range<usize>, all: bool) -> Option<Range<usize>> {
    execution_range_for_dialect(sql, selection, all, Dialect::Sqlite)
}

pub fn execution_range_for_dialect(
    sql: &str,
    selection: Range<usize>,
    all: bool,
    dialect: Dialect,
) -> Option<Range<usize>> {
    let contains_code = |text: &str| match dialect {
        Dialect::Sqlite => has_code(text),
        Dialect::Postgres => pg_has_code(text),
        Dialect::Mysql(mode) => mysql::ranges(text, mode, false).is_ok_and(|ranges| !ranges.is_empty()),
    };
    if all {
        return contains_code(sql).then_some(0..sql.len());
    }
    let start = char_to_byte(sql, selection.start);
    let end = char_to_byte(sql, selection.end);
    if start != end {
        return contains_code(&sql[start..end]).then_some(start..end);
    }
    let ranges = match dialect {
        Dialect::Sqlite => statements(sql),
        Dialect::Postgres => postgres_ranges(sql, false).unwrap_or_default(),
        Dialect::Mysql(mode) => mysql::editor_ranges(sql, mode),
    };
    ranges
        .iter()
        // A caret immediately after ';' still belongs to the statement just
        // completed. Only moving into the following text selects the next one.
        .find(|r| r.start <= start && start <= r.end)
        .cloned()
        .or_else(|| ranges.last().filter(|range| start >= range.end).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mysql_editor_and_execution_share_boundaries_including_after_delimiter() {
        let dialect = Dialect::for_engine("MariaDB");
        let sql = "SELECT '日本;'; SELECT `a;b`;";
        let caret = "SELECT '日本;';".chars().count();
        let range = execution_range_for_dialect(sql, caret..caret, false, dialect).unwrap();
        assert_eq!(&sql[range], "SELECT '日本;';");
        let sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END$$\nDELIMITER ;\nSELECT 3;";
        let caret = sql.find("SELECT 2").unwrap();
        let range = execution_range_for_dialect(sql, caret..caret, false, dialect).unwrap();
        assert_eq!(&sql[range], "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END$$");
    }

    #[test]
    fn mysql_caret_on_client_directive_does_not_run_an_unrelated_statement() {
        let sql = "DELIMITER $$\nSELECT 1$$\nDELIMITER ;\nSELECT 2;";
        for caret in [0, sql.find("DELIMITER ;").unwrap() + 3] {
            assert_eq!(execution_range_for_dialect(sql, caret..caret, false,
                Dialect::for_engine("MySQL")), None);
        }
    }

    #[test]
    fn postgres_quotes_comments_and_function_bodies_are_single_statements() {
        let sql = "/* outer /* ; */ still comment */ DO $body$ BEGIN PERFORM ';'; END $body$; SELECT E'it\\'s;escaped'; SELECT $日本$値;値$日本$;";
        let ranges = postgres_statements(sql).unwrap();
        assert_eq!(ranges.len(), 3);
        for range in ranges {
            let caret = sql[..range.end].chars().count();
            assert_eq!(
                execution_range_for_dialect(sql, caret..caret, false, Dialect::Postgres),
                Some(range)
            );
        }
        assert!(
            postgres_statements("/* outer /* inner */ outer */ -- only comment")
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn postgres_unfinished_tail_remains_current_without_accepting_execution() {
        for tail in [
            "SELECT $tag$unfinished;inside",
            "SELECT E'escaped\\'quote;",
            "SELECT 2 /* outer /* nested */",
        ] {
            let sql = format!("SELECT 1;{tail}");
            assert!(postgres_statements(&sql).is_err());
            let caret = sql.chars().count();
            let range =
                execution_range_for_dialect(&sql, caret..caret, false, Dialect::Postgres).unwrap();
            assert_eq!(&sql[range], tail);
        }
    }
    #[test]
    fn postgres_standard_strings_and_identifiers_do_not_use_escape_string_rules() {
        let sql = r"SELECT 'backslash\'; SELECT foo$bar, $1; SELECT 'it''s;quoted';";
        assert_eq!(postgres_statements(sql).unwrap().len(), 3);
        assert_eq!(
            postgres_statements("(SELECT 1); SELECT 2;").unwrap().len(),
            2
        );
    }
    #[test]
    fn semicolon_boundary_belongs_to_preceding_statement() {
        let sql = "SELECT 1;SELECT 2;";
        for caret in [8, 9] {
            assert_eq!(
                &sql[execution_range(sql, caret..caret, false).unwrap()],
                "SELECT 1;"
            );
        }
        assert_eq!(
            &sql[execution_range(sql, 10..10, false).unwrap()],
            "SELECT 2;"
        );
        assert_eq!(
            &sql[execution_range(sql, 9..17, false).unwrap()],
            "SELECT 2"
        );
    }
    #[test]
    fn strings_comments_and_triggers_are_not_split() {
        let sql = "-- ;\nSELECT 'a;''b'; /* ; */ SELECT \"x;y\"; CREATE TRIGGER t AFTER INSERT ON a BEGIN UPDATE b SET x=';'; INSERT INTO b VALUES(1); END; -- trailing";
        let ranges = statements(sql);
        assert_eq!(ranges.len(), 3);
        assert!(sql[ranges[2].clone()].contains("INSERT INTO b"));
        assert!(statements("-- only comments;\n/* ; */").is_empty());
    }
    #[test]
    fn selection_and_cursor_use_unicode_character_offsets() {
        let sql = "SELECT 'é';\nSELECT '日本';";
        let selection_start = sql[..sql.find('\n').unwrap() + 1].chars().count();
        let r = execution_range(sql, selection_start..sql.chars().count(), false).unwrap();
        assert_eq!(&sql[r], "SELECT '日本';");
        let r = execution_range(sql, selection_start + 3..selection_start + 3, false).unwrap();
        assert_eq!(sql[r].trim(), "SELECT '日本';");
        assert_eq!(execution_range(sql, 0..3, true).unwrap(), 0..sql.len());
        let end = sql.chars().count();
        assert_eq!(
            sql[execution_range(sql, end..end, false).unwrap()].trim(),
            "SELECT '日本';"
        );
    }
}
