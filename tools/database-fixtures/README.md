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
retains earlier results when a later statement errors, spills complete desktop
results after 5,000 rows or the shared 16 MiB memory budget, and caps result sets
at 128. Explicit bounded collectors used by browsing/readback still truncate. Structural/header costs are included in the budget. Rejected rows are
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

Metadata now uses the same typed connectivity classification as query execution.
Catalog streaming, database listing, schema switching, table/index definitions
and trigger requests discard the connection on transport failure. Ordinary SQL
errors leave it available. A live regression kills only its own fixture session
before catalog/list/table-metadata operations and verifies the socket is discarded
before transaction-state polling; a missing-table error preserves it instead.
All ten driver tests pass independently against MySQL and MariaDB, and strict
all-target Clippy passes. Metadata cancellation remains outstanding.

Catalog, database listing, schema switching, table/index/trigger metadata, and
browse data now use the shared command cancellation flag and a scoped KILL QUERY
watcher. The worker binds its flag before dispatch; pending cancellation stops
before the next request. The watcher is joined before returning, preventing a
late cancellation from killing the next command. Control connections are opened
only after cancellation, with bounded connection/read timeouts.

A live test holds a write lock from a separate fixture connection, starts browsing,
cancels after 100 ms, and verifies completion before the 3-second read timeout.
After releasing the lock and clearing cancellation, the same driver browses the
row successfully. All 11 driver tests pass on each engine. Full native suite:
399 passed, 56 ignored; strict Clippy and WASM cargo check pass. Initial connection
setup and session-state refresh cancellation still need coverage; cancellation
permission/control-connection failures also remain to exercise.

Cancellation control-connection failure is now exercised with intentionally invalid
credentials only on the future control connection, after establishing the main
fixture session. Both SQL and metadata paths retain the cancellation diagnostic
alongside the main read-timeout error and discard the timed-out socket. Browse
also preserves disconnected-stream state if a cancellation diagnostic arrives
at the same time. The driver never claims that the failed cancellation stopped
the server query; connection read timeout is the fallback.

All 12 driver tests pass on MySQL and MariaDB after this change, with strict
Clippy passing. The new case uses a 300 ms read timeout and a two-second SLEEP,
requests cancellation after 50 ms, and checks disconnection and both diagnostics.
Actual KILL privilege denial and initial connection cancellation still need tests.

The application worker now accepts OpenMysql for both engines. Candidate
connection, catalog and SQL dialect are prepared before replacing the old driver.
An existing open transaction blocks replacement; failed or pre-cancelled opens
preserve the current session. Cancellation is attached before catalog loading and
checked again before replacement. Network handshake cancellation remains bounded
by connect_timeout rather than interruptible mid-handshake.

A live worker-handler regression starts with SQLite, tests failed/cancelled opens
and transaction protection, then connects to the fixture and executes SELECT 42
through Command/Payload. Connected includes the detected MySQL/MariaDB engine,
selected database, active dialect and password-free label. MysqlConfig is still
not serializable: the browser transport explicitly rejects OpenMysql credentials,
and WASM rejects direct server opening. Native/browser enum shapes remain aligned.

Validation: 13 driver tests pass on each engine; native suite 399 passed, 58
ignored; strict Clippy and release WASM linking pass. Connection-profile UI,
credential storage, SSH and safe table editing are not yet wired for these engines;
this is worker integration, not official support or a released feature.

MysqlProfile now defines the persistable nonsecret host/port/database/user, TLS
and CA-file path, and timeout settings. It excludes passwords and derives only
from nonsecret fields of MysqlConfig. Serde defaults preserve verified TLS and
bounded timeouts for missing fields; an empty database is allowed for choosing
one after connection. Driver opening validates required host/user, nonzero port
and timeouts, and rejects an empty CA path before network I/O.

Ten connection-profile tests pass, including password-free serialization/Debug
and settings roundtrip, default behavior and invalid configuration rejection;
strict Clippy passes. This profile type is not yet a ConnectionKind variant or
part of the manager form, so saved MySQL/MariaDB connections remain unfinished.

Experimental MySQL and MariaDB entries are now wired into the connection manager:
profiles persist without passwords, use the existing opt-in credential-store flow,
and open separate workspace windows through OpenMysql. Forms include host, port,
optional database, user/password, verified TLS and optional CA path. The form
explicitly identifies table editing and SSH as unfinished. Test uses the real
driver with bounded five-second connection/read timeouts and SELECT 1. Database
navigation preserves the MySQL session configuration when opening another database.

