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
