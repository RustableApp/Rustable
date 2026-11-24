# Server-side PostgreSQL edit guards

This follows the [six-exchange implementation](EDIT-TRANSACTION-PERFORMANCE.md).
The ordinary data-edit path needs three sequential exchanges on first use or
a cache miss, including fresh preparation:

1. Fetch all affected tables' complete editing metadata in one bounded request.
2. Start a read-write transaction and execute a generated PL/pgSQL block. Lock
   tables in deterministic order, revalidate their full fingerprints, then check
   each original row immediately before its write and verify every affected-row
   count. Any failure raises an error and rolls back.
3. After checking client cancellation, commit and read the server's response.

The second exchange uses one simple-query message containing START TRANSACTION
and DO. There is no installed helper, prepared-statement initialization, custom
type lookup, deferred statement close, ping, or hidden setup exchange. The
additive transaction-builder API keeps rollback-on-drop and drains rollback on
initialization failure. Ordinary editor/browse APIs are unchanged.

## Safety and compatibility

The server performs the same schema, type, original-row and affected-row checks
as the client path, in the same order. Catalog reads execute dynamically after
acquiring locks, so DDL committed while waiting for a lock is visible. The server
reconstructs the existing fingerprint tuple; JSON integer fields compare as
JSONB, while type-signature strings retain their exact contents. Existing
generated SQL and text parameter values are unchanged. EXECUTE USING binds each
value as text with the original server-native casts. Dollar-quoted literals use
a delimiter absent from their contents, independent of
standard_conforming_strings; NULs and oversized batches are rejected locally.
SQL and values are never joined by substituting parameter placeholders.

The DO block runs with the caller's privileges. Planning includes a PL/pgSQL
availability/USAGE check in the existing metadata request. If unavailable, or for
older plans without the capability flag, the six-exchange client-check path is
retained. Revocation after review fails safely; an execution error never triggers
automatic fallback or replay. No function or extension is installed on the user's
database. The batch and metadata responses use the existing 16 MiB limit.

Cancellation still uses PostgreSQL cancellation while work is running and a
client check before committing. A separate commit exchange deliberately remains:
embedding COMMIT with the writes would remove that client cancellation boundary.
All writes are transactional, and an unknown commit outcome is never retried.
Statement timeout now covers the whole server block, rather than resetting for
each guard/write. A timeout rolls the batch back. Trigger functions still execute
once per original statement; their current_query() text reflects the enclosing
block, as with other procedural PostgreSQL callers.

The optimization applies to generated PostgreSQL edits, including structure
transactions. Structure planning may still require additional type-resolution
and modifier-validation requests. MySQL/MariaDB's separate prepared statement
and transaction paths were inspected and are unchanged; this PostgreSQL-specific
procedural batch is not portable to them. SQLite and rendering/input paths are
unchanged.

## Bounded preparation cache

After fresh data preparation, the connection retains only the last set of full
editing catalogs, capped by a 16 MiB size estimate. Only canonical, fully quoted
schema/table keys are eligible: unqualified names always resolve afresh. Cached
preparation is allowed only in an idle transaction, and uncommitted metadata is
never retained. SQL-editor execution, catalog refresh, schema writes and any
apply failure invalidate the cache. A local planning error retries fresh read-only
preparation; writes are never retried.

A cache hit removes the first exchange; it does not remove or cache the locked
server-side validation. External DDL can make a preview stale, but the resulting
save is rejected before writing and invalidates the cache so the next review is
fresh. Tests change a column type behind the cache, verify rejection, and then
verify a fresh successful save.

## Limit and measurement scope

Two exchanges are the minimum for a warm save while retaining the final client
cancellation boundary: guarded execution must return before that check, then
commit must be acknowledged. First use needs one additional metadata response
before Rustable can construct its validated plan. An already reviewed plan also
uses just the last two exchanges. Cached previews need no database exchange.
Data row count and affected table count do not add exchanges. A further reduction
would require committing before the final client cancellation check, or making
first-use metadata available earlier at another user-facing operation's expense.

This is a network-dependency limit, not a guarantee of constant execution time:
server work, lock waits, constraints, triggers, payload transfer and disk commit
still cost time. Confirmation, UI dispatch/rendering and post-save readback are
outside these plan-and-apply measurements. A disconnected/error session may need
cleanup/reconnection; that is not a healthy-connection fast path.

The regression benchmark compares original, metadata-only, six-exchange and
three-exchange implementations in the same process using identical edits and
whole-plan JSON equality. A proxy adds 15 ms each way and counts frontend Query
and Sync messages. It asserts exactly three groups on first/repeated saves, 128
independent row updates, and a two-table save. It separately asserts zero requests
for cached preparation and exactly two for a complete warm save. Existing timing thresholds remain
and an additional threshold requires at least 25% less time than six exchanges.

Live safety tests include conflict-before-write and row-count failure probes
using non-transactional sequence counters, concurrent DDL, trigger interactions,
custom types, cancellation, fallback, quoting, and atomic multi-table failure.
A protocol proxy drops the COMMIT completion after the server commits: Rustable
reports failure and the trigger sequence proves that the write ran exactly once.

## Measured results

Apple M4 Max (Mac16,6), 48 GiB RAM, macOS 27.2.1 ARM64, Rust 1.99.1,
PostgreSQL 17.11 ARM64 disposable Docker fixture, release profile. Benchmarks ran
serially without competing builds or benchmark workloads. Each comparison path
has one first-use sample and five repeated samples, with alternating path order.
All values below include preparation and apply through commit, at 30 ms simulated
RTT; they are not production-server measurements.

| Path | First sample | Repeated median |
| --- | ---: | ---: |
| Original v0.4.38 | 964.63 ms | 986.98 ms |
| Metadata-only optimization | 421.68 ms | 429.98 ms |
| Six-exchange optimization | 280.68 ms | 269.73 ms |
| Server guards, uncached preparation | 161.52 ms | 144.87 ms |
| Server guards, cached preparation | 85.63 ms | 92.88 ms |

The warm path removes 90.6% of original elapsed time and 65.6% relative to the
six-exchange implementation in this run. Uncached server guards remove 85.3%
relative to the original. Cached preparation alone has a 0.223 ms repeated median;
uncached preparation remains a fresh network request (55.21 ms here). Cache-hit
samples follow the alternating uncached comparison and are reported separately.
These small sample counts characterize this workload, not production tail latency.

128 independent updates measured 175.43, 167.18 and 169.99 ms with exactly three
exchanges including uncached preparation. A two-table save measured 141.28 ms,
also with three exchanges. Every cached preview had zero requests; every warm
save had two. All committed values were checked outside timing.

Verification: 18 focused editing/structure tests passed in release mode, including
all live safety cases above. The release library suite passed 942 tests, with
269 opt-in cases ignored. Editor and browse WAN regressions passed their unchanged
thresholds. Strict all-target QA Clippy and git diff whitespace checks passed.
Raw logs, environment details and machine-readable results are retained in
`target/edit-server-checks/`. No UI rendering or input-to-display timing is claimed.
The disposable fixture was removed after validation; no user database was used.
