# UI responsiveness regression checks

Network round trips and local UI responsiveness are independent requirements.
The initial automated gate measures CPU layout/tessellation using existing
production rendering paths. It does **not** measure input-to-screen latency,
GPU execution, the native event loop, display refresh rate or compositor behavior.

## Run a comparison

```sh
python3 scripts/ui-performance-test.py
python3 scripts/ui-performance.py --base <base-commit> --output target/ui-performance-report
```

Use an idle machine, the same mise-selected toolchain and release builds. Do not run
other builds or benchmarks concurrently. The runner creates a temporary detached
worktree for the base, builds both versions in separate target directories to
avoid reusing stale vendored dependencies across revisions, then executes five
paired runs in alternating base/candidate order, on the same machine. All test
processes run serially. It does not access saved connections or user databases.
The candidate is the current working tree; its commit and dirty status are recorded.
Each invocation needs a new/empty output directory to preserve prior evidence.

Performance comparisons run locally to avoid consuming GitHub Actions minutes
on repeated release builds. Keep the raw report and summarize the relevant
before/after measurements in the PR. There is no automated GitHub performance
check or branch-protection requirement for this suite.

## Initial workloads

| Scenario | Fixture | Warmup / measured frames per run |
| --- | --- | --- |
| Grid scrolling | 500 rows × 120 columns, both axes, precise and wheel input, 1200×800 logical points | 20 / 160 |
| Workspace selection | 500 rows, 12/120 columns, inspector hidden/visible, 1440×900 | 20 / 100 |
| Sidebar | 20/200/2,000 objects, 1440×900 | 20 / 100 |

These headless egui fixtures use their default scale and a synthetic 120 Hz
input timeline where applicable. That does not simulate GPU/display presentation.
Scrolling reports layout, wheel-event layout and tessellation **separately**;
do not add their percentiles to claim a total-frame percentile. Workspace and
sidebar measure combined layout+tessellation. Wheel-event-only samples are fewer
than total frames for discrete mouse input. Small-sample p99/max values are
informational, not reliable estimates of rare real-world stalls.

The scenarios and sample counts are intentionally unchanged for comparison with
existing releases. Further coverage is needed for typing/completion, tab switches,
first use and interactions after idle. Add relevant workloads when changing those
paths. This suite alone cannot establish whole-application responsiveness.

## Results and policy

The report directory contains build logs, per-run output, `environment.json`,
`results.json` and `summary.md`. JSON includes raw sorted frame samples and
p50/p95/p99/max when the revision supports structured output. Older baselines have
rounded p50/p95 only; unavailable metrics are not fabricated. Percentiles preserve
the existing fixture convention: sorted sample at floor(N × percentile), capped
at the final sample. The comparison uses each run's p95, not a pooled percentile.

For each of 19 metrics:

- **Regression (exit 1):** candidate median p95 exceeds base by 10%, at least 80%
  of paired runs exceed 10%, and measurements are not noisy.
- **Inconclusive (exit 2):** median p95 exceeds 10% without sufficient repeatability,
  or the relative median absolute deviation of either side or the paired ratios
  exceeds 10%. Investigate/repeat under quieter conditions; do not count as pass.
- **Pass (exit 0):** neither condition applies.

Missing scenarios, invalid values, fewer than five pairs, failed builds or tests
fail the runner. Preserve the local report directory even on failure. The initial 10% threshold
is an investigation policy, not a scientifically calibrated universal limit.
Tiny rounded legacy measurements and shared-runner scheduling can cause noisy
results; inspect raw data and rerun rather than relaxing thresholds reflexively.
Do not change benchmark workload/coverage or sample counts in just one side and
present that as a performance comparison. New scenarios need a baseline measured
on the same workload; changes to the expected set are reviewed with the feature.

## Native follow-up

Keep a separate manual acceptance record for scrolling, selection, typing,
resizing, multiple windows, and moving/snapping across mixed-DPI monitors, including
reconnect and wake. At 60 Hz a frame interval is 16.67 ms; at 120 Hz it is 8.33 ms.
Those are total presentation intervals, not an allowance to spend all that time
on CPU layout. Prefer keeping existing timings over merely staying under a budget.

Native input-to-present timestamps, frame-interval distributions, missed-frame
counts and idle redraw counts are still instrumentation work to implement. The
current gate cannot detect every OpenGL/macOS window-manager problem. Do not use
it to declare the outstanding external-monitor issue fixed.
