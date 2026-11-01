//! Loaded rows as a JSON array of objects, streamed into atomic publication.
use super::{Grid, csv};
use std::{
    collections::HashSet,
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

struct Cancellable<'a, W> {
    inner: W,
    cancel: &'a AtomicBool,
}
impl<W: Write> Write for Cancellable<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            // write_all retries Interrupted; cancellation must be terminal.
            return Err(io::Error::other(
                "JSON export cancelled; destination unchanged",
            ));
        }
        self.inner.write(&bytes[..bytes.len().min(8192)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(super) fn begin(writer: &mut impl Write, columns: &[String]) -> io::Result<()> {
    let mut names = HashSet::new();
    if columns.iter().any(|name| !names.insert(name)) {
        return Err(invalid(
            "JSON object export requires unique column names. Alias duplicate columns in the query, or export as CSV.",
        ));
    }
    writer.write_all(b"[\n")
}

pub(super) fn write_row<'a>(
    writer: &mut impl Write,
    columns: &[String],
    values: impl IntoIterator<Item = io::Result<csv::Cell<'a>>>,
    cancel: &AtomicBool,
    first: bool,
) -> io::Result<()> {
    let mut writer = Cancellable {
        inner: writer,
        cancel,
    };
    if !first {
        writer.write_all(b",\n")?;
    }
    writer.write_all(b"  {")?;
    let mut values = values.into_iter();
    for (column, name) in columns.iter().enumerate() {
        let value = values
            .next()
            .ok_or_else(|| invalid("Result row does not match its column headers"))??;
        if column > 0 {
            writer.write_all(b", ")?;
        }
        serde_json::to_writer(&mut writer, name)?;
        writer.write_all(b": ")?;
        match value {
            csv::Cell::Null => writer.write_all(b"null")?,
            csv::Cell::Integer(value) => serde_json::to_writer(&mut writer, &value)?,
            csv::Cell::Real(value) if value.is_finite() => {
                serde_json::to_writer(&mut writer, &value)?
            }
            csv::Cell::Real(_) => {
                return Err(invalid(
                    "JSON cannot represent non-finite numbers. Cast these values to text in the query, or export as CSV.",
                ));
            }
            csv::Cell::Text(value) => serde_json::to_writer(&mut writer, value)?,
            csv::Cell::Blob(value) => serde_json::to_writer(&mut writer, value)?,
        }
    }
    if values.next().is_some() {
        return Err(invalid("Result row does not match its column headers"));
    }
    writer.write_all(b"}")
}

pub(super) fn export(path: &Path, grid: &Grid, cancel: &AtomicBool) -> io::Result<()> {
    csv::publish(path, cancel, |writer| {
        let mut writer = super::row_export::Writer::new(
            writer,
            &grid.columns,
            cancel,
            super::ExportFormat::Json,
        )?;
        for row in &grid.rows {
            writer.row(row.iter().map(|value| Ok(csv::Cell::from(value))))?;
        }
        writer.finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Value;
    #[test]
    fn cancellation_interrupts_serialization_inside_large_text() {
        struct Stop<'a> {
            written: usize,
            cancel: &'a AtomicBool,
        }
        impl Write for Stop<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.written += bytes.len();
                self.cancel.store(true, Ordering::Relaxed);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cancel = AtomicBool::new(false);
        let mut sink = Stop {
            written: 0,
            cancel: &cancel,
        };
        let mut writer = Cancellable {
            inner: &mut sink,
            cancel: &cancel,
        };
        assert!(serde_json::to_writer(&mut writer, &"x".repeat(1_000_000)).is_err());
        assert!(sink.written <= 8192);
    }

    #[test]
    fn preserves_types_unicode_and_binary_without_overwriting_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.json");
        let cancel = AtomicBool::new(false);
        let mut grid = Grid {
            columns: ["null", "int", "real", "text", "binary"]
                .map(str::to_owned)
                .to_vec(),
            rows: vec![vec![
                Value::Null,
                Value::Integer(i64::MAX),
                Value::Real(1.25),
                Value::Text("é\n\"\\\0".into()),
                Value::Blob(vec![0, 255]),
            ]],
            ..Default::default()
        };
        export(&path, &grid, &cancel).unwrap();
        let original = std::fs::read(&path).unwrap();
        let data: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert_eq!(data[0]["int"].as_i64(), Some(i64::MAX));
        assert_eq!(data[0]["text"], "é\n\"\\\0");
        assert_eq!(data[0]["binary"], serde_json::json!([0, 255]));
        assert_eq!(data[0]["real"], 1.25);
        assert!(data[0]["null"].is_null());
        grid.rows[0][2] = Value::Real(f64::NAN);
        assert!(export(&path, &grid, &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        grid.columns[1] = grid.columns[0].clone();
        assert!(export(&path, &grid, &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        cancel.store(true, Ordering::Relaxed);
        assert!(export(&path, &Grid::default(), &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
