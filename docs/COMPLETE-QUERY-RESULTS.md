# Complete desktop query results

Desktop SQL execution fetches every result row automatically for SQLite,
PostgreSQL, MySQL and MariaDB. Explicit SQL `LIMIT` clauses still apply. There is
no application row-count ceiling. Results spill to a private temporary SQLite
file after 5,000 resident rows or the shared 16 MiB retained-row budget. The row
threshold preserves existing bounds on synchronous chart and selection work;
it no longer discards rows. These are retained-result bounds, not a bound on
server memory, SQLite's execution buffers, driver packets or total process RSS.

The UI reports the full count and provides continuous virtualized scrolling,
with no pagination controls. A background worker loads windows of up to 256
rows from the snapshot, without rerunning SQL or contacting the server. Wide
rows have compact previews created once during storage (32 KiB payload budget
per row, plus cell/vector overhead); selecting a preview loads the exact row.
Individual rows over 64 MiB fail explicitly rather than silently truncating.
The visible window and retained inspector selection stay bounded independently
of the total row count. Search traverses the entire snapshot and retains at most
100,000 matching cell coordinates, as before. CSV and JSON exports stream all
rows and retain atomic destination publication/cancellation. Pinned results
share their snapshot; the temporary file is removed on a cleanup worker when
its last owner disappears. Snapshots are session-only.

Disk-backed results are read-only. Use the table browser, or a narrower query,
for editing. Charts use rows near the current scroll position and label that
scope. Select All represents the complete result without allocating one
selection entry per row; Copy All reads the snapshot in the background, with a
32 MiB clipboard limit and an explicit export suggestion for larger selections.
Small results keep the existing editing and
chart behavior. The browser demo retains its 5,000-row / 16 MiB preview bounds
and shows an incomplete-result notice. It does not have desktop temporary-file
storage. Statement/result-set and existing transport limits remain in effect.

Storage/decoding failures drain the original statement before reporting that
results could not be retained. Completed writes are never retried automatically.
Cancellation and server errors retain the engine's transaction semantics.

## Protocol budget

Before and after, PostgreSQL editor statements with built-in or cached result
types use one established-connection exchange: safeguards and unnamed
Parse/Describe/Bind/Execute followed by one Sync. Neither result storage nor
local scrolling adds an exchange. Cold custom types still require the
same type-catalog lookups, after the complete response has drained; the new
worker-only vendor API spills raw rows instead of discarding them. This preserves
the bounded response-channel deadlock protection. The original explicitly
bounded vendor API remains available and tested. Browse/readback retain their
existing bounded collectors and pipeline paths.

MySQL/MariaDB retain COM_QUERY execution, including the existing single-exchange
safe START/read/ROLLBACK batch. Results drain on the same connection, including
later result sets and errors. Local storage adds no database requests, health
checks, preparation, cleanup waits or transaction-status queries. SQLite uses
the same prepared statement and steps it to completion.

## Initial complete-result verification (before continuous scrolling)

Raw checks and captures are retained in `target/complete-results-qa/`.
817 local tests passed (213 manual/live tests ignored by the ordinary suite).
Strict all-target Clippy with the QA feature passed. The locked browser WASM
check passed with the same 78 existing root-crate warnings. Native screenshots
were reviewed in dark mode at 1280×760, light mode at 500×760, and the last row
at 760×760, all at 2× capture scale. The review found and fixed header-only
column sizing during initial page loading; a production-layout regression now
covers it. Five-digit row numbers also receive a wider gutter.

