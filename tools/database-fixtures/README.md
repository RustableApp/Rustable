# Disposable MySQL and MariaDB integration servers

Run from the Rustable repository root:

```sh
docker compose -f tools/database-fixtures/compose.yaml up -d --wait
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib sql::mysql::server_tests -- --ignored --nocapture
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib sql::mysql::server_tests -- --ignored --nocapture
```

These fixtures bind only loopback, use separate ports from developer databases,
and store database contents in tmpfs. Credentials in the compose file are public,
local-test-only values. Tests refuse other ports and use only `rustable_test`.
Stopping/recreating these containers discards their data. Do not use them for
anything you want to keep.

```sh
docker compose -f tools/database-fixtures/compose.yaml down
```

The first live contract test covers custom DELIMITER handling for stored routines,
Unicode, multiple result sets, exact integers beyond JavaScript's safe range,
SQL mode escaping, connection reuse, and rollback. It does not yet prove the
Rustable driver, GUI, TLS, cancellation, safe editing or metadata implementation.

Verified on MySQL 8.4.11 and MariaDB 11.4.13 on 2026-11-01. Images use release-line
tags so test output must record the actual server version on every run.

Value conversion checks (both text-query and prepared-statement protocols):

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql_values -- --include-ignored
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib db::mysql_values -- --include-ignored
```

These compare unsigned BIGINT, exact DECIMAL, Unicode, binary bytes, NULL and
fractional/negative TIME values against Rustable's shared grid representation.
Empty-result column headers are checked without rerunning the query.
Unsigned values beyond i64 and DECIMAL are stored as exact text, avoiding f64
rounding. Text connections must request utf8mb4; invalid UTF-8 is rejected.

Bounded result-stream checks:

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql_results -- --ignored
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib db::mysql_results -- --ignored
```

The collector preserves multiple result sets and headers for empty SELECTs,
retains earlier results when a later statement errors, caps each grid at 5,000
rows, shares a 16 MiB retained-data budget across grids, and caps result sets at
128. Structural/header costs are included in the budget. Rejected rows are
not converted or cloned into the UI grid; transport rows are still received
and drained to preserve protocol synchronization. This is not a bound on total
process memory or network traffic. The owning driver's packet limits, socket
deadlines and cancellation remain required before exposing connections in UI.

Live checks on both fixture engines exercise 10,000 rows, a deliberately small
byte budget, 130 result sets, a later-statement error, and successful subsequent
queries on the same connection.

Cancellation contract (included in `sql::mysql::server_tests`): a separate control
connection waits until the target UPDATE is visible in this user's process list,
then sends KILL QUERY. Both MySQL and MariaDB return error 1317, leave earlier
uncommitted edits intact, and report an active transaction after ping. Explicit
ROLLBACK restores the committed value. This differs from SQLite's interrupted
write rollback behavior and must not be hidden by a generic cancellation path.
A 100ms read-timeout test verifies unknown status and discards the connection;
never retry an uncertain write automatically or reuse an unsynchronized stream.
These checks passed independently on both fixture engines on 2026-11-01.

Connection configuration checks:

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib connection::mysql_tests -- --include-ignored
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib connection::mysql_tests -- --include-ignored
```

`MysqlConfig` is ephemeral (no serialization or Debug implementation), with a
password-free Display label. TLS verifies certificates and hostnames by default;
TCP is explicit so local socket discovery cannot bypass that policy. An optional
root-certificate path is supported. Defaults are 5s connect, 120s read and 10s
write deadlines, 32 MiB packets and 32 cached statements. Connection setup requests
utf8mb4, without starting a transaction. Both servers reject the default verified
TLS attempt against these untrusted/non-TLS fixtures and pass explicitly selected
loopback plaintext plus Unicode/emoji and idle-state checks. Positive verified-TLS
and custom-root/hostname-mismatch coverage remain outstanding.

Driver integration foundation (still hidden from connection UI):

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql::tests -- --ignored
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib db::mysql::tests -- --ignored
```

The real Driver implementation now opens/reuses a session, detects MySQL versus
MariaDB, lists databases, retrieves table/view/column metadata, executes scripts,
and browses with bound literal substring filters, validated sorting and paging.
Live tests exercise quoted identifiers containing backticks, a literal percent
filter (not a wildcard), generated columns, sorting/offset, rollback and refusal
to switch databases during a transaction. SQL result byte budgets span statements.

Before exposing this driver in the connection picker: extend cancellation to
metadata/table browsing,
finish session-refresh failure handling and full UI connection wiring,
implement table-edit concurrency checks and commits, optimize metadata round trips,
add SSH/TLS fixtures, and connect profile storage/editor dialect state. Safe-mode
requests currently return an explicit unsupported error. Stream collection preserves typed connectivity classification: ordinary SQL
errors keep the session, while transport failures discard it. No MySQL/MariaDB support
claim or release is justified by the foundation alone.


Driver error-recovery coverage now includes a procedure that emits a result set
and then SIGNALs an error inside an explicit transaction. Both engines preserve
the earlier result and uncommitted edit; the same driver remains Active until
ROLLBACK. An initial statement error is likewise recoverable. A timed-out query
marks the driver Disconnected and subsequent execute calls are rejected. Value
conversion failures drain remaining rows explicitly, so later transport errors
are still observed instead of disappearing in an iterator destructor.


