//! Streaming CSV encoding and atomic publication of complete result snapshots.
use super::{Grid, Value};
use std::{
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Export cancelled; destination unchanged",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn export(path: &Path, grid: &Grid, cancel: &AtomicBool) -> io::Result<()> {
    publish(path, cancel, |writer| encode(writer, grid, cancel))
}

pub(super) fn publish(
    path: &Path,
    cancel: &AtomicBool,
    encode: impl FnOnce(&mut io::BufWriter<&mut std::fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    check_cancel(cancel)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Same-directory staging ensures publication stays on the same filesystem.
    // NamedTempFile removes the staging file on every error path.
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = io::BufWriter::new(staged.as_file_mut());
        encode(&mut writer)?;
        writer.flush()?;
    }
    staged.as_file().sync_all()?;
    check_cancel(cancel)?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn quoted(writer: &mut impl Write, bytes: &[u8], cancel: &AtomicBool) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for chunk in bytes.chunks(8192) {
        check_cancel(cancel)?;
        let mut start = 0;
        for (index, &byte) in chunk.iter().enumerate() {
            if byte == b'"' {
                writer.write_all(&chunk[start..index])?;
                writer.write_all(b"\"\"")?;
                start = index + 1;
            }
        }
        writer.write_all(&chunk[start..])?;
    }
    writer.write_all(b"\"")
}

pub(super) fn encode(writer: &mut impl Write, grid: &Grid, cancel: &AtomicBool) -> io::Result<()> {
    check_cancel(cancel)?;
    for (index, column) in grid.columns.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        quoted(writer, column.as_bytes(), cancel)?;
    }
    writer.write_all(b"\n")?;
    grid.visit_rows_io(cancel, |row| write_row(writer, row, cancel))?;
    Ok(())
}

pub(super) fn write_row(
    writer: &mut impl Write,
    row: &[Value],
    cancel: &AtomicBool,
) -> io::Result<()> {
    write_borrowed_row(
        writer,
        row.iter().map(|value| {
            Ok(match value {
                Value::Null => Cell::Null,
                Value::Integer(value) => Cell::Integer(*value),
                Value::Real(value) => Cell::Real(*value),
                Value::Text(value) => Cell::Text(value),
                Value::Blob(value) => Cell::Blob(value),
            })
        }),
        cancel,
    )
}

pub(super) enum Cell<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Blob(&'a [u8]),
}

impl<'a> From<&'a Value> for Cell<'a> {
    fn from(value: &'a Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Integer(v) => Self::Integer(*v),
            Value::Real(v) => Self::Real(*v),
            Value::Text(v) => Self::Text(v),
            Value::Blob(v) => Self::Blob(v),
        }
    }
}

pub(super) fn write_borrowed_row<'a>(
    writer: &mut impl Write,
    values: impl IntoIterator<Item = io::Result<Cell<'a>>>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancel)?;
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        match value? {
            Cell::Null => {}
            Cell::Text(text) => quoted(writer, text.as_bytes(), cancel)?,
            Cell::Integer(value) => quoted(writer, value.to_string().as_bytes(), cancel)?,
            Cell::Real(value) => quoted(writer, value.to_string().as_bytes(), cancel)?,
            Cell::Blob(bytes) => {
                const HEX: &[u8] = b"0123456789abcdef";
                let mut buffer = [0u8; 8192];
                writer.write_all(b"\"")?;
                for chunk in bytes.chunks(buffer.len() / 2) {
                    check_cancel(cancel)?;
                    for (i, byte) in chunk.iter().enumerate() {
                        buffer[i * 2] = HEX[(byte >> 4) as usize];
                        buffer[i * 2 + 1] = HEX[(byte & 15) as usize];
                    }
                    writer.write_all(&buffer[..chunk.len() * 2])?;
                }
                writer.write_all(b"\"")?;
            }
        }
    }
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_preserves_quotes_newlines_unicode_null_empty_and_binary() {
        let grid = Grid {
            columns: vec![
                "a\"b".into(),
                "text".into(),
                "null".into(),
                "empty".into(),
                "blob".into(),
            ],
            rows: vec![vec![
                Value::Integer(-42),
                Value::Text("日本,\r\n\"quoted\"".into()),
                Value::Null,
                Value::Text(String::new()),
                Value::Blob(vec![0, 15, 255]),
            ]],
            ..Default::default()
        };
        let mut output = Vec::new();
        encode(&mut output, &grid, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\"a\"\"b\",\"text\",\"null\",\"empty\",\"blob\"\n\"-42\",\"日本,\r\n\"\"quoted\"\"\",,\"\",\"000fff\"\n"
        );
    }

    #[test]
    fn cancelled_export_preserves_destination_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.csv");
        std::fs::write(&path, b"original").unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            export(&path, &Grid::default(), &cancel).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        cancel.store(false, Ordering::Relaxed);
        let grid = Grid {
            columns: vec!["value".into()],
            rows: vec![vec![Value::Integer(1)]],
            ..Default::default()
        };
        export(&path, &grid, &cancel).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"\"value\"\n\"1\"\n");
        // Publishing over a directory fails after encoding; staging is still removed.
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(export(&blocked, &grid, &cancel).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn cancellation_is_observed_inside_large_cells() {
        struct CancellingWriter<'a> {
            cancel: &'a AtomicBool,
            written: usize,
        }
        impl Write for CancellingWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.written += bytes.len();
                if self.written >= 8192 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for value in [
            Value::Text("日本\"".repeat(100_000)),
            Value::Blob(vec![255; 1_000_000]),
        ] {
            let cancel = AtomicBool::new(false);
            let mut writer = CancellingWriter {
                cancel: &cancel,
                written: 0,
            };
            let grid = Grid {
                columns: vec!["value".into()],
                rows: vec![vec![value]],
                ..Default::default()
            };
            assert_eq!(
                encode(&mut writer, &grid, &cancel).unwrap_err().kind(),
                io::ErrorKind::Interrupted
            );
            assert!(writer.written < 32_768);
        }
    }
}