Disposable live checks passed for MySQL 8.4.13 and MariaDB 11.4.13: 10,000 rows over
16 MiB in text and prepared protocols, exact last-row values, bounded browse,
multiple sets, later errors, cancellation, explicit transactions, timeout
connection disposal and subsequent reuse. PostgreSQL 17.11 checks passed for cold and
warm custom types, storage failure after a write, safe-mode setup failure,
headers/empty results, explicit transactions, cancellation and connection reuse.
SQLite checks cover full RETURNING writes, injected storage failure inside an
explicit transaction, oversized-row rejection and cancellation. Store tests cover
exact integers, non-finite floats, Unicode/NUL/binary values, byte-bounded forward
and reverse pages, full search and CSV/JSON export, and snapshot-file lifetime.

Release latency measurements on Apple M4 Max / macOS 27.2.1 / Rust 1.99.1:
30 ms simulated RTT, seven samples after two warmups. Ordinary editor query
p50/p95: **35.7/38.6 ms**, versus single-exchange reference p50 **34.8 ms** and
describe-first p50 **72.4 ms**. Complete 6,000-row disk-backed collection p50/p95:
**43.6/48.1 ms**, versus the same rows through a simple-query exchange at
**37.3/38.9 ms**. The extra local encode/store work is about 6.3 ms at p50;
there is no extra network request. Browse/refresh p50/p95: **39.1/40.8 ms**;
first sidebar load p50 **38.3 ms**, revisit p50 **38.8 ms**. Existing thresholds
were not changed. The initial debug-build forced-spill measurement failed the
new 25 ms local-overhead allowance (66.9 vs 40.1 ms); the release measurement
passes. Debug storage overhead is not claimed as a release latency regression.

The UI comparison and new-page workload results are recorded separately below.
CPU layout/tessellation does not establish input-to-display latency. Native
60/120 Hz frame intervals, missed frames, mixed-DPI/monitor reconnection and
window snapping were not instrumented for this change.

## Initial paginated UI measurements (historical)

Compared with `7910e11cfbe02d82ae813014d3cd787c0e9a72b8` on the same machine/toolchain, release profile. The standard runner used five alternating paired runs, identical fixtures, and the unchanged 10% p95 threshold. The first run had 19 passes and eight noise-related inconclusive cases. One repeat reused the exact build-directory executables (hashes recorded) with no competing builds/benchmarks: 26/27 cases passed; the 12-column workspace without inspector remained noisy, with median p95 improving 8.5%. Neither run found a repeatable regression. The overall suite is **inconclusive**, not an all-scenarios pass. All query-layout and search cases passed on the repeat.

Median per-run CPU milliseconds below, base → candidate. Maximum is the median of per-run maxima; raw samples and all other cases remain in `target/complete-results-ui-performance/` and `target/complete-results-ui-repeat/`.

| Scenario | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| query/editor-results/0/1280 | 0.021125 → 0.020959 | 0.023458 → 0.025542 | 0.034292 → 0.037333 | 0.048667 → 0.049375 |
| query/editor-results/4/1280 | 0.052834 → 0.052625 | 0.062875 → 0.059875 | 0.069166 → 0.063167 | 0.073625 → 0.079000 |
| query/editor-results/128/760 | 0.055458 → 0.055000 | 0.066791 → 0.064542 | 0.071459 → 0.071625 | 0.072625 → 0.076667 |
| query/editor-results/4/500 | 0.052625 → 0.052792 | 0.062042 → 0.060875 | 0.069125 → 0.064708 | 0.077417 → 0.071500 |
| search/scan/numeric | 0.865417 → 0.875292 | 0.907166 → 0.912875 | 0.937917 → 0.986875 | 0.972625 → 1.044083 |
| search/scan/text | 0.951583 → 0.952417 | 0.976583 → 0.988084 | 1.036000 → 1.011792 | 1.052958 → 1.025708 |
| search/scan/large-cell | 1.513208 → 1.501750 | 1.598708 → 1.550625 | 1.633375 → 1.567750 | 1.633625 → 1.569542 |
| search/panel-navigation | 0.006166 → 0.006083 | 0.007625 → 0.007417 | 0.012417 → 0.008292 | 0.015833 → 0.012542 |

