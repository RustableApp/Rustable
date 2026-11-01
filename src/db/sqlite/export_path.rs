//! Prevent CSV publication from replacing files owned by the live connection.
use rusqlite::Connection;
use std::{
    io,
    path::{Path, PathBuf},
};

fn resolved(path: &Path) -> io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path
                .file_name()
                .ok_or_else(|| io::Error::other("Export destination has no filename"))?;
            Ok(parent.canonicalize()?.join(name))
        }
        Err(error) => Err(error),
    }
}

pub(super) fn validate(connection: &Connection, destination: &Path) -> io::Result<()> {
    let resolved_destination = resolved(destination)?;
    let mut statement = connection
        .prepare("PRAGMA database_list")
        .map_err(io::Error::other)?;
    let databases = statement
        .query_map([], |row| row.get::<_, String>(2))
        .map_err(io::Error::other)?;
    for database in databases {
        let database = database.map_err(io::Error::other)?;
        if database.is_empty() {
            continue;
        } // In-memory and temporary databases.
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let protected = PathBuf::from(format!("{database}{suffix}"));
            let aliases = match same_file::is_same_file(destination, &protected) {
                Ok(same) => same,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
            if resolved_destination == resolved(&protected)? || aliases {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Choose another export destination: this file belongs to an open SQLite database",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_database_attached_database_sidecars_and_hard_links() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("main.sqlite");
        let attached = directory.path().join("attached.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE keep_me(value TEXT); INSERT INTO keep_me VALUES ('original');",
            )
            .unwrap();
        connection
            .execute("ATTACH ? AS extra", [attached.to_str().unwrap()])
            .unwrap();
        let alias = directory.path().join("alias.csv");
        std::fs::hard_link(&database, &alias).unwrap();
        for protected in [&database, &attached, &alias] {
            assert!(
                validate(&connection, protected).is_err(),
                "{}",
                protected.display()
            );
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(
                validate(
                    &connection,
                    &PathBuf::from(format!("{}{suffix}", database.display()))
                )
                .is_err()
            );
        }
        validate(&connection, &directory.path().join("valid.csv")).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM keep_me", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "original"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_file_and_parent_directory_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("main.sqlite");
        let connection = Connection::open(&database).unwrap();
        let alias = directory.path().join("alias.csv");
        symlink(&database, &alias).unwrap();
        assert!(validate(&connection, &alias).is_err());
        let folder = directory.path().join("folder");
        symlink(directory.path(), &folder).unwrap();
        assert!(validate(&connection, &folder.join("main.sqlite")).is_err());
        assert!(validate(&connection, &folder.join("main.sqlite-wal")).is_err());
        validate(&connection, &folder.join("export.csv")).unwrap();
    }
}
