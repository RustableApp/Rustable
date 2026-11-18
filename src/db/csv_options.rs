//! User-selected CSV formatting, encoded without buffering the full export.
use super::csv::{self, Cell};
use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Delimiter {
    Comma,
    Semicolon,
    Tab,
    Pipe,
}
impl Delimiter {
    pub fn byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Semicolon => b';',
            Self::Tab => b'\t',
            Self::Pipe => b'|',
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}
impl LineEnding {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::Cr => b"\r",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Quoting {
    IfNeeded,
    Always,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub null_to_empty: bool,
    pub flatten_lines: bool,
    pub header: bool,
    pub delimiter: Delimiter,
    pub quoting: Quoting,
    pub line_ending: LineEnding,
    pub decimal_comma: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            null_to_empty: true,
            flatten_lines: false,
            header: true,
            delimiter: Delimiter::Comma,
            quoting: Quoting::IfNeeded,
            line_ending: LineEnding::Lf,
            decimal_comma: false,
        }
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
fn field(
    output: &mut impl Write,
    bytes: &[u8],
    numeric: bool,
    options: Options,
    cancel: &AtomicBool,
) -> io::Result<()> {
    // Inspect in bounded chunks, including cancellation during very large fields.
    let mut quote = options.quoting == Quoting::Always
        || bytes.is_empty()
        || (!numeric && !options.null_to_empty && bytes == b"NULL");
    for chunk in bytes.chunks(8192) {
        check(cancel)?;
        quote |= chunk.iter().any(|b| {
            *b == b'"'
                || *b == options.delimiter.byte()
                || (!options.flatten_lines && matches!(b, b'\r' | b'\n'))
                || (numeric
                    && options.decimal_comma
                    && *b == b'.'
                    && options.delimiter == Delimiter::Comma)
        });
    }
    if quote {
        output.write_all(b"\"")?;
    }
    let mut previous_cr = false;
    let mut buffer = [0; 16384];
    for chunk in bytes.chunks(8192) {
        check(cancel)?;
        let mut length = 0;
        for &byte in chunk {
            if options.flatten_lines && byte == b'\n' && previous_cr {
                previous_cr = false;
                continue;
            }
            previous_cr = byte == b'\r';
            let byte = if options.flatten_lines && matches!(byte, b'\r' | b'\n') {
                b' '
            } else if numeric && options.decimal_comma && byte == b'.' {
                b','
            } else {
                byte
            };
            buffer[length] = byte;
            length += 1;
            if byte == b'"' {
                buffer[length] = byte;
                length += 1;
            }
        }
        output.write_all(&buffer[..length])?;
    }
    if quote {
        output.write_all(b"\"")?;
    }
    Ok(())
}
pub(super) fn row<'a>(
    output: &mut impl Write,
    values: impl IntoIterator<Item = io::Result<Cell<'a>>>,
    options: Options,
    cancel: &AtomicBool,
) -> io::Result<()> {
    check(cancel)?;
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 {
            output.write_all(&[options.delimiter.byte()])?;
        }
        match value? {
            Cell::Null => {
                if !options.null_to_empty {
                    output.write_all(b"NULL")?;
                }
            }
            Cell::Text(value) => field(output, value.as_bytes(), false, options, cancel)?,
            Cell::Decimal(value) => field(output, value.as_bytes(), true, options, cancel)?,
            Cell::Integer(value) => {
                field(output, value.to_string().as_bytes(), true, options, cancel)?
            }
            Cell::Real(value) => {
                field(output, value.to_string().as_bytes(), true, options, cancel)?
            }
            Cell::Blob(bytes) => {
                let quote = options.quoting == Quoting::Always || bytes.is_empty();
                if quote {
                    output.write_all(b"\"")?;
                }
                const HEX: &[u8] = b"0123456789abcdef";
                let mut buffer = [0; 8192];
                for chunk in bytes.chunks(4096) {
                    check(cancel)?;
                    for (i, b) in chunk.iter().enumerate() {
                        buffer[2 * i] = HEX[(b >> 4) as usize];
                        buffer[2 * i + 1] = HEX[(b & 15) as usize];
                    }
                    output.write_all(&buffer[..chunk.len() * 2])?;
                }
                if quote {
                    output.write_all(b"\"")?;
                }
            }
        }
    }
    output.write_all(options.line_ending.bytes())
}
pub(super) fn export(
    path: &std::path::Path,
    grid: &super::Grid,
    cancel: &AtomicBool,
    options: Options,
) -> io::Result<()> {
    csv::publish(path, cancel, |output| {
        let mut writer = super::row_export::Writer::new(
            output,
            &grid.columns,
            cancel,
            super::ExportFormat::CsvCustom(options),
        )?;
        grid.visit_rows_io(cancel, |values| {
            writer.row(values.iter().map(|v| Ok(Cell::from(v))))
        })?;
        writer.finish()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configurable_csv_is_parseable_and_preserves_numeric_precision() {
        let cancel = AtomicBool::new(false);
        for delimiter in [
            Delimiter::Comma,
            Delimiter::Semicolon,
            Delimiter::Tab,
            Delimiter::Pipe,
        ] {
            for quoting in [Quoting::IfNeeded, Quoting::Always] {
                for line_ending in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
                    let options = Options {
                        null_to_empty: false,
                        flatten_lines: true,
                        header: false,
                        delimiter,
                        quoting,
                        line_ending,
                        decimal_comma: true,
                    };
                    let mut output = Vec::new();
                    row(
                        &mut output,
                        [
                            Cell::Null,
                            Cell::Text("日本,;|\t\"quoted\"\r\nnext"),
                            Cell::Decimal("12345678901234567890.12345678"),
                            Cell::Real(1.25),
                            Cell::Text("v1.25"),
                            Cell::Blob(&[0, 255]),
                        ]
                        .into_iter()
                        .map(Ok),
                        options,
                        &cancel,
                    )
                    .unwrap();
                    assert!(output.ends_with(line_ending.bytes()));
                    let mut reader = ::csv::ReaderBuilder::new()
                        .has_headers(false)
                        .delimiter(delimiter.byte())
                        .from_reader(output.as_slice());
                    let values = reader.records().next().unwrap().unwrap();
                    assert_eq!(&values[0], "NULL");
                    assert_eq!(&values[1], "日本,;|\t\"quoted\" next");
                    assert_eq!(&values[2], "12345678901234567890,12345678");
                    assert_eq!(&values[3], "1,25");
                    assert_eq!(&values[4], "v1.25");
                    assert_eq!(&values[5], "00ff");
                }
            }
        }
    }
    #[test]
    fn header_toggle_null_empty_multiline_and_cancellation() {
        let cancel = AtomicBool::new(false);
        let columns = vec!["a,b".into(), "text".into()];
        for header in [true, false] {
            let mut output = Vec::new();
            let mut writer = super::super::row_export::Writer::new(
                &mut output,
                &columns,
                &cancel,
                super::super::ExportFormat::CsvCustom(Options {
                    header,
                    ..Default::default()
                }),
            )
            .unwrap();
            writer
                .row([Ok(Cell::Null), Ok(Cell::Text("a\r\nb"))])
                .unwrap();
            writer.finish().unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                if header {
                    "\"a,b\",text\n,\"a\r\nb\"\n"
                } else {
                    ",\"a\r\nb\"\n"
                }
            );
        }
        struct Stop<'a>(&'a AtomicBool);
        impl Write for Stop<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.store(true, Ordering::Relaxed);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let text = "long".repeat(10000);
        assert!(
            row(
                &mut Stop(&cancel),
                [Ok(Cell::Text(&text))],
                Options::default(),
                &cancel
            )
            .is_err()
        );
    }
}
