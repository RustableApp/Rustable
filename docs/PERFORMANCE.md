# Performance evidence

## SQLite million-row baseline — 2026-10-30

Reproduce with the existing mise-managed toolchain:

```sh
mise exec rust@stable -- cargo test --release million_row_browse_baseline -- --ignored --nocapture
```

The benchmark invokes the production SQLite browse and query paths. It creates a disposable disk-backed SQLite file with 1,000,000 rows (`id INTEGER PRIMARY KEY`, text name, integer score), a secondary score index, and ANALYZE statistics. The fixture is 37,154,816 bytes and is removed after the test. It refuses to overwrite an existing fixture. Each case measures 50 samples after five warm-up runs, including schema/index/trigger metadata loading and owned result construction for 500 rows.

Environment: local macOS 27.2, optimized Rust release test build. Hardware identification via sysctl was denied by the sandbox, so CPU/RAM are not established. These are warm-cache observations, not portable performance guarantees.

| Case | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| First primary-key page | 0.076 ms | 0.087 ms | 0.088 ms |
| Primary-key page at offset 900,000 | 6.480 ms | 7.220 ms | 7.561 ms |
| First page sorted by indexed score | 0.150 ms | 0.180 ms | 0.198 ms |
| Indexed-score page at offset 900,000 | 11.264 ms | 11.803 ms | 11.908 ms |

A cross join was cancelled in 30.045 ms including a scheduled 25 ms delay before cancellation. Scheduler delay is included; this is not an isolated cancellation-latency measurement. A subsequent SELECT succeeded. The benchmark also verifies 500-row browse limits and the 5,000-row query limit.

Deep OFFSET paging has measurable cost even with an index. Add keyset pagination for sequential browsing, including composite keys, duplicate sort values, NULL ordering and reverse navigation; preserve offset fallback for unsupported ordering. Database work remains on its worker, so these timings do not imply a UI frame stall of the same duration.

## Owned-result memory guard

The collector now inspects borrowed SQLite values before copying strings or blobs. It rejects rows exceeding the remaining 16 MiB payload budget without an oversized Rust-owned copy. Tests cover a 20 MB blob, invalid UTF-8 in a row that should be rejected before conversion, and an exact-budget boundary. This is a retained payload cap, not a total-process RSS limit: SQLite itself may allocate oversized values, and vectors, metadata, allocator overhead, SQL text, and GUI buffers use additional memory.

## Still required for production acceptance

- Input-to-paint and CPU/GPU p50/p95/p99 on known 60/120/144 Hz hardware.
- Idle wakeups/CPU, cold startup, total RSS and allocation peaks.
- Cold-cache browsing, wide tables, large values, many pending edits and concurrent sessions.
- Worker queue turnaround, network drivers, slow/failed connections and cancellation under locks.
- Native scroll and editing measurements against TablePlus using the same fixture.

No 144 Hz responsiveness or full performance parity claim is supported by this database-only baseline.

## Sequential primary-key paging — 2026-10-30 follow-up

The production SQLite driver now keeps one bounded page cursor. Next-page requests can seek past the final key of the preceding page when ordering by a single non-null primary key. INTEGER rowid aliases are supported, including detection of SQLite's PRIMARY KEY DESC exception. Filters and ascending/descending ordering are preserved. Changed requests, schema/data-version changes, SQL execution, or applying mutations invalidate the cursor. Other sorts, nullable/composite keys, arbitrary jumps, and Previous continue to use OFFSET.

Same fixture and release benchmark, 50 warm samples:

| Deep primary-key page at offset 900,000 | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| OFFSET | 6.231 ms | 6.607 ms | 6.763 ms |
| Keyset Next (including version checks) | 0.082 ms | 0.108 ms | 0.125 ms |

The measured p95 reduction is approximately 61× for this supported case. Every benchmark iteration compares the full returned rows against OFFSET output. Tests cover gaps in keys, filtering, descending order, jumps/backward fallback, nullable/exceptional primary keys, and invalidation after writes by a second SQLite connection. This does not improve secondary-index deep paging yet and does not establish native frame latency.

