//! UTF-8 SQL documents. Disk work runs on the database worker, never the UI thread.
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
};
/// Captured document identity and saved baseline; resolved on the worker.
#[derive(Clone)]
pub struct SaveSource {
    pub path: PathBuf,
    pub sql: Arc<String>,
}

/// Follow existing symlinks so an atomic save updates the document, not its link.
/// A dangling symlink is an error rather than permission to replace that link.
pub fn destination(path: &Path) -> Result<PathBuf, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => std::fs::canonicalize(path).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path.file_name().ok_or("Choose a SQL filename")?;
            Ok(std::fs::canonicalize(parent)
                .map_err(|e| e.to_string())?
                .join(name))
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn same_source(path: &Path, resolved: &Path, source: &SaveSource) -> bool {
    path == source.path
        || same_file::is_same_file(resolved, &source.path).unwrap_or(false)
        || destination(&source.path).is_ok_and(|original| original == resolved)
}

const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub fn read(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular SQL file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("SQL editor files are limited to 8 MiB".into());
    }
    if bytes.contains(&0) {
        return Err("This is not a UTF-8 SQL text file".into());
    }
    let sql = String::from_utf8(bytes).map_err(|_| "SQL files must use UTF-8".to_string())?;
    Ok(sql.strip_prefix('\u{feff}').unwrap_or(&sql).to_owned())
}
pub fn write_checked(
    path: &Path,
    sql: &str,
    expected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    for chunk in sql.as_bytes().chunks(65536) {
        if cancel.load(Ordering::Relaxed) {
            return Err("Saving SQL cancelled".into());
        }
        temp.write_all(chunk).map_err(|e| e.to_string())?;
    }
    if let Ok(metadata) = std::fs::metadata(path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Saving SQL cancelled".into());
    }
    if let Some(expected) = expected {
        let current = read(path).map_err(|e| {
            format!(
                "Cannot verify the original SQL file: {e}. Use Save SQL As to save a separate copy."
            )
        })?;
        if current != expected {
            return Err("SQL file changed outside Rustable. Use Save SQL As to save a separate copy, or reopen the file to inspect its changes.".into());
        }
    }
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_crlf_bom_and_cancelled_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("query.sql");
        std::fs::write(&path, "\u{feff}SELECT '日本';\r\n").unwrap();
        assert_eq!(read(&path).unwrap(), "SELECT '日本';\r\n");
        assert!(write_checked(&path, "replacement", None, &AtomicBool::new(true)).is_err());
        assert_eq!(read(&path).unwrap(), "SELECT '日本';\r\n");
        write_checked(&path, "SELECT '🙂';\n", None, &AtomicBool::new(false)).unwrap();
        assert_eq!(read(&path).unwrap(), "SELECT '🙂';\n");
    }
    #[test]
    fn save_in_place_detects_changed_and_missing_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("query.sql");
        let cancel = AtomicBool::new(false);
        std::fs::write(&path, "SELECT 1;").unwrap();
        write_checked(&path, "SELECT 2;", Some("SELECT 1;"), &cancel).unwrap();
        std::fs::write(&path, "external edit").unwrap();
        assert!(
            write_checked(&path, "SELECT 3;", Some("SELECT 2;"), &cancel)
                .unwrap_err()
                .contains("changed outside")
        );
        assert_eq!(read(&path).unwrap(), "external edit");
        std::fs::remove_file(&path).unwrap();
        assert!(write_checked(&path, "SELECT 3;", Some("SELECT 2;"), &cancel).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn rejects_binary_invalid_utf8_and_oversize() {
        let file = tempfile::NamedTempFile::new().unwrap();
        for data in [vec![0], vec![255], vec![b' '; MAX_BYTES as usize + 1]] {
            std::fs::write(file.path(), data).unwrap();
            assert!(read(file.path()).is_err());
        }
    }
}
