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
