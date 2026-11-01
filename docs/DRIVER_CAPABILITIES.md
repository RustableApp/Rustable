# Driver capability and transaction audit

Initial SQLite/PostgreSQL audit: 2026-10-31. Subsequent driver work is recorded
in the dated evidence ledger below. “Implemented” below means the
operation exists in the adapter and has the stated restrictions; it does not
establish native acceptance, server-version compatibility, or complete TablePlus
parity. The verification ledger below distinguishes newly run checks from source
inspection. Shared UI acceptance remains in [the parity requirements](TABLEPLUS_PARITY_REQUIREMENTS.md).

## Engines and operations

| Operation | SQLite | PostgreSQL | Source / limits |
| --- | --- | --- | --- |
| Saved connection | Existing local file; in-memory demo | Server, database, user, password, TLS and optional SSH | `src/connection_profiles.rs::ConnectionKind`; SQLite and PostgreSQL are the established adapters. Experimental MySQL/MariaDB adapters now implement connections, SQL, browsing, export and CSV import; Reviewed GUI row editing and CSV import are locally enabled with backend and app-level verification; visual schema administration remains gated. Native MariaDB CSV success has been verified; native row editing acceptance remains pending. Redis is not implemented. |
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

### One request for table definition, indexes and trigger names (2026-11-01)

MySQL/MariaDB table metadata now includes the trigger-name list in the same
COM_QUERY as SHOW CREATE TABLE and SHOW INDEX. This removes the additional
prepared request for that list. Trigger definitions are still fetched separately,
so unreadable MariaDB definitions keep their existing per-trigger warning behavior.
Schema/table literals use the driver's SQL-mode-aware serializer; identifiers
remain backtick-quoted. No additional connection or implicit transaction is used.

The wire-proxy test verifies one actual table_metadata COM_QUERY and identical
result sets versus three separate text requests. With 30 ms injected per request,
five-sample medians were MySQL 118.05 ms separate / 43.61 ms batched, and MariaDB
118.46 ms / 42.95 ms. This is the three-read metadata comparison, not total browse
latency or a comparison to the previous two-read-batch implementation.

All 40 driver tests passed on each engine, including SSH/TLS and unreadable trigger
metadata. New live coverage preserves indexes and trigger DDL for apostrophe,
backtick and Unicode table/trigger names under default, NO_BACKSLASH_ESCAPES and
ANSI_QUOTES+NO_BACKSLASH_ESCAPES modes. Strict all-target Clippy passed.

### Native MySQL acceptance and result timing (2026-11-01)

The debug native app at e0e8a2c connected to the disposable MySQL 8.4.11 TLS
fixture through the connection form: custom CA, Verify full, successful
“TLS verified” probe, then the workspace. Cmd+Enter with the caret in the second
statement ran only `SELECT 42`; Cmd+Shift+Enter returned both result sets,
including the server version and connection ID. Japanese text rendered correctly;
the crab emoji rendered as a missing glyph in both editor and grid (font fallback
still needs work). No production connection was used.

This uncovered a misleading result footer: 0.0 ms because timing began only after
query_iter had received the first server response. Editor and browse streams now
start timing before dispatch; subsequent result sets include their response wait.
The script total remains a separate end-to-end measurement. A live regression uses
150 ms and 50 ms server sleeps and checks each statement's elapsed time and the
sum against the script total. It passed on both MySQL 8.4.11 and MariaDB 11.4.13;
strict all-target Clippy passed. The updated timing has not yet been visually
rechecked in a rebuilt native bundle. Native MariaDB UI acceptance and emoji
fallback remain pending.

Local screenshot evidence: `native-mysql-tls-probe.png` and
`native-mysql-query-timing-before.png` in the session visualization directory
`~/.codex/visualizations/2026/10/31/01a18f82-8ef6-7c12-b1e5-94903450cc37/`.

### Emoji fallback and timing UI recheck (2026-11-01)

