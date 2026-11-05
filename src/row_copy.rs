use super::{Change, Value};
use std::fmt::Write;

/// Copy one displayed row, without cloning the result grid or truncating values.
#[cfg(test)]
fn format_row(
    columns: &[String],
    row: &[Value],
    changes: &[Change],
    table: Option<&str>,
    delimiter: char,
    headers: bool,
) -> Result<String, String> {
    format_rows(
        columns,
        std::iter::once(row),
        changes,
        table,
        delimiter,
        headers,
    )
}

pub(super) fn format_rows<'a>(
    columns: &[String],
    rows: impl IntoIterator<Item = &'a [Value]>,
    changes: &[Change],
    table: Option<&str>,
    delimiter: char,
    headers: bool,
) -> Result<String, String> {
    let mut output = String::new();
    if headers {
        for (index, name) in columns.iter().enumerate() {
            if index > 0 {
                output.push(delimiter);
            }
            field(&mut output, name, delimiter)?;
        }
        output.push('\n');
    }
    for (row_index, row) in rows.into_iter().enumerate() {
        if row_index > 0 {
            output.push('\n');
        }
        for (index, original) in row.iter().enumerate() {
            if index > 0 {
                output.push(delimiter);
            }
            let value = changes
                .iter()
                .find_map(|change| match change {
                    Change::Cell {
                        table: target,
                        column,
                        original: source,
                        value, original_columns,
                    } if original_columns.as_ref().is_none_or(|names| names.as_slice() == columns) && table == Some(target.as_str())
                        && columns.get(index) == Some(column)
                        && source == row =>
                    {
                        Some(value)
                    }
                    _ => None,
                })
                .unwrap_or(original);
            match value {
                Value::Null => output.push_str("NULL"),
                Value::Text(text) => field(&mut output, text, delimiter)?,
                Value::Blob(bytes) => {
                    check_size(output.len().saturating_add(bytes.len().saturating_mul(2)))?;
                    for byte in bytes {
                        write!(output, "{byte:02x}").unwrap();
                    }
                }
                _ => output.push_str(&super::db::display(value)),
            }
            check_size(output.len())?;
        }
    }
    check_size(output.len())?;
    Ok(output)
}

fn check_size(size: usize) -> Result<(), String> {
    if size > 32 * 1024 * 1024 {
        Err("Selection exceeds the 32 MiB clipboard limit. Export the data instead.".into())
    } else {
        Ok(())
    }
}