## Bidirectional primary-key paging — follow-up

Previous now uses the cached first key, reverses the SQL order, and restores display order after collecting the page. Navigation tests compare full rows and continuation flags across forward/backward movements, gaps, filters, both sort directions, and partial final pages. If a reverse fetch hits the byte budget before collecting a full page, it falls back to OFFSET to preserve the beginning of the requested page.

Updated same-fixture release benchmark (50 warm samples):

| Deep primary-key page at offset 900,000 | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| OFFSET | 6.400 ms | 7.195 ms | 8.969 ms |
| Keyset Next | 0.089 ms | 0.097 ms | 0.104 ms |
| Keyset Previous | 0.081 ms | 0.083 ms | 0.084 ms |

Secondary sorts, composite/nullable keys and navigation from partial cached pages still use OFFSET. The earlier statement that all Previous navigation uses OFFSET is superseded by this implementation. Timings remain database-only measurements, not native UI frame metrics.


## Full-table CSV streaming — 2026-10-31

Environment: Apple M4 Max, Mac16,6, 48 GiB RAM, macOS 27.2 (26B5101f), optimized release test executable. Hardware was read successfully for this run; earlier hardware-unknown notes describe earlier runs.

The ignored `db::sqlite::full_export_tests::export_streaming_benchmark` creates a disposable disk-backed table with an integer primary key, a 96-character text field and an integer score. It exports in primary-key order three times, including validation, encoding, flush, file sync and atomic publication. It verifies every exported ID using a streaming reader. The temporary fixture and exports are removed after each process exits normally.

| Rows | CSV bytes | Export samples (ms) | Process peak RSS |
| --- | ---: | --- | ---: |
| 1,000 | 110,804 | 5.097, 7.564, 6.675 | 10,158,080 bytes (9.69 MiB) |
| 1,000,000 | 113,778,917 | 343.379, 349.985, 335.939 | 12,468,224 bytes (11.89 MiB) |

RSS is macOS `/usr/bin/time -l` maximum resident set size for the **whole test process**, including fixture generation and validation; it is not an isolated exporter allocation count. Each row-count case ran in a separate process. Output grew about 1,027× while peak RSS grew about 2.2 MiB. This supports bounded application-side streaming for this fixture. SQLite's own memory requirements can grow for other query plans, sorts or large cells. Three samples do not establish tail percentiles; caching, storage and other workloads affect these timings. This is not a GUI frame-time/input-latency measurement or a portable throughput guarantee.

Reproduce by running `mise exec rust@stable -- cargo test --release --no-run`, then use the executable path printed by Cargo:

```sh
env RUSTABLE_EXPORT_BENCH_ROWS=1000 /usr/bin/time -l <test-executable> db::sqlite::full_export_tests::export_streaming_benchmark --exact --ignored --nocapture
env RUSTABLE_EXPORT_BENCH_ROWS=1000000 /usr/bin/time -l <test-executable> db::sqlite::full_export_tests::export_streaming_benchmark --exact --ignored --nocapture
```

## Browse regression sample after profile/window work — 2026-10-31

Apple M4 Max, 48 GiB RAM, macOS 27.2 (26B5101f). Release-mode `million_row_browse_baseline`, disposable 1,000,000-row SQLite fixture (37,154,816 bytes), five warmups then 50 samples per case. This measures database browsing, not frame times or input-to-display latency.

| Browse | p50 ms | p95 ms | p99 ms |
| --- | ---: | ---: | ---: |
| First primary-key page | 0.072 | 0.076 | 0.081 |
| Primary-key OFFSET 900,000 | 5.793 | 6.068 | 6.279 |
| First indexed score page | 0.148 | 0.163 | 0.167 |
| Indexed score OFFSET 900,000 | 10.662 | 11.108 | 11.435 |
| Deep primary-key keyset Next | 0.087 | 0.090 | 0.090 |
| Deep primary-key keyset Previous | 0.084 | 0.088 | 0.104 |