A rebuilt native acceptance bundle now renders Japanese plus crab, smile and rocket
characters in the SQL editor and MySQL result grid. The same query's `SLEEP(0.15)`
reported 164 ms in the footer and 164.0 ms in the script header, verifying the
previous timing fix visually. Evidence: `native-mysql-emoji-and-timing.png` in the
session visualization directory above.

Bundled Noto Emoji outlines supplement the default faces for both proportional
and monospace text, including WASM's font installation path; no runtime download
is needed. The unmodified font and OFL license are under assets/fonts. Native
font coverage tests and strict all-target Clippy pass. This does not implement
color emoji or combined emoji sequence shaping, and this font change has not yet
been browser-rendered or deployed.

### Native MariaDB acceptance and temporary-table browsing (2026-11-01)

The native acceptance app connected to the disposable MariaDB TLS fixture with
Verify Full and its fixture CA; Test reported TLS verified. Separate editor runs
retained connection ID 3767 and an explicit transaction: an update changed 7 to 9,
then ROLLBACK restored 7 and cleared the transaction indicator.

Opening that session's temporary table exposed a real browse failure: MariaDB
listed it in TABLES but returned no COLUMNS rows. The browse/export projection now
falls back to session-local SHOW FULL COLUMNS when catalog columns are absent
(including MySQL, where the table itself is absent). Missing or inaccessible tables
still return the server error. Ordinary catalog-backed tables incur no extra read.

`live_browse_session_temporary_table` passed independently on MySQL (43316) and
MariaDB (43317): quoted table name, generated output, PK/index metadata, descending
sort, missing-table error and idle-session preservation. The updated binary has
not yet received native acceptance. Temporary tables shadowing persistent tables
remain a separate metadata-resolution gap; this fallback does not resolve that
case, and reviewed GUI editing remains disabled.

### Session-resolved browse metadata (2026-11-01)

The temporary-table shadowing gap above is now fixed for browse/export column
resolution. SHOW FULL COLUMNS and SHOW CREATE TABLE share one protocol request;
this replaces the catalog read rather than adding a round trip to ordinary table
loads. Both resolve the object visible to the current session, including a temporary
table with the same name but different columns as a persistent table. Temporary
table metadata excludes catalog triggers belonging to the hidden persistent table.
Views without SHOW VIEW permission fall back to catalog object kind after their
session-resolved columns have been read; SELECT-only browsing remains available.

Live checks passed separately on MySQL and MariaDB: temporary table shadowing with
a different primary-key name and a trigger on the persistent table, SELECT-only
view access, locked-table cancellation, ordinary catalog/query/browse workflows,
and invisible-column browse/export alignment. GUI editing remains gated; this
change does not establish safe planning against shadowed persistent tables.

### Reviewed-save temporary-table exclusion (2026-11-01)

Reviewed schema snapshots now reject SHOW CREATE definitions for temporary tables.
This applies both while planning and while revalidating under the apply transaction,
so information_schema metadata for a hidden persistent table cannot authorize a
write to its temporary replacement. GUI editing remains gated; temporary tables
are browse/query/export targets, not reviewed-save targets yet.

`live_review_rejects_temporary_table_shadowing` passed on both disposable engines.
It creates a persistent InnoDB table and reviews an update, then shadows it with
identically shaped MyISAM and InnoDB temporary tables. Planning and applying are
rejected before any write; both the temporary and persistent values remain 7,
transaction state is idle, and the original reviewed update changes the persistent
value to 9 after the temporary replacement is removed. MariaDB independently rejects
the MyISAM case at its engine check; MySQL's persistent catalog engine requires the
new session-resolved definition check. The InnoDB temporary case exercises the new
exclusion on both engines.

### Native temporary-table browse acceptance (2026-11-01)

