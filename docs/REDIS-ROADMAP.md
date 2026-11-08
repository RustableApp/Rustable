# Redis browser roadmap

This roadmap covers the key browser and the broader Redis tooling requested for
[Redis Insight](https://redis.io/docs/latest/develop/tools/insight/) parity. The
persistent goal remains active; implemented increments are not a claim of complete
parity.

Current status:

- Implemented: browser/inspectors and guarded editing; Search field results,
  aggregation and profiles; command profiling; Pub/Sub; reviewed archives;
  Cluster discovery/routing, diagnostics and cross-slot archive operations.
- Implemented: cross-slot bulk delete/expiry, per-key results, cancellation and
  shared SQL-style review/results UI. Both the initial 39-case and final 41-case
  incremental release UI comparisons passed; native interaction limits remain below.
- Sentinel in progress: separate credential storage, configuration model,
  discovery/role verification, probes, session handoff and dedicated streaming
  tools and connection form are implemented. The 24-case incremental release
  UI comparison passed; broader reliability/native validation remains below.
- Implemented: time-series sample previews, bounded pages and inclusive range controls,
  validated on standalone Redis 8.2 and three Cluster primaries. On-demand bounded
  metadata and loaded-page charts are implemented; chart performance validation
  and additional module inspectors remain pending.
- Remaining: abrupt-loss election/partition and interrupted/concurrent resharding coverage;
  module/Search routing scope and additional module inspectors. Large-cluster read
  waves are implemented with protocol validation; broader outage coverage remains.
- Unresolved validation: native input/display/multi-monitor checks and the earlier
  whole-roadmap browser-density performance comparison. Incremental passes do not
  close those items.

The sections below retain implementation constraints, protocol budgets and raw
validation references. Earlier stage descriptions are historical; this status
list and later follow-ups describe the current scope.

Completed browser milestones:

- [x] Namespace tree, key list, binary-safe names, pattern/type filtering,
  exact-key lookup and incremental scan. See REDIS-BROWSER.md.
- [x] Paginated structured collection views and Text/JSON/Hex string views.
  See REDIS-VALUE-INSPECTOR.md.
- [x] Create strings, hashes, lists, sets, sorted sets, streams and JSON documents
  with optional expiry. Existing names are never overwritten.
- [x] Rename to an unused name, preserving expiry.
- [x] Add/edit/remove hash fields, list items, set members and sorted-set scores.
  Append/delete stream entries. Edits check the value shown to the user before
  changing it; opening an editor never writes.
- [x] Explicit bulk selection, reviewed key names, deletion and expiry changes.
- [x] RedisJSON document inspection, highlighted formatting, validated creation
  and whole-document conflict-checked editing.
- [x] Saved tree/list preference and namespace separator. Named command snippets
  persist through the app's existing preference storage and window synchronization.
  Saved-connection command history persists with recording/clear controls; saving a snippet remains explicit.
- [x] Command templates/help, bounded connection history and snippet management.
- [x] On-demand server summary, slow log and sampled key-memory analysis.

## Limits and safeguards

Read-only mode starts enabled and is checked again on the worker. Editing controls
appear when read-only mode is disabled. Draft windows have consistent control
heights, cached key labels and explicit confirmation. Failure retains the draft.
No mutation retries automatically after a lost connection; the error states that
its outcome may be unknown.

Collection values and fields are limited to 64 KiB. Hash/score edits compare the
original field value/score; set replacement checks original membership and rejects
an existing replacement. List edits compare the original element and length.
Removing by list index is limited to 1,000 elements to bound Redis-side shifting;
large-list commands remain available in the command editor. Other elements can
change between reading and applying an edit; this is not a whole-key snapshot.
Stream entries are immutable: append creates a new ID, and remove uses XDEL.
Removing the final stream entry leaves an empty stream key. The append form
accepts one field/value pair; commands can append entries with multiple fields.

Mutation Lua checks preconditions before writing, including permissions for the
multi-write set replacement, indexed-list removal, and create-with-expiry paths.
Lua is atomic with respect to other clients but is not a rollback transaction.
The tested server versions support `redis.acl_check_cmd` (Redis 7+). Restrictive
ACLs surface errors rather than being bypassed. TLS and authentication are unchanged.

Bulk actions accept 1–1,000 explicit loaded names and at most 1 MiB of key names.
They never expand a filter after confirmation. Deletion applies to the values
currently at those names, as stated in the confirmation. Collection edits preserve
TTL; optional expiry is applied on key creation. Expiry changes report the number
of changed keys. New/renamed/deleted keys trigger a browser refresh; collection
changes reload the affected inspector.

JSON uses the RedisJSON module's root document. Values over the 64 KiB retained
preview limit are read-only; JSON.GET still retrieves the server response before
truncation, so this is not a wire-size cap. Malformed documents are rejected before
submission. The RedisJSON fixture uses Redis 8.2; standalone/TLS tests remain on 7.4.

Memory analysis samples at most 1,000 currently loaded keys using MEMORY USAGE
SAMPLES 5. The report displays the largest 50 and labels the result as a sample,
not database-wide totals. Server metrics and the latest 50 slow-log entries refresh
only on request. Slow-log duration excludes client/network latency. These views do
not change server configuration or poll while idle.

Saved snippets are limited to 20 commands of at most 64 KiB each, with 80-character
names. History retains at most 100 commands and 256 KiB per saved connection, for the 16 most recently updated connections. Unsaved/demo connections retain session-only history. Recording can be disabled globally and individual connection history cleared. Authentication/admin credential commands are excluded; normal commands
may contain values; the save UI explains that saved snippets are local plaintext.
The in-memory WASM demo explicitly rejects the desktop-only structured mutations
and server diagnostics; its existing supported command subset remains available.

## Protocol budgets

All counts are on an established connection, for both first and repeated use.
Connection setup, TLS/authentication and error teardown are unchanged. Redis-rs
sends a nontransactional pipeline as a batch before consuming replies. No routine
health checks or script-load/EVALSHA warmup exchanges are added.

| Path | Previous behavior | New behavior |
| --- | --- | --- |
| Key discovery and previews | 1 SCAN exchange + 2 enrichment exchanges per 200-key batch | Unchanged |
| Existing-key inspection | 2 exchanges | Unchanged, including JSON: TYPE/PTTL then JSON.GET |
| Next/search page | 1 pipeline exchange | Unchanged |
| New guarded mutation | No structured equivalent | 1 EVAL exchange; existing inspector refresh adds 2 |
| Bulk deletion/expiry | No structured equivalent | 1 EVAL exchange, then the existing browse refresh |
| Server/slow-log report | No report | INFO + SLOWLOG GET 50 in 1 pipeline exchange |
| Loaded-key memory sample | No report | Up to 1,000 MEMORY USAGE commands in 1 pipeline exchange |
| Saved layout, snippets, expansion, selection | Local | 0 exchanges |

SQL driver/protocol code is unchanged for both PostgreSQL and MySQL. Mutation
protocol tests verify no preflight request on first/repeated use, read-only
rejection before any command, and discarded connections after unknown outcomes.
Disposable tests exercise binary fields/values, collisions, stale edits, TTL,
list duplicates, rename, bulk actions, ACL failures, JSON and report contents.

## Shared SQL styling and validation

SQL and Redis use `src/controls.rs` for the established SQL spacing, 23-point
minimum button height, 5-point rounded action buttons and 28-point segmented
switches. Redis no longer overrides these with taller, flat controls or its own
text palette. The top toolbar uses SQL's 44-point height; notices have a separate
conditional strip so they cannot be clipped. Drafts, collection actions, paging,
search and format controls follow the same convention. SQL's existing segmented
control geometry is preserved and its interaction regression test passes.

Native QA captures install the application's actual fonts and dark theme and
wait for popup animations to settle. Screenshots are in
`target/redis-sql-style-screenshots/{inspector,roadmap}`. These are native rendered
frames from disposable fixtures, not mockups. The desktop was locked, so manual
input-to-display, 60/120 Hz, mixed-DPI, multiwindow and snapping checks were not
available. CPU benchmarks do not establish those properties.

Validation: 35 Redis unit tests passed; 14 disposable Redis integration tests
passed (7.4 standalone/TLS and 8.2 RedisJSON), including guarded writes and ACL
failures. Four PostgreSQL latency tests passed against the disposable fixture.
Cross-window preference synchronization and the SQL segmented-control regression
test passed. Strict Clippy passed. WASM compilation passed with 59 existing
warnings. UI benchmark runner tests passed (8 tests). Raw test logs are retained
under `target/redis-roadmap-*.log` and `target/redis-sql-style-*.log`.

### Release CPU measurements

Apple M4 Max, macOS 27.2 arm64, rustc 1.99.1, release profile. Five alternating
base/candidate pairs; baseline `b16286d4cd55044ae10aa5a5c8ae024405593a02`.
Shared Redis fixtures use 1200×760 logical points, default scale/fonts, 20 warmup
and 160 measured frames per run. The exact fixture, toolchain, binary hashes,
all samples and comparison results are retained in
`target/redis-sql-style-ui-performance/`. Values below are median run statistics
in milliseconds; maximum is the largest observed stall across runs.

| Redis scenario | Base p95 | Candidate p50 | Candidate p95 | Candidate p99 | Candidate max |
| --- | ---: | ---: | ---: | ---: | ---: |
| redis/browser-chrome | 0.043792 | 0.046958 | 0.047917 | 0.050666 | 0.056375 |
| redis/grid | 0.120875 | 0.113666 | 0.119584 | 0.124416 | 0.143667 |
| redis/hash-inspector | 0.033750 | 0.033875 | 0.034209 | 0.035709 | 0.043333 |
| redis/string-inspector | 0.046125 | 0.044458 | 0.045833 | 0.050750 | 0.055708 |

All four Redis comparisons pass the unchanged 10% p95 threshold. Two SQL grid
tessellation measurements were inconclusive in the full run. A focused repeat
of ten alternating pairs using the same executables resolved vertical mouse to
+9.3%; horizontal mouse remained borderline/inconclusive at about +10.0%.
The overall performance gate is therefore **inconclusive**, not a blanket pass.
Grid layout and whole-workspace comparisons pass. Inspection found no grid or
tessellation implementation changes; the shared SQL control extraction preserves
its geometry. The remaining tessellation difference is not yet explained.
Earlier runs and the repeat raw samples are retained, including
`target/redis-sql-style-grid-repeat/`; thresholds were not changed.

New interaction workloads run against the final release executable (no equivalent
baseline feature) are recorded in `target/redis-sql-style-new-interactions.log`.
They include programmatic edits to a 56 KiB draft, 1,000 selected keys in a 10,000-key
browser, diagnostics, and value-mode changes. These establish current CPU cost,
not comparative performance or physical typing latency.

| New workload | p50 ms | p95 ms | p99 ms | max ms |
| --- | ---: | ---: | ---: | ---: |
| redis/edit-dialog | 0.036958 | 2.079417 | 2.863291 | 3.810000 |
| redis/edit-first-change | 2.389625 | 6.708625 | 6.708625 | 6.708625 |
| redis/bulk-selection | 0.058625 | 0.061333 | 0.067459 | 0.072084 |
| redis/diagnostics | 0.034125 | 0.034708 | 0.035750 | 0.035834 |
| redis/value-modes | 0.028875 | 0.047417 | 0.167375 | 0.171458 |
| redis/value-mode-switch | 0.079667 | 0.860458 | 0.860458 | 0.860458 |

## Search and interaction follow-up

Actions → Search indexes opens a read-only Search panel with index discovery,
field/path/type schema inspection, query input, 50-key pages and Inspect buttons
that open the existing value inspector. Refresh indexes and Refresh schema are
explicit. Query failures preserve the input and label any previous result with
its original query. Missing Search support and ACL errors remain visible; there
are no automatic retries or capability probes. The browser's background scan
pauses while this panel is open, and resumes after closing it. Query typing,
index/schema tables and results remain local and virtualized.

This first Search view returns matching key IDs, not entire documents. This
avoids pulling large values just to list matches; Inspect uses the existing
bounded retained-preview workflow. Queries use DIALECT 2, NOCONTENT, LIMIT 50,
and a 1,000 ms server timeout. Offset paging is limited to 10,000 matches.
Unsorted Search pages are not stable snapshots: Redis can repeat or skip matches
between requests, which the UI states. See the official
[FT.SEARCH documentation](https://redis.io/docs/latest/commands/ft.search/).
Sorting, aggregation, vector parameter binding, index creation and query profiles
are not included in this first panel.

Each FT._LIST, FT.INFO or FT.SEARCH operation takes **one exchange**, first and
repeated use, on the existing connection. No preceding ping, count request,
describe or module discovery is added. Listing indexes and selecting one are
separate explicit user actions. Query results carry their count in the same
response. Opening a result retains the existing two-exchange inspector budget.
PostgreSQL and MySQL driver paths are unchanged.

Names are binary-safe; Redis simple and bulk string replies are supported. Retained
index lists are limited to 1,000 names and 1 MiB; schemas to 1,000 fields and
512 characters per displayed attribute; query result pages to 50 keys/1 MiB.
Index/query inputs are limited to 4,096 bytes. These are retained-data limits,
not protocol response-size limits: FT._LIST and FT.INFO must receive the server's
reply before checking size. Search requires a desktop Redis Search connection;
the in-memory demo returns an explicit unsupported message.

The key list now supports Tab/Enter/Space activation and shares the tree's empty
and disconnected states. Existing expired-key and conflict paths remain in the
inspector/edit workflow. Search tests cover binary response decoding, schema
fields, paging, empty results, malformed queries, first/repeated one-exchange
execution, discovery pause, error retention and settling at idle. Native dark
and light screenshots include success, empty, syntax-error and retry states in
`target/redis-search-screenshots/`. CUA confirmed the Mac remains locked; physical
input, window movement, display-rate and mixed-DPI checks remain unavailable.

Validation: 41 Redis unit tests passed; all 16 disposable Redis session tests
passed, including Search against Redis 8.2. Strict Clippy passed. WASM compiled
with the same 59 existing warnings. All four PostgreSQL latency regression tests
passed against the disposable fixture. Logs: `target/redis-search-*.log`.

### Search follow-up performance results

Final release measurements use the same M4 Max/macOS 27.2/rustc 1.99.1 setup,
baseline snapshot, five alternating pairs and identical shared fixtures described
above. Raw results and binary hashes: `target/redis-search-ui-performance/`.
All SQL grid, workspace and sidebar comparisons now pass. The earlier SQL
tessellation difference does not reproduce in this final binary; no SQL grid
optimization is claimed. Its source is byte-for-byte unchanged from the baseline
(`target/redis-search-grid-source-check.txt`).

| Redis scenario | Base p95 ms | Candidate p50 ms | Candidate p95 ms | Candidate p99 ms | Candidate max ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| redis/browser-chrome | 0.044000 | 0.047375 | 0.048500 | 0.053125 | 0.057708 |
| redis/grid | 0.121209 | 0.114709 | 0.120416 | 0.130709 | 0.251042 |
| redis/hash-inspector | 0.034459 | 0.033916 | 0.034500 | 0.036584 | 0.042125 |
| redis/string-inspector | 0.047041 | 0.044708 | 0.045667 | 0.048625 | 0.050625 |

The full-browser comparison is **inconclusive** (+10.2% p95); grid/hash/string
comparisons pass. A ten-pair focused repeat remained inconclusive (+11.8%,
0.045104 → 0.050417 ms; six of ten pairs exceeded 10%). No threshold was weakened.
The overall gate is not reported as a pass. The baseline and current browser
intentionally render different visible work at an identical viewport: debug
count-only instrumentation using the same fixture reports 13 versus 15 key rows.
Those debug runs establish counts only, not comparable performance timings.
The requested compact SQL controls free room for two additional visible keys.
Keeping the old larger controls would lose that benefit; virtualization still
bounds work to visible rows. The measured full-window cost is about 4.5–5.3 µs.
This provides a concrete density benefit and investigation evidence, but does
not establish equal-row rendering speed or remove the inconclusive flag.
Count logs: `target/redis-search-row-count-{base,head}.log`; repeat samples:
`target/redis-search-chrome-repeat/`.

The new Search panel release workload (50 key matches, schema, 1200×760 points,
20 warmups, 160 measured frames, default scale/fonts) reports p50 0.040500 ms,
p95 0.040792 ms, p99 0.042625 ms and max 0.076834 ms. Raw samples are in
`target/redis-search-panel-performance.log`. This is a rendering workload for a
new feature with no baseline equivalent; it does not measure physical typing
latency, network time, GPU presentation or native scrolling. Headless keyboard
activation and idle-settling tests cover behavior separately.

## Persistent roadmap goal

The user requested continuing through the roadmap as an active goal. Work stays
uncommitted until separately requested. Current order:

- [x] Search field selection and ascending/descending server-side sorting.
- [x] Search aggregations and query profiling.
- [x] Explicit, bounded live command profiling.
- [x] Pub/Sub channel inspection and publishing with write guards.
- [x] Reviewed import/export with collision/expiry controls.
- [ ] Cluster and Sentinel discovery/routing/failover with identity validation.
- [ ] Additional module inspectors, including vector/array data where supported.
- [ ] Native interaction/display validation when the desktop is available.

### Search field/sort increment

Search can return up to eight selected schema fields alongside each key. Field
selection defaults to no values, preserving the inexpensive ID-only path. Sort
choices come from SORTABLE schema fields, with ascending/descending order.
Pagination uses the original request's query, selected fields and sort settings;
changing controls does not silently alter the current result. Inspect remains
beside the key, and additional columns scroll horizontally. Sorting by nonunique
values or changing documents is not a stable snapshot; the UI states this.

RETURN and SORTBY are part of the same FT.SEARCH request. Both ID-only and rich
queries retain one exchange, first and repeated use, verified by the fake RESP
peer. No per-result reads, preflight or count request are introduced. SQL drivers
are unchanged. Schema field names retain exact bytes rather than using truncated
labels as command arguments. Displayed field values retain the first 4,096 source
bytes, with an explicit truncation marker and binary quoting where needed; missing
or expired fields remain empty placeholders. At most 50×8 previews are retained.
Escaped binary previews can use four times the source-byte limit. Full selected
field responses arrive before truncation, so users should select small fields
and use Inspect for large values; the field menu explains this distinction.

Tests cover numeric sorting in both directions, field order, large/binary values,
missing/expired fields, malformed responses and first/repeated protocol budgets.
42 Redis unit tests and all 16 disposable session tests passed; strict Clippy
passed; WASM compiled with the existing 59 warnings. Native screenshot evidence
is in `target/redis-rich-search-screenshots-final/`. Performance validation is
recorded below after the same-machine release run.

#### Field/sort performance validation

All 24 comparisons pass for this increment against the previous Search snapshot
`099f78a` (five alternating pairs, same M4 Max/macOS 27.2/rustc 1.99.1 release
configuration). The prior candidate executable was reused only after checking its
SHA256 against the saved report, and the new candidate was rebuilt. Raw samples,
provenance and binary hashes: `target/redis-rich-search-ui-performance/`.
This increment passing does not erase the older full-roadmap browser-density
comparison or its inconclusive flag.

| Scenario | Base p95 ms | Candidate p95 ms | Change |
| --- | ---: | ---: | ---: |
| redis/browser-chrome | 0.047916 | 0.048958 | +2.2% |
| redis/grid | 0.119209 | 0.121416 | +1.9% |
| redis/hash-inspector | 0.034250 | 0.034542 | +0.9% |
| redis/search | 0.040292 | 0.044250 | +9.8% |
| redis/string-inspector | 0.045000 | 0.045084 | +0.2% |

The existing Search comparison retains 50 IDs, one schema field, a 1200×760
viewport, default fonts/scale, 20 warmup and 160 measured frames. Its fixture
was factored into a helper without changing the ID-only data; the new field
workload is a separate candidate-only scenario. That workload adds three returned
fields and programmatic query changes every 20 frames: p50 0.105000 ms, p95
0.120417 ms, p99 0.122667 ms, max 0.125041 ms. It is not a physical input-to-display
measurement and has no baseline equivalent. All four PostgreSQL latency checks
also passed. Native desktop checks remain unavailable while the Mac is locked.

### Aggregation and query-profile increment

Search now has Documents/Aggregate modes using the shared SQL segmented controls.
Aggregation groups by up to three schema fields (or all matches with no grouping),
always includes a count, and optionally computes sum, average, minimum or maximum
of a numeric field. Non-sortable fields needed by the query are explicitly loaded;
other fields are not fetched. Sorting uses count descending and group values
ascending, with a bounded top-N sorter and one-row lookahead for Next-page state.
Pages show up to 50 groups and stop at 10,000; no cursor or separate count request
is created. Changing data can move groups between pages, and server timeout policy
can yield partial aggregates; both limitations are visible in the UI.

Profile query runs the current document search or aggregation once with
FT.PROFILE LIMITED. Results and a profile tree arrive together, exposed through
Results/Profile tabs. Server time fields and operation counts retain their labels;
client elapsed is separately measured across the request and decoding. The report
collapses the schema once to make room, but users can reopen it. Reports never
poll. The scrollable window has a height limit and a regression test verifies its
bounds over repeated frames and that it settles at idle. The first native capture
exposed a growing query row and overflowing report; the final captures verify the
corrected sizing. No physical desktop input/display check is claimed.

FT.AGGREGATE and FT.PROFILE each use **one exchange**, first and repeated use,
including profile results. There are no setup, count, preflight or cleanup commands
on these established-connection paths. Profiling subsequent pages is explicit;
ordinary paging does not silently profile. Query timeout is 1,000 ms, and no server
configuration changes are made. The fake RESP peer tests all three operations
(aggregate, document profile, aggregate profile) twice without extra exchanges.
SQL/PostgreSQL/MySQL driver code remains unchanged.

Aggregate results retain at most 50×5 displayed cells; each cell follows the same
4,096-source-byte preview cap. A profile retains at most 1,000 rows, 12 levels and
4,096 traversal steps; text is bounded before entering the UI. LIMITED suppresses
some server detail but these retained limits are not a wire-size cap. Binary field
names remain exact command arguments. Internal reducer aliases avoid collisions
with chosen group names. Malformed/oversized pages are rejected and input remains
available after query failures.

The command editor now permits the known read-only FT._LIST, FT.INFO, FT.SEARCH,
FT.AGGREGATE and FT.PROFILE commands in read-only mode, so advanced aggregation
pipelines can also be entered directly. Command help includes aggregation and
profile templates. FT.CREATE, FT.DROPINDEX and FT.CONFIG remain guarded; no broad
FT.* allow rule was introduced.

Reference behavior was checked against the official
[aggregation specification](https://raw.githubusercontent.com/redis/docs/main/content/commands/ft.aggregate.md)
and [profile documentation](https://redis.io/docs/latest/commands/ft.profile/),
then verified against the disposable Redis 8.2 server using RESP2 (the session's
protocol). The visual builder currently offers the five reducers above; arbitrary
APPLY/FILTER/multi-stage pipelines and other reducers remain available in the
command editor, not as visual controls.

Validation: 46 Redis unit tests, all 18 disposable Redis session tests, strict
Clippy and all four PostgreSQL latency tests passed. WASM compiled with the 59
existing warnings. Tests cover all reducers, loading non-sortable group fields,
page lookahead, alias collisions, profile structure/bounds, actual server reports,
read-only command classification, and window bounds/idle behavior. Logs are in
`target/redis-analytics-*.log`; final native screenshots are in
`target/redis-analytics-screenshots-final/`.

Performance for this increment passed all 25 comparisons against the preceding
rich-Search snapshot `75b5854eb21ae5020e17052ee08491dafca23431`. Five alternating
base/candidate runs used identical workloads, release builds, Apple M4 Max,
macOS 27.2 arm64 and Rust 1.99.1. Redis fixtures use a 1200×760 viewport,
default scale/fonts, 20 warmups and 160 measured samples per run; SQL fixtures
use 1200×800 (workspace scenarios have 100 measured samples). Raw p50/p95/p99,
maximum stalls, environment and executable hashes are retained in
`target/redis-analytics-ui-performance/`.

| Redis scenario | Base p95 ms | Candidate p95 ms | Change |
| --- | ---: | ---: | ---: |
| Browser chrome | 0.048500 | 0.048292 | -0.4% |
| Key grid | 0.120250 | 0.124666 | +3.7% |
| Hash inspector | 0.034292 | 0.034334 | +0.1% |
| Search IDs | 0.044792 | 0.044625 | -0.4% |
| Search fields | 0.063208 | 0.061083 | -3.4% |
| String inspector | 0.045750 | 0.045417 | -0.7% |

New panel workloads have no equivalent pre-feature baseline. With 20 warmups
and 160 samples, aggregation CPU p50/p95/p99/max was
0.079666/0.087334/0.089000/0.093375 ms; query profile was
0.055250/0.058417/0.059375/0.059750 ms. The fixtures retain 50 aggregate rows
and 150 profile rows, with periodic query changes. Raw samples are retained in
`target/redis-analytics-panel-performance.log`.

These measurements cover CPU layout/tessellation, not input-to-display latency,
GPU timing or network latency. Native 60/120 Hz input, mixed-DPI, window moving,
monitor reconnection and macOS tiling remain unverified. This incremental pass
does not resolve the earlier full-roadmap browser-density comparison documented
above; its borderline/inconclusive result remains recorded.

## Live command profiler increment

Actions → Live command profiler now offers explicit Start/Stop, follow-latest,
clear and a virtualized command stream. Closing the window or dropping the
workspace cancels capture. MONITOR is server-wide: the selected browser database
is intentionally not applied to this connection. The UI distinguishes arrival
timestamps from execution duration and points to slow log for duration analysis.
It explains server overhead and potentially sensitive command arguments before
Start. No capture is started automatically or saved to disk.

The transport is a dedicated worker-owned TCP/native-TLS connection. DNS,
certificate file reads, handshake and all stream I/O remain off the UI thread.
The existing Redis 1.8.1 sync and async transports decode complete RESP values
without a configured frame-size bound. Instead, this narrow MONITOR transport
uses Redis's packed command encoder and a bounded RESP2 simple-line reader;
it does not replace the browser/session transport. Partial lines survive read
timeouts. A 64 KiB wire-frame limit stops capture before allocating a whole
oversized argument. Source previews are capped at 4,096 bytes, the queue at 64
messages, the UI at 2,000 retained rows and 16 dequeued messages per frame.
Full queues drop messages with a visible count. Worker updates and backlog drains
schedule redraws after 100 ms; idle completed reports do not poll.

AUTH (when needed) and MONITOR are pipelined in one write/response exchange after
TCP/TLS setup. No PING, SELECT, CLIENT, or cleanup request is issued. The stream
adds no exchanges to ordinary browsing, Search, PostgreSQL or MySQL. First and
repeated capture each open a fresh connection, with standard TCP/TLS handshake
cost; there is no reconnect/replay. A socket clone permits Stop to shut down the
stream during TLS, setup or reads. DNS resolution and a connect attempt already
in progress can finish in the background before worker cancellation is observed;
Stop never joins that worker on the UI thread. Certificate chains and hostname
verification remain enabled, including custom roots. Credentials are not
persisted; the saved capture password and outgoing command buffer use zeroizing storage.

A capture ends after 60 seconds of streaming, 100,000 observed commands, a wire
limit/error, Stop or window close. Preview truncation, queue drops, retention and
wire limits are separate. The stream is a bounded debugging sample, not an audit
log. Redis itself omits/redacts some commands; see the official
[MONITOR behavior and overhead documentation](https://redis.io/docs/latest/commands/monitor/).

Initial validation: bounded/fragmented line tests, one-exchange startup with
bounded overflow and prompt idle cancellation, oversized stream rejection, and
window-bounds/idle tests pass. Disposable plain/TLS capture tests pass, including
untrusted CA and wrong hostname rejection; all 18 existing Redis session tests
also pass. Final native framebuffer captures verify ready, live and stopped states; the initial capture hid the footer, corrected by shortening the list and centering the window. Physical desktop interaction/display checks remain unverified.

Final release checks: 51 Redis tests passed (25 explicitly ignored integration or
benchmark cases); strict all-targets/QA Clippy passed, and WASM compiled with the
same 59 existing warnings. The new native-only profiler does not change the web
demo. Final screenshots are in `target/redis-monitor-screenshots-final/`.

CPU comparison uses the prior analytics snapshot
`132dd1ef564eec92c63b5b39a220a8f39c1f6ecf`, with the saved executable hash verified
before rebuilding the candidate. Five alternating pairs ran the same 25 metrics
on Apple M4 Max, macOS 27.2 arm64, Rust 1.99.1, release profile, default scale/fonts,
1200×760 Redis and 1200×800 SQL fixtures. Workloads use 20 warmups and 160 samples
(100 for SQL workspace). The first pass had 22 passes and three noisy/inconclusive
results: horizontal precise tessellation, browser chrome and sidebar/20. None
was classified as a regression. Five additional alternating pairs of the affected
workloads, using the exact same executables and unchanged thresholds, passed all
16 included metrics. Original and repeated samples are retained separately in
`target/redis-monitor-ui-performance/` and `target/redis-monitor-ui-repeat/`.

| Repeated scenario | Base p95 ms | Candidate p95 ms | Change |
| --- | ---: | ---: | ---: |
| grid/horizontal precise/tessellation | 0.042833 | 0.043709 | +2.0% |
| redis/browser-chrome | 0.048208 | 0.048834 | +1.3% |
| sidebar/20 | 0.024375 | 0.024625 | +1.0% |

The new 2,000-row profiler fixture has no pre-feature equivalent. Its CPU
p50/p95/p99/max was 0.026541/0.028292/0.031292/0.033333 ms. A second workload
receives 16 distinct near-4 KiB command previews per frame with follow-latest
active; it includes queue draining, retention eviction, layout and tessellation.
Its p50/p95/p99/max was 1.400541/1.435209/1.447917/1.462292 ms. Both use 20 warmups
and 160 samples at 1200×760; raw samples are in
`target/redis-monitor-panel-performance.log`. Network/server overhead and GPU or
input-to-display timing are excluded. The earlier full-roadmap browser-density
inconclusive result remains unresolved; this incremental comparison does not
replace it.

All four disposable PostgreSQL latency tests also passed for the final candidate
(`target/redis-monitor-postgres.log`); SQL driver paths are unchanged.

## Pub/Sub increment

The native Actions menu now opens a Pub/Sub workspace with exact-channel and
pattern subscriptions, a virtualized message list, Text/JSON/Hex inspection,
active-channel discovery, direct subscriber counts, and text/hex publishing.
The existing shared SQL controls are used. Read-only mode disables Publish and
is checked again before the worker sends PUBLISH. Channel names preserve binary
bytes via the command editor's quoted/escaped name syntax. Payloads are limited
to 64 KiB when publishing; no write is automatically replayed after a lost reply.
The receiver count is explicitly not described as a delivery acknowledgement.

Subscriptions use their own connection and stop when the window/workspace closes.
They do not reconnect automatically: Redis Pub/Sub is at-most-once and has no
history/replay. Subscription mode and target are locked while subscribed.
Channels are server-wide rather than scoped to the selected Redis database.
Channel discovery lists active exact subscriptions on the connected node; count
results explicitly exclude pattern subscribers. Details were checked against the
[Redis Pub/Sub overview](https://redis.io/docs/latest/develop/use-cases/pub-sub/)
and [NUMSUB semantics](https://redis.io/docs/latest/commands/PUBSUB-NUMSUB/).

The MONITOR transport's verified TLS and cancellation implementation is now shared
in `redis_stream.rs`, with its existing protocol and TLS tests retained. The new
Pub/Sub decoder is an incremental RESP2 flat-array decoder: binary bulk payloads,
embedded newlines, and socket timeouts between arbitrary bytes are supported.
It rejects nested arrays, malformed frames, headers above 4 KiB, arrays above
1,000 entries and total frames above 1 MiB before unbounded allocation. Oversized
responses stop the dedicated connection and explain that the pattern should be
narrowed. No server-side channel cursor exists here, so the limit is an explicit
error rather than a silently partial channel list.

Retained message previews have a separate 4 KiB allocation (truncating a larger
Vec alone would retain its large backing allocation). The queue holds 32 messages,
the view retains 1,000 messages and drains at most 16 per frame. Queue overflow
increments a visible dropped count. A maximum 4 KiB channel name is preserved
exactly; JSON formatting/highlighting is cached on selection. Channel labels are
prepared on the worker, then transferred to the UI. Redraws follow incoming work
with a 100 ms delay; stopped reports and idle subscriptions do not poll the UI.
Socket read timeouts remain worker-only cancellation checks, not Redis requests.

Round-trip trace: after TCP/TLS setup, AUTH (when needed) and SUBSCRIBE/PSUBSCRIBE
are pipelined into one exchange. Channel discovery initially used a fresh socket
per query; review replaced this before completion with one persistent inspection
connection. First discovery uses AUTH plus PUBSUB CHANNELS in one exchange after
transport setup. Repeated discovery and subscriber counts each use one exchange
on that connection, with no PING, reauthentication, SELECT or cleanup request.
PUBLISH uses the existing workspace connection and one exchange, first/repeated
on an established connection. The subscriber and inspection connections never
occupy the browsing worker. Neither stream automatically retries failed work.
SQL/PostgreSQL/MySQL transport paths remain unchanged.

Validation so far: the unit tests cover binary framing, interruption after every
byte, malformed/oversized frames, preview retention, quoted names and hex input,
window bounds/idle behavior, bounded queue overflow and prompt idle cancellation.
Fake RESP peers prove pipelined subscription startup, inspection-connection reuse,
one-exchange PUBLISH and no publish replay after a lost reply. Disposable Redis
plain/TLS tests cover exact and pattern subscriptions, binary payloads, channel
lists/counts, read-only publishing guards, untrusted CA and wrong hostname
rejection. All 18 existing Redis session tests and all three MONITOR transport
checks passed again after the shared-transport extraction. Strict all-targets/QA
Clippy passed. See `target/redis-pubsub-unit-final.log`,
`target/redis-pubsub-integration-final.log`, and
`target/redis-pubsub-clippy-final.log`.

Final native framebuffer captures in `target/redis-pubsub-screenshots-final/`
cover ready, messages, highlighted JSON, active channels, publishing and stopped
states; all six were visually inspected. A fresh desktop-control check confirmed
the Mac remains locked. Physical input, multiple-window movement, mixed-DPI,
60/120 Hz input-to-display timing and monitor reconnect checks remain unverified.

The final release checks passed 60 Redis unit tests (27 explicitly ignored
integration/benchmark cases). Live plain/TLS, identity, protocol/queue/cancellation
checks passed in the disposable fixture; the suite includes all 18 existing
Redis session tests and the three MONITOR transport checks. All four PostgreSQL
latency checks passed. Strict Clippy passed and WASM compiled with the same 59
existing warnings. Logs: `target/redis-pubsub-final-checks/unit.log`,
`target/redis-pubsub-integration-final.log`, `target/redis-pubsub-postgres.log`,
`target/redis-pubsub-clippy-final.log`, and `target/redis-pubsub-wasm.log`.

All 27 existing CPU comparisons passed against the live-profiler snapshot
`00671539d23ddb5026efa536324bd49b072c5972`. Five alternating base/candidate pairs
used verified saved executables, Apple M4 Max, macOS 27.2 arm64, Rust 1.99.1,
release profile, default scale/fonts, 1200×760 Redis and 1200×800 SQL fixtures.
Workloads use 20 warmups and 160 samples (100 for SQL workspace). Raw results,
p50/p95/p99/max, executable hashes and environment are retained in
`target/redis-pubsub-ui-performance/`. Redis browser chrome p95 was
0.048417→0.048375 ms (-0.1%); the key grid was 0.123291→0.119750 ms (-2.9%).
Existing monitor incoming-work p95 was 1.425750→1.486500 ms (+4.3%). All other
comparisons are listed in that directory's summary.

A test-only extension then added a 1,000-channel refresh workload and a window
bounds/idle case with 4 KiB channel names. Production code stayed unchanged; the
rebuilt test executable/hash and the final candidate-only measurements are in
`target/redis-pubsub-final-checks/`. These new panels have no pre-feature equivalent.
Each workload uses 20 warmups/160 samples at 1200×760. Message inspection switches
between JSON/Hex with 1,000 retained messages; incoming work drains 16 new 4 KiB
previews per frame; the publish workload changes a 64 KiB draft every frame.
Channel refresh replaces 1,000 roughly 900-byte names every ten frames, including
UI transfer, old-buffer disposal and virtualized rows. Its synthetic names fit the
1 MiB wire cap. The separate bounds test deliberately exceeds that wire-level
combination to stress the UI's individual name/count limits.

| New workload | p50 ms | p95 ms | p99 ms | Max ms |
| --- | ---: | ---: | ---: | ---: |
| redis/pubsub-messages | 0.037208 | 0.046125 | 0.097000 | 0.161916 |
| redis/pubsub-incoming | 0.051167 | 0.052708 | 0.053709 | 0.054208 |
| redis/pubsub-publish-64k | 2.041500 | 2.075667 | 2.114208 | 2.140458 |
| redis/pubsub-channel-refresh | 0.043292 | 0.173958 | 0.233375 | 0.278416 |

These are CPU layout/tessellation measurements, not network/server overhead or
input-to-present timing. Original pre-extension samples remain in
`target/redis-pubsub-panel-performance.log`; no noisy or historical results were
removed. In particular, the earlier full-roadmap browser-density inconclusive
comparison remains documented and is not resolved by this incremental pass.

## Guarded import/export (implemented and validated)

`redis_archive.rs` implements a versioned `.rredis` container for Redis DUMP
payloads, exact binary key names, type labels, source database number, creation
time and absolute expiry deadlines. This is a Rustable archive, not a server RDB
backup file. It contains no saved connection credentials. A SHA-256 trailer detects
accidental file damage; it is not an authenticity signature. Redis validates each
DUMP's own version/checksum when restoring, and required modules must exist on the
target server. File parsing never interprets user-supplied commands.

Limits: 64 MiB total file, 1,000 entries, 4 KiB key names, 128-byte type labels and
1 MiB serialized payloads. Empty and non-UTF-8 key names are preserved. Duplicate
names, invalid/oversized lengths, trailing data and invalid expiries are rejected.
Timestamps are limited to the exact Lua integer range (2^53−1 milliseconds) to
avoid silently rounding exotic deadlines. Encoding preallocates the validated
output size. File reads/writes are intended exclusively for background workers;
atomic temporary-file persistence supports explicit replacement or no-clobber,
and resolves existing symlinks consistently with SQL document saves.

`Session::export_archive` captures TYPE/DUMP/PEXPIRETIME together in EVAL_RO,
in batches of up to 16 keys. Each established-connection batch takes one exchange,
first and repeated, with no separate type/expiry preflight. Replies contain at
most sixteen 1 MiB payloads plus bounded metadata. Missing/oversized/inaccessible
keys are recorded separately for the export review, including when every selected
key was skipped; an empty archive cannot be saved. The batches are individually
atomic, not a globally consistent database snapshot. The cap bounds client/wire
retention; serializing a large key can still cost server CPU and temporary memory.
Export can stop between batches, before any file is saved.

`Session::import_archive` validates read-only mode and the full immutable archive
before its first command. Defaults are skip existing keys and preserve absolute
deadlines. Replace-existing and remove-expiry are explicit alternative policies.
For preserve-expiry, a deadline already passed according to Redis TIME skips the
entry before RESTORE, leaving any existing target untouched. Removing expiry can
intentionally restore an entry whose archived deadline has passed; the UI must
make that choice clear. RESTORE receives the original decimal timestamp string,
not a potentially rounded Lua tostring conversion.

Each batch executes in one EVAL exchange and returns per-key outcomes: restored,
existing/skipped, expired/skipped or error. Earlier successes are not claimed to
roll back on a later error. A lost/invalid batch reply marks that submitted batch
unconfirmed and leaves later entries unattempted; writes are never replayed.
Cancellation is checked between batches. SQL/PostgreSQL/MySQL paths are unchanged.
Fake RESP tests cover first/repeated one-exchange batches, binary arguments,
read-only/cancellation before sending, and a lost reply for 16 of 17 keys.

Initial validation passed six codec/protocol unit tests and the live disposable
Redis round-trip test for strings, hashes, lists, sets, sorted sets, streams,
binary names/payloads, absolute expiry, skip/replace collisions, partial errors,
and removal of expiry. Invalid DUMP replacement and expired preserved entries
leave pre-existing data unchanged. The existing Redis session, MONITOR and Pub/Sub
fixture suites passed too. Logs: `target/redis-archive-unit.log` and
`target/redis-archive-integration.log`. The final codec/protocol run and disposable fixture suite also pass after adding
an all-missing export case and resolving save paths through the existing SQL
document path helper. Logs: `target/redis-archive-unit-final.log` and
`target/redis-archive-integration-final.log`. UI-specific file flow checks were still pending at that backend-only stage.

The shared-style review UI and native file dialogs are implemented. Immutable
archives and preformatted labels travel through Arc-backed storage; file I/O and
label construction run on workers. Both review tables and outcomes are virtualized.
Import review checks current types, expiry and server time in one read-only
exchange. The worker rejects a changed database before any command; it adds no
routine SELECT or PING. Replacement requires an explicit checkbox, and removal
of expiry explains that archived expired entries can become persistent.

Cancelling a picker or rejecting a new file preserves the previous review/results.
Starting a target refresh invalidates the old review. A completed import requires
a fresh target review before another attempt. Loaded bytes remain immutable even
if the original file changes. The window exposes stop-between-batches and per-key
outcomes, including unknown commit results without automatic replay.

UI/codec/protocol tests currently pass (10 passed, three opt-in tests/workloads omitted),
and the complete disposable session/MONITOR/PubSub/archive fixture suites pass.
Logs: `target/redis-archive-ui-unit.log`, `target/redis-archive-integration-ui.log`.

Final UI validation:
- Six final framebuffer captures were inspected: export, loaded archive, reviewed
  target, replacement confirmation, results and read-only state. Artifacts:
  `target/redis-archive-screenshots-final/`. The results view uses a smaller source
  table so outcomes and policy text fit in the window.
- Large incompressible values are skipped; denied DUMP access is reported; a JSON
  module payload restores on Redis 8.2 and fails on the incompatible Redis 7.4
  target without creating a key. All 29 disposable integration checks pass
  (18 session, three MONITOR, four Pub/Sub, four archive).
- Seventy release Redis tests pass; 30 opt-in tests/workloads are ignored in that
  unit run. Strict all-target QA Clippy passes. Logs:
  `target/redis-archive-release-tests.log`, `target/redis-archive-clippy-final.log`.
- All 31 existing UI metrics pass five alternating release pairs against the
  hash-verified Pub/Sub executable. Redis browser p95: 0.048459 → 0.048666 ms
  (+0.4%); grid: 0.120292 → 0.120500 ms (+0.2%). Largest increase: small SQL
  sidebar +7.9%, below the unchanged 10% threshold. Raw p50/p95/p99/max samples,
  environment and summary: `target/redis-archive-ui-performance/`.
- New archive workloads retain 1,000 entries with roughly 59 MiB payloads and
  long key labels, use a 1200×760 logical viewport, 20 warmups and 160 measured
  frames. Review/results/Arc handoff p95: 0.063959/0.078500/0.063084 ms;
  p99: 0.067208/0.081667/0.067292 ms; max: 0.068083/0.082375/0.072042 ms.
  Raw samples: `target/redis-archive-panel-performance.log`. These are initial
  CPU measurements, not a comparison against a previously available feature.
  Handoff retains another Arc and does not measure final payload destruction.
- The Mac remains locked. Native file-dialog interaction, keyboard/mouse input,
  mixed-DPI, 60/120 Hz presentation and first input after idle remain unverified.
  Headless idle settling is tested. The earlier full-roadmap browser-density
  result remains inconclusive; this incremental pass does not resolve it.

All four disposable PostgreSQL latency tests pass; the WASM build passes with
the same 59 existing browser-only warnings. Logs:
`target/redis-archive-postgres.log`, `target/redis-archive-wasm.log`.

Next: Cluster/Sentinel topology discovery, routing and failover, with explicit
identity validation and no replay of writes whose outcome is unknown.

## Cluster/Sentinel work in progress: topology discovery

Actions → Cluster topology now makes one explicit CLUSTER SHARDS request on the
existing connection. It adds no startup, refresh, scan or editor preflight. First
and repeated requests each use one established-connection exchange; the fake RESP
peer verifies exactly that command with no follow-up connections to announced
addresses. Standalone servers return their normal unsupported-cluster error.

The parser accepts RESP2 field arrays and RESP3 maps, limits retained topology to
1,000 nodes/shards and 16,384 slots, rejects overlapping ranges, duplicate fields,
node IDs/endpoints, malformed roles/health and invalid endpoints. Missing slots
remain visible as incomplete coverage. TLS discovery uses tls-port only; it never
falls back to a plaintext port. Null/empty endpoints inherit the contacted host,
while '?' remains unavailable. Announced hosts are preserved for later TLS
hostname verification. This does not authenticate a newly announced node.
The generic Redis driver's initial response decoding occurs before these limits;
these are retained topology limits, not a wire allocation bound.

The virtualized node table uses cached worker-built strings, bounds displayed
range descriptions to 16 ranges per row, and sorts endpoint rows for stable
refreshes. The shared diagnostics UI shows shard/node counts, slot coverage and
unavailable nodes. Current browser results are still local to the connected node;
this view is not a claim of cluster-wide key browsing or routing.

`scripts/test-redis-cluster.sh` creates an isolated Redis 7.4 three-primary cluster
in one disposable container, exposing only loopback ports 16400–16402. The live
discovery test verifies three primaries and complete slot coverage twice. Initial
unit/protocol checks pass and strict all-target QA Clippy passes. Logs:
`target/redis-topology-integration.log`, `target/redis-topology-unit.log`,
`target/redis-topology-clippy.log`. The final screenshot is `target/redis-topology-screenshots-final/cluster-topology.png`:
the role/health column was widened after visual inspection, and the topology
window has its own title and an 800-point default width.

All 35 existing CPU UI metrics pass five alternating release comparisons against
the archive snapshot `06736e1afb0c2fcdd896a10579b995cf86781bb0`, using its hash-verified
executable on the same Apple M4 Max / macOS 27.2 / Rust 1.99.1. Diagnostics p95:
0.038833 → 0.038125 ms; browser chrome: 0.054708 → 0.054208 ms. Raw samples,
environment and summary: `target/redis-topology-ui-performance/`. The new 1,000-row
topology workload has p50 0.040583 ms, p95 0.044709 ms, p99 0.046291 ms and max
0.046583 ms (1200×760 logical viewport, 20 warmups, 160 measured frames;
`target/redis-topology-panel-performance.log`). This is CPU layout/tessellation,
not native input-to-present timing. Earlier full-roadmap browser-density evidence
remains inconclusive, and locked-Mac native interaction limitations still apply.

Remaining topology milestone work (the checklist stays open):
- Add explicit standalone/Cluster/Sentinel connection profiles and Sentinel
  service/seed configuration, including separate credential handling where needed.
- Preserve TLS hostname/CA validation on every discovered target; constrain
  topology retries/connections and preserve cancellation/timeouts.
- Cache slot routes and connections; support MOVED and ASK correctly, without
  retrying I/O failures whose write outcome is unknown. Trace the redis crate's
  actual retry/pipeline paths before adopting its Cluster transport.
- Route key previews, mutations, search and archives correctly. Group independent
  operations without adding sequential per-key exchanges. Cross-slot atomic
  operations must be rejected or explicitly scoped, never silently split.
- Scan all primaries with bounded cursors/deduplication and explain topology
  changes during scans. Cluster uses database zero.
- Resolve Sentinel primaries on reconnect, verify ROLE before use, and exercise
  actual promotion/failover, stale discoveries, unavailable seeds and TLS failures.
- Extend disposable fixtures, protocol/latency tests and UI benchmarks for the
  new paths; retain the native interaction limitations already documented.

References: Redis [CLUSTER SHARDS](https://redis.io/docs/latest/commands/cluster-shards/),
[Cluster specification](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/),
and [Sentinel client specification](https://redis.io/docs/latest/develop/reference/sentinel-clients/).

Driver trace for the next routing step: the installed redis 1.8.1 synchronous
Cluster request loop (`cluster_handling/sync_connection/mod.rs`, around lines
850–975) retries `RetryMethod::Reconnect` through the same request loop; disabling
auto-reconnect alone does not return the error immediately. ASKING is currently a
separate `req_packed_command` exchange before the redirected command, and MOVED
refreshes slots before retrying. Its pipeline path sends node batches before
reading responses, but individually retries redirect errors after a topology
refresh. Adopting the default Cluster connection would therefore not satisfy our
unknown-write-outcome rule or the desired ASK pipelining budget without additional
control. Keep these cases explicit in the routing implementation/tests.

Final discovery validation: 74 release Redis tests pass (32 opt-in tests/workloads
ignored), all 29 existing Redis integration checks pass, five topology checks
including the real three-primary cluster pass, all four PostgreSQL latency tests
pass, and WASM compiles with the same 59 existing warnings. Logs:
`target/redis-topology-release-tests.log`, `target/redis-topology-integration.log`,
`target/redis-topology-cluster-final.log`, `target/redis-topology-postgres.log`,
`target/redis-topology-wasm.log`. No production SQL/MySQL/PostgreSQL execution
paths changed. Discovery remains opt-in; routing/failover is not complete.


## Cluster connection/routing implementation in progress

Profiles now support Standalone or Cluster deployment. Old serialized profiles
remain standalone. Cluster requires database zero, and the database picker
explains that restriction. The command pane distinguishes routed key operations
from node-local server commands. Sentinel profiles/failover are not implemented
yet; the overall topology checklist remains open.

`redis_cluster::Router` owns a validated 16,384-slot snapshot and at most 64 cached
native connections, plus at most eight connections being established in a setup
batch. Common native key commands, JSON root commands,
MEMORY USAGE and explicit script keys route locally without a key-discovery
request. Binary hash tags use Redis CRC16/XMODEM. Multi-key/script operations with
known key positions reject CROSSSLOT before sending; editor batches prevalidate
these routes before their first command. Unknown/module command key layouts are
not guessed: they go to the seed and may follow an explicit server redirect.
Server-wide destructive commands (FLUSHALL/FLUSHDB/SHUTDOWN/SELECT) are blocked
through this router. Additional command/module routing remains work to complete.

Only explicit MOVED/ASK rejections are retried, up to five attempts. MOVED updates
its slot; ASK does not. ASKING plus the rejected command share one pipeline, and
the actual command response is preserved even if ASKING itself failed. Lost I/O
or malformed protocol stops the entire router and closes its connections; no
submitted operation is automatically replayed. Ordinary server errors are fully
drained before connection reuse. TLS/CA/auth configuration is inherited by every
discovered target, using its actual hostname. No TLS-to-plaintext fallback exists.

Read previews and diagnostics group at most 1,000 commands over at most 64 nodes, send all node
batches before reading replies, and restore original result order. Each warm
metadata/preview stage is one parallel exchange, preserving the existing two-stage
key-row path. Cold node setup runs in groups of at most eight concurrent workers,
finishing authentication before any read batch is sent. The LRU connection cap
keeps cached sockets bounded. A preview spanning more than 64 nodes currently
requires narrowing the selection; extending large-cluster waves remains work.

Cluster scans snapshot unique primaries and send SCAN to up to 64 nodes together,
distributing the requested COUNT hint. Per-node cursors remain on the worker;
the UI gets an opaque continuation token. Incomplete slot coverage is rejected,
filters/cursor changes invalidate continuation, and MOVED invalidates an active
scan. At most 10,000 names are retained per returned page, with existing UI
admission/deduplication limits retained. Topology/keys can change during scanning;
this is not a consistent database snapshot. Reconnect reloads topology from the
seed; resilient multi-seed discovery and failover recovery are still pending.

Exchange trace (excluding TCP/TLS negotiation): redis 1.8.1 pipelines AUTH and
CLIENT SETINFO in its setup exchange. Standalone setup remains setup + PING +
SELECT (three exchanges). Cluster seed setup uses setup + PING + SHARDS (also
three); new target connections use setup + PING before their first operation.
Warm known-key operations use one exchange, warm ASK uses one combined redirected
exchange after rejection, and MOVED adds the explicitly rejected request before
the target exchange. Unknown key layouts may repeat the seed redirect until
routing metadata support is extended. No routine health check was added.

A TLS-to-plaintext discovery test exposed a pre-existing redis-rs native TLS
handshake timeout gap. `vendor/redis` now applies the connect deadline before and
through native TLS I/O, including slow progress, then restores normal established
connection timeouts. There are no additional network exchanges and no relaxed
certificate checks. See `vendor/README.md`. DNS remains synchronous/uninterruptible.

Current tests cover real three-primary writes, read previews, key inspection and
all-primary scanning; hash tags, same-slot guards, MOVED/ASK, denied ASKING with a
successful following command, lost write replies without replay, warm fan-out
before waiting, error drainage and TLS identity/no-downgrade failures. A fake TLS
peer closes after two seconds while the 100 ms client deadline must return within
one second.

Final routing validation: 81 release Redis tests pass (35 opt-in checks/workloads
ignored), 30 standalone/TLS integration checks pass, and five topology plus seven
routing checks pass against the disposable three-primary cluster. All four
PostgreSQL latency checks pass. Clippy passes with warnings denied; WASM builds
with 59 existing warnings. Logs are `target/redis-routing-release-tests.log`,
`target/redis-routing-standalone-final.log`,
`target/redis-routing-cluster-verification.log`, `target/redis-routing-postgres.log`,
`target/redis-routing-clippy-final.log` and `target/redis-routing-wasm.log`.

The 36 existing release UI measurements pass against discovery snapshot
`e588afe1a7cc6e690aea9f90247d84da41d36290`; raw results are in
`target/redis-routing-ui-performance/`. Its initial connection-form measurement
was invalid because the fixture did not open the form; see VALIDITY.md. The
corrected common fixture asserts that Redis Connection and Host are painted.
Five corrected alternating pairs were inconclusive amid timing drift. Ten
additional alternating pairs with the same binaries/fixture passed: median p95
0.0533335 ms to 0.0566665 ms (+6.25%), with only one of ten paired increases above
10%. Both runs remain in `target/redis-routing-profile-ui-performance/` and
`target/redis-routing-profile-repeat/`. These are CPU frame measurements, not
input-to-display latency. The historical full-roadmap browser-density comparison
remains inconclusive; these incremental results do not resolve it.

Final native QA framebuffers are in `target/redis-routing-screenshots-final/`
(cluster-topology.png and cluster-key.png). Both were visually inspected. Native
input, 60/120 Hz and mixed-DPI checks remain unavailable while the Mac is locked.

Cluster diagnostics follow-up completed: the read batch cap now matches the
1,000-key loaded selection. The read-only classifier permits MEMORY USAGE and
SLOWLOG GET specifically, retaining guards for MEMORY PURGE, SLOWLOG RESET and
unclassified subcommands. Previously the 400-command cap and classifier rejected
these diagnostics. Both cold and warm 1,000-key samples now work across the real
three-primary fixture, correctly reporting 30 present and 970 missing keys; the
server report also succeeds. Server diagnostics still describe the seed node.

A protocol regression peer withholds replies until all 1,000 requests arrive,
checks result ordering over two batches, and confirms a 1,001-command rejection
sends nothing before a subsequent PING. Together with the existing cross-node
barrier test this protects pipelining/fan-out: warm diagnostics remain one
exchange, cold node authentication is unchanged, and no per-key waits were added.
The maximum command count is still bounded. Five topology and eight routing tests,
three command-guard tests and Clippy pass; logs are
`target/redis-cluster-diagnostics-integration.log`,
`target/redis-cluster-diagnostics-guards.log` and
`target/redis-cluster-diagnostics-clippy.log`. No rendering/input path or SQL driver
changed in this follow-up, so the existing UI/SQL measurements are not claimed as
new measurements. Archive/bulk support and large-cluster waves remain pending.

Remaining before topology parity: Sentinel configuration and discovery/ROLE checks;
multi-seed reconnect/failover and real resharding; cluster-aware bulk
operations and module/Search scope; broader cluster command metadata; updated
native UI checks when the Mac is accessible. Archive export, review and import now support multiple slots as described below;
bulk actions still need their own routing and outcome audit.


## Cluster archive reads

Export now uses at most sixteen single-key EVAL_RO readers per chunk in Cluster
mode. The router groups requests by primary, sends every node batch before
waiting, drains responses, and restores selection order. Each key's type, DUMP
and absolute expiry are captured atomically by the existing script. The existing
1 MiB/key, 16 MiB/chunk and 64 MiB/archive limits remain. Missing keys retain their
skip reports. Cancellation is checked before each chunk; a cancelled export does
not save a partial file. This is not an atomic database-wide snapshot.

Import review uses up to 1,000 single-key EVAL_RO readers in one fan-out, preserving
target order. Expired-entry counts use each target node's observed clock, not the
seed clock; the displayed review time is the latest observed timestamp. Both
operations require Cluster database zero. Standalone scripts and batching remain
unchanged. Warm export remains one exchange per sixteen keys; review remains one
exchange per selection (now concurrent across primaries). Cold node setup and
explicit redirect budgets are unchanged. No additional health checks were added.

The internal read pipeline admits only the exact fixed export/review scripts with
one declared key and no extra arguments; exports additionally require a batch of
at most sixteen commands. The editor continues to reject arbitrary scripts in
read-only mode. A protocol test withholds all replies until sixteen export
requests arrive, repeats the batch, and verifies that oversized batches, unknown
scripts and EVAL send nothing. Existing fan-out tests protect cross-node ordering.

Real three-primary validation exports thirty keys across slots plus one missing
key, round-trips the archive checksum/encoding, and reviews existing/missing/
expired targets twice. Database mismatch and cancellation are rejected. Five
topology and eight routing integration checks passed, followed by the new archive
protocol check, all standalone integration checks and Clippy. Logs:
`target/redis-cluster-archive-integration.log`,
`target/redis-cluster-archive-protocol.log`,
`target/redis-cluster-archive-standalone.log`,
`target/redis-cluster-archive-clippy.log`. No rendering/input or SQL execution path
changed. No new native UI screenshot is claimed for this transport-only change.

Cluster import writes are implemented in the following milestone. Bulk mutations
remain separate work; archive tests do not establish their correctness.


## Cluster archive import writes

The guarded import path now sends at most sixteen single-key IMPORT_SCRIPT EVALs
per batch, grouped by owning primary and fanned out before replies are read. All
required connections authenticate before the first write. Standalone retains its
existing sixteen-key script. Warm Cluster imports use one exchange per sixteen
keys, including distinct slots on the same primary; cold setup uses the existing
bounded concurrent connection preparation. No preflight ping was added.

Each submitted key starts Unknown until a valid per-key script response confirms
its outcome. On a failed send, unsent node groups remain NotAttempted. On a lost
reply, confirmed outcomes are retained and other submitted nodes are drained to
recover their results; the router then closes all connections and refuses more
operations until reconnect. Later chunks remain NotAttempted. Explicit MOVED/ASK
rejections alone may be followed after the initial batch is drained and only if
it has no uncertain failures. ASKING/EVAL share a single redirected exchange.
A rejected redirect left unfollowed is NotAttempted; an uncertain redirected
attempt remains Unknown. Unknown scripts/results never trigger replay.

Read-only checks, archive validation, cancellation before each chunk, skip/replace,
absolute expiry and per-key RESTORE errors remain in force. The archive is not a
cross-node transaction and successful keys are not rolled back if another fails.
Memory stays bounded by the existing sixteen one-MiB payloads per chunk (plus
bounded packed-command copies) and sixty-four-MiB archive cap. This changes no UI
render/input path or SQL/MySQL/PostgreSQL protocol path.

The disposable three-primary test exercises thirty-key imports over multiple
chunks, skip, replace, preserved/persistent expiry, cancellation, read-only mode,
binary names and corrupt DUMP data leaving the existing value intact. Protocol
checks cover a lost reply retaining successes from both nodes with no replay,
MOVED/ASK with preserved neighboring outcomes, and setup failure sending no writes
to the healthy node. Logs: `target/redis-cluster-import-integration.log`,
`target/redis-cluster-import-protocol-final.log`. Standalone integration checks and
Clippy also pass (`target/redis-cluster-import-standalone.log`,
`target/redis-cluster-import-clippy.log`). Native interaction/visual checks remain
outstanding; no new screenshot is claimed for these transport-only changes.

Next: cluster-aware bulk actions and larger-node waves, resilient reconnect and
Sentinel, then the remaining module inspectors. Real resharding/failover coverage
and the previously documented whole-roadmap UI-performance/native checks remain
open; this import milestone does not close the overall goal.


## Mutation routing audit

Rename now declares both source and destination in EVAL's KEYS list. The fixed
script reads the destination from KEYS[2], so routing checks both slots before any
request is sent. Previously the destination was only an ARGV value, preventing
client-side CROSSSLOT validation. This preserves the single-exchange mutation
budget on standalone and Cluster; it adds no connection checks. The real Cluster
test verifies rejection leaves the source intact, then verifies same-tag rename
moves the value and removes the source. All twelve routing and five topology
checks pass, as do standalone integration checks and Clippy. Logs are
`target/redis-cluster-rename-integration.log`,
`target/redis-cluster-rename-standalone.log`,
`target/redis-cluster-rename-clippy.log`. No UI rendering path changed.

Cross-slot bulk delete/expiry remains open. Current `Session::mutate` returns an
aggregate u64, `Reply::Mutation` closes the dialog and reports that count, and the
Lua expiry operation validates all permissions before its first write. Silently
splitting that path would lose per-key uncertainty and change atomicity without
showing it. The next implementation must provide a Cluster bulk result model and
review/result UI with applied, unchanged, unknown and unattempted rows, while
retaining standalone/same-slot behavior. Reuse the bounded fan-out and
no-uncertain-replay transport rules from import, but do not label bulk results as
archive restoration. Include cancellation between batches, preflight failures,
partial replies and redirect coverage, plus the matching UI performance workload
and screenshots. This is implementation work, not a new user approval gate.


## Cluster bulk actions — implementation, validation in progress

A shared typed write-batch transport now serves archive import and Cluster bulk
mutations, retaining the existing authentication-before-send, fan-out, drainage,
explicit-redirect-only retry and unknown-write safeguards. Archive results are
mapped back to their original types. Cross-slot bulk delete/expiry uses sixteen
single-key UNLINK/PEXPIRE/PERSIST requests per chunk, with cancellation before each
chunk and per-key Applied/Unchanged/Unconfirmed/Not attempted outcomes. Standalone
and same-slot mutations keep the existing atomic Lua path and exchange budget.
Cross-slot bulk was previously rejected; the new path uses one fan-out exchange
per sixteen keys, without adding waits to existing supported operations.

The edit dialog warns about independent changes in different slots. A worker-built
result view caches quoted names, outcome labels and counts; a virtualized table
renders visible rows only. A stop control completes the currently submitted batch
before cancelling remaining chunks. A disconnected result marks the workspace
disconnected and does not issue a follow-up refresh. No automatic retry is offered.

The real Cluster test now covers thirty-key bulk expiry, persistence, deletion,
unchanged repeats, read-only protection and cancellation. The twelve routing
checks (including shared transport failure/redirect tests) and five topology
checks pass: `target/redis-bulk-integration.log`. Compilation, Clippy, WASM and
standalone results are in `target/redis-bulk-*.log`.

This UI milestone is NOT complete: add its matching release CPU workload, compare
existing hot paths against the pre-bulk base, inspect/capture review/results/stop
UI, and verify first-use/idle behavior. Native input/display checks remain open.
The QA Cluster screenshot fixture must reseed its routing keys after the new bulk
delete integration test, which now intentionally removes those keys. Inspect the
current code/results before declaring the earlier screenshot path valid again.


Bulk UI validation continuation: native QA captured the actual cross-slot expiry
review and result after changing thirty keys and observing one absent key. Images
are in `target/redis-bulk-screenshots/` and
`target/redis-bulk-screenshots-final/`; the latter name is provisional, not a
completion claim. Inspection found the results window still uses default placement
and padding: the attempted textual patch did not match its compact builder chain.
Fix that after the in-flight benchmark, then recapture and remeasure affected UI.
The review dialog was visually inspected; native mouse/keyboard/display timing
checks are still not established by framebuffer captures.

A 1,000-row bulk result workload now measures stationary and wheel-scroll CPU
layout/tessellation (20 warmup/180 measured frames, 1200×760, synthetic 120 Hz).
Names/outcomes/counts are precomputed outside frame timing. The same new standalone
presentation fixture and minimal data types are injected into the pre-bulk base;
this measures common styling cost and the absolute new interaction cost, not a
pre-existing equivalent feature. Existing workspace behavior stays unchanged in
the comparison base. First-use/after-idle and cancellation-window coverage still
need to be completed.

An in-flight comparison runs 39 SQL/Redis measurements against snapshot
`fa61e0a90adf04c55d612018fc5bc6b1d756820d` via
`/tmp/redis-bulk-performance.py`; artifacts are in
`target/redis-bulk-ui-performance/` and the orchestration log is
`target/redis-bulk-performance-run.log`. It uses separate base/head build caches,
five alternating pairs and unchanged thresholds. No performance pass is claimed
until its results are inspected. Do not restart a live run merely because a turn
ends; poll its existing process/handle first.


Sentinel implementation notes (official client specification rechecked): resolve
the named primary through bounded Sentinel seed attempts, distinguish unreachable
Sentinels from an unknown service name, and verify ROLE on the discovered data
connection before exposing it. Re-resolve on every reconnect; a successful prior
resolution is not a durable address cache. Keep the last successful Sentinel
first in the in-memory list. The optional Sentinel peer list may extend that list,
but events alone cannot replace discovery and role verification. Source:
https://redis.io/docs/latest/develop/reference/sentinel-clients/ .

Rustable-specific design still needed: separate ephemeral/persisted Sentinel and
data-node credentials (the current credential account is one password per profile),
per-endpoint TLS identity, a bounded total discovery deadline, and connection-status
reporting without ordinary-query health checks. Never serialize secrets in Settings
or replay a submitted write when reconnecting after failover.


Bulk polish validation update: the original 39-measurement comparison completed
with every case passing. Browser chrome p95 was 0.047875→0.048833 ms (+2.0%);
bulk result stationary/scroll p95 were 0.043125→0.042791 and
0.049458→0.049125 ms. These results predate the final padding/placement change.
They are CPU measurements and do not resolve historical full-roadmap or native
input/display limitations.

The final result window now uses the editor's centered, 16-point padded, rounded
window frame. Native QA screenshots in `target/redis-bulk-screenshots-polished/`
show the real thirty-applied/one-unchanged Cluster result and the review dialog;
the result image was visually inspected after applying the actual builder change.
Closing a workspace cancels remaining bulk batches; the stop button immediately
shows a disabled stopping state. New tests verify close cancellation and that a
disconnected result sends neither refresh nor replay. The bulk-related suite
passed 21 tests (five benchmark workloads ignored):
`target/redis-bulk-state-tests.log`. A debug fixture check confirms the idle view
stops requesting redraws; its timings are NOT release performance evidence.

A second comparison is running via `/tmp/redis-bulk-polished-performance.py` in
`target/redis-bulk-polished-ui-performance/` (orchestration log:
`target/redis-bulk-polished-performance-run.log`). It adds first-open (three frames
including sizing/first visible content) and first-scroll-after-60-seconds-idle CPU
workloads: five warmup/forty measured samples at 1200×760. Baseline and candidate
run identical data/events; the injected baseline view deliberately keeps the
pre-polish default padding/placement so their cost is measured rather than copied
away. Forty-one cases total, five alternating pairs, unchanged 10% threshold.
Inspect this run before claiming final performance validation. Native stop-button
interaction/close interaction and display timing remain unverified.


Final bulk CPU/build validation completed: all 41 measurements pass in
`target/redis-bulk-polished-ui-performance/`. Median p95 before→after: stationary
results 0.042708→0.042666 ms, scrolling 0.049042→0.049292 ms (+0.51%), three-frame
first-open 0.247334→0.245417 ms, and first scroll after idle 0.045000→0.046333 ms
(+2.96%). Five alternating pairs, same machine/release toolchain, raw p50/p95/p99/
maximum samples retained; no threshold was changed. These CPU measurements and
native framebuffers do not prove display latency, native clicking, mixed-DPI or
multi-monitor behavior, nor resolve the historical browser-density comparison.

The final release binary passes 88 Redis tests (37 opt-in checks/workloads ignored):
`target/redis-bulk-polished-release-tests.log`. Final Clippy passes with warnings
denied and WASM compiles with 59 existing warnings (`target/redis-bulk-polished-
clippy.log`, `target/redis-bulk-polished-wasm.log`). No production changes followed
this comparison; subsequent work is documentation and a disposable Sentinel script.

Sentinel fixture progress: `scripts/test-redis-sentinel.sh` starts three Sentinels,
one primary and two replicas with separate Sentinel/data passwords, loopback-only
published ports and no persistence. It checks quorum, named/unknown discovery,
primary ROLE, replicated test data, a requested failover and former-primary
demotion; its trap removes its own container. The smoke run passed with promotion
16410→16411 (`target/redis-sentinel-fixture-smoke.log`). This proves the fixture,
not Rustable Sentinel support. Connection profile/discovery implementation remains
next; manual failover here is not a network-partition or primary-crash test.


## Sentinel profile foundation (integration in progress)

The secure-store worker now has a typed Database/RedisSentinel purpose. Existing
`connection-{id}-password` accounts are unchanged; discovery credentials use
`connection-{id}-redis-sentinel-password`. Both purposes share the serial worker,
with independent accounts and response tokens. Tests verify legacy reads, scoped
save/delete isolation and save-before-delete ordering.

Saved Redis settings now optionally contain a non-secret Sentinel configuration:
service name, 1–8 unique host/port seeds, discovery username, independent TLS/CA and
remember-password preference. Old profiles deserialize with no Sentinel config;
Cluster and Sentinel modes cannot be combined. No secret enters serialized settings.
Until discovery is connected, the existing direct connection entry point explicitly
rejects Sentinel profiles before any network connection, preventing accidental
use of the configured direct host or data credentials against a Sentinel seed.

Two credential-store and two settings tests pass. Native and WASM builds pass
(`target/redis-sentinel-credentials.log`, `target/redis-sentinel-settings.log`,
`target/redis-sentinel-foundation-check.log`, `target/redis-sentinel-foundation-wasm.log`).
The unconnected RedisSentinel purpose currently produces one dead-code warning;
this is an unfinished integration, not a waived Clippy gate. Do not expose the
selector until discovery and credential handoff are wired. Next manager work must
key pending/latest/revision state by both profile and purpose, clear/load both
secrets on profile changes, and handle deletion/probe snapshots/window handoff for
both. Final Clippy/UI validation follows the completed integration.


## Sentinel discovery transport (UI handoff pending)

`Session::connect_with_sentinel` now resolves the named service using its separate
credentials and TLS/CA settings, validates advertised addresses, authenticates the
data connection with its own settings, and pipelines ROLE plus SELECT before
exposing that connection. Malformed/non-primary roles are rejected. Stale roles
trigger one bounded discovery retry after a short delay. Every new call re-resolves;
there is no reconnect replay of a submitted command. Successful-seed preference
and automatic reconnect lifecycle integration remain pending.

Redis exchange trace for a successful discovery: Sentinel AUTH/CLIENT SETINFO
setup + address request (two), then target AUTH/CLIENT SETINFO setup + ROLE/SELECT
pipeline (two). No PING preflight is added. Ordinary standalone setup stays at
three exchanges; Cluster setup is unchanged. TCP/TLS negotiation is additional.
The configured connection budget is checked between stages and before accepting
a primary, with seed attempts capped at 500 ms and at most two rounds over eight
seeds. This is not a hard cancellation guarantee: synchronous DNS and fragmented
responses can exceed a wall-clock budget because socket timeouts apply to I/O
inactivity. Do not claim those cases are bounded until addressed/tested.

Protocol tests verify separate AUTH passwords, no extra PING, ROLE/SELECT arriving
before either response, subsequent direct data access, malformed discovery, and
stale-role rechecks without exposing a replica. The live fixture runs Rust tests
before and after manual failover (16410→16412), including controlled failed-seed
fallback, database 3, repeated discovery, unknown service and wrong credentials.
Tests contact only fixture ports and a failing listener owned by the test, never
an assumed-unused local service. Logs: `target/redis-sentinel-live-discovery.log`,
`target/redis-sentinel-protocol-checked-deadline.log`. Standalone integration checks
also pass (`target/redis-sentinel-standalone.log`).

The profile manager, connection probes, window handoff, MONITOR/PubSub and reconnect
must still carry both credential purposes before the selector is exposed. This
transport milestone is not complete Sentinel support; its currently-unused entry
points will cease producing dead-code warnings when that integration lands.


## Sentinel manager and session handoff

Credential pending/latest/delete-retry state is now keyed by profile and purpose,
with independent revisions for data and discovery secrets. Profile changes clear
and load both; stale reads cannot overwrite a typed discovery password. Save,
delete, duplication and repaired identities handle both secure-store references.
As with SQL profiles, remember flags track successful secure-store persistence,
not a separate password-storage checkbox. Failure of one scoped write leaves the
other flag intact and retains the failed secret only for the session. Probe
snapshots include both secrets; preferences serialize neither.

New-window requests carry both secrets in memory. Redis settings are boxed in that
enum to avoid inflating every SQL window request. Connection probes, workspace
startup and explicit reconnect/database changes use Sentinel discovery with both
credentials. Direct standalone/Cluster paths retain their existing protocol path.
The Sentinel selector remains hidden until dedicated streaming tools are wired;
the direct streaming transport explicitly rejects Sentinel profiles before opening
any socket, preventing accidental direct-host fallback.

Validation: the manager suite passes 37 tests, plus a selective secure-store failure
test covering each scope independently. Tests cover private window handoff, stale
reads, serialization, deletion, copied/repaired identities and legacy profiles.
Profile/probe suites, WASM and Clippy pass; WASM retains its existing 59 warnings.
Live discovery and the actual asynchronous connection probe pass before and after
promotion to 16412, including wrong discovery credentials. Logs are
`target/redis-sentinel-handoff-*.log` and `target/redis-sentinel-stream-guard.log`.
No final UI performance pass is claimed for this manager increment; compare its
completed form/streaming integration against `429663ce4e17964dc3a95d2559d574d58a632ff1`.

Next: share discovery with MONITOR/PubSub while preserving their pipelined startup
and verifying ROLE on the actual streaming connection, then expose and visually
validate the Sentinel form. Avoid resolving through a throwaway data connection
and authenticating a second one merely for implementation convenience.


## Sentinel streaming integration

MONITOR, subscriptions and channel queries now carry both credentials and share
Sentinel discovery. The actual streaming socket sends AUTH (when needed), ROLE
and the read-only operation in one batch. A stale replica is discarded before
its acknowledgement or messages reach the UI. No throwaway primary connection,
PING, SELECT or extra role-check exchange is added. Standalone streaming remains
one exchange; Sentinel streaming requires seed setup + discovery + target startup
(three sequential exchanges on a successful first seed). Normal Sentinel sessions
retain their four-exchange setup. Explicit reconnect resolves again; automatic
stream failover/replay is not implemented.

The startup RESP2 parser caps bytes (64 KiB), items (1,000), nesting (four levels)
and headers (4 KiB), and preserves buffered operation acknowledgements/messages.
Steady-state decoders and UI drain/virtualization budgets are unchanged. Discovery
checks cancellation between seeds and before exposing a connection; seed socket
inactivity timeouts remain bounded but synchronous DNS is not interruptible.

Protocol tests enforce AUTH/ROLE/operation arrival before any response, separate
credentials, stale-role rejection, nested replica replies, parser limits and
preserved acknowledgements. The disposable fixture passes MONITOR delivery and
Pub/Sub acknowledgements/messages both before and after promotion 16410 → 16412.
Logs: `target/redis-sentinel-stream-tests.log`, `target/redis-sentinel-stream-live.log`.
The connection form and its screenshots/release UI comparison remain next.


## Sentinel connection form

Deployment now exposes Sentinel with the SQL connection form's shared input,
button, section and spacing conventions. Separate discovery/data credentials and
TLS settings, 1–8 seed editors, and fixed footer actions are available. Stable
seed row identities preserve partially edited ports when an earlier seed is
removed. The existing form scroll area contains the longer settings on smaller
windows; no polling, database operation or disk access was added to form rendering.

Native framebuffer captures were visually reviewed at 1200×1080 and 1000×760:
`target/redis-sentinel-form.png` and `target/redis-sentinel-form-compact.png`.
These are rendered layout evidence, not native input-to-display latency evidence.
Connection-manager tests pass 39 cases, including one/eight seed layouts,
independent ports and visible footer actions. The pre-Sentinel comparison uses
`python3 scripts/ui-performance.py --base 429663ce4e17964dc3a95d2559d574d58a632ff1 --redis --redis-connections --output target/redis-sentinel-form-performance`.
All 24 scenarios pass across five alternating pairs on Apple M4 Max. Standalone
connection-form p95 was 0.049417 → 0.049458 ms (+0.08%); Redis browser chrome
0.049417 → 0.047750 ms. Raw results and environment are in that output directory.
New Sentinel scroll workloads cover one/eight seeds (160 samples after 20 warmups,
1400×900 logical viewport, default egui scale); their absolute release CPU timings
are retained in `target/redis-sentinel-scroll-release.log`. They have no pre-feature
counterpart in this baseline and are not a comparative pass. Native input/display,
first-interaction-after-idle and whole-roadmap density limits remain unresolved.

## Sentinel outage and independent TLS checks

The disposable three-Sentinel fixture now also stops its promoted primary after
confirming replication on the writer connection. Automatic recovery 16411 → 16410
retained the proof value; fresh sessions, the actual async probe and both streaming
tools passed again. This proves discovery on new/reconnected transports, not
automatic workspace reconnection or safe outcomes for in-flight writes.

A separate TLS-only fixture uses independent ephemeral CAs for Sentinel and data
endpoints. Sessions, MONITOR and subscriptions pass with the correct per-scope
trust and reject swapped/missing trust and a wrong discovery hostname. Neither
fixture touches saved profiles or user databases. Raw logs:
`target/redis-sentinel-outage.log`, `target/redis-sentinel-tls.log`.
Network partitions, in-flight failover outcomes and automatic reconnect policy
remain to be covered. Sentinel scroll p95 was 0.129166 ms (one seed) and 0.156667 ms
(eight), with p99 0.132375/0.159666 and maximum 0.134042/0.163666 ms respectively;
these are absolute CPU costs, not display-latency measurements.

## Demotion and uncertain-write handling

A verified Sentinel session now closes when a command receives READONLY, exposing
the existing Reconnect action instead of leaving the workspace on a demoted node.
The error identifies the stale primary and states that the rejected command was
not replayed. Ordinary standalone connections retain their existing READONLY
behavior. Read pipelines also invalidate a verified Sentinel connection on this
response. No preflight, heartbeat, retry, or extra successful-path exchange was
added; standalone and Sentinel startup budgets are unchanged.

A mock protocol test verifies both READONLY demotion and a dropped reply after
INCR: the session closes, a subsequent command is refused locally, and later
commands in the submitted batch are not sent. Lost replies retain the warning
that a write may already have completed. This is bounded in-flight failure
coverage; automatic reconnection and real partition/in-flight failover tests are
still outstanding. UI drafts/errors must survive any future automatic transport
recovery, and an uncertain write must never be replayed.

## Bounded workspace transport recovery

After an established Sentinel workspace loses its connection during an operation,
the worker now makes one bounded discovery/reconnect attempt. It discards the
completed job before recovery and never resends it. Startup errors and explicit
connection/database changes do not trigger this recovery. Partial archive/bulk
reports are delivered first. Failed automatic recovery leaves manual Reconnect
available; there is no periodic retry/heartbeat or idle polling.

Recovery messages preserve the original error, draft editor text, mutation draft,
command text and displayed results. Success warns that displayed data may be
stale, invalidates the previous scan cursor, and clears pending background work;
it does not issue a refresh. Manual Reconnect now also preserves drafts and the
uncertain-write warning, including when reconnect itself fails. Dedicated MONITOR
and Pub/Sub workers still require an explicit restart after losing their socket.

Mock protocol coverage verifies two primary setups with exactly one submitted
INCR, a lost response, then no command/refresh on the recovered socket. UI state
tests verify recovery, failed manual reconnect with a dirty value, retained
warnings/drafts, and no queued background work. All 100 targeted Redis tests and
Clippy pass. WASM passes with its existing 59 warnings. Logs:
`target/redis-sentinel-reconnect-{tests,clippy,wasm}.log`.
The release comparison and its repeat are recorded below; the hash-inspector
scenario remains inconclusive. No native recovery interaction claim is made. Live
workspace recovery during failover, recovery UI captures, and dedicated streaming
reconnection policy remain outstanding.


## Live workspace recovery and repeat measurements

The live fixture now keeps a workspace connected through primary demotion. Redis
closed existing clients during this handoff; only the independent fixture observer
was reopened for ROLE checks. The workspace recovered itself. The failed INCR
left the counter at zero, and a fresh explicit INCR after recovery advanced it to
one. The original command editor text and failure warning were retained. The
fixture passed in `target/redis-sentinel-workspace-live-final.log`; the first run's
observer-EOF failure is retained separately in `target/redis-sentinel-workspace-live.log`.
This covers a new primary already available when recovery starts, not every
outage timing or an ambiguous committed write on a real server (the latter has
mock protocol coverage).

The 24-case release comparison had two noisy scenarios; all timings were retained.
A repeat used the exact same binaries and unchanged fixtures/thresholds, five
additional alternating pairs, with no competing builds/fixtures. Across all ten
pairs, 23 scenarios pass and Redis hash-inspector remains inconclusive due to
noise (median p95 0.037938 → 0.036625 ms, no repeatable slowdown). This is not an
all-pass result. Both runs and the combined assessment are retained at
`target/redis-sentinel-reconnect-performance` and
`target/redis-sentinel-reconnect-repeat`; binary hashes are in the repeat metadata.
Native input/display and first-interaction-after-idle checks remain outstanding.

Recovery layout capture (disposable data with simulated recovery state through
the same production UI handler) is visually verified at
`target/redis-recovery-screenshots-final/recovery-restored.png`. Edit mode retains
an unsaved quoted value with Save/Discard controls; the connection is restored
while the uncertain-write warning and stale-data notice remain visible. This is
rendered UI evidence, separate from the real failover test above.

## Time-series inspector foundation

RedisTimeSeries keys (`TSDB-TYPE`) now have timestamp-based sample pages in the
existing virtualized inspector, with shared Previous/Next controls, local-page
filtering, and no sample mutation actions. The key browser recognizes the type
and requests its latest sample for the preview. TS.RANGE/REVRANGE/GET/INFO are
recognized as read-only, single-key commands by the editor and Cluster router.

TS.RANGE requests 201 samples, retains 200 and uses the extra sample to decide
whether another page exists. The next lower bound is the last displayed timestamp
plus one, with ordered/nonnegative timestamps and numeric sample values checked.
Bulk strings, simple strings (observed on Redis 8.2), and RESP double values are
accepted without replacing their displayed representation with a rounded float.
The latest-sample grid preview retains the existing 256-character display cap.

On established healthy/cached-node connections, initial inspection remains two
exchanges (TYPE/PTTL, then TS.RANGE), and later pages send TYPE/PTTL/TS.RANGE in
one pipeline. Protocol tests enforce request arrival before replies. No TS.INFO
or per-page metadata preflight was added; the total sample count is currently
unknown rather than fetching potentially large labels/rules merely to display it.
This is a moving dataset, not a snapshot: concurrent insertions before the current
cursor require refresh to discover.

Unit/protocol coverage validates page bounds, order, empty data and i64 maximum
timestamps. The Redis 8.2 disposable fixture verifies 435 samples across three
pages without loss/duplicates, empty series, type changes and latest-sample
preview. Logs: `target/redis-timeseries-*.log`. Cluster live module routing,
visual screenshots, release UI comparisons/new workloads, range controls and
additional module inspectors remain pending. No completed parity/performance
claim is made for this increment.

Command reference: https://redis.io/docs/latest/commands/ts.range/

## Time-series ranges and visible controls

Time-series cursors now retain an optional inclusive end bound. Blank inputs mean
unbounded; explicit bounds are nonnegative Unix milliseconds through i64::MAX and
are parsed as integers without floating-point precision loss. Invalid/reversed
ranges are rejected before network work. Next/Previous preserve the selected end
bound, and applying a range resets page history through the existing search path.

The inspector uses persistent From/Through labels, shared Apply/Previous/Next
buttons, and an unapplied-range indicator. Its metadata explicitly reports samples
in the page rather than implying a known total. The live fixture verifies a
300-sample finite range across pages, including its inclusive end. Unit tests cover
blank/equal/reversed bounds and timestamps above 2^53. The idle inspector test
confirms no continuing repaint requests. Targeted tests and Clippy pass.

Native framebuffer capture was visually inspected at
`target/redis-timeseries-screenshots-final/timeseries-range.png` (Redis 8.2 disposable
fixture, 600 samples). This is layout evidence, not native input/display latency.
The 24-case release comparison in `target/redis-timeseries-ui-performance` is
running. The new `redis_timeseries_inspector_frame_benchmark` covers 200 displayed
samples, range text changes and scrolling; its release measurement remains pending.
Cluster module fixture coverage, metadata/chart views, and additional modules
remain outstanding.


## Time-series Cluster and release validation

The optional Redis 8.2 Cluster fixture explicitly loads RedisTimeSeries on three
primaries. Its test chooses one key per owner and verifies latest-sample preview
order, 200+5 sample pagination, inclusive 50–100 ranges and repeated reads with
cached connections. All 18 fixture tests pass (5 topology, 12 routing, 1 module
integration); no module resharding/failover claim is made. Invocation:
`RUSTABLE_REDIS_CLUSTER_MODULE_QA=1 bash scripts/test-redis-cluster.sh`.
Raw log: `target/redis-timeseries-cluster.log`. Clippy passes, and WASM passes with
its existing 59 warnings (`target/redis-timeseries-cluster-clippy.log`,
`target/redis-timeseries-range-wasm.log`).

The initial 24-case CPU comparison had one inconclusive connection-profile case.
Five additional alternating pairs used the exact same compiled binaries with no
competing builds/fixtures; thresholds and workloads were unchanged. The repeat
and combined ten-pair assessment both pass all 24 cases. Baseline remains
429663ce4e17964dc3a95d2559d574d58a632ff1. Combined Redis p95 base → head (ms):
browser chrome 0.048875 → 0.048500; connection profile 0.049813 → 0.049229;
key grid 0.119042 → 0.119812; hash inspector 0.034395 → 0.034249;
string inspector 0.045959 → 0.045563. All raw measurements/environment/binary hashes
are retained under `target/redis-timeseries-ui-performance` and
`target/redis-timeseries-ui-repeat`, including `combined-results.json`.

The new time-series range-edit/scroll workload has 200 samples displayed in a
420-point inspector within a 1200×760 viewport; 160 measurements follow 20 warmups.
On the same Apple M4 Max release binary, CPU p50/p95/p99/max were
0.070125/0.081625/0.084417/0.097000 ms. Raw samples are in
`target/redis-timeseries-scroll-release.log`. This new workload has no pre-feature
counterpart and is an absolute cost measurement, not a comparative pass. The idle
repaint test passes. Native input-to-display/multi-monitor checks and the older
whole-roadmap density comparison remain unresolved.

### On-demand time-series metadata

The inspector now opens a shared SQL-style diagnostics window for series totals,
retention, memory, timestamp bounds, chunks, properties, labels and compaction
rules. A fixed `EVAL_RO` script declares the selected key for Cluster routing and
calls `TS.INFO` without `DEBUG`. Healthy established/cached-owner paths use one
exchange on explicit open/refresh, with zero additional exchanges on sample
browsing. Redirects and cold node setup retain the existing Cluster routing costs.
The server response caps labels/rules at 64 each and text at 256 bytes before
transmission; this does not cap Redis's internal TS.INFO construction work.
`TS.INFO` and `EVAL_RO` permissions are required; no uncapped fallback is attempted.
Lua numbers at or above 2^53 are visibly approximate. Key labels are cached off the
render path, and metadata rows use the existing virtualized table.

Live standalone validation covers 70 oversized labels, visible truncation counts,
a real compaction rule, sample pagination and type changes. The optional Redis 8.2
Cluster fixture validates metadata on keys owned by all three primaries. Logs:
`target/redis-timeseries-info-live-final.log` and
`target/redis-timeseries-info-cluster.log`. The first standalone attempt exposed
Lua status-string wrappers in TS.INFO responses; normalization fixed that issue,
and the original failure log remains available. Unit checks cover malformed and
oversized responses, empty key names, and idle repaint settling. The native
screenshot is `target/redis-timeseries-info-screenshots/timeseries-info.png`.
The protocol mock confirms a single declared-key EVAL_RO request and no fallback
on NOPERM, while retaining the existing first-page/follow-up exchange assertions
(`target/redis-timeseries-info-protocol.log`). All 106 targeted Redis tests pass;
Clippy passes, and WASM retains its existing 59 warnings. Final logs use the
`target/redis-timeseries-info-{unit,clippy,wasm}-final.log` prefix.

All 26 release CPU scenarios pass over five alternating pairs against baseline
429663ce4e17964dc3a95d2559d574d58a632ff1. Shared diagnostics p95 base → head:
0.034417 → 0.033792 ms; topology 0.040375 → 0.039709 ms; browser chrome
0.047792 → 0.047791 ms; key grid 0.118959 → 0.118875 ms. Environment, raw
samples, all comparisons and build logs are retained in
`target/redis-timeseries-info-ui-performance`. The runner now accepts
`--redis-tools` to include the existing diagnostics and topology workloads.

The new metadata scrolling workload uses 132 capped rows, a 640-point window,
1200×760 viewport, 20 warmups and 160 samples. On the same release build its
p50/p95/p99/max were 0.079417/0.089125/0.091875/0.092166 ms
(`target/redis-timeseries-info-scroll-release.log`). This is an absolute CPU cost,
not a comparative pass or presentation-latency measurement. Native input-to-display,
mixed-monitor coverage, and historical density-comparison uncertainty remain open.

### Loaded time-series chart (validation in progress)

Table/Chart uses the shared SQL-style segmented control. Numeric values and
normalized points are prepared with the page on the worker, capped at 200 samples.
The plot uses loaded samples only and preserves text-filter/non-finite gaps.
Timestamp offsets are subtracted as integers before floating-point conversion;
hover labels retain the original exact strings. Extreme finite values normalize
before subtraction to avoid overflow. Selecting Chart focuses the plot within the
inspector's scroll area rather than leaving it below the viewport. No commands or
network exchanges are added. The initial viewport issue and its corrected native
capture are preserved under `target/redis-timeseries-chart-screenshots*`.

Unit checks cover adjacent timestamps near i64::MAX, extreme values, NaN, empty and
single samples, preparation caps, and idle rendering. The initial 108 targeted
Redis tests and Clippy passed; WASM retains 59 existing warnings. The final 109 targeted tests include the workspace focus-scroll idle check and
pass, as does final Clippy. The 26-case release comparison is running in `target/redis-timeseries-chart-ui-performance`. The new
absolute chart benchmark is `redis_timeseries_chart_frame_benchmark` and still
needs execution after the comparison. Native hover/input-to-display and mixed-DPI
checks remain unverified. Do not treat the chart's performance milestone as complete
until these pending automated results have been inspected.

### Chart performance follow-up and bounded Cluster read waves

The first chart release comparison completed with 16 passes and 10 inconclusive
cases across unrelated SQL/Redis workloads; it is not a pass. See
`target/redis-timeseries-chart-ui-performance/{results.json,summary.md}`. A process
snapshot immediately afterward shows heavy Simulator location/diagnostic and host
diagnostic activity (`processes-after-run.txt`). That observation may explain
noise but does not prove its cause; no user processes were stopped. Repeat the
same compiled binaries under quieter conditions before assessing the chart cost.
The absolute chart workload still needs execution. This uncertainty is separate
from earlier metadata performance results.

Cluster read batches now run in waves of at most 64 owners instead of rejecting
larger selections. Every response in a wave is drained before preparing the next,
so cache eviction cannot discard unread responses. Result indexes preserve input
order. The existing 1,000-command read cap and 64-entry connection cache remain;
cold setup retains at most eight concurrent handshakes in addition to cached
sockets. Writes retain their existing per-batch limit and are never replayed.

For <=64 warm owners, before/after remains one fan-out exchange; cold setup is
unchanged. Previously >64 owners failed before any read exchange; now N owners
need ceil(N/64) read exchanges plus cold handshakes where needed. The extra waves
allow a formerly unsupported operation while bounding open sockets and outstanding
responses. Batching within each wave preserves fan-out; retaining all sockets for
one exchange would exceed the chosen cache bound. A 65-node protocol fixture
withholds replies until all 64 first-wave requests arrive, then permits the final
wave. It checks order, cache size and successful completion. With a 25ms response
delay per wave the observed cold operation took 81.003ms; this is a synthetic
protocol diagnostic under current machine load, not a stable WAN timing claim.
Existing repeated two-owner and 1,000-command protocol tests still pass. All 12
Cluster unit tests and Clippy pass (`target/redis-cluster-read-waves-unit.log`,
`target/redis-cluster-read-waves-clippy.log`). The live three-primary Redis 8.2
fixture passes all 19 cases (`target/redis-cluster-read-waves-live.log`);
>64 real-node topology/failover coverage remains unverified.

The wave fixture additionally closes the first-wave connection before its reply
and verifies that the router stops and never even connects to the second-wave
owner. Replies from other first-wave nodes may race the intentional close; the
fixture permits that expected socket error only in this failure case. No request
is replayed. Log: `target/redis-cluster-read-waves-failure.log`.

### Quiet chart repeat and established Cluster recovery

Five more alternating CPU pairs reused the exact compiled baseline/candidate
binaries without changing fixtures or thresholds after the heavy background
activity subsided. All 26 repeat scenarios pass. Combining all ten pairs, retaining
the noisy first run, yields 24 passes and two inconclusive scenarios (vertical
precise tessellation and 12-column workspace without inspector), with no classified
regression. Raw results, binary hashes and combined data are in
`target/redis-timeseries-chart-ui-repeat`; the earlier noisy data remains intact.
The new absolute 200-sample chart workload (420-point inspector, 1200×760,
20 warmups/160 samples) reports CPU p50/p95/p99/max
0.026875/0.031542/0.035500/0.039167 ms in
`target/redis-timeseries-chart-release.log`. No pre-feature counterpart exists;
this is not an input/display measurement. Combined uncertainty is still open.

Established Cluster sessions now reconnect through up to eight deduplicated known
primary addresses, starting with their current seed, after a transport failure.
The owning worker makes one bounded recovery attempt and never retains/replays
the failed job. Manual Reconnect uses the same path. Discovery uses the existing
TLS/authentication settings and validated CLUSTER SHARDS parser; only successful
metadata validation installs new routes. Failed recovery remains disconnected and
retains known addresses for another explicit attempt. Initial startup still uses
the configured seed: this is session recovery, not saved multi-seed configuration.

Ordinary operations add zero exchanges. A successful recovery candidate uses
transport/authentication setup followed by CLUSTER SHARDS (no PING preflight),
then restores normal socket timeouts. Each candidate reserves a share of the
remaining connect budget capped at 500ms per setup/read stage; at most eight are
tried. Deadline checks occur between stages. Synchronous DNS and per-stage setup
retain the existing deadline limitations; this is not a hard wall-clock abort.
A fresh transport and topology are required after loss; cached stale routes alone
cannot establish either. No refresh or write follows recovery automatically.

A worker protocol test loses a write reply and closes the original seed listener,
then verifies discovery on an alternate and EOF with no replay or refresh. The
20-case live Redis 8.2 Cluster fixture ends by explicitly shutting down the original
seed and verifies fresh PING after recovery through a surviving node. This proves
transport recovery, not replica promotion or restoration of unavailable slots.
All 111 targeted Redis tests and Clippy pass. Logs:
`target/redis-cluster-recovery-{worker,unit-final,clippy,live}.log`.

### Live slot migration coverage

The disposable three-primary fixture now performs a real slot migration following
Redis's [CLUSTER SETSLOT](https://redis.io/docs/latest/commands/cluster-setslot/)
and [MIGRATE](https://redis.io/docs/latest/commands/migrate/) lifecycle. It chooses
an empty slot owned by the fixture seed, creates two same-slot keys, migrates one,
and verifies that the source actually returns ASK. A routed INCR reaches 1 exactly
once without updating the cached owner. A mixed GET/GET/PTTL batch reads both the
migrated key and the key still on the source, retaining original result order and
expiry. After removing the remaining source key and assigning ownership on all
primaries, the source actually returns MOVED; the next INCR reaches 2 and updates
the cached route, and a repeated routed INCR reaches 3. The test deletes its key
and restores the empty slot's original ownership before subsequent fixture cases.

All 21 fixture checks pass, including module routing and the final seed-shutdown
recovery test; Clippy passes. Logs:
`target/redis-cluster-resharding-live-final.log` and
`target/redis-cluster-resharding-clippy.log`. This increment changes tests only;
production request sequencing and UI paths are unchanged. It covers one controlled
slot migration, not interrupted/concurrent resharding, partitions, or replica
promotion. Recovery currently retains primary routing addresses; retaining
validated replica discovery addresses is the next prerequisite for robust recovery
after promotion when the original primary cannot be reached.

### Replica discovery and planned promotion

Cluster discovery now retains up to eight validated addresses for recovery,
including replicas. Selection reserves up to four candidates of each role before
filling unused capacity, favors online addresses within each pass and distributes
candidates across shards. Valid loading/failed addresses may occupy remaining room:
health is a snapshot, not a permanent exclusion. The current seed is still tried
first, with eight total attempts. Key routes remain online-primary-only. The
candidate list comes from the already-read CLUSTER SHARDS response, so initial
connection and normal browsing add zero exchanges. Fresh recovery metadata replaces
both routes and candidates; TLS identity/authentication handling is unchanged.

The worker protocol test now covers both an alternate primary and an address that
was originally advertised only as a replica. It verifies no replay or automatic
refresh after the lost write. Candidate tests check role balance, the eight-address
cap, unavailable/unannounced endpoints and primary-only routing. All 112 targeted
Redis tests and Clippy pass; the existing three-primary fixture also passes.
Logs: `target/redis-cluster-replica-discovery-{unit-final,clippy-final,regression}.log`.

New `scripts/test-redis-cluster-failover.sh` owns six disposable Redis 7.4 nodes at
16600–16605. The live test writes and waits for replication, performs planned
CLUSTER FAILOVER for all three replicas, waits for valid slot ownership to converge
on all six nodes, then shuts down every original primary. A session opened before
promotion recovers through its retained replica addresses. The counter remains
zero after recovery and reaches one only after a fresh explicit increment. The
settled run passes (`target/redis-cluster-replica-promotion-settled.log`). Earlier
failure logs are retained: checking only the old seed allowed the test to proceed
while other nodes still reported inconsistent transitional shard metadata. The
production decoder was not relaxed. This covers planned promotion and subsequent
loss of old primaries, not abrupt-loss elections or partitions.

Promotion fixture follow-up: repeat execution exposed two additional fixture
weaknesses. Nodes originally shared a working directory, allowing replication RDB
files to collide; each Redis process now has a separate `--dir`. A missing-value
failure from the shared-directory fixture is preserved in
`target/redis-cluster-promotion-preview.log`; file collision is a plausible cause,
not a proven causal diagnosis. The test now checks the exact value on the replica
before promotion and on its promoted owner both before and after old-primary
shutdown. One isolated run also timed out starting manual promotion before all
replica links were ready. The fixture now waits for each ROLE link to be connected
and for every node's valid topology to agree after each promotion, with the same
15-second per-stage failure bounds. It does not retry user operations or relax
production topology validation. Repeat results are recorded below once finished.

The native capture `target/redis-cluster-promotion-screenshots/cluster-topology.png`
shows three promoted primaries and the old primaries as replicas. Health values are
the responding node's reported snapshot and can lag shutdown; this screenshot is
not evidence that the stopped nodes remain reachable. It uses the actual topology
view after the live fixture, not synthetic node rows.

Fixture stability remains open: the first readiness-adjusted run passed, but its
repeat timed out during a manual promotion (`redis-cluster-replica-promotion-ready*.log`).
A diagnostic run then passed with all value checkpoints intact. The fixture now
retains per-node Redis logs on failures and includes both roles/addresses in the
timeout assertion. Three targeted diagnostic repeats are running to capture the
intermittent condition; a passing isolated run must not be reported as stable
failover coverage. Candidate selection's 112 unit/protocol tests and Clippy remain
green, and the 22-case three-primary regression fixture passed. No production
deadline, topology validation or no-replay rule was weakened.

All three targeted diagnostic repeats passed, with value checkpoints and no-replay
checks intact (17.17s, 25.72s, 14.12s whole fixture-test time, including staged
replication/promotion/convergence waits). Logs:
`target/redis-cluster-replica-promotion-diagnostic-{1,2,3}.log`. The earlier promotion
timeout was not reproduced in these runs; its exact cause remains unresolved.
These passes provide repeated planned-promotion evidence, not a claim that all
Cluster failover timing is stable or that abrupt elections/partitions are covered.

### Saved Cluster seeds (UI performance validation pending)

Profiles now retain up to seven alternate seed addresses beside the preferred
host/port. Older profiles default to an empty alternate list. Validation rejects
duplicates, invalid host/port values and oversized lists; credentials remain shared
and separate from serialized settings. The connection form adds SQL-style controls
with stable row IDs and keeps its footer actions visible at the maximum row count.
Native capture: `target/redis-cluster-seeds-form.png`.

Initial Connect and Test now try saved addresses, validate discovered topology,
and retain configured candidates for later recovery. Successful setup restores
normal read/write timeouts: a protocol test delays the first GET beyond its startup
budget to guard against accidentally retaining shortened socket timeouts. Invalid
duplicate settings are rejected before connection attempts. With no alternates,
the old connection path is unchanged. Successful candidate work remains transport/
authentication setup, PING and CLUSTER SHARDS; there are no new exchanges on normal
query/browse paths. Fallback adds failed-candidate setup only during connection.
The connect budget is divided among remaining candidates, with deadline checks
between attempts and after discovery. DNS and individual setup stages retain
existing limits; it is not a hard wall-clock abort. TLS is never disabled for a
fallback, and no user command is replayed.

All 114 targeted Redis tests, the maximum-row form check, Clippy, and WASM pass
(the latter retains 59 existing warnings). The live Cluster fixture shuts down its
preferred seed, then verifies both established-session recovery and a fresh
connection through a saved alternate. Logs use `target/redis-cluster-seeds-*`.
The 26-case release UI comparison is running in
`target/redis-cluster-seeds-ui-performance`. New absolute scrolling workloads for
zero/seven alternates are `redis_cluster_profile_frame_benchmark`; they still need
execution after that comparison. Performance validation is not yet complete.
Dedicated MONITOR/Pub/Sub startup still uses the preferred host and needs separate
alternate-seed integration; the form explicitly scopes its fallback description to
Connect and Test.

### Cluster seed form validation and streaming fallback

The five-pair release UI comparison passes all 26 scenarios against baseline
429663ce4e17964dc3a95d2559d574d58a632ff1. Redis connection-form p95 base → head is
0.049042 → 0.049792 ms; browser chrome 0.047791 → 0.048459 ms; key grid
0.119916 → 0.119666 ms. All environment/build/raw data are retained in
`target/redis-cluster-seeds-ui-performance`. The new absolute Cluster-form scrolling
workloads use a 1400×900 viewport, 20 warmups and 160 samples: zero alternates
p50/p95/p99/max 0.089667/0.101125/0.103792/0.104625 ms; seven alternates
0.121250/0.142458/0.143750/0.144708 ms. Raw data:
`target/redis-cluster-seeds-form-release.log`. These are CPU costs, not comparative
passes for a pre-existing multi-seed form, or input-to-display measurements. Older
chart/density uncertainty and native monitor/display checks remain open.

MONITOR, SUBSCRIBE, PSUBSCRIBE and PUBSUB startup now use saved alternate seeds.
A failed TCP/TLS connection or startup transport failure closes its socket before
trying another seed within the connection budget. This includes a proxy accepting
TCP after Redis has stopped and then closing before any Redis startup response.
Authentication and malformed/protocol errors do not trigger fallback. Only the
existing four read-only streaming commands are accepted; application writes are
not eligible. Once a reader has been returned, connection loss does not restart
or replay the stream. Buffer lookahead preserves the command ACK/message bytes for
the existing parser. AUTH and the start command remain pipelined in one exchange,
with no PING, role or topology preflight added to Cluster streaming. Normal socket
timeouts are restored after startup; TLS identity validation is unchanged.

117 targeted Redis tests and Clippy pass. Protocol cases cover authenticated and
passwordless startup for all four commands, a TCP proxy closing after the startup
requests, pre-connect cancellation, no retry after an authentication error, and a
TLS handshake (never plaintext AUTH) on the alternate. The 22-case live Cluster
fixture passes and starts all four streaming operations after shutting down the
preferred seed. Initial socket-only fallback failed this live proxy case; the
original failure is preserved in `target/redis-cluster-stream-seeds-live.log`.
Final logs: `target/redis-cluster-stream-{startup-final,seeds-unit-final,seeds-clippy-final,seeds-live-final}.log`.

### Vector-set browsing increment (in progress)

Added Redis 8.4 ordered element pages, binary cursors, bounded server output and
shared virtualized collection UI. Disposable Redis 8.4.8 validation passed for
438 elements including empty and binary names, oversized-name rejection and
changed-type errors (`target/redis-vectors-live.log`). Existing Redis tests passed
118 cases; the added protocol test proves two initial exchanges and one follow-up
exchange (`target/redis-vectors-all-unit.log`, `target/redis-vectors-protocol.log`).
Clippy passed with all targets and QA enabled (`target/redis-vectors-clippy.log`).
The first native capture is `target/redis-vectors-screenshots/vector-elements.png`;
its long helper caption has since been shortened with details moved to a tooltip.
Release UI comparisons, final capture and live Cluster vector routing remain
pending. Embedding/attribute inspection and similarity queries remain roadmap work.

Vector Cluster follow-up: the Redis 8.4.8 three-primary fixture now passes
cold/warm element counts and two-page inspection on each slot owner, plus its
existing routing, topology and seed-recovery checks
(`target/redis-vectors-cluster-live-final.log`). The first fixture attempt used
an invalid module path; vector sets are built into this image, and the corrected
fixture uses no extra module load. Shared release UI comparison is running in
`target/redis-vectors-ui-performance`; no performance result is claimed yet.
A dedicated 200-element inspector scrolling workload has been added for absolute
cost measurement; it has no equivalent vector view on the older baseline.
WebAssembly compilation also passed with existing warnings
(`target/redis-vectors-wasm.log`).

Vector browsing performance follow-up: the 26-case, five-pair release comparison
against `429663ce4e17964dc3a95d2559d574d58a632ff1` passed 25 cases with sidebar/20
inconclusive (`target/redis-vectors-ui-performance`). Five targeted sidebar pairs
using the same saved binaries passed; combined ten-pair sidebar results also
passed (`target/redis-vectors-sidebar-repeat/results.json`). All seven Redis
comparison scenarios passed initially. This does not close earlier whole-roadmap
uncertainty or native presentation checks. Raw logs, p50/p95/p99/max and machine
metadata remain in those directories.

The new 200-element vector inspector workload measured p50/p95/p99/max
0.051708/0.059042/0.059959/0.061000 ms, 20 warmups and 160 samples at a 1200×760
logical viewport (`target/redis-vectors-inspector-release.log`). This is absolute
CPU layout/tessellation cost with wheel input, not comparative evidence against
an equivalent older vector inspector or native input-to-display latency.

### Selected vector elements (in progress)

Implemented explicit element detail loading, virtualized components and cached
JSON highlighting/copy. One declared-key EVAL_RO request combines VEMB/VGETATTR;
no requests are added to ordinary paging. Server output and client validation cap
4,096 dimensions, 128 bytes per component and 64 KiB attributes. Redis constructs
values internally before these limits. No automatic write or fallback fetch.

Standalone tests cover binary names, absent attributes, large integer preservation,
oversized attributes and missing elements. Live Cluster tests load details from
all three primaries on first/repeated use. The protocol test proves permission
errors return without an uncapped fallback. 120 Redis unit tests, all-target QA
Clippy and WebAssembly compilation pass (existing WASM warnings):
`target/redis-vector-element-{live-final,cluster,unit,clippy,wasm}.log`.
Native captures and release CPU validation are in progress; the previous browsing
comparison does not validate the new detail interaction. Similarity queries and
metadata remain further vector work.

Native vector detail capture reviewed:
`target/redis-vector-detail-screenshots/vector-detail.png` shows the component table,
shared copy controls and syntax-highlighted attributes with a >2^53 integer intact.
The shared 26-case release comparison is running in
`target/redis-vector-detail-ui-performance`; the new
`redis_vector_detail_frame_benchmark` covers 4,096 components and roughly 64 KiB
of attributes and must be run from the resulting candidate release executable.
No detail performance result is claimed yet.

### Similarity search investigation

Redis 8.4 VSIM supports element-based queries, explicit COUNT and EF controls.
A disposable 8.4 probe verified ELE/WITHSCORES results inside EVAL_RO, but source
inspection found that Lua and MULTI disable VSIM's normal background execution:
https://raw.githubusercontent.com/redis/redis/8.4/modules/vector-sets/vset.c
The UI should therefore use direct VSIM, without TRUTH/NOTHREAD or a Lua wrapper.
COUNT alone does not bound returned element-name bytes, and ordinary Redis parser
limits currently cap nesting rather than total bytes. Before implementing the UI,
add an opt-in bounded response path that closes the connection on limit/protocol
failure, retains normal TLS handling and pipelines ASKING plus VSIM during ASK.
Keep ordinary query/parser paths unchanged. Cover oversized replies, exact limits,
server errors, connection reuse and redirect exchange counts; never replay after
an uncertain transport failure. This transport work is pending, not implemented.

Vector detail CPU validation completed: all 26 shared scenarios passed across five
alternating release pairs (`target/redis-vector-detail-ui-performance/results.json`).
The new maximum-size detail workload (4,096 dimensions, approximately 64 KiB JSON,
1200×900 logical viewport, 20 warmups, 160 samples) measured p50/p95/p99/max
0.052000/0.054833/0.060792/0.062000 ms on Apple M4 Max/macOS 27.2. Raw samples are
in `target/redis-vector-detail-release.log`; environment/toolchain metadata is in
the comparison directory. This is absolute CPU cost, not an equivalent-view
baseline comparison or a native presentation measurement. Earlier unresolved
whole-roadmap/native checks remain open.

The opt-in bounded RESP2 transport API is now implemented in vendored Redis, but
not yet used by application queries. It preserves the normal parser/query paths,
limits received bytes including buffered data, refuses dirty/RESP3/PubSub sessions,
and closes on any parse/transport failure. A socket test proves one exchange for
a two-command batch, ordinary-command reuse, preserved server errors, exact byte
limits and closure without replay on an oversized response
(`target/redis-bounded-response-test-final.log`). Further batch-error/TLS/redirect
coverage and application integration remain pending. The initial test failure
was a formatted-error assertion mismatch (`target/redis-bounded-response-test.log`).

### Element-based similarity search (in progress)

Implemented Find similar on a selected vector element, top-20 score results and
Inspect actions into component/attribute details. Direct VSIM uses EF 100 and the
opt-in bounded RESP2 transport, preserving server background execution. No Lua,
TRUTH, NOTHREAD or routine preflight is added. Warm direct reads use one exchange;
MOVED/ASK adds the existing rejected-source exchange plus target execution, with
ASKING/query pipelined. Cold endpoint handshakes retain existing routing costs.
Transport/protocol failures stop the connection/router without replay.

Live standalone checks cover binary source/result names, exact ordering for
matching/perpendicular/opposite vectors, removed keys and missing source errors.
Redis 8.4 returns an error for an absent element in an existing set; the first
fixture assertion expected an empty result and was corrected to preserve the
observed server error. Logs: `target/redis-vector-similar-{live,scores-live}.log`.
Cluster checks cover cold/repeated queries on all three primaries, MOVED/ASK,
denied ASKING with a valid following response, and oversized-response shutdown
(`target/redis-vector-similar-cluster.log`). 124 unit cases, Clippy and WASM passed
before the database-switch correction; its targeted test/Clippy are recorded in
`target/redis-vector-similar-{database-test,clippy-final}.log` when complete.
A new maximum-result UI workload is added. Native capture, release UI comparison,
and additional bounded transport TLS/batch-error coverage remain pending.
Arbitrary query vectors, filters and adjustable search controls remain roadmap work.

Similarity follow-up: native capture reviewed at
`target/redis-vector-similar-screenshots/vector-similar.png`. It shows ranked scores
and per-result Inspect actions using shared controls. The database-switch test
and final all-target QA Clippy pass. A protocol regression test also now proves
that ordinary single-query and pipeline parse failures discard the connection
before any later bounded search (`target/redis-vector-similar-malformed-test.log`);
previously these malformed-response paths could retain parser state. Healthy
paths add no exchanges; only invalid-response cleanup changes.

The 26-case shared release comparison is running in
`target/redis-vector-similar-ui-performance`. Run
`redis_vector_matches_frame_benchmark` from its candidate executable afterward
for the new 20-result scrolling view. No similarity UI performance pass is
claimed yet. Bounded TLS and additional batch-error coverage are still pending.

## Table polish and local IntelliSense — 2026-11-07

Analyze now groups summary metrics and namespace charts above styled top-namespace
and top-15-key tables, with aligned human-readable memory, exact-byte hover,
restrained striping and memory bars. The same table/header conventions cover
Search, Pub/Sub, vector results and diagnostics. Remaining Redis copy actions use
stable-width, temporary in-place confirmation. `UI-STYLE-GUIDE.md`, linked from
AGENTS.md, makes table/list treatment, action feedback and whole-view visual review
required conventions.

CLI/Workbench now offer command/subcommand completion, argument-aware options,
syntax/help, loaded-key/index and matching-inspector field/member suggestions,
keyboard/mouse acceptance and token-safe replacement. The catalog contains 427
commands/subcommands including module/vector signatures. See
`REDIS-INTELLISENSE.md` for shortcuts, provenance and limits: advanced module clauses
and count-dependent grammar are not exhaustive, and this is not full Insight parity.
No driver/execution path or network round-trip budget changed.

Validation: 743 tests passed (192 ignored), strict all-target Clippy passed,
browser locked WASM check passed with 61 existing warnings, packaging helper check
passed, and performance-runner tests passed (8). Final release CPU comparison:
31/35 cases passed; four justified visible-work increases and all raw results are
recorded in `UI-PERFORMANCE.md`. No blanket performance or native-input claim.

Actual captures: `target/redis-polish/verified-dark/`, `verified-light/`,
`verified-completion/command-editor.png` and `verified-keys/command-editor.png`.
The attempted narrow capture retained 1200 logical pixels and is not narrow-layout
evidence. Desktop lock prevented manual native interaction/display checks.
This increment is local implementation work, not a new release.


### Bounded final-release Workbench milestone

Saved-connection history, recording/clear controls and individual result sections
are implemented and validated; successful replies remain available on later
failure, with explicit not-run commands and preserved unknown-write warnings.
Results use bounded transport and virtualized layout. See
[the release checklist](REDIS-FINAL-RELEASE-PLAN.md) for tests, theme captures and
the accepted +0.010792 ms p95 CPU cost for separate result controls. This is not a
release announcement; the remaining bounded checklist and publication gates are open.


### Bounded final-release IntelliSense milestone

Common count-dependent clauses now follow declared key/weight counts; explicit
server-hint refresh caches version and representative module availability for local
advisory completion. Metadata denials remain unknown, suggestions stay insertable,
and typing never requests server metadata. Standalone/Cluster protocol, cache
invalidation, browser and dark/light native captures are verified. The five-pair
same-machine release CPU comparison passed all measured scenarios, including the
new counted-key and capability workloads. Detailed percentiles, bounds, caveats
and evidence are in [the release checklist](REDIS-FINAL-RELEASE-PLAN.md).
Vector controls and the remaining checklist/publication gates are still open.


### Bounded final-release configurable vector milestone

The existing similarity workflow now supports supplied numeric vectors, attribute
filters, result count and bounded search/filter effort. Direct background VSIM,
worker validation, the 1 MiB response cap and virtualized results are preserved.
Live Redis, protocol/error-alignment, UI draft submission, browser and dark/light
native captures are validated. The result panel has an explicitly accepted
+0.009333 ms (+17.0%) p95 CPU cost for visible controls; typing and after-idle
workloads passed within their documented baseline-adapter scope. Full measurements
and tradeoffs are in [the release checklist](REDIS-FINAL-RELEASE-PLAN.md).
This milestone completion is not release publication; editing and remaining gates
are still open.


### Final Redis release scope — editing, workflows and layout

Inline RedisJSON edits now target one explicit path or replace the whole document,
with atomic whole-document conflict checks and retained failed drafts. Creation of
hashes, lists, sets and sorted sets accepts up to 100 entries / 64 KiB combined.
Creation never overwrites an existing key. Path syntax, expiry and validation limits
are documented in [REDIS.md](REDIS.md).

The [isolated workflow assessment](REDIS-COMMAND-WORKFLOWS.md) explicitly defers
transactions and blocking commands. Dedicated write cancellation/outcome tracking,
database-selected startup and transaction node ownership are missing; shared-editor
guards remain intact. This is an evaluated deferral, not implemented parity.

Final layout work fixes Analyze tables collapsing below the fold and aligns the
Pub/Sub composer. A scrolling regression test fails before the Analyze fix and
passes afterward. Five paired release performance runs for final layout changes
passed; earlier editing-control costs have narrow measured justifications in the
[release checklist](REDIS-FINAL-RELEASE-PLAN.md), not blanket performance claims.

The standalone harness now uses disposable ephemeral ports and verified TLS tests
cover batch-error alignment, oversized-reply closure and no replay of later writes.
The remaining release acceptance, exact-tag demo and publication evidence belong
to the release checklist; this entry does not assert publication.


### Final candidate tagged; publication pending

User-requested follow-ups now include exact-first shared completion ranking,
continuous padded table headers, row hover over child text, native macOS double-click
timing, and inline highlighted JSON-string editing in the desktop and WASM demo.
Value/format and bytes/copy controls share compact aligned rows with copy feedback.
The final performance review records 36 passing repeated scenarios and four
accepted completion costs of 0.006–0.013 ms p95; see UI-PERFORMANCE.md for tail
metrics and limits. All main CI jobs passed. v0.4.21 is tagged at `16e53fa`; signed
package inspection, matching live demo and publication are still pending in the
[release checklist](REDIS-FINAL-RELEASE-PLAN.md).


### Final bounded Redis release published

[v0.4.21 verification](RELEASE-0.4.21-VERIFICATION.md) records the completed
bounded checklist, signed/notarized packages and matching deployed WASM demo.
Live SQL, Redis-first startup/PING, provenance/checksum/headers, public downloads
and updater feed passed. Transactions/blocking workflows remain explicitly
deferred for the documented safety requirements; this is not exhaustive parity.