Query layout uses 20 warmup / 160 measured frames at 1280×760, 760×760 and 500×760 logical points, default headless scale (1×). Search uses the same 20/160 sampling: 5,000 text rows, 500×120 numeric cells, one 16 MiB cell, and a 1200×800 navigation panel. Both include the existing synthetic idle jumps where applicable. The new paging workload has no equivalent complete-result baseline and is therefore an absolute measurement only: 10,021 stored rows × 3 columns, 500-row page, 20 warmup / 160 measured frames, five runs, and a page request after a synthetic 30-second idle jump. Initial fixture creation/loading is outside timing; request dispatch and subsequent page application are inside it.

| Stored-result viewport | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| 500×760 | 0.069750 | 0.081000 | 0.103250 | 0.270875 |
| 760×760 | 0.073042 | 0.083417 | 0.096458 | 0.236000 |
| 1280×760 | 0.075708 | 0.088000 | 0.096708 | 0.237709 |

The disposable database containers and native capture windows were stopped after verification. No user database or saved connection was used.

## Continuous scrolling verification

Pagination controls are removed. Tests cover a real wheel event crossing the
256-row cache boundary, retained inspector selection, End-key navigation to row
10,021, rapid search jumps, complete logical Select All/Copy, clipboard limits,
compact wide-row windows and exact selected-row hydration. The complete local
suite passed: **822 tests**, with 213 manual/live tests ignored. Strict all-target
QA Clippy passed. The locked browser check passed with the same 78 warnings.
Native screenshots were inspected at 1280×760 dark, 500×760 light and 760×760 at
the final row (2× capture scale). The last-row capture waits for scrolling and
background loading to settle. Logs/captures: `target/continuous-results-qa/`.

The viewport reads compact previews stored once alongside exact row values;
scrolling over large cells does not repeatedly decode their full payloads.
Search, exports and full-row selection continue to use exact values. All local
snapshot reads and writes remain on workers. Server exchange counts are
unchanged for all drivers, including cold/warm PostgreSQL type resolution.

Repeated PostgreSQL release checks against a disposable PostgreSQL 17 fixture
passed at 30 ms simulated RTT (two warmups, seven samples): ordinary editor
p50/p95 **37.3/38.0 ms**, one-exchange reference p50 **36.3 ms**, describe-first
p50 **74.7 ms**. Complete 6,000-row disk-backed collection p50/p95 **45.9/48.3 ms**,
versus equivalent simple-query results **38.1/39.6 ms**. The local storage cost
is 7.8 ms at p50, with no added server exchange. Validated browse pipeline
p50/p95 **40.0/43.1 ms**, revisit **40.6/41.0 ms**, first sidebar load
**40.1/43.3 ms**. The disposable container was removed afterward.

The continuous candidate was compared with `7910e11` on the same Apple M4 Max,
macOS 27.2.1, Rust 1.99.1, release profile. The first run returned 26 passes and
one inconclusive case, but a final verification build overlapped its first pair,
so it is excluded as clean performance evidence. A serial repeat used the saved
executables, five alternating pairs, unchanged fixtures and the unchanged 10%
p95 threshold, with no competing builds or benchmark workloads. It returned
11 passes and 16 noise-related inconclusive cases. **The comparison remains
inconclusive.** No stable regression was established. The 12-column inspector
workspace had a +23.5% median p95 change, but base samples ranged 0.286–0.818 ms
and candidate samples 0.293–0.836 ms; the initial comparison was +0.1%. This is
unresolved measurement noise, not an accepted performance regression or a pass.
Other desktop applications remained active and were not closed for the test.
Raw runs, environment, executable hashes and all metrics are retained in
`target/continuous-results-ui-final/` and `target/continuous-results-ui-repeat/`.
Formatting-only cleanup after compilation was checked by comparing normalized
source; no measured runtime logic changed.

Selected comparison medians in CPU milliseconds (base → candidate):