Rebuilt the disposable native acceptance bundle from `392a46a` with mise/Cargo.
Connected through the MariaDB form to localhost:43319 using Verify Full and the
fixture CA, without remembering the password. Cmd+Shift+Enter executed CREATE
TEMPORARY TABLE, INSERT and SELECT as three result sets. Double-clicking the new
sidebar item opened both rows successfully, replacing the previously observed
“No readable columns” error. Refresh retained the correct values; selecting the
first row populated all three inspector fields (id=1, label="🦀 fixed", doubled=2).
The generated second-row value was 4, and the outline crab rendered in both grid
and inspector. Evidence: `native-mariadb-temporary-browse-fixed.png` in the session
visualization folder. This is native acceptance of browsing/refresh/inspection,
not GUI editing or full MySQL/MariaDB production acceptance.

### Reuse prepared browse statements across pages (2026-11-01)

LIMIT and OFFSET are now bound values in MySQL/MariaDB browse requests. Changing
the page offset previously changed the statement text and required another
COM_STMT_PREPARE; the same table/filter/sort now reuses the connection's prepared
statement cache. Binary result decoding remains in use for accurate FLOAT and
other typed values. The SQL activity payload carries the bound pagination values.

Extended the disposable protocol-proxy check to populate 502 rows, browse the
first 500, then fetch rows 500–501. On each engine it observes one prepare for the
first page and no prepare for the second, with one execute and correct results.
With 30 ms artificial delay per QUERY/PREPARE/EXECUTE request, this run measured:
MySQL first/next page 151.35/111.43 ms; MariaDB 158.12/105.35 ms. These are diagnostic
fixture timings (different row counts), not production latency benchmarks or a
claim that metadata/data now run concurrently. Existing ordinary/filter/sort browse
tests passed on both engines; strict all-target Clippy passed.

### Initial MySQL/MariaDB CSV import driver (2026-11-01)

Added a streaming driver path using prepared INSERT batches of at most 64 rows,
a 1 MiB batching threshold (individual records retain the shared CSV parser's
limits), and one transaction for the entire source snapshot. Mapping is validated,
column names are quoted, values are bound, defaults apply to omitted columns, and
empty-to-NULL follows the request. The transaction holds metadata while checking
current columns, InnoDB, constraint enforcement, temporary-table exclusion and
trigger visibility; tables with triggers remain unsupported. Every batch checks
warnings and affected rows. Errors roll back all batches; unknown commit/rollback
outcomes disconnect without replay.

`live_csv_batches_are_atomic_and_validate_destination` passed on MySQL and MariaDB:
130-row multi-batch Unicode import, omitted defaults/generated values, exact large
integer/NULL mapping, duplicate failure after an earlier successful batch, schema
change rejection, pre-cancellation, non-strict conversion-warning rollback, retry,
and active-transaction exclusion. Strict all-target Clippy passed.

This does not enable UI import. Remaining gates include in-flight cancellation,
transport-failure tests, schema identity across preview/database switches, broader
expression/privilege coverage and native mapping/review/import acceptance.

### CSV cancellation and destination database binding (2026-11-01)

An in-flight cancellation test now verifies an actual completed 64-row batch via
an independent READ UNCOMMITTED observer while a second connection blocks the
last insert. Cancellation rolls back all 64 rows, leaves the driver idle on the
same connection ID, and permits a successful 65-row retry. Both MySQL and MariaDB
passed, with a three-second cancellation bound in the disposable local test.

Import mapping/review now retains the destination schema from fresh metadata.
MySQL/MariaDB require that captured database to match the active session; absent
identity or a USE/database change rejects the request before beginning an import.
Live tests switch to information_schema after constructing a request and verify
rejection, then return to the original database and verify normal operation.
The UI preserves the schema when re-previewing the same CSV with changed options.

Both CSV live tests passed on both engines. Full library suite: 433 passed,
92 fixture/manual tests ignored. Strict all-target Clippy passed. GUI import is
still gated pending transport failure, expression/privilege and native acceptance.

### CSV transaction reply-loss acceptance (2026-11-01)

Extended the disposable protocol fault proxy to exercise both CSV import and
reviewed edits. For CSV COMMIT it consumes the real server OK packet and closes
the connection before the client sees it: the independent observer sees exactly
one committed row, while the driver reports unknown commit outcome, publishes
Disconnected, and rejects replay. For ROLLBACK, non-strict conversion warnings
force cleanup; withholding the server OK likewise disconnects without claiming
rollback was confirmed. The observer sees zero rows and repeat import is rejected.