Cancellation completed in 30.051 ms including the deliberate 25 ms trigger delay. Fixture correctness, matching keyset/OFFSET rows and bounded page size passed. Secondary-sort deep pagination remains the slowest measured case and still uses OFFSET; native frame-time verification remains outstanding.

## SQLite language-service transport (2026-10-31)

Release-mode `completion_latency_baseline`, real Syntaqlite 0.13.3 subprocess, 100-table catalog, five warmups and 50 samples. Submission-to-completion latency fell from p50 29.178 / p95 30.711 / p99 30.941 ms to 0.136 / 0.164 / 0.168 ms after replacing the transport polling delay with thread wakeups on document submission and server output. Startup plus catalog took 10.649 ms in the latter run. Each sample verified the expected column suggestion. This measures local service transport and completion, not keystroke-to-screen latency. Lifecycle tests also passed for coalescing, startup failure, and killing/reaping an unresponsive server. Included in the QA and main bundles built on 2026-10-31.

## PostgreSQL completion wakeup — 2026-10-31

The cancellation watcher previously slept for 20 ms between polls, and successful
queries joined that sleeping thread before returning. It now parks with a timeout;
completion unparks it before joining. Cancellation retains its 20 ms poll bound.

A debug-build local loopback PostgreSQL fixture benchmark runs the production
`execute("SELECT 42", safe=true)` path, including transaction setup, preparation,
collection, commit and watcher cleanup. Fifty samples follow five warmups:

| Watcher | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| Sleeping cleanup | 23.881 ms | 25.211 ms | 25.388 ms |
| Immediate wakeup | 0.710 ms | 1.129 ms | 1.207 ms |

These observations isolate a fixed client delay; they do not establish remote
network performance or input-to-paint latency. The live integration test also
cancelled `pg_sleep(10)` and successfully executed a subsequent query.

```sh
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test --locked query_completion_latency -- --ignored --nocapture
```

## PostgreSQL decoded-result budget — 2026-10-31

Result collection now borrows protocol-owned cell bytes and charges the row's
`Value` storage before constructing owned values. Text/blob payloads that cannot
fit are rejected before copying. Numeric output length is calculated before
expansion; array output growth checks include bounds, braces, UTF-8, escaping and
hex expansion, with capacity requests capped by the remaining payload budget.
A row that does not fit marks the result truncated while retaining earlier rows.

Live fixture tests exercise 20 MB text, 20 MB bytea, and an array whose escaping
expands beyond the 16 MiB budget, plus exact-boundary rows, NULL overhead, retained
prefixes and subsequent connection reuse. Unit tests reject oversized invalid
UTF-8 before validation and test compact NUMERIC wire values that expand to
thousands of decimal characters. The full suite passes with these paths enabled.

This bounds owned retained result payload, not total process RSS. The PostgreSQL
client already owns incoming protocol frames; transient conversion buffers,
network/client buffering, column metadata, GUI copies and allocator overhead are
outside this limit. End-to-end RSS/allocation measurements remain required.

## PostgreSQL table open/refresh round trips — 2026-10-31

The previous browse path performed roughly eleven serial protocol exchanges:
relation lookup, column lookup, result-shape preparation, data preparation and
execution, index lookup, and trigger lookup. The parameterized catalog queries
also prepared separately before execution. Index/trigger lookup happened after
`grid.elapsed` was assigned, so the displayed time omitted part of the work.

Browsing now fetches relation identity, estimate, column/type metadata, indexes,
and triggers in one fresh catalog query, then streams the bounded page in one
typed query. Both use the existing connection and PostgreSQL autocommit. There
is no metadata cache requiring invalidation: external DDL is visible on the next
refresh. Domain base types are resolved in the catalog query; supported domain
values retain native decoding and unsupported/extension values use lossless
server text. Sorting still uses the native source column. Metadata is internally
consistent within its catalog snapshot, while—as before—DDL between metadata
and data queries can require a retry. Browsing does not commit or roll back a
user-opened SQL transaction. Reported elapsed time now includes all browse work.

