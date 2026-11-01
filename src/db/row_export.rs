//! Shared streaming record sink for loaded grids and full-table exports.
use super::{ExportFormat, csv, json};
use std::{
    io::{self, Write},
    sync::atomic::AtomicBool,
};

pub(super) struct Writer<'a, W> {
    output: W,
    columns: &'a [String],
    cancel: &'a AtomicBool,
    format: ExportFormat,
    first: bool,
}
impl<'a, W: Write> Writer<'a, W> {
    pub(super) fn new(
        mut output: W,
        columns: &'a [String],
        cancel: &'a AtomicBool,
        format: ExportFormat,
    ) -> io::Result<Self> {
        match format {
            ExportFormat::Csv => csv::write_borrowed_row(
                &mut output,
                columns.iter().map(|name| Ok(csv::Cell::Text(name))),
                cancel,
            )?,
            ExportFormat::Json => json::begin(&mut output, columns)?,
        }
        Ok(Self {
            output,
            columns,
            cancel,
            format,
            first: true,
        })
    }
    pub(super) fn row<'v>(
        &mut self,
        values: impl IntoIterator<Item = io::Result<csv::Cell<'v>>>,
    ) -> io::Result<()> {
        match self.format {
            ExportFormat::Csv => csv::write_borrowed_row(&mut self.output, values, self.cancel)?,
            ExportFormat::Json => json::write_row(
                &mut self.output,
                self.columns,
                values,
                self.cancel,
                self.first,
            )?,
        }
        self.first = false;
        Ok(())
    }
    pub(super) fn finish(mut self) -> io::Result<()> {
        if matches!(self.format, ExportFormat::Json) {
            self.output.write_all(b"\n]\n")?;
        }
        Ok(())
    }
}