All four reply-loss tests passed independently on MySQL and MariaDB. Strict
all-target Clippy passed. These checks cover lost transaction acknowledgements;
other transport phases, effective-privilege/expression coverage and native import
acceptance remain distinct work. UI import remains gated.

### MySQL CSV review SQL (2026-11-01)

The shared review flow now selects MySQL-specific INSERT generation for MySQL and
MariaDB, quoting the captured database, table and column names with backticks.
Display-only expansion uses the existing MySQL literal renderer, including safe
representation of backslashes/NUL independently of session escaping mode. Driver
validation uses the same generated request contract; writes remain bound batches.
Unit coverage verifies backticks and literal question marks in identifiers plus
apostrophes/backslashes in values. Both live CSV tests passed again on each engine;
strict all-target Clippy passed. This prepares review for UI enablement but does
not change the current disabled import capability or close native acceptance.

### MariaDB import expression validation (2026-11-01)

Managed CSV imports now inspect column defaults, generated expressions and CHECK
clauses while holding destination metadata locks, before inserting any batch.
MariaDB explicitly permits native UDFs in generated columns and CHECK expressions:
https://mariadb.com/docs/server/reference/sql-statements/data-definition/create/generated-columns
https://mariadb.com/docs/server/reference/sql-statements/data-definition/constraint

A conservative, session-mode-aware token validator permits common scalar builtins,
arithmetic and literal defaults. Unknown, quoted or schema-qualified function calls,
comments (including executable comments) and NEXT VALUE sequence syntax are rejected.
This is intentionally not a full expression interpreter: safe functions outside the
current list are also rejected, with the column/constraint and function in the error.
It does not assert that arbitrary native UDF effects are transactional.

Unit coverage includes nested/disguised calls, quoted string contents, SQL modes,
sequence syntax and ordinary generated/CHECK expressions. Disposable MariaDB tests
exercise rejected defaults, generated columns and CHECKs before inserting any rows,
then restore supported expressions and complete the import. MySQL import and
cancellation tests also pass. Full library suite: 436 passed, 94 ignored.
Strict all-target Clippy passes.

This closes part of the expression gate, not the entire MySQL/MariaDB release gate:
effective privileges, managed-edit expression validation, remaining transport phases
and native import acceptance still need work. The import UI remains disabled.

### Managed-save expression validation (2026-11-01)

The MariaDB expression validator is now shared by CSV imports and reviewed table
saves. Apply checks every directly edited table under its metadata lock, after
verifying the reviewed schema and before running any row precondition or write.
Error messages refer to managed saves rather than imports alone.

A two-table live test puts unsupported default, generated and CHECK expressions
on the second table in turn. Each rejected batch leaves both tables empty and the
connection idle; restoring supported expressions allows both inserts, including
default and generated values. All 11 live reviewed-save tests pass on MySQL 8.4
and MariaDB 11.4, covering conflict checks, warnings, rollback, cancellation,
privileges, temporary replacements and metadata locks. Strict Clippy also passes.

This does not enable the editing UI. Indirect effects through foreign-key cascades,
effective privilege coverage and native editing/import acceptance still require
verification before declaring these workflows supported.

### Limited-account CSV acceptance and MariaDB metadata panic (2026-11-01)

A disposable account with only SELECT/INSERT on one table exposed a real MariaDB
11.4 failure: information_schema.TRIGGERS returned a visible trigger name but NULL
ACTION_STATEMENT and DEFINER. Converting those fields to String panicked while
building a review/schema snapshot. Both fields now preserve NULL; imports also
establish trigger visibility before reading the snapshot. Apply still requires
complete visibility and rejects tables containing triggers.