Validation: native library suite 403 passed, 58 ignored; strict Clippy and WASM
check pass. New tests cover both profile variants, persistence without passwords,
window requests, and database navigation with TLS settings intact. Live connection
probe/open tests pass separately on MySQL and MariaDB. Native visual and interactive
form verification is still required, as are actual system-keychain checks for these
variants, safe edits, SSH and remaining SQL-language intelligence. Experimental UI
availability is not official supported-engine status; no release was published.

The experimental connection forms now expose connection/read timeouts (seconds)
alongside TLS and CA-path settings. Headless egui frame verification checks the
fields/actions are painted and the form fits the large-screen dialog for both
engines. The existing credential-worker remember/load/forget regression now runs
for PostgreSQL, MySQL and MariaDB: opening waits for credential loading, passwords
never enter saved preferences, and forgetting removes the stored value. These
credential checks use the test backend, not the macOS Keychain itself.

All 29 connection-manager tests and strict Clippy pass. Native visual inspection,
small-window MySQL form interaction and real-keychain acceptance remain open.

Typed prepared-parameter conversion now covers all shared grid Value variants:
NULL, signed integer, finite double, UTF-8 text and binary bytes. Exact DECIMAL and
large unsigned text are bound unchanged for destination-column conversion.
Nonfinite floating-point values are rejected. Browse filtering uses this same
conversion instead of a text-only special case.

Live temporary-table tests roundtrip u64::MAX, DECIMAL(30,10), i64::MIN, NULL,
finite real, binary bytes and Unicode/NUL/quote/backslash/SQL-looking text under
both default and ANSI_QUOTES + NO_BACKSLASH_ESCAPES modes. All 15 driver tests
pass separately on MySQL/MariaDB; strict Clippy passes. This establishes value
binding for the upcoming change planner; table plan/apply remain unimplemented.

MySQL/MariaDB data change planning now generates parameterized grouped cell
updates, inserts (omitted columns retain defaults), and deduplicated deletes.
Delete supersedes staged edits to that row. Updates/deletes require a complete
row snapshot and non-NULL primary key; generated/unknown columns and nonfinite
parameters are rejected. Plans retain qualified names and reviewed table DDL.
Only InnoDB base tables are accepted; DDL changes are rejected for separate review
because their implicit commits cannot be mixed into the data transaction.

Row preconditions use a primary-key predicate plus NULL-safe binary comparisons
of the loaded row, followed by FOR UPDATE. The indexed key narrows the check;
binary comparisons catch case-only changes under case-insensitive collations.
Live tests cover grouped updates, SQL-looking bound data, generated-column
rejection, delete precedence, insert omission and case-only conflicts. All 16
driver tests pass on each engine; strict Clippy passes. Execution is intentionally
still rejected by apply(): schema/trigger revalidation under metadata locks,
transaction ownership, affected-row semantics, rollback and ambiguous commit
handling must be implemented before this can save data.

### Reviewed MySQL/MariaDB trigger snapshots (2026-11-01)

Data-change plans now capture a structured snapshot of SHOW CREATE TABLE and
all visible triggers, including body, timing, event, execution order, definer,
creation SQL mode and character/collation settings. The snapshot uses JSON
encoding rather than SQL-text delimiters. Live planner regression passes on
both disposable engines (ports 43316 and 43317): unchanged metadata is stable,
adding/replacing/removing a trigger changes the snapshot appropriately, and
recreating identical trigger SQL under a different SQL mode changes it too.
Strict all-target Clippy passes.

This is preparation for transactional apply, which remains disabled. It does
not yet establish metadata locking, privilege-complete visibility, save-time
revalidation or safe handling of trigger effects on nontransactional tables.
Those must be resolved before claiming safe MySQL/MariaDB table editing.

### Exact floating-point edit conflict checks (2026-11-01)

A live unchanged-row precondition test exposed a false conflict for FLOAT 1.2:
its server decimal rendering differs from the exact binary FLOAT promoted to
f64 by the prepared result decoder. Floating-point originals now use DOUBLE
arithmetic equality instead of comparing decimal strings. Text/binary fields
retain bytewise NULL-safe comparison, preserving case/trailing-space detection.
No rounding tolerance is applied.