The driver now integrates in-flight SQL cancellation using a scoped watcher.
Ordinary queries open no additional connection; only a raised cancellation flag
creates a control connection with short connect/read/write timeouts. It sends
KILL QUERY and retries until the operation finishes, covering cancellation that
races statement arrival. The watcher joins before returning, preventing a late
KILL from affecting the next query. No automatic rollback is added: the server's
active transaction and earlier uncommitted edits remain available.

The live driver test cancels a ten-second UPDATE, verifies completion below two
seconds (before its three-second read deadline), confirms the following UPDATE
did not run, checks the earlier uncommitted value, runs another delayed query to
catch late cancellation, then explicitly rolls back to the original value.
This passes independently on MySQL 8.4.11 and MariaDB 11.4.13. Control-connection
failure and permission-denied cancellation paths still need fault coverage;
the main connection's configured read timeout remains the fallback.


Table browsing now returns ordered multipart indexes and triggers. Index details
include the complete server-authored SHOW CREATE TABLE definition (rather than
reconstructing and losing prefix lengths, sort order, expressions or visibility).
Triggers use SHOW CREATE TRIGGER, preserving the server's original definition.
Live checks on both engines verify primary/secondary uniqueness, a prefix index
with two ordered columns, and a trigger with a quoted backtick in its name.
The disposable MySQL fixture disables binary logging so its database-scoped test
user can create triggers without SUPER. Existing developer databases are unchanged.
Metadata fetching is currently sequential; batching and cancellation of metadata
work remain outstanding before performance parity is claimed.


Driver execution now scans one statement at a time, carrying the active client
DELIMITER and consuming SQL-mode updates from protocol session tracking. The
connection negotiates CLIENT_SESSION_TRACK and tracks sql_mode. Live tests on
both engines toggle NO_BACKSLASH_ESCAPES and ANSI_QUOTES mid-script, verify the
following statement's boundaries and values, and run another query using the
retained mode. Ordinary queries add no mode-refresh SQL round trip. A scanner
regression combines custom delimiters with a mode change and delimiter reset.
A malformed later statement stops at that point; earlier autocommitted statements
remain executed, matching the sequential execution model.

Active modes now propagate through connection/query worker replies into editor
selection, highlighting and formatting. Completion/LSP parity remains outstanding.


Session-changing SET/CALL/EXECUTE/USE statements now refresh authoritative mode,
tracking policy and current database. Leading ordinary comments are skipped;
executable comments conservatively refresh. When sql_mode tracking is disabled,
the driver refreshes before scanning each following statement rather than parsing
with stale modes. Re-enabling tracking restores the ordinary-query fast path.
Schema notifications also update the selected database. Live tests on both
engines disable tracking, toggle quoting modes, re-enable tracking, disable schema
tracking, and issue USE with and without a leading comment. Selected-database
state and query values match the server. Normal SELECT/DML with tracking enabled
adds no session-refresh exchange. A failed session-changing command now refreshes state before returning its
reply when the connection remains usable. Refresh failure coverage still needs
extending; failed refreshes preserve the dirty marker for a later retry.


Worker payloads now carry the driver's active dialect on connect and after SQL
execution, including errors. Rustable keeps it as transient session state and
uses it for statement selection, editor rendering and formatter requests.
The headless app regression delivers a MySQL mode update and verifies Run current
selects the second statement after a backslash-terminated literal under
NO_BACKSLASH_ESCAPES. Binary transport verifies both mode flags survive roundtrip.
Full native library suite: 399 passed, 53 ignored; strict Clippy passed. This does
not imply the hidden driver is connected to the profile picker or that a MySQL
language server is integrated.


Failed-routine regression: a procedure disables session tracking and then raises
SQLSTATE 45000. Before returning its error, the driver now updates tracking mode,
clears the dirty state after a successful refresh, and publishes a dialect that
matches the server. Later statements do not execute. The original SQL error is
preserved; if refresh also fails, its diagnostic is appended instead of replacing
the original. A failed refresh is not retried twice within one execution call.

Validation: all nine db::mysql* tests, including live browse, results, cancellation,
timeout, mode changes and failed-routine coverage, pass independently against
MySQL 8.4 and MariaDB 11.4 fixtures. Strict all-target Clippy passes. Ordinary
SELECT/DML with session tracking enabled still adds no session-refresh exchange.

Table browsing now constrains the information_schema table/column query to the
requested table with a bound TABLE_NAME parameter. It no longer loads columns
for every object in the database on each page/filter/refresh. Full catalog
requests still load all objects for navigation and completion. No metadata cache
was added, so each browse still sees current column definitions.

The live catalog/browse regression verifies a quoted table name returns exactly
one object, and an injection-shaped name returns no matches. All nine driver
checks pass on both MySQL and MariaDB; strict Clippy passes. This reduces metadata
volume, not the number of round trips; index/trigger retrieval and cancellation
remain further optimization work. No WAN latency improvement is claimed yet.
