# Fresh editing metadata with fewer PostgreSQL exchanges

This records the metadata-only phase. The [transaction follow-up](EDIT-TRANSACTION-PERFORMANCE.md) further reduces the one-row plan-and-apply path from ten exchanges to six; the [server-side guard phase](EDIT-SERVER-CHECKS-PERFORMANCE.md) reduces it to three on first use and two on warm saves.

Save and explicit Preview still prepare a validated change plan on the database
worker. PostgreSQL now reads the complete editing catalog in one statement and
one snapshot. The planner uses the existing bounded single-exchange driver API,
with one bound text parameter and one built-in TEXT output. It does not rely on
sidebar metadata, a cached schema fingerprint, or cached custom result types.

## Safety and scope

The catalog retains canonical relation identity, column order/types/modifiers,
nullability/defaults, primary keys, generated/identity flags, collations,
constraints, trigger definitions/functions, indexes, cascade-trigger eligibility,
and recursively dependent type fingerprints. OIDs are explicitly converted to
JSON numbers and decoded losslessly. Regression tests compare the complete
fingerprint and editing columns against the previous implementation, including
enums, domains, generated columns, triggers, indexes and zero-column tables.

Apply still starts its transaction, locks each affected table in deterministic
order, re-reads the full catalog under that lock, compares the fingerprint, checks
original rows, executes parameterized writes, verifies affected-row counts and
commits. Schema/type changes, stale rows, errors and cancellation retain rollback.
The locked metadata recheck uses the generic transaction API; it has two exchanges
rather than introducing a new transaction driver API. No writes are replayed.

Metadata responses are capped on the server at the existing 16 MiB client budget;
oversized metadata returns an explicit error, never a partial fingerprint. Missing
tables, unsupported types and missing primary keys retain their rejection rules.
The planner drains ReadyForQuery before decoding/returning errors. Its result is
built-in TEXT even for enum/domain/array tables, so no custom-type lookup can occur
behind unread rows. Explicit-transaction refusal for Apply is unchanged.

Data and structure planners share the optimization; additional type-resolution
checks for structure drafts are unchanged. Multiple affected tables still require
one fresh planning request per table. No UI rendering/input paths change.
MySQL/MariaDB were inspected: they have a different catalog/snapshot sequence and
prepared-statement caching, so this PostgreSQL implementation is not applied to
those drivers. SQLite's local planning and the other drivers are unchanged.

## Exchange budget

For each affected PostgreSQL table, first and repeated use:

| Operation | Before | After |
| --- | ---: | ---: |
| Planning metadata | Four prepare/execute pairs: 8 | One Parse/Describe/Bind/Execute/Sync group: 1 |
| Metadata revalidation after table lock | Four prepare/execute pairs: 8 | One prepare/execute pair: 2 |

Preparation/description are included. The optimized planner uses an unnamed
statement, so it has no named-statement cleanup to await. Transaction boundaries,
table locks, per-row concurrency guards, writes and post-save reads are unchanged.
No additional ping, connection, authentication, or cancellation exchange is added.
The existing ordinary editor and browsing single-exchange paths are untouched.

## Verification

The opted-in fixture is a disposable PostgreSQL 17 container bound to loopback
54329 (`rustable-edit-planning-qa`), never a user database. The latency regression
uses the same worker driver, table, changes and results for both paths. A retained
`cfg(test)` implementation of the previous metadata reader supplies the baseline.
Six alternating pairs record first use separately and five repeated samples;
whole-plan JSON equality and actual committed values are checked for every pair.
The proxy adds 15 ms in each direction with overlapping in-flight packets.
Thresholds require at least 65% lower preview latency and 40% lower plan-and-apply
latency; existing editor/browse thresholds are unchanged. Timing includes the
cancel-watch lifecycle and driver response handling, not just server execution.

Raw results and validation logs are retained under `target/edit-planning/`.

### Measured results

macOS 27.2.1 ARM64 host, Rust 1.99.1, PostgreSQL 17.11 ARM64 disposable
fixture, optimized release profile. No competing benchmark/build workload.

| Driver operation, 30 ms simulated RTT | Previous | Optimized | Reduction |
| --- | ---: | ---: | ---: |
| First planning call | 303.55 ms | 52.98 ms | 82.5% |
| Repeated planning, median of five | 289.99 ms | 45.85 ms | 84.2% |
| First plan + apply | 890.78 ms | 389.10 ms | 56.3% |
| Repeated plan + apply, median of five | 868.90 ms | 379.15 ms | 56.4% |

The save measurement includes transaction commit but excludes confirmation time,
UI event dispatch/rendering and the subsequent result/table refresh. It measures
one row edit; large saves retain their existing per-row checks. These are results
on a delayed disposable connection, not measurements of a user's production DB.

Nine focused editing/structure tests passed (including seven live fixture tests),
as did all 941 active library tests (261 ignored). The existing editor and browse
WAN regression tests passed in release mode with their original thresholds. An
initial debug-mode editor comparison exceeded its unchanged 6,000-row spooling
threshold (72.0 ms vs 43.7 ms reference); that test does not call editing metadata.
The release run measured 49.1 ms vs 39.3 ms and passed. Both logs are retained;
no threshold was relaxed and no debug performance pass is claimed. Strict all-target
QA Clippy also passed.
