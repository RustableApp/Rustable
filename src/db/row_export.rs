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
    sql: Option<super::sql_export::Spec>,
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
            ExportFormat::CsvCustom(options) => {
                if options.header {
                    super::csv_options::row(
                        &mut output,
                        columns.iter().map(|name| Ok(csv::Cell::Text(name))),
                        options,
                        cancel,
                    )?;
                }
            }
            ExportFormat::Json => json::begin(&mut output, columns)?,
            ExportFormat::Sql => {
                return Err(io::Error::other("SQL export requires table metadata"));
            }
        }
        Ok(Self {
            output,
            columns,
            cancel,
            format,
            first: true,
            sql: None,
        })
    }
    pub(super) fn table(
        output: W,
        columns: &'a [String],
        cancel: &'a AtomicBool,
        format: ExportFormat,
        sql: impl FnOnce() -> super::sql_export::Spec,
    ) -> io::Result<Self> {
        if format != ExportFormat::Sql {
            return Self::new(output, columns, cancel, format);
        }
        let sql = sql();
        let mut output = output;
        sql.begin(&mut output)?;
        Ok(Self {
            output,
            columns,
            cancel,
            format,
            first: true,
            sql: Some(sql),
        })
    }
    pub(super) fn row<'v>(
        &mut self,
        values: impl IntoIterator<Item = io::Result<csv::Cell<'v>>>,
    ) -> io::Result<()> {
        match self.format {
            ExportFormat::Csv => csv::write_borrowed_row(&mut self.output, values, self.cancel)?,
            ExportFormat::CsvCustom(options) => {
                super::csv_options::row(&mut self.output, values, options, self.cancel)?
            }
            ExportFormat::Sql => {
                self.sql
                    .as_ref()
                    .unwrap()
                    .row(&mut self.output, values, self.cancel)?
            }
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
        if let Some(sql) = self.sql {
            sql.finish(&mut self.output)?;
        }
        Ok(())
    }
}
