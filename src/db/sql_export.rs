//! Streaming INSERT statements. Identifiers are quoted by the owning driver.
use super::csv::Cell;
use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy)]
pub(super) enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}
pub(super) struct Spec {
    pub dialect: Dialect,
    pub table: String,
    pub columns: Vec<String>,
    pub included: Vec<bool>,
    pub after: String,
}
impl Spec {
    pub fn begin(&self, output: &mut impl Write) -> io::Result<()> {
        writeln!(output, "-- Saved rows as SQL INSERT statements.")?;
        if matches!(self.dialect, Dialect::Mysql) {
            writeln!(
                output,
                "SET @rustable_export_sql_mode = @@SESSION.sql_mode;\nSET SESSION sql_mode = CONCAT_WS(',', @@SESSION.sql_mode, 'NO_AUTO_VALUE_ON_ZERO');"
            )?;
        }
        Ok(())
    }
    pub fn finish(&self, output: &mut impl Write) -> io::Result<()> {
        if matches!(self.dialect, Dialect::Mysql) {
            writeln!(output, "SET SESSION sql_mode = @rustable_export_sql_mode;")?;
        }
        output.write_all(self.after.as_bytes())
    }
    pub fn row<'a>(
        &self,
        output: &mut impl Write,
        values: impl IntoIterator<Item = io::Result<Cell<'a>>>,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        check(cancel)?;
        write!(output, "INSERT INTO {}", self.table)?;
        let columns = self
            .columns
            .iter()
            .zip(&self.included)
            .filter_map(|(name, yes)| yes.then_some(name.as_str()))
            .collect::<Vec<_>>();
        if columns.is_empty() {
            writeln!(
                output,
                "{};",
                if matches!(self.dialect, Dialect::Mysql) {
                    " () VALUES ()"
                } else {
                    " DEFAULT VALUES"
                }
            )?;
            return Ok(());
        }
        write!(
            output,
            " ({}){} VALUES (",
            columns.join(", "),
            if matches!(self.dialect, Dialect::Postgres) {
                " OVERRIDING SYSTEM VALUE"
            } else {
                ""
            }
        )?;
        let mut first = true;
        let mut width = 0;
        for (index, value) in values.into_iter().enumerate() {
            width += 1;
            let value = value?;
            if !self.included.get(index).copied().unwrap_or(false) {
                continue;
            }
            if !first {
                output.write_all(b", ")?;
            }
            first = false;
            match value {
                Cell::Null => output.write_all(b"NULL")?,
                Cell::Integer(v) => write!(output, "{v}")?,
                Cell::Real(v) if v.is_finite() => write!(output, "{v:?}")?,
                Cell::Real(v) if matches!(self.dialect, Dialect::Sqlite) && v.is_infinite() => {
                    output.write_all(if v.is_sign_negative() {
                        b"-9e999"
                    } else {
                        b"9e999"
                    })?
                }
                Cell::Real(_) => {
                    return Err(io::Error::other(
                        "Cannot export a non-finite number as an SQL literal",
                    ));
                }
                Cell::Blob(bytes) => {
                    output.write_all(b"X'")?;
                    hex(output, bytes, cancel)?;
                    output.write_all(b"'")?;
                }
                Cell::Text(value) | Cell::Decimal(value) => self.text(output, value, cancel)?,
            }
        }
        if width != self.included.len() {
            return Err(io::Error::other("SQL export column count changed"));
        }
        output.write_all(b");\n")
    }
    fn text(&self, output: &mut impl Write, value: &str, cancel: &AtomicBool) -> io::Result<()> {
        if (matches!(self.dialect, Dialect::Mysql)
            && (value.contains('\\') || value.contains('\u{1a}')))
            || value.contains('\0')
        {
            if matches!(self.dialect, Dialect::Postgres) {
                return Err(io::Error::other("PostgreSQL text cannot contain NUL"));
            }
            output.write_all(if matches!(self.dialect, Dialect::Mysql) {
                b"CONVERT(X'"
            } else {
                b"CAST(X'"
            })?;
            hex(output, value.as_bytes(), cancel)?;
            return output.write_all(if matches!(self.dialect, Dialect::Mysql) {
                b"' USING utf8mb4)"
            } else {
                b"' AS TEXT)"
            });
        }
        output.write_all(if matches!(self.dialect, Dialect::Postgres) {
            b"E'"
        } else {
            b"'"
        })?;
        for chunk in value.as_bytes().chunks(8192) {
            check(cancel)?;
            let mut start = 0;
            for (index, byte) in chunk.iter().enumerate() {
                if *byte == b'\'' || (*byte == b'\\' && matches!(self.dialect, Dialect::Postgres)) {
                    output.write_all(&chunk[start..index])?;
                    output.write_all(&[*byte, *byte])?;
                    start = index + 1;
                }
            }
            output.write_all(&chunk[start..])?;
        }
        output.write_all(b"'")
    }
}
fn check(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Export cancelled",
        ))
    } else {
        Ok(())
    }
}
fn hex(output: &mut impl Write, bytes: &[u8], cancel: &AtomicBool) -> io::Result<()> {
    const DIGITS: &[u8] = b"0123456789ABCDEF";
    let mut buffer = [0; 8192];
    for chunk in bytes.chunks(buffer.len() / 2) {
        check(cancel)?;
        for (i, byte) in chunk.iter().enumerate() {
            buffer[2 * i] = DIGITS[(byte >> 4) as usize];
            buffer[2 * i + 1] = DIGITS[(byte & 15) as usize];
        }
        output.write_all(&buffer[..chunk.len() * 2])?;
    }
    Ok(())
}
