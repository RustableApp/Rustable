# Autocomplete review follow-up

The branch includes current main (`925795b`) and these local fixes:

- Slice the original SQL before Unicode case folding. Lowercase can expand byte
  length (`İ` becomes `i` plus a combining dot); using the folded prefix length
  to slice `SELECT é,İİ` previously panicked inside a UTF-8 character.
- Share the caret's clause tokens across context detection and inference. Skip
  clause analysis for qualified references, which already identify their source.
- Deduplicate column candidates before fuzzy ranking when multiple tables are
  involved. Repeated names no longer consume prefix reservation slots or cause
  the same column to be scored thousands of times. Table candidates retain their
  schema-qualification behavior; the existing fuzzy matcher is unchanged.

Completed clause keywords now remain quiet until a separator is typed; keyword
suggestions use prefix matching locally and for server keyword items. This prevents
`SELECT * FROM` from offering `DELETE FROM` while preserving fuzzy identifier
matching and passive table suggestions after the space. Regression coverage checks
this behavior across SQLite, PostgreSQL and MySQL.

Validation: 585 regular library tests passed (130 opt-in tests ignored), and strict
all-target Clippy passed. Regression tests cover Unicode insertion for SQLite,
PostgreSQL and MySQL, and prefix ordering with repeated catalog columns.

## Completion CPU measurements

Apple M4 Max, macOS 27.2 arm64, Rust 1.99.1, release profile. Five alternating
base/candidate pairs, each with 20 warmups and 100 measured refreshes per scenario.
Both binaries were built before measuring; no competing builds or benchmarks ran
during measurements. The same benchmark fixture was copied into a disposable
main worktree. Each catalog has 200 or 2,000 tables with 24 shared column names.
This intentionally exercises duplicate-name pressure, not every possible schema.

The measurements below precede the final keyword-filter adjustment.
Median per-run p95, milliseconds:

| Tables | Scenario | Main | Fixed PR | Change |
| --- | --- | ---: | ---: | ---: |
| 200 | Qualified columns | 0.0117 | 0.0125 | +7.5% |
| 200 | Table prefix | 0.0637 | 0.0614 | -3.6% |
| 2,000 | Qualified columns | 0.0246 | 0.0256 | +4.1% |
| 2,000 | Table prefix | 0.5447 | 0.5540 | +1.7% |

Existing measured paths stay below the 10% regression threshold. New unscoped
column inference is additional work absent on main: at 2,000 tables its typed
prefix p95 is 1.5243 ms, versus main's 0.9004 ms for table/keyword suggestions only.
The approximately 0.62 ms cost buys catalog-column suggestions before FROM.
The PR before deduplication measured 13.4955 ms on that same fixture. Empty-slot
inference measures 0.5662 ms; main simply returned without suggestions there.
These expanded-work cases are reported separately rather than claiming equivalent
results or silently relaxing a regression threshold.

No database requests were added. Measurements cover completion refresh CPU only,
not language-server latency, popup painting, input-to-display latency, or catalogs
whose columns are mostly unique. Raw samples and environment metadata are under
`target/pr3-completion-performance{,-optimized,-final}`.

Run the opt-in fixture locally:

```sh
mise exec -- cargo test --locked --release --lib completion_catalog_latency_benchmark -- --ignored --nocapture --test-threads=1
```

Use the identical fixture on both revisions and compare repeated paired runs.
The benchmark is local and adds no GitHub CI work.
