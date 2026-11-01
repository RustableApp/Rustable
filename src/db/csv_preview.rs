//! Bounded, cancellable UTF-8 CSV preview. Parsing runs only on the worker.
use std::{
    cell::Cell,
    io::{self, Read},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::File, io::Write};
const RECORD_BYTES: usize = 2 * 1024 * 1024;
const PREVIEW_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub delimiter: u8,
    pub quote: u8,
    pub header: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
            header: true,
        }
    }
}
/// Immutable source snapshot retained across parser changes and eventual import.
#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
pub struct Source {
    storage: Storage,
    path: PathBuf,
    pub bytes: u64,
}
#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
enum Storage {
    #[cfg(not(target_arch = "wasm32"))]
    File(tempfile::NamedTempFile),
    #[cfg(any(test, target_arch = "wasm32"))]
    Memory(Vec<u8>),
}
impl Source {
    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > 8 * 1024 * 1024 { return Err("Browser CSV files are limited to 8 MiB".into()); }
        Ok(Self { path, bytes: bytes.len() as u64, storage: Storage::Memory(bytes) })
    }
    #[cfg(target_arch = "wasm32")]
    fn capture(_path: &Path, _cancel: &AtomicBool) -> Result<Self, String> {
        Err("Choose a CSV file using the browser file picker".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn capture(path: &Path, cancel: &AtomicBool) -> Result<Self, String> {
        let mut input = File::open(path).map_err(|e| e.to_string())?;
        let before = input.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() {
            return Err("Choose a regular CSV file".into());
        }
        let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
        let bytes = copy_snapshot(&mut input, file.as_file_mut(), before.len(), cancel)?;
        let after = input.metadata().map_err(|e| e.to_string())?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("CSV file changed while reading; choose it again".into());
        }
        Ok(Self {
            storage: Storage::File(file),
            path: path.to_owned(),
            bytes,
        })
    }
    fn open(&self) -> Result<Box<dyn Read + '_>, String> {
        match &self.storage {
            #[cfg(not(target_arch = "wasm32"))]
            Storage::File(file) => file.reopen().map(|file| Box::new(file) as Box<dyn Read>).map_err(|e| e.to_string()),
            #[cfg(any(test, target_arch = "wasm32"))]
            Storage::Memory(bytes) => {
                if bytes.len() > 8 * 1024 * 1024 || bytes.len() as u64 != self.bytes {
                    return Err("Invalid browser CSV snapshot".into());
                }
                Ok(Box::new(io::Cursor::new(bytes)))
            }
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn copy_snapshot(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected: u64,
    cancel: &AtomicBool,
) -> Result<u64, String> {
    let mut buffer = [0_u8; 65536];
    let mut copied = 0_u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("CSV preview cancelled".into());
        }
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > expected {
            return Err("CSV file changed while reading; choose it again".into());
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
    }
    if copied != expected {
        return Err("CSV file changed while reading; choose it again".into());
    }
    Ok(copied)
}

#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
pub struct Preview {
    pub source: Option<Arc<Source>>,
    pub options: Options,
    pub path: PathBuf,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub more: bool,
}
struct Limited<'a, R> {
    inner: R,
    remaining: Rc<Cell<usize>>,
    cancel: &'a AtomicBool,
}
impl<R: Read> Read for Limited<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("CSV preview cancelled"));
        }
        let max = bytes.len().min(self.remaining.get());
        if max == 0 {
            return Err(io::Error::other(
                "CSV record exceeds the 2 MiB preview limit",
            ));
        }
        let read = self.inner.read(&mut bytes[..max])?;
        self.remaining.set(self.remaining.get() - read);
        Ok(read)
    }
}

pub fn load(
    path: &Path,
    options: Options,
    source: Option<Arc<Source>>,
    cancel: &AtomicBool,
) -> Result<Preview, String> {
    let source = match source {
        Some(source) if source.path == path => source,
        Some(_) => return Err("CSV source does not match this preview".into()),
        None => Arc::new(Source::capture(path, cancel)?),
    };
    let mut preview = parse(source.open()?, path, options, cancel)?;
    preview.source = Some(source);
    Ok(preview)
}

