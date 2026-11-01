# Driver capability and transaction audit

Audited 2026-10-31 against the current source. “Implemented” below means the
operation exists in the adapter and has the stated restrictions; it does not
establish native acceptance, server-version compatibility, or complete TablePlus
parity. The verification ledger below distinguishes newly run checks from source
inspection. Shared UI acceptance remains in [the parity requirements](TABLEPLUS_PARITY_REQUIREMENTS.md).

## Engines and operations

| Operation | SQLite | PostgreSQL | Source / limits |
| --- | --- | --- | --- |
| Saved connection | Existing local file; in-memory demo | Server, database, user, password, TLS and optional SSH | `src/connection_profiles.rs::ConnectionKind`; only these two engine variants exist. MySQL/MariaDB, Redis and other engines are not implemented. |
| Session reuse | One owned connection per worker | One owned client per worker | `src/db.rs::Driver`, adapter structs. Independent windows own independent sessions. |
| SQL editor execution | Engine-prepared statements; multiple results | Split statements executed sequentially; multiple results | `sqlite.rs::script`, `postgres.rs::execute`; at most 128 statements per run. Earlier autocommitted statements survive a later error. |
| Explicit transactions across editor runs | Implemented: BEGIN/savepoint state persists until explicit finish or an engine-driven rollback | Implemented: BEGIN persists until COMMIT/ROLLBACK or connection teardown | Native acceptance remains open; see transaction table below. |
| Safe query mode | `query_only` plus prepared-statement read-only check | Statement restrictions plus server `transaction_read_only` in the query's implicit transaction | Adapter execution paths; not a substitute for least-privilege database permissions. |
| Object discovery | SQLite schema tables/views and columns | Selected-schema relation catalog and columns | `sqlite.rs::objects`, `postgres.rs::selected_objects`. PostgreSQL functions/procedures are not enumerated as sidebar objects here. |
| Schema selection | No adapter override for the schema picker | Catalog-backed selection, prefers public when available | `Driver` defaults and `postgres.rs::schema_catalog/select_schema`. Do not describe SQLite attached-database browsing as complete. |
| Table/view browsing | Bounded page; reusable metadata; eligible sequential pages use keyset cursor | Bounded page; fresh metadata/data pipelined for 16 recently visited autocommit table shapes | `sqlite.rs::browse`, `postgres/browse.rs`. First PostgreSQL loads and explicit transactions use sequential metadata then data. |
| Sort/filter | One sort column; one substring filter | One sort column; one substring filter | `src/db.rs::Browse`; multi-column sort and a general visual predicate builder are absent. |
| Result limits | 500-row browse, 5,000-row query display, byte budget | Same shared limits | `src/db.rs` constants; 16 MiB value budget. These are result-retention limits, not a bound on all process memory or server execution. |
| Update/delete loaded rows | Requires primary-key identity and current row metadata | Requires primary key, supported writable values and current metadata | `sqlite.rs::plan_changes`, `postgres/changes.rs`; no heuristic all-column row identity for keyless tables. Generated columns are excluded. |
| Insert rows | Omitted values use defaults; explicit NULL distinct | Same model, with PostgreSQL type-aware binding/casts | Shared `Change::InsertRow`; adapter validation can reject unsupported targets/types. |
| Pending-change review and apply | Bound statements plus concurrency/schema checks in one transaction | Bound statements plus row/schema checks in one transaction | `ChangePlan`; preview SQL is separate from bound execution. See transaction table. |
| Visual column changes | Add, rename, alter type/nullability/default, including guarded rebuild where needed | Add, rename, alter type/nullability/default with restricted type grammar and catalog checks | `Change` variants, `sqlite/schema.rs`, `postgres/changes/structure.rs`. Arbitrary default expressions and column deletion have no shared visual change variant. |
| Visual indexes | Create/drop, uniqueness and per-column ordering | Create/drop, uniqueness and per-column ordering | `Change::CreateIndex/DropIndex`; not full expression/partial/concurrent/index-method administration. |
| Other visual administration | No complete table/view/trigger/role/constraint administration workflow | Same limitation | No corresponding shared `Change` variants; handwritten SQL remains separate from staged GUI changes. |
| Mixed data and schema save | Planner supports multiple change types subject to rebuild/metadata restrictions | Explicitly rejects mixing row edits and structure changes | `sqlite.rs` planner/apply; `postgres/changes.rs::plan`. No blanket cross-engine guarantee. |
| CSV import | Streaming bound inserts into validated table, defaults for omitted columns | Streaming COPY or optional bounded INSERT batches | `sqlite.rs::import_csv`, `postgres/import.rs`; PostgreSQL INSERT mode supports cases where COPY is unsuitable, including RLS. Both reject changed destination metadata. |
| Export loaded result | CSV/JSON from retained grid | CSV/JSON from retained grid | Shared export commands; a truncated result export is not a full query rerun. |
| Full-table export | Streams fresh SELECT with current filter/order | Read-only transaction and bounded server portal | `sqlite.rs::export_table`, `postgres/export.rs`; temporary-file publication protects existing destination on failure. |
| Cancellation | Progress handler; SQLite busy timeout 750 ms | Worker cancellation watcher/server cancel; separate cancellable Test path | Not a hard end-to-end latency guarantee under all network/filesystem conditions. |
| Non-connecting Test | Read-only open and schema_version check; never creates a missing file | Separate connection and SELECT 1, optional SSH, field events | `src/connection_probe.rs`; handshake/readiness deadlines ten seconds each. TLS/DNS/native-close edge cases remain unverified. |

