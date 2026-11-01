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

Before exposing this driver in the connection picker: integrate in-flight
cancellation, preserve typed connectivity errors through result collection,
refresh SQL-mode/delimiter semantics when a script changes session modes,
implement table-edit concurrency checks and commits, load indexes/triggers/DDL,
add SSH/TLS fixtures, and connect profile storage/editor dialect state. Safe-mode
requests currently return an explicit unsupported error. Stream-result failures
conservatively close the session; this avoids reusing an uncertain stream but
still needs differentiated recoverable-error handling. No MySQL/MariaDB support
claim or release is justified by the foundation alone.
