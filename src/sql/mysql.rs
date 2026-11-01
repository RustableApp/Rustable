//! MySQL/MariaDB client-side statement boundaries. DELIMITER is a client directive,
//! never part of SQL sent to the server. The session must supply its SQL mode.
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mode {
    pub no_backslash_escapes: bool,
    pub ansi_quotes: bool,
}

pub fn ranges(sql: &str, mode: Mode, strict: bool) -> Result<Vec<Range<usize>>, String> {
    scan(sql, mode, strict, false)
}

pub fn editor_ranges(sql: &str, mode: Mode) -> Vec<Range<usize>> {
    scan(sql, mode, false, true).unwrap_or_default()
}

fn scan(sql: &str, mode: Mode, strict: bool, include_delimiter: bool) -> Result<Vec<Range<usize>>, String> {
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
    Ok(result)
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
