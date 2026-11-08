# Rustable engineering guidance

## Network latency is a product requirement

Rustable must feel extremely fast, including over SSH and high-latency database
connections. Minimize sequential network round trips on every user-facing path:
query execution, table loading, refresh, paging, filtering, sorting, saving edits,
and loading catalogs. A fast localhost result is not sufficient evidence of good
performance. Each additional sequential exchange adds roughly one network RTT,
even when the database does almost no work.

### Preserve the round-trip budget

- Trace the actual driver/protocol path before changing database operations.
  Count sequential request/response dependencies, not SQL statements, API calls,
  TCP packets, or Rust `await`s. Concurrent requests may still be serialized by
  a driver. Include setup, preparation, description, metadata, cleanup, transaction
  status checks, and cancellation-worker teardown in the investigation.
- Prefer combining or pipelining independent work, reusing connections and
  prepared statements, and obtaining metadata/status from responses already
  being read. Do not add routine ping/health checks before each operation.
- Do not reintroduce separate PostgreSQL describe-before-execute or statement
  cleanup waits on ordinary editor and table-load paths. See `vendor/README.md`
  for the single-exchange APIs and their constraints.
- Current PostgreSQL targets on an established healthy connection, with built-in
  or cached result types and an unchanged schema, are one exchange per editor
  statement and one pipelined exchange for an initial sidebar table load,
  refresh, or revisit. Table browsing uses text results, so cold custom types
  must not add lookup exchanges. Unknown relations (not in the sidebar/cache),
  schema-change recovery, and explicit transactions retain a metadata-first
  fallback for safe query construction. Index/trigger definitions are fetched
  only when their views are opened. These are budgets to preserve or improve,
  not an excuse to add work up to the limit.
- Inspect MySQL/MariaDB and other affected drivers too. Do not assume an
  optimization or regression is PostgreSQL-specific without checking.

### Any increase requires strong justification

Treat an added sequential round trip as a performance regression until justified.
Before keeping it, document in the change description:

1. The before/after exchange count, including cold and warm paths.
2. The concrete correctness or safety requirement that makes it necessary.
3. Why batching, pipelining, existing response metadata, or validated caching
   cannot satisfy that requirement without the extra wait.
4. Measured latency impact under simulated network delay, and whether the cost
   can be limited to first use, schema changes, errors, or reconnection.

Implementation convenience alone is not sufficient justification. This is an
engineering requirement, not a new user-approval gate.

### Measure and protect the behavior

- For changes affecting database execution or driver/protocol sequencing, run
  the relevant latency regression tests in `src/db/postgres/latency_tests.rs`
  against the disposable fixture. Add or extend coverage for newly changed
  paths. Prefer protocol/exchange-count checks where practical; retain delayed
  network benchmarks to detect hidden waits outside the protocol itself.
- Compare equivalent work and results. A baseline called "single exchange"
  must actually use one exchange; a typed-query API may secretly describe first.
  Measure the complete operation, not just server-reported execution time.
- Cover first use and repeated use, and retain correctness checks for empty
  results, custom types, schema changes, cancellation, errors, and explicit
  transactions where applicable. Run benchmarks without competing benchmark
  workloads when comparing timings.
- Do not weaken performance thresholds merely to make a regression pass.
  Investigate and explain failures; distinguish unrelated existing failures
  from regressions introduced by the change. Do not claim unrun checks passed.
- Use disposable test databases, never a user's database, for latency experiments.
  Fixture setup is documented in `tools/database-fixtures/README.md`. PostgreSQL
  benchmarks require explicit `RUSTABLE_POSTGRES_QA=1` opt-in, for example:

  ```sh
  RUSTABLE_POSTGRES_QA=1 mise exec -- cargo test --lib db::postgres::latency_tests::editor_wan_latency_comparison -- --ignored --nocapture
  RUSTABLE_POSTGRES_QA=1 mise exec -- cargo test --lib db::postgres::latency_tests::browse_wan_latency_comparison -- --ignored --nocapture
  ```

### Correctness still comes first

Never reduce latency by bypassing safe mode, TLS/SSH identity verification,
transaction semantics, schema validation, or cancellation. Never automatically
replay a write whose commit outcome is unknown. Keep memory bounded. In
PostgreSQL, do not issue custom-type catalog lookups behind unread result rows:
that can deadlock the bounded response channel. Preserve the bounded-drain
fallback and the built-in-type restriction on pipelined metadata queries.

## UI responsiveness is a product requirement

Keep typing, completion, scrolling, selection, editing, tab switching, resizing,
and window management responsive. A feature is not complete if it makes those
interactions materially slower. Network latency and UI CPU cost are separate
budgets: improving one does not justify degrading the other.

- For changes on a rendering/input hot path, run the relevant release-mode UI
  benchmarks against the PR base on the same machine. Use
  `python3 scripts/ui-performance.py --base <base-commit>`; see
  `docs/UI-PERFORMANCE.md` for scope, thresholds, artifacts and limitations.
- Treat a repeatable p95 increase above 10% as a regression requiring investigation
  and explicit justification. Noisy or incomplete measurements are inconclusive,
  not a pass. Do not weaken thresholds or change fixtures to hide a slowdown.
  Compare identical work; if fixtures change, run the same revised workload on
  both implementations and record why. Historical timings from another machine
  or debug build are not an acceptable comparison baseline.
- Review p50, p95, p99 and maximum stalls where available. Record hardware, OS,
  toolchain, profile, sample counts, warmups, viewport, scale and fixture. Run
  benchmarks serially, without competing builds/benchmarks. Retain raw results.
- Preserve row/column virtualization, bounded allocations and cached text/layout.
  Avoid rescanning whole catalogs/results or cloning large buffers every frame.
  No database/network calls, blocking file I/O, process waits or long computations
  on the UI thread. Keep background result handling bounded too.
- Request redraws only while needed. Test that animations/scrolling settle at idle;
  do not solve responsiveness by permanently repainting or polling faster. Include
  first-use and first-interaction-after-idle behavior for affected paths.
- For newly introduced interactions, extend the benchmark suite or add a matching
  workload; passing unrelated existing scenarios is not evidence of performance.
- CPU layout/tessellation p95 is not input-to-display latency or sustained FPS.
  Changes to the event loop, renderer, scaling, resize or repaint scheduling also
  need native checks: 60/120 Hz where available, multiple windows, mixed-DPI
  displays, monitor disconnect/reconnect, moving and macOS snapping/tiling. Capture
  frame intervals, input-to-present timing and missed frames when instrumentation
  is available. State unavailable checks explicitly; never claim they passed.
- In PR descriptions, report before/after timings, measurement scope and untested
  cases. Any accepted regression needs a concrete user benefit/correctness reason,
  alternatives considered and the measured cost. Existing speed is worth preserving
  even when both versions fit within a display frame budget.

## UI consistency and completion

Follow [the UI style guide](docs/UI-STYLE-GUIDE.md) for all UI changes. Review the
whole affected view rather than only the requested control. Tables and lists must
have intentional styling, alignment, empty states and bounded rendering. Copy
controls must provide temporary in-place confirmation. Reuse shared controls and
SQL viewer conventions; inspect actual screenshots before calling visual work done.
