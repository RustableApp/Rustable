//! Bounded loaded-result exports using the desktop streaming encoders.
use super::{ExportFormat, Grid, csv, row_export};
use std::{io::{self, Write}, sync::atomic::AtomicBool};
const LIMIT: usize = 16 * 1024 * 1024;
struct Bounded { bytes: Vec<u8>, limit: usize }
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("Browser downloads are limited to 16 MiB. Export fewer rows."));
        }
        self.bytes.extend_from_slice(bytes); Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
pub(crate) fn encode(grid: &Grid, format: ExportFormat) -> Result<Vec<u8>, String> {
    let mut output = Bounded { bytes: Vec::new(), limit: LIMIT };
    let cancel = AtomicBool::new(false);
    let mut writer = row_export::Writer::new(&mut output, &grid.columns, &cancel, format).map_err(|e| e.to_string())?;
    for row in &grid.rows { writer.row(row.iter().map(|value| Ok(csv::Cell::from(value)))).map_err(|e| e.to_string())?; }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(output.bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Value;
    #[test]
    fn exports_preserve_exact_integer_null_unicode_and_binary() {
        let grid = Grid { columns: vec!["id".into(), "text".into(), "empty".into(), "bytes".into()], rows: vec![vec![Value::Integer(9007199254740993), Value::Text("日本,\"quote\"".into()), Value::Null, Value::Blob(vec![0,255])]], ..Default::default() };
        let json = String::from_utf8(encode(&grid, ExportFormat::Json).unwrap()).unwrap();
        assert!(json.contains("9007199254740993"));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value[0]["id"].as_i64(), Some(9007199254740993));
        assert_eq!(value[0]["bytes"], serde_json::json!([0,255]));
        assert!(value[0]["empty"].is_null());
        let csv = String::from_utf8(encode(&grid, ExportFormat::Csv).unwrap()).unwrap();
        assert!(csv.contains("\"日本,\"\"quote\"\"\""));
    }
    #[test]
    fn refuses_oversized_output_without_partial_download() {
        let mut writer = Bounded { bytes: Vec::new(), limit: 3 };
        writer.write_all(b"abc").unwrap();
        assert!(writer.write_all(b"d").is_err());
        assert_eq!(writer.bytes, b"abc");
        let grid = Grid { columns: vec!["x".into(),"x".into()], ..Default::default() };
        assert!(encode(&grid, ExportFormat::Json).is_err());
    }
}