| Scenario | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| query/editor-results/4/1280 | 0.053542 → 0.054333 | 0.060125 → 0.061542 | 0.092709 → 0.073083 | 0.129750 → 0.090916 |
| query/editor-results/4/500 | 0.054666 → 0.054292 | 0.058000 → 0.059208 | 0.425250 → 0.069125 | 0.495958 → 0.074459 |
| workspace/12/true | 0.301041 → 0.286417 | 0.346667 → 0.428041 | 0.446667 → 0.511542 | 0.446667 → 0.511542 |
| workspace/120/true | 0.439208 → 0.425542 | 0.974542 → 0.436666 | 1.050375 → 0.455167 | 1.050375 → 0.455167 |

The new continuous workload has no equivalent complete-result baseline and is
reported as absolute CPU cost only. It uses 10,021 rows, three or 120 columns,
256-row viewport caching, real wheel events, a search jump and a synthetic
30-second idle gap. Each of five runs has 20 warmup and 160 measured frames,
760-point height and default headless 1× scale. Fixture creation and initial
loading are outside timing; request dispatch and cache application are inside.
Medians of per-run CPU measurements, milliseconds:

| Columns / viewport | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| 3 / 500×760 | 0.079042 | 0.086875 | 0.138458 | 0.201792 |
| 3 / 760×760 | 0.081750 | 0.091542 | 0.122500 | 0.171084 |
| 3 / 1280×760 | 0.084292 | 0.093584 | 0.126916 | 0.177083 |
| 120 / 1280×760 | 0.241584 | 0.272958 | 0.456667 | 0.715875 |

These CPU measurements do not establish presentation latency or sustained FPS.
Native 60/120 Hz timing, multiple-window load, mixed DPI, monitor reconnection
and snapping remain unmeasured for this change.

## Scroll preloading and cache-miss presentation

The same bounded 256-row window now refills before the viewport comes within
64 rows of either cache edge. New windows start approximately 96–127 rows above
the viewport, leaving look-ahead space below it too. Starts are aligned to
32-row boundaries and clamped at the ends of the result. Existing rows remain
visible until the replacement arrives. A pending read that covers the visible
rows is allowed to finish; only a jump outside its coverage supersedes it.
This prevents wheel events from continually cancelling useful work.

Missing rows use a static, muted rounded bar across the visible row instead of
"Loading…" in a cell. Row numbers, row heights, column widths and scrollbar
extent stay fixed. The placeholder does not schedule animation or polling.
All reads still use the local snapshot on the existing worker: neither server
round trips nor the maximum cache allocation increase. Large scrollbar jumps
and unusually slow disk reads can still show the placeholder.

Tests cover refill in both directions before a miss, retaining the old window,
coalescing wheel events, superseding stale jumps, no refill in the interior at
idle, and visible placeholder geometry after clipping. The full suite passed
824 tests (213 ignored); the final viewport-specific run passed all 10 active
checks. Strict Clippy and the locked browser build passed (78 existing WASM
warnings). Native forced-cache-miss screenshots in dark/wide and light/narrow
layouts, plus the loaded last row, are kept in `target/scroll-prefetch-qa/`.

Release measurements for preloading used the same Apple M4 Max / macOS 27.2.1 /
Rust 1.99.1, five alternating paired runs, no competing builds or benchmarks,
and the unchanged 10% p95 threshold. The PR-base comparison against `7910e11`
passed 26/27 cases; vertical precise-scroll tessellation was noise-related
inconclusive (+1.6% median p95). Overall this broader comparison remains
inconclusive rather than an all-cases pass; no stable regression was detected.

A separate direct comparison uses the saved preloading-before executable and
the final candidate, with identical continuous-scroll fixtures: 10,021 rows,
three or 120 columns, 20 warmup / 160 measured frames, five runs, 1× headless
scale, wheel events and a search jump after a synthetic 30-second idle gap.
This measures the actual behavior, including the increased opportunity to show
loaded cells instead of placeholders. All four scenarios passed. CPU ms,
median of per-run percentiles/maxima (before → after):

