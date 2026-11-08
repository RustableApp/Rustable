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

Redis work can add `--redis` (shared browser fixtures), `--redis-connections`
(connection form), and `--redis-tools` (diagnostics and topology). The latter two
require those benchmark functions to exist in the baseline revision. With all
three flags, the runner requires all 26 cases; it never silently skips a missing
workload. New module-specific workloads are measured separately when the baseline
has no equivalent interaction, and are not reported as comparative passes.

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

Redis command editor comparisons: `--redis-editor` adds four shared workloads
(20/200 command lines, unchanged/edited text) to `scripts/ui-performance.py`.
The exact `redis_browser_benchmark.rs` fixture is injected into both revisions.
Viewport is 1000×440 logical points, with 20 warmup and 160 measured frames.
Edited cases change one trailing character before each frame; these measure CPU
layout/tessellation after text changes, not native key-event or display latency.

### Redis full-page tools

Use `--redis-pages` with `--redis --redis-editor` to include Search (200 index
names), Analyze (50 report rows), and inactive Pub/Sub at 1200×760 logical points.
The exact shared fixture is copied to both builds. It opens the same existing
tools via their open flags; the baseline displays windows while the candidate
displays full pages. These are intentionally the before/after layouts, not
identical geometry. There are 20 warmup and 160 measured frames per run, including
a synthetic 30-second time jump after frame 100. That time jump does not measure
native wake latency or input-to-display latency. Charts add geometry in Analyze.
The command editor workload covers idle and changed drafts separately.

### Redis layout comparison — 2026-11-07

Command: `python3 scripts/ui-performance.py --base 39b4ad902c9c5f6e0ffb883ec161a3c0604034cc --redis --redis-editor --redis-pages --output target/redis-insight-layout-performance`. The unreferenced baseline snapshot preserves the other Redis roadmap work but predates the command editor and full-page layout. HEAD/index were not changed.

Apple M4 Max, macOS 27.2 arm64, rustc 1.99.1, release profile. Five alternating paired runs, serial builds and measurements, default egui scale. Editor: 1000×440; inspectors/tool pages: 1200×760. Each Redis case uses 20 warmup/160 measured frames. Raw samples, environment, shared fixture and logs are retained in `target/redis-insight-layout-performance/`. The earlier `target/redis-layout-ui-performance-final/` run predates the full-page redesign; it is not the final result.

The gate returned exit 1: five regressions and one inconclusive editor case. SQL grid/workspace, sidebar, Redis grid/chrome and string inspector passed. Analyze and Pub/Sub improved. These are CPU layout/tessellation measurements, not a claim about display latency or sustained FPS.

| Redis scenario | Base p50 / p95 / p99 ms | Candidate p50 / p95 / p99 ms | Base / candidate median maximum ms | Result |
| --- | --- | --- | --- | --- |
| redis/browser-chrome | 0.0481 / 0.0587 / 0.0617 | 0.0532 / 0.0593 / 0.0662 | 0.0653 / 0.0689 | pass |
| redis/command-editor/20/false | 0.0103 / 0.0115 / 0.0126 | 0.0238 / 0.0265 / 0.0275 | 0.0147 / 0.0282 | inconclusive |
| redis/command-editor/20/true | 0.0167 / 0.0173 / 0.0203 | 0.0331 / 0.0362 / 0.0388 | 0.0211 / 0.0408 | regression |
| redis/command-editor/200/false | 0.0122 / 0.0130 / 0.0144 | 0.0473 / 0.0520 / 0.0575 | 0.0151 / 0.0576 | regression |
| redis/command-editor/200/true | 0.0650 / 0.0728 / 0.0755 | 0.1316 / 0.1433 / 0.1985 | 0.0770 / 0.2456 | regression |
| redis/grid | 0.1184 / 0.1297 / 0.1400 | 0.1182 / 0.1313 / 0.1464 | 0.1538 / 0.1727 | pass |
| redis/hash-inspector | 0.0345 / 0.0403 / 0.0435 | 0.0522 / 0.0581 / 0.0755 | 0.0440 / 0.0836 | regression |
| redis/page/analysis | 0.0642 / 0.0701 / 0.0761 | 0.0512 / 0.0562 / 0.0657 | 0.0785 / 0.0658 | pass |
| redis/page/pubsub | 0.0437 / 0.0512 / 0.0553 | 0.0324 / 0.0356 / 0.0405 | 0.0590 / 0.0411 | pass |
| redis/page/search | 0.0469 / 0.0510 / 0.0586 | 0.0568 / 0.0622 / 0.0767 | 0.0611 / 0.0928 | regression |
| redis/string-inspector | 0.0449 / 0.0501 / 0.0549 | 0.0452 / 0.0513 / 0.0573 | 0.0557 / 0.0610 | pass |

The retained costs are deliberate layout/feature tradeoffs, not threshold exemptions:

