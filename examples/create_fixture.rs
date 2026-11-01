//! Create a disposable shared fixture for Rustable/TablePlus comparison.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Usage: create_fixture PATH")?;
    if std::path::Path::new(&path).exists() {
        return Err("Refusing to overwrite an existing file".into());
    }
    let conn = rusqlite::Connection::open(path)?;
    conn.execute_batch(include_str!("../src/demo.sql"))?;
    Ok(())
}
