-- Run in the disposable browser database only.
-- Execute each numbered section separately: Run all stops at the expected error.

-- 1. Set up the fixture.
CREATE TABLE memory_probe(id INTEGER PRIMARY KEY, payload BLOB);

-- 2. Expect SQLITE_NOMEM from the browser's 64 MiB SQLite allocator limit.
-- Each value is below the separate 8 MiB value limit.
WITH RECURSIVE n(x) AS (
    VALUES(1) UNION ALL SELECT x + 1 FROM n WHERE x < 100
)
INSERT INTO memory_probe SELECT x, zeroblob(1048576) FROM n;

-- 3. Expect zero partial rows, a successful write, and a 1024-byte payload.
SELECT count(*) AS partial_rows FROM memory_probe;
INSERT INTO memory_probe VALUES(1, zeroblob(1024));
SELECT id, length(payload) AS bytes FROM memory_probe;

-- 4. Remove the disposable fixture after inspecting the results.
DROP TABLE memory_probe;