The live regression on both engines covers an unchanged row containing FLOAT,
DOUBLE, DECIMAL(30,10), DATETIME(6), negative TIME(6), BIT(9), binary bytes and text.
It also rejects small FLOAT and DOUBLE changes. The full db::mysql test selection
passed 17 tests against each disposable engine, including the earlier case-only
conflict check; strict all-target Clippy passed. Transactional apply remains
unfinished and disabled; these are planner/precondition results.

### MySQL review and activity SQL literals (2026-11-01)

Review statements, concurrency checks, table history and applied-statement logs
now select the display renderer by engine. MySQL text containing backslashes or
NUL uses CONVERT(X'…' USING utf8mb4), preserving bytes under both normal and
NO_BACKSLASH_ESCAPES modes; ordinary text remains readable quoted text. Floating
values use an exponent suffix when needed to retain approximate numeric typing.
Execution still uses bound parameters; rendered SQL is presentation only.

Seven renderer tests and the existing activity test pass. The live bound-value
regression now executes each displayed literal on both MySQL and MariaDB under
normal and ANSI_QUOTES/NO_BACKSLASH_ESCAPES modes, checking exact Unicode/NUL/
quote/backslash text, binary bytes, integer extremes, NULL and DOUBLE values.
Both engines pass, as does strict all-target Clippy. This does not enable the
unfinished transactional editing executor.

### Beautify under MySQL/MariaDB quote modes (2026-11-01)

Beautify no longer rejects ANSI_QUOTES or NO_BACKSLASH_ESCAPES categorically.
Its tokenizer adapter preserves MySQL token categories while applying the active
session's identifier and string rules. Quoted spans remain opaque to whitespace
formatting, with token signatures checked before and after, including selection
boundaries. Six formatter tests pass (one manual throughput test ignored),
including all four combinations of the two flags, backslashes before closing
quotes, doubled backticks, double-quoted strings/identifiers and hash comments.

Live SELECT comparisons on both disposable MySQL and MariaDB servers confirm
identical results before/after Beautify for each mode combination. Strict
all-target Clippy passes. This verifies those mode-sensitive expressions, not
complete MySQL/MariaDB grammar support or every stored-program construct.

### Local MySQL/MariaDB schema completion (2026-11-01)

Catalog completions now insert backtick-quoted MySQL schema, table and column
identifiers, escaping embedded backticks. Unqualified tables include their
catalog schema; schema-qualified suggestions replace only the table fragment.
This fixes SQLite/PostgreSQL double quotes being emitted as MySQL string
literals in normal SQL mode. Columns such as rank are quoted without relying
on SQLite's reserved-word list.

Completion and Beautify now share the mode-aware MySQL tokenizer. Caret checks
suppress suggestions inside hash comments, whitespace-qualified dash comments,
strings and quoted identifiers. Relation discovery ignores string/comment
contents and honors backslash escape modes. Tests cover names with spaces and
backticks, schema qualification, column suggestions and false FROM/JOIN text
inside literals/comments. Seventeen completion tests, six formatter tests (one
manual benchmark ignored) and strict all-target Clippy pass. This remains local
catalog-based completion, not a complete MySQL semantic language server.

### Streaming full-table MySQL/MariaDB export (2026-11-01)

Both engines now implement All matching rows as CSV/JSON, enabled in the table
Export menu. Browsing and export share filter/sort SQL construction; export
omits paging LIMIT/OFFSET. The native query iterator decodes and encodes one row
at a time through the shared row writer and same-directory atomic file
publisher. This avoids retaining the entire result, but a single row and driver
packet buffers still consume memory; it is not a whole-process memory bound.

Live tests on both engines exported 1,200 filtered rows despite browse offset
500, verified descending sort, literal %/_ filter semantics, Unicode, NULL and
binary byte arrays, and checked both CSV and JSON. Active-transaction and
pre-cancelled exports are rejected. Cancelling a slow view export preserves an
existing destination, removes staging files, completes within three seconds,
and allows SELECT 42 on the same connection. Encoder errors signal the scoped
query canceller before the driver's result-drop drain; errored sessions are
pinged afterward and discarded if synchronization/transport is unusable.

Validation: native library 410 passed, 63 existing manual/integration tests
ignored; db::mysql selection 19 passed against each fixture; strict all-target
Clippy passed. Disk-full fault injection and native UI interaction verification
remain outstanding. No release or main push performed.

## Verified TLS fixtures

These separate servers require TLS and bind IPv4/IPv6 loopback ports 43318 and
43319. The certificate is signed by a generated local CA and valid for
`localhost` only, so connecting by IP tests hostname verification independently
of CA trust. Certificate files are generated outside the repository and expire
after two days. Regenerate them for later runs.

