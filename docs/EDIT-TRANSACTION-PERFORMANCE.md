# Fewer PostgreSQL edit-transaction exchanges

This records the intermediate six-exchange phase. The [server-side guard follow-up](EDIT-SERVER-CHECKS-PERFORMANCE.md) reduces ordinary data preparation and commit to three exchanges on first use and two on warm saves, including multi-table saves.

This follows the [fresh metadata optimization](EDIT-PLANNING-PERFORMANCE.md).
The first phase reduced preview preparation; this phase reduces the subsequent
transaction waits without moving validation past the writes it protects.

## Protocol and ordering

For a one-table, one-row update with one original-row check:

| Step | Metadata-only implementation | Current implementation |
| --- | ---: | ---: |
| Generate validated plan | 1 | 1 |
| BEGIN READ WRITE | 1 | 1 |
| Table lock and fresh metadata | 3 | 1 |
| Original-row check and row lock | 2 | 1 |
| Parameterized update | 2 | 1 |
| COMMIT | 1 | 1 |
| Total, including planning | 10 | 6 |

The synchronous transaction now exposes the existing bounded prefixed-query API.
LOCK is a separate Parse/Bind/Execute operation before the metadata query, with a
shared Sync. PostgreSQL executes the lock before taking the metadata query's
snapshot. A failed lock skips the metadata query, and the transaction's existing
RAII rollback handles the error. A concurrency fixture commits DDL while the save
is waiting on its lock and verifies that the subsequent metadata sees the change
and rejects the save without updating the row.

Checks use the same bounded single-exchange API and built-in INT8 results.
Writes use the existing execute_typed API: unnamed Parse/Bind/Describe/Execute/Sync,
with all generated parameters bound as TEXT and their existing explicit casts.
Affected-row counts come from CommandComplete; ReadyForQuery is drained before
returning. No prepare-first wait, named-statement cleanup wait, custom-type lookup,
or additional connection is introduced. Behavior is the same on first/repeated
use and with cold enum/domain table types.

The application still checks each original-row count before sending the write,
checks affected-row counts before continuing, and checks cancellation before
COMMIT. COMMIT is never pipelined with an unchecked write. Transaction begin,
validation and commit remain distinct barriers. Lock order, schema fingerprints,
parameter values, per-statement checks and write order are unchanged. No retry is
introduced for an unknown commit outcome. Explicit transactions are still refused.

This targets PostgreSQL only. MySQL/MariaDB use different prepared-statement and
transaction APIs; their paths were inspected and are unchanged. SQLite is unchanged.
Rendering/input paths are unchanged, so this does not require a UI rendering benchmark.

## Verification

The retained same-process baseline has three paths: original v0.4.38, metadata-only
optimization, and this transaction optimization. The same table, edit and results
are used for all paths, and complete plan JSON equality is asserted. One initial
sample and five alternating repeated samples per path run through a proxy adding
15 ms each direction. Timings include planning, transaction commit and cancel-watch
cleanup, but exclude confirmation time and subsequent UI/result refresh.

The proxy also parses frontend protocol messages and asserts exactly six request
groups (Query or Sync) for the optimized plan-and-apply operation, on first and
repeated use. This is not packet counting. The optimized path has no deferred
Close/Sync requests to confuse the count. The threshold requires at least 25%
less remaining latency than the metadata-only implementation; no previous
performance threshold was weakened.

Focused live tests cover exact metadata equivalence, complex values/domains,
changed schemas/types, triggers changing other rows, suppressed writes, row-count
mismatch, cancellation while waiting for a row lock, atomic rollback, and session
reuse. A failed lock prefix is followed by a nextval probe: the sequence remains
uncalled, proving that the query was skipped rather than merely rolled back.
Dropped-table lock errors also roll back and permit a subsequent successful save.

Artifacts are retained under `target/edit-transaction/`; the disposable PostgreSQL
17 fixture binds only loopback port 54329 (`rustable-edit-transaction-qa`).

## Measured results

macOS 27.2.1 ARM64, Rust 1.99.1, optimized release profile, PostgreSQL 17.11
ARM64 disposable fixture; no competing benchmark/build workload. At 30 ms
simulated RTT, five repeated samples per path gave:

| Plan + apply through commit | Median |
| --- | ---: |
| Original v0.4.38 implementation | 880.54 ms |
| Metadata-only optimization | 384.89 ms |
| Transaction optimization | 233.76 ms |

This removes 39.3% of the remaining time and 73.5% relative to the original path.
First samples were 892.88, 379.26 and 247.48 ms respectively. Planning itself
remains one exchange; cold custom-type decoding is not introduced. Results are
for a one-row update, not a promise of identical timings on every server.

Final validation: 11 focused editing/structure tests passed, including the live
lock-prefix and concurrent-DDL regressions. The release-mode library suite passed
941 tests (263 opt-in tests ignored). The editor and browse WAN regression tests
passed with their original thresholds, and strict all-target QA Clippy passed.
The test fixture was removed after verification. No native UI rendering or
input-to-display performance claim is made by these database benchmarks.