The new live test verifies that both review/apply and CSV import reject the limited
account without mutation, that granting table-level TRIGGER reveals and rejects the
actual trigger, that removing the trigger permits import, and that revoking INSERT
rejects a subsequent import without changing existing data. Both engines pass.
All 11 reviewed-save regression tests pass on each engine; strict Clippy passes.
Import UI enablement/native acceptance remains pending.


### Experimental MySQL/MariaDB import UI enabled locally (2026-11-01)

The shared CSV preview, destination mapping and confirmation workflow now permits
MySQL/MariaDB imports. The review explains the current InnoDB/no-trigger/visibility
restrictions; the connection editor no longer says CSV import is unavailable.
Table editing remains gated. No release or deployment is implied by this change.

An ignored, disposable-server app integration test connects through the real worker,
loads a CSV preview, requests destination metadata, verifies schema binding and
reordered header mapping, then confirms with Cmd+Enter and Cmd+S. It verifies
Unicode data reaches the database, successful imports dismiss the review/preview,
and duplicate-key failures retain the review without adding rows. It passes on
both MySQL 8.4 and MariaDB 11.4. Existing busy/read-only/shortcut ownership and
unsupported-engine checks pass. Full library suite: 436 passed, 97 ignored;
strict all-target Clippy passes.

Native preview/mapping/review screenshots, cancellation interaction, and version
compatibility remain release acceptance work. This is experimental functionality,
not a declaration of official MySQL/MariaDB parity.

### Native MariaDB CSV import acceptance (2026-11-01)

Built source 95cce29 with mise/Cargo and replaced only the disposable
/tmp/Rustable Acceptance.app binary. Connected to localhost:43319, rustable_test,
using Verify Full TLS and the fixture CA; password remembering remained off.
The production connection was not opened.

Created native_csv_acceptance with id/label and a stored doubled=id*2 column.
Opened /tmp/rustable-native-import.csv through the native file picker. The CSV
headers were label,id (opposite table order), with Rusty crab plus a crab emoji
and Japanese text. Selecting the destination automatically mapped both headers;
the generated column was omitted. The review displayed syntax-highlighted,
database-qualified INSERTs with the actual values. Cmd+Enter confirmed, dismissed
both dialogs and refreshed the grid to two rows with doubled values 2 and 4.

Evidence under the session visualization directory:
- native-mariadb-csv-mapping.png
- native-mariadb-csv-review.png
- native-mariadb-csv-imported.png

The acceptance app remains connected to the disposable fixture with these two
rows available for follow-up. Native cancellation/error interaction and MySQL
visual acceptance remain outstanding (both engines already have app integration
and driver failure/cancellation coverage). Discoverability issue observed: the
CSV entry point is currently named Preview CSV file under Export; improve the
import/export entry point before declaring the workflow polished.

### CSV import discovery (2026-11-01)

The table toolbar now labels its menu Import / Export, and the file picker entry
is Import CSV file. Cmd+P Find Anything also includes Import CSV file, using the
existing preview/mapping workflow and respecting the navigator's busy guard.
The toolbar geometry measures the longer label rather than allocating the old
Export width. Existing narrow-window/pending-change toolbar layout and navigator
keyboard ownership tests pass; all-target Clippy passes. The native acceptance
binary remains at 95cce29 until the next rebuild.

### Reviewed foreign-key cascade acceptance (2026-11-01)

A new disposable integration test verifies parent-key updates and deletes with
both CASCADE and SET NULL children. For each operation, a later duplicate-key
failure restores the child rows/foreign-key values and leaves the session idle;
retrying the valid parent change commits the expected child effects. The test
passes on MySQL 8.4.11 and MariaDB 11.4.13. Child UPDATE/DELETE triggers write to a
MyISAM audit table so even a rolled-back accidental trigger effect would remain
observable. The audit stays empty throughout cascaded changes; a direct child
update then verifies the trigger/audit fixture is functional. Clippy passes.

Version-specific behavior matters:
https://dev.mysql.com/doc/refman/8.4/en/create-table-foreign-keys.html
https://mariadb.com/docs/server/ha-and-performance/optimization-and-tuning/optimization-and-indexes/foreign-keys
https://dev.mysql.com/doc/refman/9.7/en/create-table-foreign-keys.html