```sh
export RUSTABLE_TLS_FIXTURE_DIR="$(mktemp -d)/certs"
sh tools/database-fixtures/generate-tls.sh "$RUSTABLE_TLS_FIXTURE_DIR"
docker compose -f tools/database-fixtures/tls.compose.yaml up -d --wait
export RUSTABLE_MYSQL_TEST_CA="$RUSTABLE_TLS_FIXTURE_DIR/ca.pem"
for tls_port in 43318 43319; do
  RUSTABLE_MYSQL_TLS_TEST_PORT="$tls_port" mise exec -- cargo test --lib live_tls_verifies -- --ignored --nocapture
  RUSTABLE_MYSQL_TLS_TEST_PORT="$tls_port" mise exec -- cargo test --lib live_driver_over_verified_tls -- --ignored --nocapture
  RUSTABLE_MYSQL_TLS_TEST_PORT="$tls_port" mise exec -- cargo test --lib live_tls_cancel -- --ignored --nocapture
done
docker compose -f tools/database-fixtures/tls.compose.yaml down
```

When running all ignored `db::mysql` tests, also set
`RUSTABLE_MYSQL_TLS_TEST_PORT` and `RUSTABLE_MYSQL_TEST_CA`; the ordinary
`RUSTABLE_MYSQL_TEST_PORT` still selects the separate plaintext fixture.

Verified on MySQL 8.4.11 and MariaDB 11.4.13: CA-trusted connection succeeds with
`TLS_AES_256_GCM_SHA384`, omitted CA fails certificate verification, IP hostname
mismatch fails name verification, and plaintext is rejected by the server.
Rustable's driver initializes over TLS, returns exact Unicode text, loads its
catalog and preserves the connection ID. This does not yet verify client
certificates, remote certificate rotation or the native certificate-picker UI.

TLS cancellation checks cover an interrupted statement inside a transaction,
retained earlier changes, explicit rollback, unchanged connection identity, and
no delayed cancellation affecting the following query. Since these servers
require secure transport, the separate successful KILL QUERY connection also
uses TLS. Removing its trusted CA after the main connection is established
produces a certificate error; the main query's read deadline then disconnects
the session rather than reporting successful cancellation or reusing it.

### Focused partial-revoke acceptance test

Run this alone, with no other MySQL fixture tests/processes. It temporarily
changes the disposable server's global partial_revokes setting, which changes
how existing schema grants are interpreted. The test restores the old setting
and removes its restriction even if an assertion fails.

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 RUSTABLE_MYSQL_PARTIAL_REVOKES_TEST=1 \
  mise exec -- cargo test --lib live_review_global_trigger_privilege -- --ignored
```

Ordinary driver-suite runs leave this server-wide branch disabled. Both engines
still exercise unrestricted global grants, actual triggers, late grants, and
revocation in the ordinary ignored integration suite.

### MySQL/MariaDB over SSH with verified database TLS

This disposable Docker Desktop gateway forwards only localhost:43318 and
localhost:43319 to the TLS fixtures on the host. Start the TLS fixtures first.
Keys are generated locally; known_hosts is derived from the generated host key,
not accepted from a network scan. The gateway is exposed only on loopback:43320.
It permits key authentication and local forwarding, with no password login.

```sh
sh tools/database-fixtures/generate-ssh.sh /tmp/rustable-ssh-fixture
RUSTABLE_SSH_FIXTURE_DIR=/tmp/rustable-ssh-fixture \
  docker compose -f tools/database-fixtures/ssh.compose.yaml up -d --build
RUSTABLE_SSH_FIXTURE_DIR=/tmp/rustable-ssh-fixture \
RUSTABLE_MYSQL_TLS_TEST_PORT=43318 \
RUSTABLE_MYSQL_TEST_CA=/tmp/rustable-tls-fixture/ca.pem \
  mise exec -- cargo test --lib live_ssh_tls -- --ignored
```

Repeat with TLS port 43319 for MariaDB. Provide RUSTABLE_SSH_FIXTURE_DIR as well
when running the full ignored driver suite. Test coverage includes retained TLS,
transaction-preserving cancellation using the same tunnel even with the direct
port invalidated, connection reuse, listener closure on drop, rejected unknown
SSH hosts, and rejected database CAs. This fixture does not verify Windows SSH
(OpenSSH transport remains unavailable there) or arbitrary remote jump-host setups.

```sh
RUSTABLE_SSH_FIXTURE_DIR=/tmp/rustable-ssh-fixture \
  docker compose -f tools/database-fixtures/ssh.compose.yaml down
