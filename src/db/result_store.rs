//! Complete query results with bounded resident storage. All methods that read
//! or write a spill file are called on database/background workers, never the UI.
use super::{Grid, Value};
use std::sync::atomic::{AtomicBool, Ordering};

// Preserve the existing bound on synchronous chart/selection work. This is a
// spill threshold, never a native result limit.
const RESIDENT_ROWS: usize = 5_000;
pub const MAX_ROW_BYTES: usize = 64 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const PAGE_BYTES: usize = 4 * 1024 * 1024;

pub fn row_bytes(row: &[Value]) -> usize {
    std::mem::size_of::<Vec<Value>>().saturating_add(row.iter().fold(0usize, |sum, value| {
        sum.saturating_add(std::mem::size_of::<Value>())
            .saturating_add(match value {
                Value::Text(s) => s.capacity(),
                Value::Blob(b) => b.capacity(),
                _ => 0,
            })
    }))
}

pub struct Collector {
    rows: Vec<Vec<Value>>,
    bytes: usize,
    budget: usize,
    #[cfg(test)]
    fail_after: Option<usize>,
    #[cfg(not(target_arch = "wasm32"))]
    writer: Option<native::Writer>,
    #[cfg(target_arch = "wasm32")]
    truncated: bool,
}
impl Collector {
    pub fn new(budget: usize) -> Self {
        Self {
            rows: Vec::new(),
            bytes: 0,
            budget,
            #[cfg(test)]
            fail_after: None,
            #[cfg(not(target_arch = "wasm32"))]
            writer: None,
            #[cfg(target_arch = "wasm32")]
            truncated: false,
        }
    }
    #[cfg(test)]
    pub(crate) fn failing_after(rows: usize) -> Self {
        Self {
            fail_after: Some(rows),
            ..Self::new(0)
        }
    }
    pub fn push(&mut self, row: Vec<Value>) -> Result<(), String> {
        #[cfg(test)]
        if let Some(remaining) = &mut self.fail_after {
            if *remaining == 0 {
                return Err("Injected result storage failure".into());
            }
            *remaining -= 1;
        }
        let bytes = row_bytes(&row);
        if bytes > MAX_ROW_BYTES {
            return Err("A result row exceeds the 64 MiB per-row safety limit".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.truncated
                || self.rows.len() >= RESIDENT_ROWS
                || bytes > self.budget.saturating_sub(self.bytes)
            {
                self.truncated = true;
                return Ok(());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.writer.is_none()
                && (self.rows.len() >= RESIDENT_ROWS
                    || bytes > self.budget.saturating_sub(self.bytes))
            {
                let mut writer = native::Writer::new()?;
                for previous in &self.rows {
                    writer.push(previous)?;
                }
                self.rows = Vec::new();
                self.bytes = 0;
                self.writer = Some(writer);
            }
            if let Some(writer) = &mut self.writer {
                return writer.push(&row);
            }
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.rows.push(row);
        Ok(())
    }
    pub fn finish(self, grid: &mut Grid) -> Result<(), String> {
        grid.rows = self.rows;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(writer) = self.writer {
            grid.stored = Some(writer.finish()?);
        }
        #[cfg(target_arch = "wasm32")]
        {
            grid.truncated = self.truncated;
        }
        Ok(())
    }
}

impl Grid {
    pub fn row_count(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(stored) = &self.stored {
            return stored.len();
        }
        self.rows.len()
    }
    pub fn is_stored(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.stored.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
    pub fn visit_rows_io(
        &self,
        cancel: &AtomicBool,
        mut visit: impl FnMut(&[Value]) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let mut failure = None;
        let result = self.visit_rows(cancel, |_, row| {
            if let Err(error) = visit(row) {
                failure = Some(error);
                return Ok(false);
            }
            Ok(true)
        });
        if let Some(error) = failure {
            return Err(error);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Export cancelled; destination unchanged",
            ));
        }
        result.map_err(std::io::Error::other)
    }
    /// Worker-only streaming traversal. Returning false stops without an error.
    pub fn visit_rows(
        &self,
        cancel: &AtomicBool,
        mut visit: impl FnMut(usize, &[Value]) -> Result<bool, String>,
    ) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(stored) = &self.stored {
            return stored.visit(cancel, visit);
        }
        for (index, row) in self.rows.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            if !visit(index, row)? {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use bincode::Options;
    use rusqlite::{Connection, OpenFlags};
    use std::sync::{Arc, OnceLock, mpsc};
    use tempfile::TempPath;

    pub type WindowRows = (usize, Vec<Vec<Value>>, Vec<bool>);

    fn decode(bytes: &[u8]) -> Result<Vec<Value>, String> {
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(MAX_ROW_BYTES as u64)
            .reject_trailing_bytes()
            .deserialize(bytes)
            .map_err(|e| e.to_string())
    }

    fn cleanup() -> Result<mpsc::Sender<TempPath>, String> {
        static CLEANUP: OnceLock<mpsc::Sender<TempPath>> = OnceLock::new();
        if let Some(sender) = CLEANUP.get() {
            return Ok(sender.clone());
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("result-file-cleanup".into())
            .spawn(move || {
                for path in receiver {
                    drop(path);
                }
            })
            .map_err(|e| e.to_string())?;
        // A concurrent first creator may have installed another working sender.
        let _ = CLEANUP.set(sender.clone());
        Ok(sender)
    }

    #[derive(Debug)]
    pub struct StoredRows {
        path: Option<TempPath>,
        count: usize,
        cleanup: mpsc::Sender<TempPath>,
    }
    impl Drop for StoredRows {
        fn drop(&mut self) {
            if let Some(path) = self.path.take() {
                let _ = self.cleanup.send(path);
            }
        }
    }
    impl StoredRows {
        #[cfg(test)]
        pub(super) fn path(&self) -> &std::path::Path {
            self.path.as_ref().unwrap().as_ref()
        }
        pub fn len(&self) -> usize {
            self.count
        }
        fn open(&self) -> Result<Connection, String> {
            let conn = Connection::open_with_flags(
                self.path.as_ref().unwrap(),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA cache_size=-2048; PRAGMA mmap_size=0;")
                .map_err(|e| e.to_string())?;
            Ok(conn)
        }
        pub fn visit(
            &self,
            cancel: &AtomicBool,
            mut visit: impl FnMut(usize, &[Value]) -> Result<bool, String>,
        ) -> Result<(), String> {
            let conn = self.open()?;
            let mut statement = conn
                .prepare("SELECT id, value FROM result_rows ORDER BY id")
                .map_err(|e| e.to_string())?;
            let mut rows = statement.query([]).map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let index = row.get::<_, i64>(0).map_err(|e| e.to_string())? as usize;
                let bytes = row
                    .get_ref(1)
                    .map_err(|e| e.to_string())?
                    .as_blob()
                    .map_err(|e| e.to_string())?;
                let values: Vec<Value> = decode(bytes)?;
                if !visit(index, &values)? {
                    break;
                }
            }
            Ok(())
        }
        /// Bounded display window. Oversized rows retain compact display values;
        /// selecting one loads its exact values separately on the same worker.
        pub fn window(
            &self,
            position: usize,
            full_row: Option<usize>,
            cancel: &AtomicBool,
        ) -> Result<WindowRows, String> {
            let conn = self.open()?;
            let mut statement = conn
                .prepare("SELECT r.id, CASE WHEN r.id=?2 THEN r.value ELSE COALESCE(p.value,r.value) END, r.id != ?2 AND p.value IS NOT NULL FROM result_rows r LEFT JOIN result_previews p ON p.id=r.id WHERE r.id >= ?1 ORDER BY r.id LIMIT 256")
                .map_err(|e| e.to_string())?;
            let mut rows = statement
                .query(rusqlite::params![
                    i64::try_from(position).map_err(|e| e.to_string())?,
                    full_row
                        .map(|v| i64::try_from(v).unwrap_or(i64::MAX))
                        .unwrap_or(-1)
                ])
                .map_err(|e| e.to_string())?;
            let mut values = Vec::new();
            let mut previews = Vec::new();
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let bytes = row
                    .get_ref(1)
                    .map_err(|e| e.to_string())?
                    .as_blob()
                    .map_err(|e| e.to_string())?;
                let cells = decode(bytes)?;
                let preview = row.get::<_, bool>(2).map_err(|e| e.to_string())?;
                values.push(cells);
                previews.push(preview);
            }
            Ok((position, values, previews))
        }

        /// Legacy bounded-page regression fixture.
        #[cfg(test)]
        pub fn page(
            &self,
            position: usize,
            backwards: bool,
            cancel: &AtomicBool,
        ) -> Result<(usize, Vec<Vec<Value>>), String> {
            let conn = self.open()?;
            let sql = if backwards {
                "SELECT id,value FROM result_rows WHERE id < ?1 ORDER BY id DESC LIMIT 500"
            } else {
                "SELECT id,value FROM result_rows WHERE id >= ?1 ORDER BY id LIMIT 500"
            };
            let mut statement = conn.prepare(sql).map_err(|e| e.to_string())?;
            let mut rows = statement
                .query([i64::try_from(position).map_err(|e| e.to_string())?])
                .map_err(|e| e.to_string())?;
            let mut result = Vec::new();
            let mut start = position;
            let mut used = 0usize;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                let index = row.get::<_, i64>(0).map_err(|e| e.to_string())? as usize;
                let bytes = row
                    .get_ref(1)
                    .map_err(|e| e.to_string())?
                    .as_blob()
                    .map_err(|e| e.to_string())?;
                if !result.is_empty() && bytes.len() > PAGE_BYTES.saturating_sub(used) {
                    break;
                }
                let values: Vec<Value> = decode(bytes)?;
                let resident = row_bytes(&values);
                if !result.is_empty() && resident > PAGE_BYTES.saturating_sub(used) {
                    break;
                }
                used = used.saturating_add(resident);
                if result.is_empty() || backwards {
                    start = index;
                }
                result.push(values);
            }
            if backwards {
                result.reverse();
            }
            Ok((start, result))
        }
    }

    fn compact_row(row: &[Value]) -> Option<Vec<Value>> {
        if row_bytes(row) <= 32 * 1024 {
            return None;
        }
        let budget = (32 * 1024 / row.len().max(1)).max(1);
        let mut changed = false;
        let cells = row
            .iter()
            .map(|value| match value {
                Value::Text(text) if text.len() > budget => {
                    let mut end = budget;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    changed = true;
                    Value::Text(format!("{}…", &text[..end]))
                }
                Value::Blob(blob) if blob.len() > budget => {
                    changed = true;
                    Value::Text(format!("BLOB · {} bytes", blob.len()))
                }
                _ => value.clone(),
            })
            .collect();
        changed.then_some(cells)
    }

    pub struct Writer {
        conn: Connection,
        store: StoredRows,
        encoded: Vec<u8>,
    }
    impl Writer {
        pub fn new() -> Result<Self, String> {
            let cleanup = cleanup()?;
            let path = tempfile::Builder::new()
                .prefix("rustable-result-")
                .suffix(".sqlite")
                .tempfile()
                .map_err(|e| e.to_string())?
                .into_temp_path();
            let conn = Connection::open(&path).map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-2048; PRAGMA mmap_size=0; CREATE TABLE result_rows(id INTEGER PRIMARY KEY, value BLOB NOT NULL); CREATE TABLE result_previews(id INTEGER PRIMARY KEY, value BLOB NOT NULL); BEGIN;").map_err(|e| e.to_string())?;
            Ok(Self {
                conn,
                store: StoredRows {
                    path: Some(path),
                    count: 0,
                    cleanup,
                },
                encoded: Vec::new(),
            })
        }
        pub fn push(&mut self, row: &[Value]) -> Result<(), String> {
            self.encoded.clear();
            bincode::serialize_into(&mut self.encoded, row).map_err(|e| e.to_string())?;
            self.conn
                .prepare_cached("INSERT INTO result_rows VALUES (?1,?2)")
                .map_err(|e| e.to_string())?
                .execute(rusqlite::params![
                    i64::try_from(self.store.count).map_err(|e| e.to_string())?,
                    &self.encoded
                ])
                .map_err(|e| e.to_string())?;
            if let Some(preview) = compact_row(row) {
                self.encoded.clear();
                bincode::serialize_into(&mut self.encoded, &preview).map_err(|e| e.to_string())?;
                self.conn
                    .prepare_cached("INSERT INTO result_previews VALUES (?1,?2)")
                    .map_err(|e| e.to_string())?
                    .execute(rusqlite::params![
                        i64::try_from(self.store.count).map_err(|e| e.to_string())?,
                        &self.encoded
                    ])
                    .map_err(|e| e.to_string())?;
            }
            self.store.count += 1;
            // Do not retain a rare large-cell scratch allocation for the whole run.
            if self.encoded.capacity() > PAGE_BYTES {
                self.encoded = Vec::new();
            }
            Ok(())
        }
        pub fn finish(self) -> Result<Arc<StoredRows>, String> {
            self.conn
                .execute_batch("COMMIT")
                .map_err(|e| e.to_string())?;
            drop(self.conn);
            Ok(Arc::new(self.store))
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::StoredRows;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    pub(crate) fn fixture(count: usize) -> Grid {
        let mut collector = Collector::new(1024);
        for index in 0..count {
            collector
                .push(vec![
                    Value::Integer(index as i64),
                    Value::Text(format!("row-{index}")),
                ])
                .unwrap();
        }
        let mut grid = Grid {
            columns: vec!["id".into(), "label".into()],
            ..Default::default()
        };
        collector.finish(&mut grid).unwrap();
        grid
    }

    #[test]
    fn files_live_with_snapshots_and_cleanup_after_last_owner() {
        let grid = fixture(100);
        let store = grid.stored.as_ref().unwrap().clone();
        let path = store.path().to_owned();
        drop(grid);
        assert!(path.exists());
        assert_eq!(
            store.page(99, false, &AtomicBool::new(false)).unwrap().1[0][0],
            Value::Integer(99)
        );
        drop(store);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!path.exists());
    }

    #[test]
    fn viewport_previews_bound_wide_rows_and_selected_values_remain_exact() {
        let mut collector = Collector::new(0);
        for _ in 0..40 {
            collector
                .push(vec![Value::Blob(vec![42; 1024 * 1024])])
                .unwrap();
        }
        let mut grid = Grid::default();
        collector.finish(&mut grid).unwrap();
        let stored = grid.stored.as_ref().unwrap();
        let cancel = AtomicBool::new(false);
        let (_, rows, previews) = stored.window(0, None, &cancel).unwrap();
        assert_eq!(rows.len(), 40);
        assert!(previews.iter().all(|preview| *preview));
        assert!(rows.iter().map(|row| row_bytes(row)).sum::<usize>() < 32 * 1024);
        let (_, rows, previews) = stored.window(0, Some(39), &cancel).unwrap();
        assert!(!previews[39]);
        assert_eq!(rows[39][0], Value::Blob(vec![42; 1024 * 1024]));
        assert!(rows.iter().map(|row| row_bytes(row)).sum::<usize>() < 2 * 1024 * 1024);
    }

    #[test]
    fn complete_spill_pages_export_search_and_cancellation() {
        let grid = fixture(10021);
        assert!(grid.rows.is_empty());
        assert_eq!(grid.row_count(), 10021);
        assert!(!grid.truncated);
        let cancel = AtomicBool::new(false);
        let stored = grid.stored.as_ref().unwrap();
        let (offset, rows) = stored.page(5000, false, &cancel).unwrap();
        assert_eq!(offset, 5000);
        assert_eq!(rows.len(), 500);
        assert_eq!(rows[499][0], Value::Integer(5499));
        let (offset, rows) = stored.page(5000, true, &cancel).unwrap();
        assert_eq!(offset, 4500);
        assert_eq!(rows.last().unwrap()[0], Value::Integer(4999));
        let (offset, rows) = stored.page(grid.row_count(), true, &cancel).unwrap();
        assert_eq!(offset, 9521);
        assert_eq!(rows.last().unwrap()[0], Value::Integer(10020));
        let mut count = 0;
        grid.visit_rows(&cancel, |index, values| {
            assert_eq!(values[0], Value::Integer(index as i64));
            count += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(count, 10021);
        let mut csv = Vec::new();
        crate::db::csv::encode(&mut csv, &grid, &cancel).unwrap();
        assert_eq!(csv.iter().filter(|&&b| b == b'\n').count(), 10022);
        assert!(String::from_utf8(csv).unwrap().contains("row-10020"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.json");
        crate::db::json::export(&path, &grid, &cancel).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 10021);
        assert_eq!(json[10020]["id"], 10020);
        cancel.store(true, Ordering::Relaxed);
        assert!(stored.page(0, false, &cancel).is_err());
        assert!(
            grid.visit_rows(&cancel, |_, _| panic!("cancelled visit"))
                .is_err()
        );
    }

    #[test]
    fn spill_preserves_exact_values_and_bounded_wide_pages() {
        let mut collector = Collector::new(0);
        collector
            .push(vec![
                Value::Null,
                Value::Integer(i64::MAX),
                Value::Real(f64::INFINITY),
                Value::Real(f64::NAN),
                Value::Text("日本\0🦀".into()),
                Value::Blob(vec![0, 255]),
            ])
            .unwrap();
        let mut grid = Grid::default();
        collector.finish(&mut grid).unwrap();
        let (_, rows) = grid
            .stored
            .as_ref()
            .unwrap()
            .page(0, false, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(rows[0][1], Value::Integer(i64::MAX));
        assert!(matches!(rows[0][3], Value::Real(v) if v.is_nan()));
        assert_eq!(rows[0][4], Value::Text("日本\0🦀".into()));
        assert_eq!(rows[0][5], Value::Blob(vec![0, 255]));
        let mut collector = Collector::new(0);
        for _ in 0..8 {
            collector
                .push(vec![Value::Blob(vec![42; 1024 * 1024])])
                .unwrap();
        }
        collector.finish(&mut grid).unwrap();
        let store = grid.stored.as_ref().unwrap();
        let (_, first) = store.page(0, false, &AtomicBool::new(false)).unwrap();
        assert!(first.len() <= 4);
        let (offset, previous) = store
            .page(first.len(), true, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(offset, 0);
        assert_eq!(previous.len(), first.len());
    }
}