fn field(output: &mut String, text: &str, delimiter: char) -> Result<(), String> {
    // Quote literal NULL and empty text so they remain distinct from database NULL.
    let quote = text.is_empty() || text == "NULL" || text.contains([delimiter, '"', '\r', '\n']);
    let extra = if quote {
        2 + text.bytes().filter(|b| *b == b'"').count()
    } else {
        0
    };
    check_size(
        output
            .len()
            .saturating_add(text.len())
            .saturating_add(extra),
    )?;
    if quote {
        output.push('"');
    }
    for ch in text.chars() {
        output.push(ch);
        if ch == '"' {
            output.push('"');
        }
    }
    if quote {
        output.push('"');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_special_values_and_headers() {
        let columns = vec![
            "a,b".into(),
            "text".into(),
            "empty".into(),
            "null".into(),
            "blob".into(),
        ];
        let row = vec![
            Value::Text("雪\t\"x\"\n".into()),
            Value::Text("NULL".into()),
            Value::Text(String::new()),
            Value::Null,
            Value::Blob(vec![0, 255]),
        ];
        assert_eq!(
            format_row(&columns, &row, &[], None, ',', true).unwrap(),
            "\"a,b\",text,empty,null,blob\n\"雪\t\"\"x\"\"\n\",\"NULL\",\"\",NULL,00ff"
        );
        assert!(
            format_row(&columns, &row, &[], None, '\t', false)
                .unwrap()
                .starts_with("\"雪\t")
        );
    }
    #[test]
    fn copies_staged_values_only_for_matching_table_and_row() {
        let columns = vec!["name".into()];
        let row = vec![Value::Text("before".into())];
        let changes = vec![Change::Cell { original_columns: None,
            table: "t".into(),
            column: "name".into(),
            original: row.clone(),
            value: Value::Text("after".into()),
        }];
        assert_eq!(
            format_row(&columns, &row, &changes, Some("t"), ',', false).unwrap(),
            "after"
        );
        assert_eq!(
            format_row(&columns, &row, &changes, None, ',', false).unwrap(),
            "before"
        );
        assert_eq!(
            format_row(&columns, &row, &changes, Some("other"), ',', false).unwrap(),
            "before"
        );
    }
    #[test]
    fn multi_row_headers_once_and_total_size_limit() {
        let columns = vec!["value".into()];
        let rows = [vec![Value::Integer(3)], vec![Value::Integer(1)]];
        assert_eq!(
            format_rows(
                &columns,
                rows.iter().map(Vec::as_slice),
                &[],
                None,
                ',',
                true
            )
            .unwrap(),
            "value\n3\n1"
        );
        let large = vec![Value::Text("x".repeat(17 * 1024 * 1024))];
        assert!(
            format_rows(
                &columns,
                [large.as_slice(), large.as_slice()],
                &[],
                None,
                ',',
                false
            )
            .is_err()
        );
    }
    #[test]
    fn refuses_oversize_instead_of_truncating() {
        let mut output = String::new();
        assert!(field(&mut output, &"x".repeat(32 * 1024 * 1024 + 1), ',').is_err());
        assert!(output.is_empty());
    }
}

#[derive(Clone, Copy)]
pub(super) enum Format {
    Plain,
    Csv,
    CsvHeaders,
    Json,
    Html,
    Markdown,
    Insert,
}
pub(super) const FORMATS: &[(&str, Format)] = &[
    ("Plain Text", Format::Plain),
    ("JSON", Format::Json),
    ("HTML", Format::Html),
    ("Markdown Table", Format::Markdown),
    ("CSV", Format::Csv),
    ("CSV with Header", Format::CsvHeaders),
    ("INSERT Statement", Format::Insert),
];

#[allow(clippy::too_many_arguments)]
pub(super) fn formatted<'a>(
    columns: &[String],
    rows: impl IntoIterator<Item = &'a [Value]>,
    changes: &[Change],
    table: Option<&str>,
    selected_column: Option<usize>,
    format: Format,
    dialect: crate::sql::Dialect,
) -> Result<String, String> {
    let indices: Vec<_> = (0..columns.len())
        .filter(|i| selected_column.is_none_or(|c| c == *i))
        .collect();
    let names: Vec<_> = indices.iter().map(|i| columns[*i].clone()).collect();
    let mut output = String::new();
    let html = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let markdown = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace('\r', "")
            .replace('\n', "<br>")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let ident = |s: &str| {
        let q = if matches!(dialect, crate::sql::Dialect::Mysql(_)) {
            '`'
        } else {
            '"'
        };
        format!("{q}{}{q}", s.replace(q, &format!("{q}{q}")))
    };
    match format {
        Format::Json => output.push('['),
        Format::Html => {
            output.push_str("<table><thead><tr>");
            for n in &names {
                write!(output, "<th>{}</th>", html(n)).unwrap();
            }
            output.push_str("</tr></thead><tbody>");
        }
        Format::Markdown => {
            writeln!(
                output,
                "| {} |",
                names
                    .iter()
                    .map(|n| markdown(n))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
            .unwrap();
            writeln!(
                output,
                "| {} |",
                names.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")
            )
            .unwrap();
        }
        Format::CsvHeaders => {
            output.push_str(&format_rows(
                &names,
                std::iter::empty(),
                &[],
                None,
                ',',
                true,
            )?);
        }
        _ => {}
    }
    for (number, row) in rows.into_iter().enumerate() {
        let values: Vec<_> = indices
            .iter()
            .filter_map(|i| {
                row.get(*i).map(|v| {
                    changes
                        .iter()
                        .find_map(|change| match change {
                            Change::Cell {
                                table: target,
                                column,
                                original,
                                value,
                             ..} if table == Some(target.as_str())
                                && column == &columns[*i]
                                && original == row =>
                            {
                                Some(value)
                            }
                            _ => None,
                        })
                        .unwrap_or(v)
                })
            })
            .collect();
        // Bound temporary serialization as well as the final output.
        check_size(
            output.len().saturating_add(
                values
                    .iter()
                    .map(|v| match v {
                        Value::Text(s) => s.len().saturating_mul(6),
                        Value::Blob(b) => b.len().saturating_mul(5),
                        _ => 32,
                    })
                    .sum::<usize>(),
            ),
        )?;
        match format {
            Format::Plain | Format::Csv | Format::CsvHeaders => {
                if number > 0 {
                    output.push('\n');
                }
                let values: Vec<_> = values.iter().map(|v| (*v).clone()).collect();
                output.push_str(&format_rows(
                    &names,
                    [values.as_slice()],
                    &[],
                    None,
                    if matches!(format, Format::Plain) {
                        '\t'
                    } else {
                        ','
                    },
                    false,
                )?);
            }
            Format::Json => {
                if number > 0 {
                    output.push(',');
                }
                output.push('{');
                for (i, (name, value)) in names.iter().zip(&values).enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(name).unwrap());
                    output.push(':');
                    let json = match value {
                        Value::Null => serde_json::Value::Null,
                        Value::Integer(n) => (*n).into(),
                        Value::Real(n) => serde_json::json!(n),
                        Value::Text(s) => serde_json::json!(s),
                        Value::Blob(b) => serde_json::json!(b),
                    };
                    output.push_str(&json.to_string());
                }
                output.push('}');
            }
            Format::Html => {
                output.push_str("<tr>");
                for v in &values {
                    write!(output, "<td>{}</td>", html(&super::db::clipboard_text(v))).unwrap();
                }
                output.push_str("</tr>");
            }
            Format::Markdown => {
                writeln!(
                    output,
                    "| {} |",
                    values
                        .iter()
                        .map(|v| markdown(&super::db::clipboard_text(v)))
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
                .unwrap();
            }
            Format::Insert => {
                let table = table.ok_or("INSERT requires a table result.")?;
                let table = if matches!(dialect, crate::sql::Dialect::Postgres) {
                    let mut parser =
                        sqlparser::parser::Parser::new(&sqlparser::dialect::PostgreSqlDialect {})
                            .try_with_sql(table)
                            .map_err(|e| e.to_string())?;
                    let name = parser.parse_object_name(false).map_err(|e| e.to_string())?;
                    if parser.peek_token().token != sqlparser::tokenizer::Token::EOF {
                        return Err("Invalid table name".into());
                    }
                    name.to_string()
                } else {
                    ident(table)
                };
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Null => "NULL".into(),
                        Value::Integer(n) => n.to_string(),
                        Value::Real(n) if n.is_finite() => n.to_string(),
                        Value::Real(_) => "NULL".into(),
                        Value::Text(s) => crate::db::filters::literal(s, dialect),
                        Value::Blob(b) => {
                            let hex = b.iter().map(|b| format!("{b:02x}")).collect::<String>();
                            if matches!(dialect, crate::sql::Dialect::Postgres) {
                                format!("decode('{hex}', 'hex')")
                            } else {
                                format!("X'{hex}'")
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(
                    output,
                    "INSERT INTO {} ({}) VALUES ({});",
                    table,
                    names
                        .iter()
                        .map(|n| ident(n))
                        .collect::<Vec<_>>()
                        .join(", "),
                    values
                )
                .unwrap();
            }
        }
        check_size(output.len())?;
    }
    match format {
        Format::Json => output.push(']'),
        Format::Html => output.push_str("</tbody></table>"),
        _ => {}
    }
    check_size(output.len())?;
    Ok(output)
}

#[cfg(test)]
mod format_tests {
    use super::*;
    #[test]
    fn formats_escape_and_copy_staged_selected_column() {
        let columns = vec!["id".into(), "body".into()];
        let row = vec![Value::Integer(1), Value::Text("old".into())];
        let changes = vec![Change::Cell { original_columns: None,
            table: "t".into(),
            column: "body".into(),
            original: row.clone(),
            value: Value::Text("<b>|\"雪\n".into()),
        }];
        let copy = |format, column| {
            formatted(
                &columns,
                [row.as_slice()],
                &changes,
                Some("t"),
                column,
                format,
                crate::sql::Dialect::Sqlite,
            )
            .unwrap()
        };
        let json: serde_json::Value = serde_json::from_str(&copy(Format::Json, Some(1))).unwrap();
        assert_eq!(json, serde_json::json!([{"body": "<b>|\"雪\n"}]));
        assert!(copy(Format::Html, None).contains("&lt;b&gt;|&quot;雪"));
        assert!(copy(Format::Markdown, None).contains("\\|"));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t (id INTEGER, body TEXT)")
            .unwrap();
        db.execute_batch(&copy(Format::Insert, None)).unwrap();
        let value: String = db
            .query_row("SELECT body FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "<b>|\"雪\n");
        assert!(copy(Format::CsvHeaders, Some(1)).starts_with("body\n"));
    }
}

/// Clipboard text uses literal values, with no export quoting or headers.
pub(super) fn plain_cells<'a>(
    columns: &[String], rows: impl IntoIterator<Item = &'a [Value]>,
    changes: &[Change], table: Option<&str>, range: (usize, usize),
) -> Result<String, String> {
    let mut output = String::new();
    for (row_index, row) in rows.into_iter().enumerate() {
        if row_index > 0 { output.push('\n'); }
        for (field_index, (i, original)) in row.iter().enumerate()
            .filter(|(i, _)| (range.0..=range.1).contains(i)).enumerate()
        {
            if field_index > 0 { output.push('\t'); }
            let value = changes.iter().find_map(|change| match change {
                Change::Cell { table: target, column, original: source, value , ..}
                    if table == Some(target.as_str()) && columns.get(i) == Some(column) && source == row => Some(value),
                _ => None,
            }).unwrap_or(original);
            let text = super::db::clipboard_text(value);
            check_size(output.len().saturating_add(text.len()))?;
            output.push_str(&text);
        }
    }
    Ok(output)
}