A disposable PostgreSQL fixture behind a local TCP proxy adds 15 ms each way.
Debug build, seven measured samples after two warmups, same one-row indexed
fixture on both paths; result rows/headers/columns and metadata counts compared:

| Table open / refresh | p50 | p95 (maximum of 7 samples) |
| --- | ---: | ---: |
| Legacy serial path | 422.0 ms | 430.8 ms |
| Fresh catalog + typed data | 81.6 ms | 84.4 ms |

This is about an 81% median reduction under controlled latency, not a claim
about the user's server or GUI input-to-paint time. The proxy overlaps delayed
packets and disables Nagle. A retained test-only legacy path allows repeatable
comparison. Live tests cover nested domains, enums, arrays, expression/partial
indexes, trigger metadata, externally changed schema metadata, empty-result
headers, dropped tables, estimates, and explicit transaction preservation.

```sh
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test --locked browse_wan_latency_comparison -- --ignored --nocapture
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test --locked refresh_reads_fresh_metadata -- --ignored --nocapture
```

## PostgreSQL object-list catalog round trips — 2026-10-31

Connection setup and catalog refresh now use two typed exchanges: accessible
schema names/current schema, then selected-schema relations and columns in a
single streamed LEFT JOIN. This removes separate preparation exchanges, the
separate relation-list query, the extra current-schema query on initial load,
and the duplicate schema refresh when switching schemas. No metadata cache or
second connection is used. Relation and column metadata share a statement
snapshot; explicit SQL transactions remain open. Zero-column relations remain
visible, and dropped attributes are excluded.

The opted-in `catalog_wan_latency_comparison` uses the same disposable PostgreSQL
server and 15 ms delay in each direction as the table benchmark. It compares
the previous three prepared catalog queries against the new production
`objects()` path, verifying names and every column metadata field. Debug build,
seven measured samples after two warmups, one table with two columns:

| Catalog refresh | p50 | p95 (maximum of 7 samples) |
| --- | ---: | ---: |
| Previous separate prepared queries | 234.0 ms | 240.3 ms |
| Typed schema query + streamed objects/columns | 82.9 ms | 85.3 ms |

This is a 65% median reduction in this controlled catalog fixture, not a
measurement of production-server or full connection-to-paint latency. Ordinary
table data browsing still uses fresh metadata followed by data (two exchanges);
this change does not implement metadata/data parallelism.

Live regressions cover zero-column tables, dropped attributes, identity/generated
columns, defaults, nullability, primary keys, views, quoted names, schema changes,
and transaction preservation/rollback. Catalog results stream directly into the
owned object list rather than keeping a separate full PostgreSQL row vector;
the final object list itself is not newly capped by this change.

```sh
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test --locked catalog_ -- --ignored --nocapture
RUSTABLE_POSTGRES_QA=1 mise exec rust@stable -- cargo test --locked db::postgres::schema_tests -- --ignored --nocapture
```

## Validated PostgreSQL refresh pipeline — 2026-10-31

Repeat autocommit browsing now sends the fresh catalog query and the data query
without a client-side wait between them, using the same connection. A single
previous catalog row supplies the speculative projection and primary-key order.
The fresh relation names and complete column metadata are compared before data
is displayed. Changes discard the speculative result and retry sequentially;
fresh indexes, triggers and row estimates are used even when columns match.
First loads, different tables, explicit transactions and unknown transaction
status retain sequential metadata-then-data behavior. This supersedes the
previous statement that metadata/data pipelining is unimplemented.

This is network pipelining, not parallel execution on the PostgreSQL server.
Each query has its own Sync boundary. Schema changes between the fresh catalog
snapshot and data execution can still cause an error, as in the sequential path.
The one-entry cache is connection-local, never persisted, and does not suppress
metadata refresh. Failed speculative projections from renamed/dropped columns
retain the fresh catalog response so the fallback can recover. Filter/sort
columns absent from the hint force a fresh read rather than a premature error.

