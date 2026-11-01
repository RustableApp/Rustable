//! Surgical CREATE TABLE edits preserve constraints that the UI is not changing.
use std::ops::Range;

#[derive(Debug)]
struct Token {
    span: Range<usize>,
    word: String,
    depth: usize,
    quoted: bool,
}

fn tokens(sql: &str) -> Result<Vec<Token>, String> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            i += 2;
            while i + 1 < bytes.len() && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            if i + 1 >= bytes.len() {
                return Err("Unclosed schema comment".into());
            }
            i += 2;
            continue;
        }
        let start = i;
        let quoted = matches!(bytes[i], b'\'' | b'"' | b'`' | b'[');
        let word: String = if quoted {
            let close = if bytes[i] == b'[' { b']' } else { bytes[i] };
            i += 1;
            let mut value = Vec::new();
            loop {
                if i >= bytes.len() {
                    return Err("Unclosed schema identifier or literal".into());
                }
                if bytes[i] == close {
                    i += 1;
                    if close != b']' && bytes.get(i) == Some(&close) {
                        value.push(close);
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    value.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8(value).map_err(|e| e.to_string())?
        } else if b"(),;".contains(&bytes[i]) {
            i += 1;
            sql[start..i].into()
        } else {
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !b"(),;\"'`[".contains(&bytes[i])
            {
                if bytes[i..].starts_with(b"--") || bytes[i..].starts_with(b"/*") {
                    break;
                }
                i += 1;
            }
            sql[start..i].into()
        };
        if !quoted && word == ")" {
            depth = depth
                .checked_sub(1)
                .ok_or("Unbalanced schema parentheses")?;
        }
        out.push(Token {
            span: start..i,
            word: word.clone(),
            depth,
            quoted,
        });
        if !quoted && word == "(" {
            depth += 1;
        }
    }
    if depth != 0 {
        return Err("Unbalanced schema parentheses".into());
    }
    Ok(out)
}
fn keyword(token: &Token, name: &str) -> bool {
    !token.quoted && token.word.eq_ignore_ascii_case(name)
}
fn constraint_start(tokens: &[&Token], i: usize) -> bool {
    let t = tokens[i];
    [
        "CONSTRAINT",
        "PRIMARY",
        "UNIQUE",
        "CHECK",
        "DEFAULT",
        "COLLATE",
        "REFERENCES",
        "GENERATED",
        "AS",
    ]
    .iter()
    .any(|k| keyword(t, k))
        || (keyword(t, "NOT") && tokens.get(i + 1).is_some_and(|next| keyword(next, "NULL")))
}

/// Replace one column's type, explicit NOT NULL and DEFAULT, preserving all
/// other column/table clauses verbatim. A None patch leaves a property intact.
pub(super) fn edit_column(
    ddl: &str,
    name: &str,
    data_type: Option<&str>,
    not_null: Option<bool>,
    default_sql: Option<Option<&str>>,
) -> Result<String, String> {
    let ts = tokens(ddl)?;
    let open = ts
        .iter()
        .position(|t| !t.quoted && t.word == "(")
        .ok_or("Expected CREATE TABLE column list")?;
    if ts[..open].iter().any(|t| keyword(t, "VIRTUAL")) {
        return Err("Virtual tables require their module's schema editor".into());
    }
    let mut start = open + 1;
    let mut range = None;
    for i in (open + 1)..ts.len() {
        if (!ts[i].quoted && ts[i].word == "," && ts[i].depth == 1)
            || (!ts[i].quoted && ts[i].word == ")" && ts[i].depth == 0)
        {
            if start < i && ts[start].word.eq_ignore_ascii_case(name) {
                range = Some(start..i);
                break;
            }
            start = i + 1;
        }
    }
    let range = range.ok_or("Column was not found in CREATE TABLE")?;
    let first = &ts[range.start];
    let top: Vec<&Token> = ts[range.clone()].iter().filter(|t| t.depth == 1).collect();
    let mut starts = Vec::new();
    let mut i = 1;
    while i < top.len() {
        if constraint_start(&top, i) {
            let constraint = i;
            if keyword(top[i], "CONSTRAINT") {
                i += 2;
            }
            if i >= top.len() {
                return Err("Incomplete named column constraint".into());
            }
            starts.push((constraint, i));
        }
        i += 1;
    }
    // CONSTRAINT name NOT NULL produced only a single start above.
    let end = ts[range.end].span.start;
    let type_end = starts.first().map_or(end, |(i, _)| top[*i].span.start);
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    if let Some(data_type) = data_type {
        if data_type.trim().is_empty() || data_type.contains('\0') {
            return Err("Data type must not be empty or contain NUL".into());
        }
        edits.push((
            first.span.end..type_end,
            format!(" {} ", super::quote(data_type)),
        ));
    }
    for (index, (begin, kind)) in starts.iter().enumerate() {
        let t = top[*kind];
        let remove = (not_null.is_some() && keyword(t, "NOT"))
            || (default_sql.is_some() && keyword(t, "DEFAULT"));
        if remove {
            let finish = starts
                .get(index + 1)
                .map_or(end, |(i, _)| top[*i].span.start);
            edits.push((top[*begin].span.start..finish, String::new()));
        }
    }
    // A trailing -- comment must not swallow the added constraint.
    let mut suffix = String::from("\n");
    if not_null == Some(true) {
        suffix.push_str(" NOT NULL");
    }
    if let Some(Some(default)) = default_sql {
        suffix.push_str(" DEFAULT ");
        suffix.push_str(default);
    }
    suffix.push(' ');
    edits.push((end..end, suffix));
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = ddl.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

pub(super) fn with_table_name(ddl: &str, name: &str) -> Result<String, String> {
    let ts = tokens(ddl)?;
    let table = ts
        .iter()
        .position(|t| keyword(t, "TABLE"))
        .ok_or("Expected CREATE TABLE")?;
    let mut i = table + 1;
    if ts.get(i).is_some_and(|t| keyword(t, "IF")) {
        i += 3;
    }
    let token = ts.get(i).ok_or("Missing table name")?;
    let mut result = ddl.to_owned();
    result.replace_range(token.span.clone(), &super::quote(name));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    #[test]
    fn edits_preserve_nested_defaults_checks_comments_and_named_constraints() {
        let ddl = "CREATE TABLE t(id INTEGER PRIMARY KEY, \"strange,name\" VARCHAR(12) CONSTRAINT required NOT NULL ON CONFLICT FAIL DEFAULT ('a,b') COLLATE NOCASE CHECK(length(\"strange,name\") > 0), other TEXT DEFAULT 'NOT NULL', UNIQUE(other)) STRICT";
        let result = edit_column(
            ddl,
            "strange,name",
            Some("TEXT"),
            Some(false),
            Some(Some("'new''value'")),
        )
        .unwrap();
        assert!(result.contains("COLLATE NOCASE CHECK"));
        assert!(result.contains("other TEXT DEFAULT 'NOT NULL'"));
        assert!(result.contains("UNIQUE(other)"));
        assert!(!result.contains("CONSTRAINT required"));
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&result).unwrap();
        let (kind, required, default): (String, bool, String) = conn.query_row("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('t') WHERE name='strange,name'",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            (kind, required, default),
            ("TEXT".into(), false, "'new''value'".into())
        );
    }
    #[test]
    fn trailing_line_comments_do_not_swallow_added_constraints() {
        let ddl = "CREATE TABLE t(value TEXT -- comment\n, other TEXT)";
        let sql = edit_column(ddl, "value", None, Some(true), Some(Some("'a'"))).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&sql).unwrap();
        assert!(
            conn.query_row(
                "SELECT \"notnull\" FROM pragma_table_info('t') WHERE name='value'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
    }

    #[test]
    fn strings_and_generated_expressions_are_untouched() {
        let ddl = "CREATE TABLE [t](a TEXT DEFAULT 'a''b', b TEXT GENERATED ALWAYS AS (a || ',DEFAULT') STORED, CONSTRAINT u UNIQUE(a))";
        let result = edit_column(ddl, "a", None, Some(true), None).unwrap();
        assert!(result.contains("DEFAULT 'a''b'"));
        assert!(result.contains("GENERATED ALWAYS AS (a || ',DEFAULT') STORED"));
        Connection::open_in_memory()
            .unwrap()
            .execute_batch(&result)
            .unwrap();
        assert!(
            with_table_name(&result, "new\"table")
                .unwrap()
                .starts_with("CREATE TABLE \"new\"\"table\"")
        );
    }
}