MySQL 9.7 documents configurable cascade-trigger execution, unlike the tested
8.4 behavior. Do not extrapolate the test to that mode or declare all MySQL
versions equivalent. Native editing acceptance, generated-expression effects in
indirectly changed tables, and compatibility with newer cascade modes remain
open before enabling table editing.

### Generated columns on cascade destinations (2026-11-01)

Extended the reviewed-cascade fixture with virtual parent_twice=parent_id*2 on
both CASCADE and SET NULL children, plus stored row_twice=id*2 on the CASCADE
child. On both MySQL 8.4.11 and MariaDB 11.4.13, the virtual value changes from 2
to 6 when the parent key changes 1 to 3, becomes NULL after SET NULL, and returns
to its original value after a later duplicate-key failure rolls back the batch.
The independent stored value remains 20. All assertions pass; strict Clippy passes.
This establishes common builtin generated-expression behavior on indirect writes,
not arbitrary child UDF effects or newer MySQL cascade-trigger modes. The directly
edited-table expression gate remains unchanged. Next editing work should address
server-mode compatibility and the app/native edit-review-confirm path rather than
repeating these successful cascade tests without a relevant change.

### Experimental grid editing enabled locally (2026-11-01)

MySQL/MariaDB now use the shared table-cell editing and reviewed-save UI when
column metadata is available. Existing primary-key, base-table, busy and read-only
gates remain in effect. Backend restrictions still reject unsupported storage,
triggers, visibility and expressions rather than silently bypassing checks.

The new live application test opens a real table through the worker, stages a
Unicode cell edit, checks that preview has not written anything, confirms the
review and verifies refreshed grid values. It then modifies the same row from an
independent connection and confirms that save fails, the external value survives,
and pending local changes and review remain available. It passes on both local
MySQL 8.4 and MariaDB 11.4 fixtures.

Reviewed MySQL updates/deletes now inspect the cascade implementation settings.
Legacy servers without these settings retain their existing behavior. Native
InnoDB cascades or explicitly disabled cascade triggers are accepted; unknown or
active SQL-layer cascade-trigger configurations are rejected before the write
transaction. The application does not change either server setting. Unit coverage
checks setting combinations; a real MySQL 9.7 configuration remains unverified.

Validation: all 12 live reviewed-write regressions pass on each fixture; the new
application test passes on each; the library suite reports 437 passed, 99 ignored;
Clippy all-targets with warnings denied passes. Native grid editing interaction,
newer server versions and wider effective-privilege coverage remain release gates.
No push, deployment or release accompanies this local change.

### Native MariaDB grid editing acceptance (2026-11-01)

Built revision 53c7d6a using mise/Cargo and installed it into the local acceptance
app bundle. Connected to the MariaDB fixture at localhost:43319 with Verify Full
TLS and the fixture CA; no production connection was opened.

On native_csv_acceptance, double-clicked the label for id=2, entered Japanese text
plus an ASCII suffix, pressed Return, then Cmd+S. The review showed highlighted,
schema-qualified UPDATE SQL with inline values. Cmd+Enter saved and refreshed both
the grid and inspector; pending indicators disappeared.

Changed the same row's primary key to the existing id=1. Confirmation reported
server error 1062 inside the review and retained the pending edit. Back to editing
returned to the highlighted pending cell. Correcting the value to id=3, reviewing
again and confirming succeeded: the selected row followed its new identity and
the stored generated doubled column refreshed from 4 to 6. The error cleared.

Screenshots are under the local acceptance evidence directory:
- native-mariadb-grid-review.png
- native-mariadb-grid-saved.png
- native-mariadb-grid-rejected.png
- native-mariadb-grid-recovered.png

This verifies native update, rejection and correction on MariaDB 11.4. It does not
cover native insert/delete/cancel, MySQL visual acceptance, or additional server
versions. The fixture remains with ids 1 and 3 for subsequent checks. No release.
