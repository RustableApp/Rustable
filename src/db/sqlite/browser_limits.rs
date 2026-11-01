//! Browser-only SQLite bounds. Native database sessions keep SQLite's normal limits.
use rusqlite::{Connection, limits::Limit};
const VALUE_BYTES: i32 = 8 * 1024 * 1024;

#[cfg(target_arch = "wasm32")]
pub(super) fn limit_heap() {
    // This WASM instance belongs exclusively to the database worker. SQLite's
    // allocator returns SQLITE_NOMEM rather than growing without a bound.
    // This bounds SQLite allocations, not the Rust/UI heaps or total tab memory.
    unsafe { rusqlite::ffi::sqlite3_hard_heap_limit64(64 * 1024 * 1024); }
}
pub(super) fn configure(connection: &Connection) -> rusqlite::Result<()> {
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, VALUE_BYTES)?;
    connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, VALUE_BYTES)?;
    // The demo owns one disposable database. ATTACH must not create additional
    // databases outside that budget or imply access to a user's disk files.
    connection.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_values_fail_without_poisoning_the_connection() {
        let connection = Connection::open_in_memory().unwrap();
        configure(&connection).unwrap();
        assert!(connection.query_row("SELECT zeroblob(8388609)", [], |row| row.get::<_, Vec<u8>>(0)).is_err());
        assert!(connection.execute_batch("ATTACH ':memory:' AS extra").is_err());
        assert_eq!(connection.query_row("SELECT 42", [], |row| row.get::<_, i64>(0)).unwrap(), 42);
        assert!(connection.is_autocommit());
    }
    #[test]
    fn rejected_statement_preserves_explicit_transaction_for_rollback() {
        let connection = Connection::open_in_memory().unwrap();
        configure(&connection).unwrap();
        connection.execute_batch("CREATE TABLE t(value); BEGIN; INSERT INTO t VALUES (1);").unwrap();
        assert!(connection.execute_batch("INSERT INTO t VALUES (zeroblob(8388609))").is_err());
        assert!(!connection.is_autocommit());
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(connection.query_row("SELECT count(*) FROM t", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
}