```


## Disposable Redis Sentinel fixture

Run `bash scripts/test-redis-sentinel.sh` from the repository root. It uses
`redis:7.4-alpine`, publishes data ports 16410–16412 and Sentinel ports
26410–26412 on loopback, uses public test-only credentials, and removes its own
uniquely named container on exit. It never reads saved connections. Existing
listeners cause Docker port binding to fail rather than being reused.

The smoke check verifies a three-Sentinel quorum, discovery and unknown service
behavior, replicated data, manual promotion of a replica, and demotion of the old
primary. Sentinel and data nodes use different passwords. Rustable discovery tests
run before and after promotion, verifying the current target, independent auth,
database selection and failed-seed fallback. It also stops the promoted primary
and repeats the live session, probe and streaming checks after Sentinel performs
automatic recovery. A final test keeps a workspace open through another primary
handoff. It leaves the workspace socket untouched, verifies automatic recovery,
checks that the failed INCR was not replayed, and then submits a fresh explicit
INCR. Its separate fixture observer reconnects if Redis closes it during demotion.
This tests recovery after the new primary is available; it does not cover every
outage timing or network partition.

`bash scripts/test-redis-sentinel-tls.sh` uses separate ephemeral certificate
authorities for the TLS-only data endpoint (16420) and Sentinel (26420). It tests
successful session/stream startup and rejection of wrong/missing trust anchors
and the wrong discovery hostname. It requires OpenSSL, uses the same disposable
Redis image, and removes its own container/certificate directory. TLS fixture
execution is separate from the plaintext failover fixture.

Network partitions and in-flight failover/write outcome coverage remain pending.

## Disposable Redis Cluster and module fixtures

`bash scripts/test-redis-cluster.sh` creates three Redis 7.4 primaries inside its
own uniquely named container. Only data ports 16400–16402 are published on
loopback; cluster bus traffic stays inside the container. The script requires
those ports to be free and removes only its own container on exit. It uses
public test credentials and never reads saved connections or user databases.

`RUSTABLE_REDIS_CLUSTER_MODULE_QA=1 bash scripts/test-redis-cluster.sh` uses Redis
8.2 and explicitly loads RedisTimeSeries on each primary. It runs the existing
routing/topology checks plus `redis_session::tests::live_cluster_timeseries`.
That test chooses one key owned by each primary, checks latest-sample previews,
multiple pages and inclusive time ranges, and repeats the reads with cached
connections. It does not claim module resharding/failover or cluster-wide query
aggregation coverage. Standalone `scripts/test-redis.sh` excludes this Cluster-only
case and still runs the standalone time-series inspection test on its Redis 8.2
fixture.

The Redis Cluster fixture now ends with a seed-loss recovery check: it shuts down
only its disposable primary at 16400 after the other cases, then reconnects using
a surviving discovered node. The test requires `RUSTABLE_REDIS_CLUSTER_RECOVERY_QA=1`
and is invoked by `scripts/test-redis-cluster.sh`; do not run it against a shared
Cluster. It validates transport rediscovery, not replica promotion or slot recovery.

The Cluster fixture also performs a controlled live slot migration before seed
shutdown. It uses only an initially empty slot, checks ASK and MOVED with exact
counter increments and mixed-owner reads, then deletes its keys and restores slot
ownership. This test runs only with `RUSTABLE_REDIS_CLUSTER_QA=1` in the fixture.

For planned Cluster replica promotion and recovery with all original primaries
stopped, run `bash scripts/test-redis-cluster-failover.sh`. It owns a separate
six-node Redis 7.4 container on loopback ports 16600–16605, waits for replication
and converged topology, and removes the container on exit. It sets the explicit
`RUSTABLE_REDIS_CLUSTER_FAILOVER_QA=1` opt-in; the standalone Redis fixture excludes
this test. For a native post-promotion topology capture, set
`RUSTABLE_REDIS_TOPOLOGY_CAPTURE=1 RUSTABLE_REDIS_CAPTURE=target/redis-cluster-promotion-screenshots`
before invoking the script; stop the preview after capture to clean up the fixture.

Each Cluster fixture process has an isolated working directory, including its
replication snapshot files. The six-node test waits for replica links and each
promotion's topology to converge on every node before the next promotion. It also
checks the replicated value before promotion and before/after primary shutdown;
these correctness checks must not be removed to make a flaky fixture pass.

### Redis vector sets

Run `bash scripts/test-redis-vectors.sh` for an isolated Redis 8.4 container on
loopback port 16382. It exercises ordered binary-safe pages and output limits,
uses public fixture-only credentials, and removes its container on exit.
For native screenshots use `RUSTABLE_REDIS_PREVIEW_IMAGE=redis:8.4-alpine
RUSTABLE_REDIS_VECTOR_CAPTURE=1 RUSTABLE_REDIS_CAPTURE=target/redis-vectors-screenshots
bash scripts/preview-redis-browser.sh` (environment assignments on one line).

`RUSTABLE_REDIS_CLUSTER_VECTOR_QA=1 bash scripts/test-redis-cluster.sh` selects
Redis 8.4 and additionally validates vector counts and pages across all three
primary owners, before the final seed-shutdown recovery test.


## Disposable PostgreSQL complete-result checks

The following isolated container uses public fixture credentials, loopback port
54329 and disposable storage. Never point these tests at a user database.

```sh
docker run -d --name rustable-results-postgres-qa --tmpfs /var/lib/postgresql/data \
  -p 127.0.0.1:54329:5432 -e POSTGRES_PASSWORD=rustable_qa_only \
  -e POSTGRES_DB=rustable_qa postgres:17
