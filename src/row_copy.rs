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
                        value,
                    } if table == Some(target.as_str())
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
        let changes = vec![Change::Cell {
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