/// Visit every record in the captured file without retaining the entire dataset.
pub(super) fn visit(
    source: &Source,
    options: Options,
    width: usize,
    cancel: &AtomicBool,
    mut row: impl FnMut(&::csv::StringRecord) -> Result<(), String>,
) -> Result<u64, String> {
    let remaining = Rc::new(Cell::new(RECORD_BYTES));
    let input = Limited {
        inner: source.open()?,
        remaining: remaining.clone(),
        cancel,
    };
    let mut reader = ::csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(options.delimiter)
        .quote(options.quote)
        .buffer_capacity(8192)
        .from_reader(input);
    let mut record = ::csv::StringRecord::new();
    let mut first = true;
    let mut count = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("CSV import cancelled".into());
        }
        remaining.set(RECORD_BYTES);
        if !reader.read_record(&mut record).map_err(|e| e.to_string())? {
            break;
        }
        if record.len() != width || record.len() > 2048 || record.as_slice().len() > RECORD_BYTES {
            return Err(format!(
                "CSV record {} has an invalid width or exceeds the 2 MiB limit",
                count + 1
            ));
        }
        if first {
            first = false;
            if options.header {
                continue;
            }
        }
        row(&record).map_err(|error| format!("CSV data row {}: {error}", count + 1))?;
        count += 1;
    }
    Ok(count)
}