docker exec rustable-results-postgres-qa pg_isready -U postgres
RUSTABLE_POSTGRES_QA=1 mise exec -- cargo test --release --lib db::postgres::custom_type_stream_tests -- --ignored --nocapture --test-threads=1
RUSTABLE_POSTGRES_QA=1 mise exec -- cargo test --release --lib db::postgres::latency_tests::editor_wan_latency_comparison -- --ignored --nocapture
RUSTABLE_POSTGRES_QA=1 mise exec -- cargo test --release --lib db::postgres::latency_tests::browse_wan_latency_comparison -- --ignored --nocapture
docker rm -f rustable-results-postgres-qa
```

The complete-result latency workload includes 6,000 rows spilled locally, compared
with the same rows over a single simple-query exchange. Run latency benchmarks
serially, without competing builds or benchmark workloads. The custom-type test
covers cold and warm enum results over 16 MiB, exactly-once INSERT RETURNING,
explicit transactions, errors and subsequent connection reuse. The older bounded
API tests remain separate and continue to assert intentional truncation.

### PostgreSQL encrypt-only TLS (issue #8)

`bash scripts/test-postgres-tls.sh` starts a disposable PostgreSQL 17 server on
loopback port 54329 with a 10-year, untrusted certificate for the wrong hostname.
The port must be free. It verifies encrypted Require/Prefer connections, rejection
by Verify CA/Verify full, explicit plaintext Disable, the real driver, connection
probe and query monitor. The script deletes its container and certificates.
The non-ignored protocol test additionally verifies that Require/Verify CA/Verify
full close the connection without sending startup credentials if TLS is refused.

Capture the actual native connection editor without opening a database:

```sh
mise exec -- cargo build --features qa
RUSTABLE_TLS_CAPTURE=/tmp/postgres-tls.ppm target/debug/rustable
sips -s format png /tmp/postgres-tls.ppm --out /tmp/postgres-tls.png
```

Use `RUSTABLE_TLS_LIGHT=1`, `RUSTABLE_TLS_WIDTH=780`, or `RUSTABLE_TLS_MENU=1`
for light mode, a narrow window, or the expanded mode picker, respectively.

The PostgreSQL modes use the driver's existing SSLRequest negotiation. Prefer
allows plaintext only when the server declines TLS; it does not retry a failed
TLS handshake as plaintext. Require, Verify CA and Verify full never allow that
fallback. Verify CA uses the platform trust store without hostname verification;
Verify full also checks the configured database host, including through SSH.
Legacy saved `tls: true` remains Verify full and `tls: false` remains Disabled.
TablePlus PostgreSQL values 0/1/2/4/5 map to Prefer/Disabled/Require/Verify CA/Verify
full; unsupported Allow (3) and unknown values are rejected for manual review.
The picker ordering is also documented in TablePro's
[TablePlus import vocabulary](https://github.com/TableProApp/TablePro/blob/main/apple/TablePro/Core/Services/Export/ForeignApp/TablePlusFormValues.swift).