- Command editor: changed 200-command drafts add 0.0705 ms p95 (0.0728 → 0.1433); idle 200-command drafts add 0.0390 ms. The user requested syntax colors, line numbers, focused execution shortcuts and shared SQL controls. Plain text avoids the additional colored text sections but removes the requested highlighting. The LayoutJob cache, 64 KiB highlighting fallback and 4,096-section cap remain; a custom persistent Galley cache was rejected because font-atlas/DPI changes can invalidate it. Both baseline and candidate use the same text fixture. The largest draft case has higher tails as well, shown above; no native input-latency claim is made.
- Hash inspector: 0.0403 → 0.0581 ms p95 (+0.0178 ms). The compact header and taller 22-point-row table expose more values in the same viewport. The previous fixed 300-point table and 28-point rows draw less content. Keeping that small table would underuse the wider detail workspace the user requested. Row virtualization remains; the cost is tied to visible rows, not all 200 fixture entries.
- Search: 0.0510 → 0.0622 ms p95 (+0.0112 ms). The former index selector starts collapsed; the new catalog renders a virtualized index table with per-row Query actions. A collapsed selector would avoid this work but lose the requested catalog layout. The same 200 index names are supplied to both implementations.

No thresholds or fixtures were weakened. The inconclusive 20-command idle case is not marked passed. Individual-run maxima and all sorted samples are in results.json. Native screenshots were reviewed at 1200×760 logical points / 2× capture scale. Mixed-DPI, monitor reconnection, 60/120 Hz input-to-present timing, missed frames, multiple-window motion and OS snapping were not instrumented or validated by this check.

No driver/protocol sequencing changed in this layout pass. Search first entry uses the existing index-list request; loaded catalogs (including empty ones) are reused on tab revisits, with an explicit Refresh action. Discovery pauses outside Browse. Chart data comes from existing bounded reports; no telemetry polling, preflight ping or additional metadata request was introduced. The 59 ignored Redis integration/performance tests were not claimed as passing; 131 selected Redis unit/protocol tests passed. Native Clippy passed with warnings denied; the wasm check passed with 61 existing warnings.

## Redis inspector/tool refinement comparison — 2026-11-07

Compared against the exact pre-refinement working tree snapshot
`0a41d949cd4561956c1c286d089e2011689e102d`, using
`--redis --redis-editor --redis-pages`. Raw results, environment, build logs and
all five alternating pairs are in `target/redis-refinement-performance/`.
Apple M4 Max, macOS 27.2 arm64, Rust 1.99.1, release profile; no concurrent builds
or benchmark workloads. Page fixtures use 1200×760 logical points, headless
scale 1, 20 warmups and 160 measured frames per run, including a synthetic idle
interval. The identical extended fixture is copied to both revisions. New cases
cover 100 retained Workbench commands and a 30-entry JSON draft (old modal versus
new inline editor). Command editor scenarios separately cover idle/editing.

Median per-run CPU layout+tessellation milliseconds (base → candidate):

| Case | p50 | p95 | p99 | Maximum per run |
| --- | --- | --- | --- | --- |
| Workbench with history | .0316 → .0453 | .0352 → .0503 | .0408 → .0582 | .0428 → .0870 |
| JSON draft | .0682 → .0594 | .0783 → .0646 | .0889 → .0735 | .0994 → .0922 |
| Pub/Sub empty page | .0328 → .0308 | .0364 → .0347 | .0442 → .0372 | .0656 → .0390 |
| Search index catalog | .0563 → .0565 | .0617 → .0606 | .0705 → .0729 | .0745 → .0755 |
| Analysis page | .0507 → .0493 | .0575 → .0564 | .0625 → .0616 | .0726 → .0898 |

Workbench history is a repeatable **43% p95 regression**, accepted for the
explicitly requested visible history sidebar: approximately .0151 ms additional
CPU work. The baseline retains the same 100 commands but only exposes a closed
menu. Hiding the sidebar would avoid the cost but remove the requested immediate
access; rendering all history rows would increase it. The implementation renders
only visible fixed-height rows and bounds each preview to 100 characters. It
performs no I/O, network calls or full-history filtering per frame. This is not a
performance pass merely because the absolute duration is small.

All other Redis scenarios passed except 200-command editing, which was
inconclusive (+1.8% median p95 with noisy runs). Four unrelated SQL/sidebar metrics
were also inconclusive; thresholds and fixtures were not weakened. See summary.md
for all 32 cases. Post-comparison changes only disable draft-overwriting actions
inside the closed Actions menu; benchmark rendering is unchanged.

Scope limits: the catalog case does not measure typing in the redesigned Search
editor, and the JSON case measures an idle draft, not typing latency. Pub/Sub's
empty-page comparison does not establish saturated subscription performance.
Native captures at 1200×760 logical / 2× scale verify layout, not FPS or
input-to-present latency. Mixed-DPI, multiple displays, monitor reconnection,
60/120 Hz presentation and native first-input-after-idle timing were not measured.
No database execution/protocol sequencing changed: before/after request counts
are identical; namespace aggregation uses existing MEMORY USAGE responses.