## Transaction boundaries and failure behavior

| Workflow | SQLite | PostgreSQL |
| --- | --- | --- |
| Ordinary editor statement | Engine autocommit unless SQL starts a transaction | Engine autocommit; no explicit BEGIN/COMMIT wrapper. Session safety/escaping settings share its protocol cycle. |
| Editor BEGIN without COMMIT in the same run | Transaction remains open across runs; visible state blocks close/replacement | Transaction remains open across runs. A statement failure can leave it aborted until explicit recovery. |
| Editor script failure/cancel | Stops subsequent statements; earlier autocommit work remains; transaction follows SQLite error semantics, without an application-forced rollback | Stops subsequent statements; earlier autocommit work remains; user transaction is not silently committed or rolled back. |
| GUI row/schema save | Rejects active SQL transaction; `BEGIN IMMEDIATE`; schema/version and row preconditions; expected row counts; commit only after all checks, including rebuild foreign-key validation | Requires idle SQL transaction; one transaction, relation locks, metadata fingerprints, row preconditions and exact expected counts; failure rolls back batch. |
| CSV import | Rejects active transaction; immediate transaction; failures/cancel roll back import | Requires idle transaction; one transaction; cancellation/error rollback; destination metadata validated under lock. |
| Full-table export | Requires idle transaction; streams one SELECT, with no explicit multi-statement snapshot wrapper | Requires idle transaction; read-only transaction with bounded portal. |
| Failed reviewed plan | Pending changes retained by UI; stale/conflicting plan rejected | Same intended UI contract; adapter does not apply a partial GUI batch. |

SQLite cross-run transaction retention is now implemented with worker state,
close/replacement guards and explicit GUI apply/import/export exclusion. Regular
app/worker integration covers separate BEGIN/INSERT/COMMIT runs, external visibility,
ordinary statement errors, cancellation, safe-mode recovery, engine-driven rollback
and savepoints. Native SQLite BEGIN/SELECT, quit/last-tab close guards and ROLLBACK
recovery passed; native reconnect acceptance remains open. SQL-12 is not yet fully accepted.

PostgreSQL transport loss detected by a database operation is published as
Disconnected, distinct from idle or an open/failed transaction. The connection
bar warns to verify writes before retrying; the app does not infer a commit or
rollback outcome or automatically replay SQL. A dead session permits close and
replacement, while local pending edits still block both. Live app/worker tests
terminate the backend during a transaction and in autocommit, then verify failed
query, disconnected status and successful replacement. A protocol fault-injection
test also drops COMMIT CommandComplete/ReadyForQuery after the server commits:
an independent connection sees exactly one committed row; later script statements
are skipped, the client reports unknown outcome and does not replay the write.
Native QA verifies autocommit loss after fixture shutdown, visible lost-connection
status and unknown-outcome message, last-tab/window close, reopening the saved
profile after restart, and a successful fresh SELECT. Explicit-transaction native
loss, idle monitoring and other disconnect timings remain open.

PostgreSQL safe mode permits parsed session COMMIT/END/ROLLBACK/ABORT recovery,
including rollback to savepoint. It still rejects transaction settings, BEGIN,
prepared-transaction recovery and writes. Live app/worker tests cover safe-mode
ROLLBACK from failed state and COMMIT from active state.