fn parse(
    reader: impl Read,
    path: &Path,
    options: Options,
    cancel: &AtomicBool,
) -> Result<Preview, String> {
    if !options.delimiter.is_ascii()
        || !options.quote.is_ascii()
        || options.delimiter == options.quote
        || [b'\r', b'\n', 0].contains(&options.delimiter)
        || [b'\r', b'\n', 0].contains(&options.quote)
    {
        return Err("Choose distinct ASCII delimiter and quote characters".into());
    }
    let remaining = Rc::new(Cell::new(RECORD_BYTES));
    let source = Limited {
        inner: reader,
        remaining: remaining.clone(),
        cancel,
    };
    let mut reader = ::csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(options.delimiter)
        .quote(options.quote)
        .buffer_capacity(8192)
        .from_reader(source);
    let mut record = ::csv::StringRecord::new();
    let mut preview = Preview {
        source: None,
        options,
        path: path.to_owned(),
        columns: Vec::new(),
        rows: Vec::new(),
        more: false,
    };
    let mut used = 0;
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("CSV preview cancelled".into());
        }
        remaining.set(RECORD_BYTES);
        if !reader.read_record(&mut record).map_err(|e| e.to_string())? {
            break;
        }
        if record.len() > 2048 || record.as_slice().len() > RECORD_BYTES {
            return Err("CSV record exceeds the preview limit (2 MiB or 2,048 columns)".into());
        }
        if first {
            first = false;
            if options.header {
                preview.columns = record.iter().map(str::to_owned).collect();
                used += record.as_slice().len();
                continue;
            }
            preview.columns = (1..=record.len()).map(|n| format!("Column {n}")).collect();
        }
        if preview.rows.len() == 100 || used + record.as_slice().len() > PREVIEW_BYTES {
            preview.more = true;
            break;
        }
        used += record.as_slice().len();
        preview
            .rows
            .push(record.iter().map(str::to_owned).collect());
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview(text: &[u8], options: Options) -> Result<Preview, String> {
        parse(
            text,
            Path::new("test.csv"),
            options,
            &AtomicBool::new(false),
        )
    }
    #[test]
    fn memory_snapshot_reparses_and_visits_all_rows_without_filesystem_access() {
        let path = PathBuf::from("browser.csv");
        let text = format!("id;name\n{}", (1..=150).map(|n| format!("{n};日本\n")).collect::<String>());
        let source = Arc::new(Source::from_bytes(path.clone(), text.into_bytes()).unwrap());
        let cancel = AtomicBool::new(false);
        let options = Options { delimiter: b';', ..Default::default() };
        let preview = load(&path, options, Some(source.clone()), &cancel).unwrap();
        assert_eq!(preview.rows.len(), 100);
        assert!(preview.more);
        assert_eq!(preview.rows[0], ["1", "日本"]);
        let mut visited = 0;
        assert_eq!(visit(&source, options, 2, &cancel, |row| { visited += 1; assert_eq!(row.get(1), Some("日本")); Ok(()) }).unwrap(), 150);
        assert_eq!(visited, 150);
        assert!(load(Path::new("different.csv"), options, Some(source), &cancel).is_err());
        assert!(Source::from_bytes(path, vec![0; 8 * 1024 * 1024 + 1]).is_err());
    }
    #[test]
    fn snapshot_survives_source_replacement_and_is_removed_when_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.csv");
        std::fs::write(&path, "id,name\n1,original\n").unwrap();
        let cancel = AtomicBool::new(false);
        let preview = load(&path, Options::default(), None, &cancel).unwrap();
        let source = preview.source.clone().unwrap();
        let Storage::File(file) = &source.storage else { panic!("expected file snapshot") };
        let snapshot_path = file.path().to_owned();
        std::fs::write(&path, "id,name\n2,replaced\n").unwrap();
        let reloaded = load(
            &path,
            Options {
                header: false,
                ..Options::default()
            },
            Some(source.clone()),
            &cancel,
        )
        .unwrap();
        assert_eq!(reloaded.rows[1], ["1", "original"]);
        assert!(
            load(
                &dir.path().join("other.csv"),
                Options::default(),
                Some(source.clone()),
                &cancel
            )
            .is_err()
        );
        drop(preview);
        drop(source);
        assert!(snapshot_path.exists());
        drop(reloaded);
        assert!(!snapshot_path.exists());
        assert_eq!(
            load(&path, Options::default(), None, &cancel).unwrap().rows[0],
            ["2", "replaced"]
        );
    }

    #[test]
    fn snapshot_copy_checks_cancellation_and_length_changes() {
        struct CancelWriter<'a>(&'a AtomicBool);
        impl Write for CancelWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.store(true, Ordering::Relaxed);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let cancel = AtomicBool::new(false);
        let data = vec![b'x'; 200_000];
        assert!(
            copy_snapshot(
                &mut data.as_slice(),
                &mut CancelWriter(&cancel),
                data.len() as u64,
                &cancel
            )
            .unwrap_err()
            .contains("cancelled")
        );
        cancel.store(false, Ordering::Relaxed);
        assert!(copy_snapshot(&mut &b"abc"[..], &mut io::sink(), 2, &cancel).is_err());
        assert!(copy_snapshot(&mut &b"abc"[..], &mut io::sink(), 4, &cancel).is_err());
    }

    #[test]
    fn parses_multiline_quotes_bom_and_custom_delimiter() {
        let p = preview(
            "\u{feff}id,name\r\n1,\"日本,\r\n\"\"quoted\"\"\"\r\n".as_bytes(),
            Options::default(),
        )
        .unwrap();
        assert_eq!(p.columns, ["id", "name"]);
        assert_eq!(p.rows[0], ["1", "日本,\r\n\"quoted\""]);
        let p = preview(
            b"1;'hello;there'\n",
            Options {
                delimiter: b';',
                quote: b'\'',
                header: false,
            },
        )
        .unwrap();
        assert_eq!(p.columns, ["Column 1", "Column 2"]);
        assert_eq!(p.rows[0], ["1", "hello;there"]);
    }
    #[test]
    fn bounds_preview_and_rejects_bad_records_and_cancel() {
        let data = format!("id\n{}", "1\n".repeat(101));
        let p = preview(data.as_bytes(), Options::default()).unwrap();
        assert_eq!(p.rows.len(), 100);
        assert!(p.more);
        assert!(preview(b"a,b\n1\n", Options::default()).is_err());
        assert!(preview(b"a\n\xff", Options::default()).is_err());
        assert!(preview(&vec![b'a'; RECORD_BYTES * 2], Options::default()).is_err());
        assert!(
            parse(
                &b"a\n1"[..],
                Path::new("test.csv"),
                Options::default(),
                &AtomicBool::new(true)
            )
            .is_err()
        );
    }
}