Same disposable PostgreSQL proxy fixture, 15 ms delay each way, debug build,
two warmups followed by seven measured samples, full row equivalence checked:

| Path | p50 | p95 (maximum of 7 samples) |
| --- | ---: | ---: |
| Legacy serial browse | 408.9 ms | 414.5 ms |
| Fresh sequential catalog + data | 75.5 ms | 80.7 ms |
| Validated repeat-load pipeline | 41.3 ms | 45.9 ms |

The pipeline reduced median repeat-load latency by about 45% relative to the
previous optimized path. This is a controlled database-path measurement, not
production-server timing or native input-to-paint evidence. Reproduce with the
existing `browse_wan_latency_comparison` command above; it now measures all
three paths and compares their results.

## Full-table JSON streaming — 2026-10-31

The existing release `export_streaming_benchmark` now accepts
`RUSTABLE_EXPORT_BENCH_FORMAT=json` (default remains CSV). It uses the same
integer/text/score SQLite fixture, exports three times, then validates every
JSON row and value using one line at a time. Both fixture and destination are
removed after normal completion. Each row-count run is a separate process.

| Rows | JSON bytes | Export samples (ms) | Whole-process peak RSS |
| --- | ---: | --- | ---: |
| 1,000 | 137,786 | 9.084, 4.998, 5.029 | 11,059,200 bytes |
| 1,000,000 | 140,778,899 | 359.318, 388.373, 360.999 | 13,402,112 bytes |

This is the local macOS release test executable measured with `/usr/bin/time -l`.
RSS includes fixture generation and streaming validation, not just the encoder.
Output grew about 1,022× with about 2.23 MiB additional peak RSS, supporting
bounded application-side streaming for this fixture. This is not a PostgreSQL
memory measurement, a total memory guarantee for large cells/sorts, or native
UI frame-time evidence. Three timing samples do not establish tail percentiles;
background packaging work may have affected disk timings.

```sh
mise exec rust@stable -- cargo test --locked --release --no-run
env RUSTABLE_EXPORT_BENCH_ROWS=1000 RUSTABLE_EXPORT_BENCH_FORMAT=json /usr/bin/time -l <test-executable> db::sqlite::full_export_tests::export_streaming_benchmark --exact --ignored --nocapture
env RUSTABLE_EXPORT_BENCH_ROWS=1000000 RUSTABLE_EXPORT_BENCH_FORMAT=json /usr/bin/time -l <test-executable> db::sqlite::full_export_tests::export_streaming_benchmark --exact --ignored --nocapture
```

## PostgreSQL scoped completion — 2026-10-31

The offline parsed-query resolver handles qualified CTE outputs and derived-table
aliases, including explicit column lists, star projections and quoted names. Its
release benchmark used 100 catalog tables / 2,000 columns, five warmups and 50
samples: median 0.031 ms, p95 0.036 ms, maximum 0.124 ms. This measures resolver
CPU time only, excluding subprocess transport, rendering and database work.
Resolution does not execute SQL. Recursion, relation and projection limits bound
traversal; this is not a whole-process memory measurement.

After adding per-scope CTE environments and qualified correlated alias lookup,
the existing scoped-completion benchmark was rerun: median 0.025 ms, p95
0.030 ms, maximum 0.034 ms. Same fixture/sample count as above; this remains a
local resolver measurement of the existing CTE case, not native latency or a
benchmark of correlated queries.

## Correlated completion resolver measurements — 2026-10-31

Expanded the release `scoped_completion_latency` benchmark to cover bare and ambiguous correlated references in addition to the existing qualified CTE case. Each case uses an offline 100-table/2,000-column catalog, five warmups and 50 samples, and asserts the expected suggestion exists.