## Evidence and outstanding gates

Fresh checks run for this audit:

- `cargo test --locked script_transactions_persist_until_explicit_recovery_and_safe_mode_cannot_be_bypassed`: passed; SQLite same-run commit, retained failed/incomplete transactions and safe-mode recovery.
- `cargo test --locked conflict_rolls_back_all_statements_and_plan_can_be_retried`: passed; SQLite GUI batch rollback and retry.
- `RUSTABLE_POSTGRES_QA=1 cargo test --locked editor_autocommit_and_explicit_transactions_follow_postgres -- --ignored`: passed on the disposable local PostgreSQL fixture; editor autocommit, retained explicit transaction and guards around other operations.

Commands use `mise exec rust@stable -- cargo`. All other entries above are
source-audited claims, backed by existing tests where present, **not rerun or
newly natively accepted as part of this audit**. Detailed prior evidence is in
[PostgreSQL editing](POSTGRES_EDITING.md), [performance measurements](PERFORMANCE.md)
and `PROGRESS.md`.

Release remains blocked on requirements work, not on this document: additional
engine coverage, native transaction acceptance, richer visual administration,
multi-column sorting/filtering, native failure/close/reconnect matrices, and
platform/version/performance acceptance are unfinished. This matrix closes the
inventory task only; it does not close any of those implementation requirements.

### MySQL/MariaDB review transaction evidence (2026-11-01)

`live_review_transaction_holds_metadata_and_row_locks_until_rollback` passed
independently on disposable MySQL 8.4 and MariaDB 11.4 (ports 43316/43317).
It exercises the generated plan against two real connections:

- `START TRANSACTION` followed by `SELECT * ... LIMIT 0` retains a metadata lock.
  Competing ALTER TABLE and CREATE TRIGGER both receive lock timeout 1205.
- The generated original-value `SELECT ... FOR UPDATE` matches exactly one row;
  a competing update receives timeout 1205 while the review transaction holds it.
- Executing the planned update then rolling back restores the original value.
- A competing update succeeds after rollback; the old review precondition then
  returns no matches. A subsequent ALTER succeeds and changes the fingerprint.

This verifies the lock/revalidation ordering needed by the future apply path;
`MysqlDriver::apply` is still deliberately unimplemented and GUI editing remains
disabled. It does not prove trigger privilege visibility, transactional trigger
side effects, zero-change affected-row handling, cancellation cleanup, or ambiguous
COMMIT recovery. Those are remaining implementation/acceptance gates.

