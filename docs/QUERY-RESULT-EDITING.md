# Query-result editing review

PR #5 follow-up fixes:

- Rerunning a query preserves workspace edits, including edits from other tabs
  on the same table. Save still validates the original row before writing.
- SQLite result origin must be `main`; generated row writes and precondition
  checks explicitly qualify `main` to avoid temporary-table shadowing.
- PostgreSQL results use returned column relation OIDs and the existing catalog
  cache to pin the actual schema. Unknown origins remain read-only. No new
  metadata request is added to query execution.
- After saving, fetch only affected rows by their post-edit keys and replace their
  displayed values. Do not rerun the original SQL. Delete operations remove rows.
  Triggers, generated columns and server coercions must not leave fabricated
  values displayed as authoritative. Pinned snapshots remain unchanged.
- A failed readback is a stale-result error after a successful save. Clear staged
  changes and the save review; never offer to replay the committed write.

## Round-trip budget

Existing planning, validation and write/commit exchanges are unchanged. Previously
there were zero post-save result-read exchanges and the UI guessed saved values.
The added cost applies only when saved changes affect an editable query result:

| Driver | Additional exchanges per readback batch, cold / warm |
| --- | --- |
| PostgreSQL | 1 / 1, including uncached custom result types |
| MySQL and MariaDB | 2 / 1 (prepare + execute / cached execute) |
| SQLite | 0 network exchanges; one local read |

Rows from the same table are batched in bound UNION ALL reads, up to 64 rows per
batch with a parameter threshold. Each further batch adds the same cost. Ordinary
editor execution and table loading retain their previous budgets. Results are
bounded to 5,000 refreshed rows and a shared 16 MiB value budget.

This extra read is needed to obtain actual database values after commit. Cached
metadata cannot predict trigger effects. RETURNING alone does not cover SQLite
AFTER triggers or MySQL. Reads must follow successful writes, so they cannot be
pipelined ahead of commit without changing the correctness guarantee. Missing or
trigger-moved keys, changed columns and exceeded limits safely mark results stale.

## Validation

- 573 regular library tests passed; strict all-target Clippy passed.
- Disposable PostgreSQL, MySQL and MariaDB live readback tests passed, including
  changed keys, generated values, PostgreSQL triggers and cold custom types.
- SQLite regressions cover temporary-table shadowing, pending edits across tabs,
  trigger-produced values and successful commit followed by failed readback.
- PostgreSQL delayed-network editor benchmark: simulated 30 ms RTT, seven measured
  samples after two warmups, debug profile on the same Mac. One-exchange reference
  p50 38.5 ms; readback p50 38.2 ms / p95 45.9 ms. Editor p50 37.4 ms / p95 39.0 ms.
  The extra read therefore costs approximately one RTT, not a describe+execute pair.
  A regression assertion compares readback with the single-exchange reference.
- PostgreSQL browse WAN regression also passed: refresh p50 42.0 ms / p95
  44.1 ms; first sidebar load p50 40.8 ms / p95 47.3 ms.
- MySQL/MariaDB live correctness tested; delayed-network timing not measured.
  Native UI visual testing is not included in these database checks.

## Partial column selections

Direct column lists from a single base table are editable when every primary-key
column is returned, in any order. Staged edits carry column names alongside their
original values. Each driver validates those names and the complete key against
fresh metadata, checks the selected original values, and updates only edited
columns. Omitted values are not treated as NULL and are not fetched before editing.
Readback projects the same columns in the same order; batches are grouped by table
and projection. This adds no exchanges to detection, planning or saving relative
to the existing full-row path (one PostgreSQL readback per batch).

Missing keys produce an error on Enter/double-click instead of starting an editor.
Joins, expressions, duplicate columns and column aliases remain read-only. Row
deletion from partial projections is disabled; cell editing is supported.

Coverage includes reordered subsets, composite-key omission, an actual keyboard
edit attempt without the key, preserving omitted values, post-save triggers and
concurrent edits to selected values. Live PostgreSQL/MySQL/MariaDB fixtures also
exercise partial save/readback and reject missing keys.

Local follow-up validation: 576 library tests and strict Clippy passed. Existing
release-mode grid/selection/sidebar benchmarks against `bcfe84f` passed all p95
thresholds (five paired runs; largest increase 3.4%). Raw environment and timing
artifacts are in `target/ui-performance-pr5-projection`. These benchmarks cover
CPU layout/tessellation, not native display latency or the new save interaction.

## PostgreSQL citext results (v0.4.41)

SQL execution uses PostgreSQL's binary result protocol. Previously `citext` fell
through to `Value::Blob`, making an otherwise editable result cell read-only;
the table browser already used text-format results and did not have this problem.
`PgValue` now explicitly decodes the documented `citext` text wire format using
postgres-types' string decoder. Extension OIDs and schemas vary, so recognition
follows postgres-types' type-name convention, restricted to simple types. Domains
recurse through their base type, and arrays use the existing bounded array decoder.
Case and Unicode are preserved. Invalid UTF-8 fails decoding; unknown binary
formats remain opaque. The normal source-table, selected-primary-key, generated
column, read-only and save-concurrency guards still apply.

This changes only local decoding after existing result/type resolution. No new
SQL, preparation, metadata request, cleanup wait or retry is introduced. Tracing
`query_typed_raw_spooled_with_prefix` through `query::resolve_types` and
`prepare::get_type` gives unchanged established-connection exchange counts:
scalar citext is three exchanges on a completely cold type cache (execution,
type-info statement preparation, type-info query), two when that statement is
already cached but citext is not, and one once citext is cached. Cold array or
domain types add their existing type-info lookup; no lookup runs behind unread
rows. Table browsing remains text-format and retains its existing one-pipelined-
exchange cached-relation path, including cold custom types. MySQL/MariaDB have
separate value decoders and protocol paths; neither is changed by this fix.

Regression coverage includes bounded scalar/domain/array decoding, Unicode,
case, empty strings, NULLs, malformed UTF-8, opaque-type preservation, cold/warm
live results, empty result headers and explicit transactions. The PostgreSQL
query-result editing test now opens the actual citext cell editor with Enter,
stages the replacement, reviews/saves it, and verifies the stored and refreshed
values against the disposable fixture on loopback port 54329. The old decoder
fails the new scalar regression. Live tests require `RUSTABLE_POSTGRES_QA=1`.

Release validation on the disposable PostgreSQL 17 fixture passed: 955 ordinary
tests (273 ignored), strict all-target QA Clippy, both citext tests including the
opted-in live case, the live editor/save/reread test, all three cold custom-type
streaming tests, and both release-mode WAN regression tests. At 30 ms simulated
RTT (seven samples), editor lookup p50/p95 was 39.0/40.1 ms against a 36.9 ms
p50 single-exchange reference; browse/refresh was 40.5/42.7 ms, and initial
sidebar load was 40.6/43.7 ms against a 36.8 ms p50 reference. These are existing
regression workloads, not a before/after citext benchmark; unchanged exchange
counts are established by the decoder-only diff and protocol trace above. Raw
logs are retained under `target/release-v0.4.41/`. No rendering/input hot path
changes in this fix; native click measurements remain in UI-PERFORMANCE.md.