| Case | p50 | p95 | Max |
| --- | ---: | ---: | ---: |
| Qualified CTE | 0.037 ms | 0.048 ms | 0.073 ms |
| Bare nested correlated | 0.037 ms | 0.040 ms | 0.042 ms |
| Ambiguous correlated | 0.035 ms | 0.039 ms | 0.042 ms |

These measure the in-process resolver only, excluding helper IPC, editor debounce and popup rendering. No native input-to-paint claim. The actual LanguageSession subprocess regression independently checks whole-token acceptance for quoted bare correlated names and alias-qualified ambiguous replacements.

## PostgreSQL CSV import process memory and throughput — 2026-10-31

Added the opt-in release `postgres_import_streaming_benchmark`. It streams a UTF-8 CSV fixture to disk, captures it through the actual immutable preview source, imports into a temporary PostgreSQL table with bigint primary key/text value, then checks count, key sum and every value on the server. Four separate release-test processes were measured using macOS `/usr/bin/time -l`; compiler/Cargo memory is excluded. Each row-count/method combination below has one measured run, not a latency distribution. Server: local disposable PostgreSQL 17.11 container, TLS disabled, no SSH, no RLS policy on the benchmark table.

| Method | Rows | Source bytes | Snapshot/preview | Import/commit | Peak client process RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| COPY | 1,000 | 12,795 | 0.238 ms | 11.383 ms | 10,649,600 bytes |
| COPY | 1,000,000 | 18,777,801 | 4.657 ms | 570.486 ms | 11,091,968 bytes |
| INSERT | 1,000 | 12,795 | 0.179 ms | 18.413 ms | 10,780,672 bytes |
| INSERT | 1,000,000 | 18,777,801 | 4.741 ms | 3,735.344 ms | 10,878,976 bytes |

Peak memory includes fixture generation, connection, snapshot, parsing, import and verification. Server-process memory is excluded. Client RSS remains approximately flat across a 1,000-fold increase in row count for this two-column fixture; wide/maximum-size records and native UI memory remain unmeasured. All million rows were validated, not just the visible page. These local throughput results do not predict SSH/WAN performance, RLS policy cost or production-server timing.

Reproduce: build `mise exec rust@stable -- cargo test --locked --release postgres_import_streaming_benchmark --no-run`, then use its reported test executable with `RUSTABLE_POSTGRES_QA=1 RUSTABLE_IMPORT_BENCH_ROWS=1000000 RUSTABLE_IMPORT_BENCH_METHOD=copy /usr/bin/time -l <release-test-executable> postgres_import_streaming_benchmark --ignored --nocapture`. Repeat in separate processes with rows 1000 and method insert. The opted-in fixture must be ready on localhost54329. Temporary files/tables are cleaned up normally.

## Recently visited table pipelining — 2026-10-31

Extended the connection-local catalog hint from one table to the 16 most recently
used tables. Revisiting a retained table now pipelines fresh metadata and data,
with the same shape validation and sequential fallback. First visits, evicted
tables and explicit transactions still fetch metadata first. No result rows are
cached. Successful use moves the table to the end; oldest entries are evicted.
Import clears all hints. This supersedes the one-entry limitation above.

Re-ran `browse_wan_latency_comparison` against the disposable fixture through
15 ms delay each way, two warmups and seven measured samples (debug build):

| Path | p50 | p95 (maximum of 7) |
| --- | ---: | ---: |
| Fresh sequential catalog + data | 83.4 ms | 90.6 ms |
| Same-table refresh | 49.7 ms | 52.3 ms |
| Revisit after loading another table | 48.6 ms | 51.8 ms |

Full row equality is asserted for all paths. This is database-path timing, not
native input-to-paint or production-server measurement. Live browse tests also
cover 17-table eviction, returning to an evicted table, schema-change recovery,
explicit transactions and cancellation/connection reuse.

### Recursive completion resolver (2026-10-31)