Reference semantics:
[MySQL metadata locking](https://dev.mysql.com/doc/refman/8.0/en/metadata-locking.html)
and [MariaDB transactions](https://mariadb.com/docs/server/reference/sql-statements/transactions/start-transaction).

### Stable MySQL/MariaDB review fingerprints (2026-11-01)

Review snapshots now tokenize the SHOW CREATE definition and exclude only the
unquoted, top-level `AUTO_INCREMENT=<integer>` table option. This counter changes
with ordinary inserts and is not a schema conflict. Column AUTO_INCREMENT
attributes, quoted identifiers/literals, comments, engine options and trigger
creation context remain fingerprinted. Malformed definitions fail closed.

New live coverage on both engines verifies the empty-to-first-insert transition,
an unrelated insert after review, an explicit counter advance, and detection of a
changed default containing the literal text `AUTO_INCREMENT=23`. Lexical tests
also retain the column attribute, comments and engine changes. All 22 MySQL driver
tests passed independently on MySQL and MariaDB; strict all-target Clippy passed.
This change does not enable the still-incomplete GUI apply path.

### Initial MySQL/MariaDB apply implementation (2026-11-01)

The driver now has an initial reviewed data-save implementation. The UI capability
remains disabled; this is not yet an officially supported editing workflow.
It starts one transaction, acquires table metadata locks in sorted order, checks
InnoDB and trigger visibility, rejects tables with triggers, revalidates schema
snapshots, checks original rows under FOR UPDATE, and executes parameterized writes.
Warnings (including non-strict truncation) reject and roll back the batch. No-op
updates accept zero changed rows only after the one-row original-value check.
Cancellation is checked before commit; rollback ignores the cancellation flag.
COMMIT transport/error outcomes disconnect and require verification, never replay.

Trigger visibility currently accepts explicit table or exact database grants only,
including MySQL's escaped database grant names. Global privileges, roles and broader
wildcard grants are deliberately not inferred. The restricted-account fixture shows
MySQL hides triggers, whereas MariaDB exposes their names but denies SHOW CREATE.
Both accounts are rejected before any write.

All 24 MySQL driver tests passed on each engine. New apply integration coverage:
commit visible to another session, unchanged updates, stale original rejection,
constraint failure rolling back an earlier write, non-strict truncation warning
rollback, trigger rejection, pre-cancellation, retaining user-owned transactions,
and changed schema rejection. Strict Clippy passed before the final test-only
restricted-account adjustment.

Before enabling the UI: implement/verify complete effective privileges and trigger
side effects; audit default/generated expressions and session constraint modes;
test cancellation during lock waits/writes, rollback transport failure and lost
COMMIT replies; expand inserts/deletes and native review UI acceptance. Also fix
MariaDB browsing so inaccessible SHOW CREATE TRIGGER metadata does not prevent
reading otherwise permitted table rows. These are remaining work, not reasons to
weaken the safety gate.

### Restricted-account trigger browsing fixed (2026-11-01)

Table browsing now retains a visible trigger as an Object with `metadata_error`
when SHOW CREATE TRIGGER returns permission errors 1142/1227 or no readable DDL.
The Triggers view shows "Trigger definition unavailable" with the server detail
on hover and omits the empty copy action. Its empty state says "No visible
triggers" rather than claiming that no hidden triggers exist. Cancellation,
connectivity and other server failures still propagate normally.

The restricted-account integration test now also browses and checks the actual
row values on each engine. MariaDB returns one unavailable trigger; MySQL hides
it entirely. Both still reject the reviewed save. These tests passed independently
on both fixtures, and strict all-target Clippy passed. Native visual acceptance
of the unavailable-definition label remains unverified.

### Mid-batch MySQL/MariaDB save cancellation (2026-11-01)

`live_review_cancel_after_first_write_rolls_back_batch_and_reuses_connection`
passed on each engine. A competing transaction locks the second row. A separate
test observer uses READ UNCOMMITTED to directly observe the first tentative write
before setting the save cancellation flag. The save returns an error within the
three-second bound, both rows retain their original committed values, transaction
state is Idle, and the connection ID remains unchanged. Clearing cancellation and
retrying the same reviewed batch then commits both rows successfully.

This covers cancellation after a real write during a later row-lock wait, rather
than just pre-cancellation. The original INNODB_TRX counter observation did not
reliably expose the first write, so the regression uses direct row evidence instead.
Strict all-target Clippy passed. Lost rollback/COMMIT replies and the other editing
gates above remain outstanding; the native UI save capability remains disabled.

### Session constraint enforcement guard (2026-11-01)

Before starting a reviewed save, MySQL/MariaDB now read session foreign_key_checks
and unique_checks; MariaDB additionally reads check_constraint_checks. A disabled
check rejects the save without altering the user's settings. Ordinary editor SQL
keeps its existing semantics and does not acquire this extra metadata round trip.
The live regression disables each supported setting separately, verifies the row
is unchanged and the setting remains disabled, restores it, and then saves the same
review successfully. All 26 driver tests passed on each engine; strict all-target
Clippy passed. This closes the session enforcement guard, not the other apply gates.

References: [MySQL session variables](https://dev.mysql.com/doc/refman/8.4/en/server-system-variables.html)
and [MariaDB CHECK enforcement](https://mariadb.com/docs/server/reference/sql-statements/data-definition/constraint).

### Insert/delete acceptance and export UI wiring (2026-11-01)

The live reviewed CRUD test passed on both engines: empty-field inserts use server
defaults, generated values are calculated by the server, Unicode/apostrophes and
binary bytes survive binding, explicit NULL stays distinct from omission, and
writes to generated/non-null columns are rejected appropriately. A delete followed
by a duplicate-key insert restores the deleted row on rollback. Delete/reinsert
with the same primary key succeeds, repeated deletes deduplicate, and a stale
second deletion is rejected. Strict Clippy passed for this regression addition.

Current-source inspection found `supports_table_export` still excluded MySQL and
MariaDB despite the implemented streaming backend. Both now expose All matching
rows as CSV/JSON through that existing menu and dialog path. The live full-export
filter/cancellation regression was rerun successfully on both engines. Native UI
click-through remains pending; GUI row editing remains gated independently.

### Lost transaction acknowledgements (2026-11-01)

A bounded local packet proxy now forwards the real fixture protocol, detects the
exact COM_QUERY COMMIT/ROLLBACK command, consumes the server's successful OK
reply, and closes both sockets before the client sees that reply. Two live tests
passed on both MySQL and MariaDB:

- Lost COMMIT acknowledgement: apply reports unknown outcome, disconnects, and an
  independent observer finds exactly one committed insertion. Reapplying on that
  disconnected driver fails without replay or a second insertion.
- Lost ROLLBACK acknowledgement after a truncation warning: apply says rollback
  could not be confirmed and disconnects. An independent observer finds no row;
  the client correctly does not infer that outcome from a missing reply.

Disconnected idle-operation checks now ask the user to reconnect, rather than
incorrectly asking them to finish a SQL transaction. The proxy uses only the opted-in
disposable fixture ports with TLS disabled and bounded packet sizes/timeouts.
Strict Clippy passed for the new fault tests before the final error-text adjustment;
both engine tests were rerun after that adjustment. The native UI editing gate
remains closed pending effective privileges, expression/trigger auditing and native
workflow acceptance.

### Invisible columns and expression audit (2026-11-01)

MySQL/MariaDB table browsing and full-table export now select the explicit catalog
column list in ordinal order. SELECT * omitted invisible columns while the catalog
included them, misaligning row values and preventing original-row checks when the
primary key was invisible. Handwritten SELECT * is unchanged. The live test covers
an invisible PK, a backtick-containing invisible column, a successful reviewed
update, rejection after a competing invisible-column change, and CSV headers/values.
MariaDB requires a default on its invisible non-null PK, so the shared fixture uses
an explicit default. All 30 driver tests passed on each engine; strict Clippy passed.

Expression audit references establish that MariaDB disallows stored functions in
DEFAULT and generated expressions, but allows loadable UDFs in generated columns
(and CHECK expressions). This is not evidence that arbitrary expression side
effects roll back. Keep the expression/trigger safety gate open rather than inferring
transactionality from the target table's InnoDB engine alone.

- [MariaDB defaults](https://mariadb.com/docs/server/server-usage/tables/create-table)
- [MariaDB generated columns](https://mariadb.com/docs/server/reference/sql-statements/data-definition/create/generated-columns)
- [MariaDB constraints](https://mariadb.com/docs/server/reference/sql-statements/data-definition/constraint)
- [MySQL default expressions](https://dev.mysql.com/doc/refman/8.0/en/data-type-defaults.html)

### CSV import capability consistency (2026-11-01)

UI audit found the MySQL/MariaDB CSV buttons disabled but commit_csv_import's
capability guard enabled, allowing modal Cmd+Enter/Cmd+S to dispatch an unsupported
import. The buttons and keyboard path now use the same SQLite/PostgreSQL-only
capability. CSV preview stays available and explains why import is unavailable.
Connection forms now accurately list SQL, browsing and exports as available, with
editing, CSV import and SSH unfinished. This prevents a dead-end operation while
preserving the requirement to implement MySQL/MariaDB import later.

All four CSV review keyboard tests passed, including both shortcuts for each
unsupported engine; existing busy, Escape and failed-import retention tests passed.
Strict all-target Clippy passed. Native visual review of the new explanatory text
remains pending.

### Quoted alias completion verification (2026-11-01)

MySQL/MariaDB local relation scanning now retains identifier quote information,
so quoted aliases containing spaces, escaped backticks, Unicode or reserved
keywords resolve to catalog columns. Quoted punctuation and keywords no longer
act as relation/clause delimiters. ANSI_QUOTES aliases follow the session mode.
Completion regression coverage verifies joined-table column isolation and exact
backtick-escaped insertion (19 completion tests passed). The live catalog/browse
fixture test now completes and executes qualified column SQL using actual catalog
metadata on both MySQL 8.4 and MariaDB 11.4; both passed. Strict all-target clippy
also passed. This improves local suggestions; it does not establish complete
CTE/subquery scope resolution or language-server parity.

### Batched table metadata reads (2026-11-01)

MySQL/MariaDB table browsing now sends SHOW CREATE TABLE and SHOW INDEX together
in one COM_QUERY on the existing connection, removing one sequential request
from each base-table metadata fetch. All result sets are consumed with errors
propagated, including errors after a successful first set. Catalog, data and
trigger requests remain separate; this is not yet parallel metadata/data loading
and no WAN timing improvement has been measured.

Both disposable engine suites passed (31 tests each), including temporary-table
metadata, unchanged connection ID, active-transaction preservation and rollback,
a deterministic later-statement error followed by successful connection reuse,
restricted trigger visibility, cancellation, and transport failures. Strict
all-target Clippy passed. The test also established that MariaDB SHOW INDEX on a
view need not fail; it is not used as a cross-engine error-injection assumption.

### Metadata wire-count and simulated-latency evidence (2026-11-01)

A loopback packet proxy now verifies that the definition/index pair emits exactly
one COM_QUERY instead of two and returns identical complete result sets. It also
calls the actual table_metadata path and verifies that its SHOW requests are
combined. The fixture uses a session-local temporary table, and all sockets are
closed and proxy threads joined at completion.

With a deterministic 30 ms delay added to each COM_QUERY request, five paired
samples produced these local medians:

| Engine | Separate reads | Batched reads |
| --- | ---: | ---: |
| MySQL 8.4 | 75.33 ms | 39.22 ms |
| MariaDB 11.4 | 78.28 ms | 41.46 ms |

This simulates per-request latency; it is not a real WAN or complete browse
benchmark. Assertions check packet counts and result equality, not wall-clock
thresholds. Both engine runs and strict all-target Clippy passed. Reproduce with
`RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql::transport_tests::metadata_batch_uses_one_wire_request -- --ignored --nocapture`
(use port 43317 for MariaDB).

### Verified TLS connections (2026-11-01)

Added separate disposable TLS-required MySQL/MariaDB fixtures and generated,
short-lived localhost certificates. Both actual servers now pass trusted-CA
success, untrusted-CA rejection, explicit hostname-verification failure, and
plaintext rejection tests. The native Rustable driver also passes initialization,
Unicode queries, catalog loading and same-connection reuse over verified TLS.
No verification bypass or fallback to plaintext was introduced. See
`tools/database-fixtures/README.md` for commands and remaining TLS UI coverage.

### TLS cancellation verification (2026-11-01)

The transaction/cancellation/reuse contract now runs against both plaintext and
verified-TLS fixtures. TLS-required servers accept cancellation on the separate
control connection; the primary connection retains earlier transaction changes,
rolls back explicitly, keeps its connection ID, and survives a subsequent slow
query without a late KILL. A separate fault test removes the control connection's
CA trust: certificate failure is reported, the query read deadline bounds the
wait, and the timed-out main connection is discarded. No plaintext fallback.

Combined `db::mysql` suites passed with both fixture types configured: 35 tests
on MySQL 8.4 and 35 on MariaDB 11.4. Strict all-target Clippy passed. Native
connection-form/keychain acceptance and broader production-readiness gates remain.

### Global trigger grants and live-session changes (2026-11-01)

Reviewed-save trigger visibility now also recognizes direct global TRIGGER grants
present in the connection's startup privilege snapshot and still present at save
time. A grant added after login cannot establish visibility for that existing
session: the live MySQL fixture showed information_schema reports the new global
grant while the session still hides the trigger. A fresh connection is required.
The startup snapshot adds one metadata request per connection, not per query.

MySQL global grants are accepted only with partial_revokes disabled. With it
enabled, a global grant may coexist with a schema restriction that hides triggers.
The focused fixture proves both the retained global metadata row and hidden
trigger, and verifies rejection before writes. Explicit table/exact-schema grants
continue to work. Roles, wildcard grants and restricted global grants remain
unimplemented; GUI editing remains disabled pending the other acceptance gates.

Validation: 36 driver tests passed on each MySQL 8.4 and MariaDB 11.4 fixture,
including TLS/cancellation/transport tests. The new live test checks successful
commit, actual-trigger rejection, a late grant on an existing session, and revoked
grants on a new session. The focused partial-revoke variant passed separately.
Strict all-target Clippy passed. An initial concurrent run exposed fixture
interference from partial_revokes changing schema-grant interpretation; this
server-wide test now requires an explicit opt-in and must run alone.

References:
- https://dev.mysql.com/doc/mysql-infoschema-excerpt/8.0/en/information-schema-user-privileges-table.html
- https://dev.mysql.com/doc/refman/8.4/en/partial-revokes.html
- https://mariadb.com/docs/server/reference/system-tables/information-schema/information-schema-tables/information-schema-user_privileges-table

### Generated/default column acceptance (2026-11-01)

Added a live reviewed-save acceptance test on MySQL 8.4 and MariaDB 11.4 for
virtual and stored generated columns, an arithmetic expression default,
CURRENT_TIMESTAMP defaults, and ON UPDATE timestamps. Catalog flags correctly
separate generated columns from writable default-expression/timestamp columns.
Both insert and update plans reject explicit writes to generated columns.
An omitted expression default evaluates to 5; updating its base value to 7
produces virtual/stored values 14/21, visible from a separate connection.
The ordinary creation timestamp stays unchanged while the automatic update
timestamp advances. Replaying the old review fails original-row checks.
Changing a generated expression after review rejects the save before mutation,
leaves the session idle, and preserves the base value.

The focused live test passed on each engine; strict all-target Clippy passed.
This verifies built-in expressions and computed-field behavior, not arbitrary
MariaDB UDF side effects. The broader expression/trigger and UI gates remain open.

### MySQL SSH transport prerequisite (2026-11-01)

The vendored MySQL client now supports a TCP destination independent of its TLS
peer name, allowing a future SSH loopback endpoint without weakening certificate
verification. Direct and overridden transport TLS tests passed on MySQL 8.4 and
MariaDB 11.4 (two tests each). The override uses a distinct destination port and
IP while keeping localhost as the certificate identity; wrong peer names and
untrusted certificates fail. Strict all-target Clippy passed.

This is transport plumbing only: MySQL/MariaDB SSH profile fields, tunnel lifetime,
connection probes and cancellation routing still need integration and live SSH
acceptance. The UI continues to describe SSH as unavailable.

### MySQL SSH driver wiring (2026-11-01)

MySQL/MariaDB connection configs and persisted profiles now carry optional SSH
settings, retaining only gateway/user/key paths. Passwords stay session-only.
The driver owns the tunnel lease, derives its loopback destination and applies
that destination to both the main connection and scoped cancellation connection.
TLS still uses the original database host; forwarded remote host/port are derived
from that same database configuration rather than trusting stale embedded values.
Worker connection/probe entry points now pass cancellation into tunnel startup.
Metadata access checks tunnel health; dropping the driver releases the lease.

Validation: 30 MySQL-filtered native tests passed; 38 driver tests passed on each
engine including direct TLS, cancellation, export and transport regression paths.
New unit coverage verifies SSH profile restoration without credentials and rejects
pre-cancelled/invalid SSH connections without direct fallback. Strict all-target
Clippy passed. These runs did not establish a live SSH tunnel: live SSH lifecycle,
TLS-through-tunnel/cancellation acceptance and connection-form/probe presentation
remain pending. The UI still labels SSH unavailable until that work is verified.

### Live MySQL/MariaDB SSH and form controls (2026-11-01)

Verified each TLS fixture through a disposable, key-authenticated OpenSSH gateway.
The driver overrides stale remote settings with the selected database endpoint,
retains TLS cipher verification, cancels a slow transactional write in under two
seconds and preserves the prior transaction value. The test invalidates the direct
config port after connecting, proving the cancellation connection uses the tunnel.
The next query succeeds on the same database connection; rollback restores the
original value, and driver drop closes the loopback listener. Unknown SSH hosts
and untrusted database certificates both fail closed. Both engine tests passed.

MySQL and MariaDB connection forms now use the same SSH controls as PostgreSQL,
including gateway/config-inherited port, user, agent and key-file picker. Their
connection probes include SSH fields in status reporting. Native form tests cover
both expanded engine forms and key selection; all 31 connection-manager tests
passed, and strict all-target Clippy passed. No desktop visual click-through was
performed in this pass. Built-in Windows SSH remains unsupported; remote jump-host
and gateway interruption acceptance remain follow-up coverage. This does not lift
the experimental database support or reviewed-editing gates.

### SSH connection-browser discoverability (2026-11-01)

MySQL/MariaDB saved connections now match searches by SSH gateway, user and port,
in addition to database endpoint metadata. The connection row displays the SSH
gateway using the same format as PostgreSQL. Empty database names no longer leave
a dangling separator in network endpoint rows. Key-file paths are not searched.
All 12 connection-profile tests passed, including case-insensitive gateway search,
engine filtering, stable result order and exclusion of key-file paths. Strict
all-target Clippy passed. This was not a new live database or visual acceptance run.