| Columns / viewport | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| 120 / 1280×760 | 0.246041 → 0.251666 | 0.285208 → 0.304666 | 0.457375 → 0.770667 | 0.727333 → 0.800250 |
| 3 / 1280×760 | 0.085166 → 0.085250 | 0.099917 → 0.102084 | 0.142500 → 0.173750 | 0.183625 → 0.184333 |
| 3 / 500×760 | 0.079167 → 0.079083 | 0.092417 → 0.097750 | 0.145209 → 0.183583 | 0.209584 → 0.189041 |
| 3 / 760×760 | 0.080958 → 0.080417 | 0.095084 → 0.098000 | 0.121833 → 0.165709 | 0.173708 → 0.181166 |

Raw timings, environment, binary hashes and comparison results are in
`target/scroll-prefetch-ui/`. CPU measurements do not establish input-to-present
latency or sustained FPS. The previously listed native frame-timing, mixed-DPI,
multiple-window and monitor-transition limitations remain untested. No database
execution or driver sequencing changed in this preloading follow-up.

## Progressive desktop display

SQL results and forward table pages now publish headers and their first decoded
row before the operation finishes. Subsequent snapshots are coalesced at most
every 50 ms while rows arrive. A single-slot mailbox replaces obsolete snapshots;
the UI never drains an unbounded queue of result batches. The footer distinguishes
rows received from a completed result. Editing, whole-result sharing, pinning,
and full-result search wait for completion. Cancel/error replies remain terminal
and partial table data is discarded; receiving RETURNING rows never implies commit.

Large results keep the same 5,000-row / 16 MiB spill thresholds. The temporary
SQLite store uses WAL and publishes committed prefixes for background readers.
Each snapshot has a fixed row count; window/search/export readers cannot read
past that prefix even if the writer has appended more rows. Readers are bounded
and do no work on the UI thread. Snapshot identity keeps scrolling and selection
stable across updates and the transition to disk. This still fetches the entire
original SQL result, without rewriting it or requesting additional server pages.

PostgreSQL table browsing uses a synchronous text iterator over the existing
pipelined metadata and data responses. Fresh metadata is validated before any
rows are exposed. Built-in/cached editor types and ordinary sidebar table loads
retain their one-exchange budgets, cold and warm. Uncached editor custom types
still drain before type resolution; their first display must wait for that safe
fallback. Unknown relations, schema recovery, explicit transactions, safeguards,
transaction status handling and cancellation teardown retain their existing
protocol paths. MySQL/MariaDB use the same text/prepared streams, setup and metadata
requests as before; SQLite steps the same statement. No new database request,
health check, description pass or cleanup exchange is introduced.

SQLite reverse keyset pages are buffered until their rows can be reversed into
the requested order, preserving the existing fast seek and stable row positions.
The browser demo still uses its existing bounded, completed-result transport.
A server-side sort, aggregate, lock or delayed first packet can still postpone
the first visible row. Progressive display reduces the client-side wait, not the
server's work or the total bytes transferred.

Established healthy connection exchange audit (before → after):

| Path | Cold / warm exchanges |
| --- | --- |
| PostgreSQL editor, built-in/cached result types | 1 → 1 / 1 → 1 |
| PostgreSQL sidebar table, valid catalog hint | 1 pipelined → 1 pipelined / same |
| PostgreSQL unknown table or explicit-transaction metadata-first browse | 2 → 2 / same fallback |
| MySQL/MariaDB ordinary editor SELECT, including safe START/read/ROLLBACK batch | 1 → 1 / 1 → 1 |
| MySQL/MariaDB ordinary base-table browse, no triggers | 6 → 6 / 4 → 4 |
| SQLite | 0 → 0 |