Offline release helper, 100 tables/2,000 columns, 5 warmups then 50 samples:
recursive CTE p50 0.027 ms, p95 0.033 ms, max 0.035 ms. Qualified CTE p50
0.026 ms/p95 0.031 ms; bare correlated p50 0.026 ms/p95 0.032 ms; ambiguous
correlated p50 0.027 ms/p95 0.030 ms. Suggestions were checked per sample.
This excludes IPC, debounce, editor rendering and server query time. Reproduce
with the helper's ignored `scoped_completion_latency` release test.

## Indexed schema completion — 2026-10-31

Offline release benchmark on Apple M4 Max / arm64. Fixture: 5,000 tables,
20 columns each (100,000 columns), three joined catalog sources. Five warmups,
100 measured requests; each request asserts the expected column suggestion.

| Catalog lookup | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| Full table/column scans per binding | 0.251 ms | 0.316 ms | 0.352 ms |
| Prebuilt shared column index | 0.040 ms | 0.045 ms | 0.049 ms |

The helper builds qualified and unqualified lookup maps once per loaded catalog.
Both maps share column lists via Arc, and alias renaming copies on write.
Unqualified names shared by multiple schemas remain unresolved. Each indexed
list retains at most the existing 2,048-column scope limit; the input catalog
remains capped at 16 MiB. The index adds memory for names/maps; this pass did not
measure process RSS or end-to-end UI latency and does not establish those gates.

Reproduce with the helper release test `large_catalog_completion_latency` using
`--ignored --nocapture`. This benchmark excludes catalog construction and IPC.

### Completion helper protocol and memory follow-up

Apple M4 Max, arm64, macOS 27.2; release helper through a real stdio subprocess.
The generated 5,000-table/100,000-column catalog is 2,533,902 bytes. Initialize
response arrived 19.550 ms after process launch (warm filesystem caches, not a
cold startup measurement). Five warmups then 500 verified completion responses:
p50 0.102 ms, p95 0.119 ms, p99 0.133 ms. Timings include Python framing, pipes,
helper work and response decoding, but exclude editor debounce and rendering.

Sampled RSS after catalog load: 50,608 KiB (49.4 MiB). Ten cycles each opened
eight documents containing the query plus a 1,000,000-character comment, verified
completion on every document, rejected a ninth document, closed all eight, and
verified no closed document returned suggestions. RSS after open/completion and
after close was 61,760 KiB for the first nine cycles and 61,776 KiB for the last
(about 60.3 MiB). Retained allocator pages mean RSS need not drop on close; this
run showed only 16 KiB growth after the first cycle, not per-cycle document growth.
These are snapshots, not peak RSS measurements or proof against every leak.

Reproduce with `python3 tools/postgres-language-server/benchmark_protocol.py`.
Raw output: `docs/benchmarks/completion-protocol-m4-max.json`. The harness has
bounded response waits, verifies protocol results, shuts down the helper and
removes its temporary catalog. Whole-app/concurrent-session RSS, pathological SQL
parsing, and native input-to-paint measurements remain separate open gates.

### MySQL/MariaDB trigger metadata batching (2026-11-01)

Table browsing previously issued one SHOW CREATE TRIGGER command per trigger
while waiting to return the grid. Definitions now use multi-statement batches of
up to 64 on the worker's existing session. For N readable triggers this reduces
the definition command count from N to ceil(N/64); it does not remove the earlier
column, row or index/catalog requests. No second connection or implicit transaction
is introduced.

A privilege error stops a server's multi-statement batch. That chunk therefore
falls back to individual reads, preserving per-trigger metadata errors and the
existing behavior for restricted accounts. Other errors still fail the request.

Live checks on MySQL 8.4 and MariaDB 11.4 compare two batched definitions with
individual server definitions across SQL modes and quoted names. Limited-account
checks cover two hidden MariaDB triggers, subsequent query synchronization, then
readable definitions after granting the fixture account TRIGGER. All passed.
This establishes the functional path and reduced command structure, not a measured
WAN latency improvement; overall browse latency still needs remote-style timing.
