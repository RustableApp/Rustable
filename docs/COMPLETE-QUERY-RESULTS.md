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
