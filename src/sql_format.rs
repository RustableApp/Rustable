//! Background SQL formatting. Verify lexical content before changing the editor.
use crate::sql::Dialect;
use eframe::egui;
use sqlparser::{
    dialect::{PostgreSqlDialect, SQLiteDialect},
    tokenizer::{Token, Tokenizer, Whitespace},
};
use std::{ops::Range, sync::mpsc};
const INPUT_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 8 * INPUT_LIMIT;

pub struct Formatted {
    pub original: String,
    pub text: String,
    pub selection: Range<usize>,
}
#[derive(Default)]
pub struct Formatter {
    worker: Option<mpsc::Receiver<Result<Formatted, String>>>,
    pub status: Option<String>,
}
impl Formatter {
    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        sql: &str,
        selection: Range<usize>,
        dialect: Dialect,
    ) {
        if self.busy() {
            return;
        }
        if sql.len() > INPUT_LIMIT {
            self.status = Some("Beautify supports documents up to 1 MiB.".into());
            return;
        }
        self.status = None;
        let original = sql.to_owned();
        let ctx = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| format_document(original, selection, dialect))
                .unwrap_or_else(|_| Err("Formatter failed; SQL was left unchanged.".into()));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, current: &str) -> Option<Formatted> {
        let result = match self.worker.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Formatter stopped; SQL was left unchanged.".into())
            }
        };
        self.worker = None;
        match result {
            Ok(result) if result.original == current => Some(result),
            Ok(_) => {
                self.status = Some("Document changed while formatting. Run Beautify again.".into());
                None
            }
            Err(error) => {
                self.status = Some(error);
                None
            }
        }
    }
}
fn signature(sql: &str, dialect: Dialect) -> Result<Vec<(Token, bool)>, String> {
    let dialect: &dyn sqlparser::dialect::Dialect = match dialect {
        Dialect::Postgres => &PostgreSqlDialect {},
        Dialect::Sqlite => &SQLiteDialect {},
    };
    let tokens = Tokenizer::new(dialect, sql)
        .with_unescape(false)
        .tokenize()
        .map_err(|_| "Cannot safely format incomplete SQL. SQL was left unchanged.".to_owned())?;
    let mut result = Vec::new();
    let mut newline = false;
    for token in tokens {
        match token {
            Token::Whitespace(Whitespace::Newline) => newline = true,
            Token::Whitespace(Whitespace::Space | Whitespace::Tab) => {}
            // PostgreSQL uses a newline between adjacent string literals as concatenation.
            token => {
                let significant_newline = newline
                    && matches!(token, Token::SingleQuotedString(_))
                    && result.last().is_some_and(|(previous, _)| {
                        matches!(previous, Token::SingleQuotedString(_))
                    });
                result.push((token, significant_newline));
                newline = false;
            }
        }
    }
    Ok(result)
}
// Give the formatter opaque stand-ins for text it must never rewrite. Locations
// are consumed monotonically, avoiding repeated scans of long one-line scripts.
fn protected_format(input: &str, dialect: Dialect) -> Result<String, String> {
    let lexer: &dyn sqlparser::dialect::Dialect = match dialect {
        Dialect::Postgres => &PostgreSqlDialect {},
        Dialect::Sqlite => &SQLiteDialect {},
    };
    let tokens = Tokenizer::new(lexer, input)
        .with_unescape(false)
        .tokenize_with_location()
        .map_err(|_| "Cannot tokenize SQL for formatting.")?;
    let mut prefix = "_rustable_format_".to_owned();
    while input.contains(&prefix) {
        prefix = prefix.repeat(2);
    }
    let mut cursor = input.char_indices().peekable();
    let mut location = (1u64, 1u64);
    let mut seek = |target: sqlparser::tokenizer::Location| -> Result<usize, String> {
        while location < (target.line, target.column) {
            let (_, ch) = cursor.next().ok_or("Invalid SQL token location")?;
            if ch == '\n' {
                location = (location.0 + 1, 1);
            } else {
                location.1 += 1;
            }
        }
        if location != (target.line, target.column) {
            return Err("Invalid SQL token location".into());
        }
        Ok(cursor.peek().map_or(input.len(), |(byte, _)| *byte))
    };
    let mut masked = String::with_capacity(input.len());
    let mut originals = Vec::new();
    let mut previous = 0;
    let mut consumed = 0;
    for (index, token) in tokens.iter().enumerate() {
        if index < consumed {
            continue;
        }
        let block = matches!(
            token.token,
            Token::Whitespace(
                Whitespace::MultiLineComment(_) | Whitespace::SingleLineComment { .. }
            )
        );
        let quoted = matches!(&token.token, Token::Word(word) if word.quote_style.is_some());
        let string = matches!(
            token.token,
            Token::SingleQuotedString(_)
                | Token::DoubleQuotedString(_)
                | Token::DollarQuotedString(_)
                | Token::EscapedStringLiteral(_)
                | Token::UnicodeStringLiteral(_)
                | Token::NationalStringLiteral(_)
                | Token::HexStringLiteral(_)
                | Token::SingleQuotedByteStringLiteral(_)
        );
        if !block && !quoted && !string {
            continue;
        }
        let start = seek(token.span.start)?;
        let mut end_location = token.span.end;
        // Treat adjacent string literals as one opaque span so their separating
        // newline (PostgreSQL concatenation syntax) survives formatting exactly.
        if matches!(
            token.token,
            Token::SingleQuotedString(_) | Token::EscapedStringLiteral(_)
        ) {
            let mut next = index + 1;
            loop {
                while next < tokens.len()
                    && matches!(
                        tokens[next].token,
                        Token::Whitespace(
                            Whitespace::Space | Whitespace::Tab | Whitespace::Newline
                        )
                    )
                {
                    next += 1;
                }
                if next < tokens.len() && matches!(tokens[next].token, Token::SingleQuotedString(_))
                {
                    end_location = tokens[next].span.end;
                    next += 1;
                    consumed = next;
                } else {
                    break;
                }
            }
        }
        let end = seek(end_location)?;
        let raw = input.get(start..end).ok_or("Invalid SQL token bounds")?;
        masked.push_str(&input[previous..start]);
        let index = originals.len();
        if block {
            masked.push_str(&format!("/*{prefix}{index}*/"));
        } else if quoted {
            masked.push_str(&format!("\"{prefix}{index}\""));
        } else {
            masked.push_str(&format!("'{prefix}{index}'"));
        }
        if masked.len() > OUTPUT_LIMIT {
            return Err("Protected SQL is too large to format.".into());
        }
        originals.push(raw);
        previous = end;
    }
    masked.push_str(&input[previous..]);
    let formatted = sqlformat::format(
        &masked,
        &sqlformat::QueryParams::None,
        &sqlformat::FormatOptions {
            dialect: if dialect == Dialect::Postgres {
                sqlformat::Dialect::PostgreSql
            } else {
                sqlformat::Dialect::Generic
            },
            uppercase: None,
            ..Default::default()
        },
    );
    if formatted.len() > OUTPUT_LIMIT {
        return Err("Formatted SQL is too large.".into());
    }
    let prefix = regex::escape(&prefix);
    let markers = regex::Regex::new(&format!(r#"(?:['"]{prefix}(\d+)['"]|/\*{prefix}(\d+)\*/)"#))
        .map_err(|_| "Could not restore protected SQL")?;
    let restored = markers
        .replace_all(&formatted, |captures: &regex::Captures<'_>| {
            let index = captures
                .get(1)
                .or_else(|| captures.get(2))
                .unwrap()
                .as_str()
                .parse::<usize>()
                .unwrap();
            originals
                .get(index)
                .copied()
                .unwrap_or(captures.get(0).unwrap().as_str())
                .to_owned()
        })
        .into_owned();
    Ok(restored)
}
fn format_document(
    original: String,
    selection: Range<usize>,
    dialect: Dialect,
) -> Result<Formatted, String> {
    if original.len() > INPUT_LIMIT {
        return Err("Document exceeds Beautify limit.".into());
    }
    let chars = original.chars().count();
    let selected = !selection.is_empty();
    let range = if selected {
        selection.start.min(chars)..selection.end.min(chars)
    } else {
        0..chars
    };
    let byte = |index| {
        original
            .char_indices()
            .nth(index)
            .map_or(original.len(), |(i, _)| i)
    };
    let bytes = byte(range.start)..byte(range.end);
    let input = &original[bytes.clone()];
    let before = signature(input, dialect)?;
    if before.len() > 100_000 {
        return Err("Too many SQL tokens to format at once. Select fewer statements.".into());
    }
    let mut depth = 0usize;
    for (token, _) in &before {
        match token {
            Token::LParen => {
                depth += 1;
                if depth > 128 {
                    return Err("SQL nesting exceeds the formatter limit.".into());
                }
            }
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let text = protected_format(input, dialect)?;
    if text.len() > OUTPUT_LIMIT {
        return Err("Formatted output is too large; SQL was left unchanged.".into());
    }
    if before != signature(&text, dialect)? {
        return Err(
            "This SQL construct cannot be formatted safely yet. SQL was left unchanged.".into(),
        );
    }
    let end = range.start + text.chars().count();
    let mut full = original.clone();
    full.replace_range(bytes, &text);
    // Also verify boundaries against the unselected prefix/suffix.
    if signature(&original, dialect)? != signature(&full, dialect)? {
        return Err(
            "Selection crosses SQL token boundaries. Select complete statements and try again."
                .into(),
        );
    }
    Ok(Formatted {
        original,
        text: full,
        selection: if selected { range.start..end } else { 0..0 },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_queries_and_preserves_unicode_selection_boundaries() {
        let sql = "select id,name from customers where id=2;";
        let formatted = format_document(sql.into(), 0..0, Dialect::Sqlite).unwrap();
        assert!(formatted.text.contains('\n'));
        assert_eq!(
            signature(sql, Dialect::Sqlite).unwrap(),
            signature(&formatted.text, Dialect::Sqlite).unwrap()
        );
        let prefix = "-- café\n";
        let doc = format!("{prefix}{sql}\nselect 9;");
        let start = prefix.chars().count();
        let formatted =
            format_document(doc, start..start + sql.chars().count(), Dialect::Sqlite).unwrap();
        assert!(formatted.text.starts_with(prefix));
        assert!(formatted.text.ends_with("\nselect 9;"));
        assert_eq!(formatted.selection.start, start);
    }
    #[test]
    fn dialect_literals_comments_and_operators_never_silently_change() {
        for (dialect, sql) in [
            (
                Dialect::Postgres,
                "select $$hello  world;$$, E'it\\'s', '{1,2}'::int[];",
            ),
            (Dialect::Postgres, "select 'one'\n 'two';"),
            (
                Dialect::Postgres,
                "select 1 /* outer /* inner */ still comment */ + 2;",
            ),
            (Dialect::Postgres, "select data #>> '{a,b}' from items;"),
            (Dialect::Sqlite, "select [a b], `c d`, 'a  b' from t;"),
            (
                Dialect::Sqlite,
                "create trigger t after insert on a begin update a set x=1; select 2; end;",
            ),
        ] {
            if let Ok(result) = format_document(sql.into(), 0..0, dialect) {
                assert_eq!(
                    signature(sql, dialect).unwrap(),
                    signature(&result.text, dialect).unwrap()
                );
            }
        }
        assert!(format_document("select 'unfinished".into(), 0..0, Dialect::Postgres).is_err());
        assert_ne!(
            signature("select 'a'\n'b'", Dialect::Postgres).unwrap(),
            signature("select 'a' 'b'", Dialect::Postgres).unwrap()
        );
    }
    #[test]
    fn procedural_bodies_nested_comments_and_quoted_names_are_preserved() {
        let joined =
            format_document("select 'one'\n 'two';".into(), 0..0, Dialect::Postgres).unwrap();
        assert!(joined.text.contains("'one'\n 'two'"));
        for (dialect, sql, exact) in [
            (
                Dialect::Postgres,
                "create function f() returns text language plpgsql as $body$begin\n return 'hello  world;';\nend$body$;",
                "$body$begin\n return 'hello  world;';\nend$body$",
            ),
            (
                Dialect::Postgres,
                "select 1 /* café /* inner */ still outer */ +2;",
                "/* café /* inner */ still outer */",
            ),
            (
                Dialect::Postgres,
                "select E'it\\'s  fine', \"a  b\" from t;",
                "E'it\\'s  fine'",
            ),
            (
                Dialect::Sqlite,
                "select [a b], `c d`, 'x  y' from t;",
                "[a b]",
            ),
            (
                Dialect::Sqlite,
                "-- café\r\nselect '\t_日本', [two words] from t;",
                "'\t_日本'",
            ),
        ] {
            let result = format_document(sql.into(), 0..0, dialect).expect(sql);
            assert!(result.text.contains(exact), "{}", result.text);
            assert_eq!(
                signature(sql, dialect).unwrap(),
                signature(&result.text, dialect).unwrap()
            );
        }
    }
    #[test]
    fn placeholder_like_values_are_not_replaced_and_sqlite_values_roundtrip() {
        let sql = "select '_rustable_format_0', '_rustable_format__rustable_format_1', 'a  b', x'00ff', 2+3;";
        let formatted = format_document(sql.into(), 0..0, Dialect::Sqlite).unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let read = |query: &str| {
            db.query_row(query, [], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .unwrap()
        };
        assert_eq!(read(sql), read(&formatted.text));
    }
    #[test]
    #[ignore = "manual formatter throughput measurement"]
    fn formatting_latency() {
        let input = (0..2000)
            .map(|i| format!("select 'café {i}' as label,id from t where id={i};\n"))
            .collect::<String>();
        let mut samples = Vec::new();
        for run in 0..9 {
            let start = std::time::Instant::now();
            let formatted = format_document(input.clone(), 0..0, Dialect::Postgres).unwrap();
            assert!(formatted.text.len() > input.len());
            if run > 1 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "formatter {} bytes / 2000 statements: p50 {:.2}ms, max-of-7 {:.2}ms",
            input.len(),
            samples[3],
            samples[6]
        );
    }
    #[test]
    fn stale_background_result_is_discarded() {
        let ctx = egui::Context::default();
        let mut formatter = Formatter::default();
        formatter.start(&ctx, "select 1;", 0..0, Dialect::Sqlite);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while formatter.busy() {
            assert!(formatter.poll("select 2;").is_none());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(formatter.status.unwrap().contains("Document changed"));
    }
}