The MySQL browse count includes one SHOW columns/definition batch, prepared data
execution (prepare + execute cold, execute warm), one index/trigger-name batch,
and the prepared foreign-key query (prepare + execute cold, execute warm).
Trigger-definition batches, view-permission fallbacks, session refresh after
session-changing SQL, errors, cancellation and reconnection retain their existing
additional work. Healthy completion reads protocol transaction status; cancellation
watchers wake on completion and do not open a control connection unless cancelled.
Cold PostgreSQL custom types retain their existing type-dependent catalog lookups
after draining; no lookup is added by progressive display.

### Progressive verification (2026-11-18)

Raw results and native captures: `target/progressive-results-qa/`. Final release
library suite: **933 passed, 253 opt-in checks ignored**. Explicit disposable
checks passed for PostgreSQL streaming query/table first rows, cold custom types,
no write replay, empty results and schema changes, plus MySQL and MariaDB text
and prepared collection, bounded storage and connection reuse. Unit/UI regressions
cover cancellation after the first publication, readable committed prefixes while
writing, snapshot isolation, selection across growth/spill/completion, failure
cleanup, and full export after selecting an earlier partial prefix.

On the deliberately slow 300-row fixture, PostgreSQL first publication took
8–16 ms versus 920–952 ms completion; MySQL/MariaDB took 12–18 ms versus
922–969 ms completion. These measure worker publication, not input-to-display
latency. No synthetic delay was added to the UI.

Serial release latency comparison against `b0d16ae54c9e57c808539b9aafca7936826f79aa`,
with 30 ms simulated network RTT, seven measured samples after two warmups:

| Complete operation | Base p50 / p95 ms | Progressive p50 / p95 ms |
| --- | --- | --- |
| Editor indexed lookup | 38.2 / 45.2 | 44.2 / 46.1 |
| Complete 6,000-row disk-backed result | 48.7 / 53.4 | 50.6 / 58.0 |
| Table refresh | 39.3 / 46.9 | 40.0 / 48.3 |
| First sidebar table load | 45.8 / 47.7 | 43.7 / 46.4 |
| Table revisit | 43.1 / 48.4 | 41.7 / 44.0 |

Both revisions passed the existing latency thresholds, including after 0/1/3-second
idle periods. Candidate ordinary editor/browse measurements enable publication;
reference queries retain their original execution path. Small sample sets and
transport jitter limit conclusions about small CPU differences. Neither thresholds
nor result workloads were weakened.

Final strict `cargo clippy --all-targets --features qa -- -D warnings` passed.
The locked `wasm32-unknown-unknown` library check passed (87 warnings; browser
transport remains non-progressive). Disposable PostgreSQL, MySQL and MariaDB
containers were removed after validation. UI comparison and unmeasured native
presentation cases are recorded in `UI-PERFORMANCE.md`.

### Streaming inspector stability (2026-11-19)

The inspector draws before the workspace synchronizes a new result snapshot.
A strict Arc-identity check therefore briefly returned the empty disk-backed
source on each publication, flashing “No row selected” between valid frames.
`Pages::grid` now accepts a matching nonzero stream identity when returning the
cached window/selection. Distinct streams still reset selection. This adds no
row copy, database exchange, disk read or persistent repaint request.

The regression test reproduced the empty-row failure before the fix and now
checks the production inspector-before-workspace order over repeated snapshots,
including a switch to a different stream. The full debug library suite passed
(938 passed; 253 ignored at that run), and strict all-target QA Clippy passed.
A separate opt-in streaming-inspector benchmark was subsequently added. Native
captures in both themes are in `target/streaming-inspector-qa`; see
`UI-PERFORMANCE.md` for CPU measurements and presentation limitations.

Final release library run after adding the inspector workload: **938 passed,
254 ignored**. Fresh release UI comparison passed all 24 gates; the dedicated
streaming-inspector workload passed at both widths.
