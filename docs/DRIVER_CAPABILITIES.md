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
