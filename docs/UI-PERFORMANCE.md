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

## Redis table polish and IntelliSense — 2026-11-07

Compared against `5311d86376eee8d77f1b2d65c88ac9a5f5b14456` on Apple M4 Max,
macOS 27.2 arm64, rustc 1.99.1 / LLVM 23.1.1, release profile. The shared Redis
fixture was compiled into both revisions. Five alternating pairs ran serially;
Redis cases use 20 warmups and 160 samples per run, 1200×760 logical viewport,
default egui scale (1×). The provider-only typing workload uses 50 warmups and
500 samples. Fixture source, source hashes, environment, all sorted samples and
pair-level results are retained in `target/redis-polish-performance-verified/`.

Reproduction: `python3 scripts/ui-performance.py --base 5311d86 --redis
--redis-pages --redis-editor --redis-completion --output <new-directory>` (one line).
Two exploratory runs were superseded. The final run rebuilt the frozen candidate
and reused the exact baseline release executable from the preceding runner build;
all timings were freshly measured. Its binary SHA and provenance are in
`environment.json`. No thresholds or fixtures were weakened. There were no
competing builds/benchmarks during measurement.

31 of 35 comparisons passed the unchanged 10% p95 gate. Four repeatable increases
are retained and explicitly justified below; this is not a blanket performance pass.
Values below are milliseconds; p50/p95/p99 are medians across five runs and maximum
is the largest observed sample.

| Scenario | Base p50/p95/p99/max | Candidate p50/p95/p99/max |
| --- | --- | --- |
| redis/page/analysis | 0.04850/0.05133/0.05679/0.06088 | 0.06521/0.06887/0.08650/0.09654 |
| redis/completion/commands | 0.00450/0.00567/0.00675/0.02571 | 0.01292/0.01462/0.01771/0.04767 |
| redis/completion/arguments | 0.00479/0.00504/0.00513/0.00713 | 0.01196/0.01225/0.01425/0.01575 |
| redis/completion/after-idle | 0.00475/0.00504/0.00600/0.00667 | 0.01342/0.01396/0.01717/0.01767 |

Analyze adds 0.0176 ms p95 (+34.2%) for structured table surfaces, headers,
aligned metrics and memory bars. Removing these would restore the unstyled report
that this change corrects. Rows remain virtualized and bounded; using a raw grid
or pre-rendered image would respectively lose the requested hierarchy or text
selection/hover/theme behavior. No database work was added to rendering.

Completion adds approximately 0.007–0.009 ms p95 (+143–177%) for the visible
suggestion menu, syntax/help and keyboard selection that the baseline lacks.
The same input workload runs on both; the visible output necessarily differs.
An external completion service would add latency/deployment cost, while a static
keyword list would lose argument context. The chosen local grammar is generated
once at build time, cached between edits and capped at 50 virtualized suggestions.
Ordinary 20/200-line editor workloads stayed within 3.6%; no idle polling or
per-keystroke database exchange was introduced (before/after: zero exchanges).
These are accepted costs for concrete added behavior, not threshold exemptions.

Candidate-only provider typing p50/p95/p99/max (ms): commands
0.00550/0.00621/0.00688/0.00788; options 0.00742/0.00763/0.00938/0.00971;
1,000 loaded keys 0.01108/0.01146/0.01412/0.01592. These are new-workload
measurements, not baseline passes. Binary-prefix lookup supports the full loaded
10,000-key cache without a full per-keystroke scan; correctness tests cover names
past the first 1,000 and escaped binary prefixes.

Native captures were reviewed in dark/light themes at 1200×760 logical pixels
(2× capture scale). The attempted 800-wide native capture retained the wider
window size, so it does **not** verify narrow native layout. The Mac was locked:
manual keyboard/mouse, mixed-DPI, resize/tiling, 60/120 Hz presentation and
input-to-display latency remain unverified. Automated egui input tests cover
completion acceptance/dismissal, execution shortcut ownership and copy feedback.
CPU measurements and screenshots do not close those native validation gaps.


## Final Redis UI follow-up: d9876f1 versus e3372d3

The initial standard five-pair comparison is retained in
`target/redis-value-final-performance` (command: `python3 scripts/ui-performance.py
--base e3372d361f20e6243d55bcf55585812de2f037e7 --redis --redis-pages --redis-editor
--redis-completion --output target/redis-value-final-performance`). It detected
four completion regressions and five inconclusive cases. Do not describe it as a pass.

Seven additional alternating serial pairs reused those exact compiled executables
and unchanged workloads through the script's `measure`/`compare` functions, with
no competing builds or benchmarks. Raw samples, executable paths/SHA-256/mtime,
original environment and repeat scope are in `target/redis-value-final-repeat`;
runner `/tmp/redis-repeat-measurements.py`. The removed baseline worktree meant both
executables ran from the root checkout for this repeat; these workloads use embedded
or in-memory fixtures, not files relative to cwd. Thresholds and fixtures were not
changed. All 36 non-completion-regression scenarios pass in the repeat; four
completion cases remain measured regressions, explicitly accepted below.

Machine: Apple M4 Max, macOS 27.2 arm64, Rust 1.99.1, release/thin LTO. Redis
fixtures use 1200×760 logical pixels at default egui scale, 20 warmups plus 160
samples per run; standard SQL grid/workspace/sidebar fixtures retain their original
sizes and sample settings. Completion alternates GE/GET for changed text, navigates
suggestions, and includes an after-idle time jump. Pub/Sub renders/scrolls 1,000
retained messages. These are CPU layout/tessellation measurements, not native
input-to-display latency, sustained FPS or a 120 Hz display test.

All values below are milliseconds; pairs are baseline→candidate. Maximum is the
median per-run maximum, followed by the worst single sample across the seven runs.

| Scenario | p50 | p95 | p99 | Median max | Worst max |
| --- | --- | --- | --- | --- | --- |
| redis/completion/commands | 0.012958→0.018125 | 0.014125→0.020125 | 0.022417→0.025208 | 0.032458→0.026292 | 0.034416→0.044833 |
| redis/completion/after-idle | 0.013416→0.018750 | 0.014375→0.020708 | 0.016208→0.022542 | 0.016917→0.024542 | 0.018583→0.049417 |
| redis/completion/capability | 0.012000→0.015833 | 0.012459→0.022459 | 0.014958→0.024750 | 0.019000→0.026583 | 0.023333→0.027541 |
| redis/completion/typing | 0.033666→0.044583 | 0.035167→0.048000 | 0.039041→0.052292 | 0.040250→0.054292 | 0.044667→0.071875 |
| redis/page/search | 0.116666→0.126417 | 0.124500→0.134584 | 0.127125→0.137875 | 0.130750→0.139459 | 0.147750→0.151667 |
| redis/pubsub-messages | 0.051833→0.055750 | 0.055125→0.060083 | 0.059250→0.063666 | 0.060500→0.065750 | 0.088125→0.091541 |
| redis/page/analysis-scroll | 0.107042→0.091666 | 0.115417→0.097250 | 0.117542→0.101625 | 0.119834→0.105208 | 0.146667→0.141334 |

Accepted completion cost: p95 increases 0.006–0.013 ms, with the tail measurements
shown above. The old prefix-only list excluded exact GET and did not offer inner/fuzzy
name matches. The requested shared ranking evaluates the bounded static catalog;
matching command/subcommand names are normalized once, suggestions remain capped at
50 and results recompute only when editor context changes. The initial uncached
implementation cost about 0.10 ms extra while typing; preparing names removed most
of that cost. Keeping the old exclusion/prefix-only scan would preserve speed but
fail the requested matching behavior. The popup now shows six complete rows instead
of clipping the last row, and fuzzy matching fills previously sparse lists; this
accounts for the small idle/navigation rendering increase. Avoiding those rows would
reduce visible suggestions. No extra database exchange, permanent repaint, unbounded
cache or whole-result cloning was added. This is an explicit feature/correctness
tradeoff, not a claim that the regressions passed the threshold.

Candidate-only new JSON-string editing workload (`redis_json_string_edit_frame_benchmark`)
uses a 100-item document, 700×760 viewport, idle and alternating edits, including an
after-idle clock jump. `/tmp/redis-json-string-performance.log`: idle p50/p95/p99/max
0.052916/0.059833/0.062208/0.062291 ms; editing
0.118125/0.135000/0.136875/0.137125 ms. First cold UI frame including font/layout setup
was 2.483750 ms; first editing frame 0.133708 ms. These absolute costs do not assert
parity against a nonexistent old pretty editor. Existing string-inspector comparison
passes (+1.6% p95). `/tmp/redis-completion-first-use.log` records first cold command
suggestion 0.333583 ms (including static-name preparation), warm matching p95 0.028333
ms; full completion UI costs are in the paired table above. Both manual workloads ran
serially using the exact candidate release test executable.

Native screenshots and local WASM editing/save/validation/copy checks passed as
recorded in the release checklist. High-refresh input-to-present, mixed-DPI,
multiple-monitor and native Windows/Linux/Intel Mac interactive checks remain
unverified; no claim about those is inferred from CPU timing or browser screenshots.

## macOS title-bar resize — 2026-11-12

Native diagnosis: intermediate `Resized` events arrive during title-bar fill/zoom,
and the layer-backed NSView already has `DuringViewResize` redraw policy. Its
default `ScaleAxesIndependently` placement stretches cached contents between
frames. Synchronous resize painting alone was tested in a fresh dev build and
did **not** fix the report. Adding `TopLeft` contents placement fixed the stretch
in the user's native preview test. The final patch uses the equivalent named
AppKit API once per OpenGL surface and retains synchronous resize painting.
It does not change database operations, table layout, DPI, or ordinary input
repaint scheduling. Temporary diagnostic logging was removed.

The disposable SQLite preview used the production table at 2× capture scale;
native settled screenshots were reviewed before and after resizing. The user
confirmed the animation no longer stretches. This is functional validation,
not measured input-to-present latency. `cargo build --locked --features qa` and
`cargo run --locked --example macos_gl_context_recovery` pass; the latter verifies
four painted frames after clearing the GL thread context.

Mixed-DPI movement, monitor reconnection, secondary-window animations, light
theme, 60/120 Hz presentation, missed-frame counts and idle-redraw counts remain
unverified. The CPU suite below cannot establish those properties.

Final comparison: `python3 scripts/ui-performance.py --base
44ad36b441684b87ec8e389d82c065e278f504c1 --output
target/macos-resize-final-performance` (one command). Apple M4 Max, macOS 27.2.1
arm64, Rust 1.99.1 / LLVM 23.1.1, release/thin LTO. Five alternating paired runs,
serial builds and measurements; unchanged default-scale fixtures: grid 1200×800,
20 warmups/160 samples; workspace/sidebar 1440×900, 20 warmups/100 samples.
Raw samples, p50/p95/p99/max, environment, build logs and all 19 comparisons are
retained in `target/macos-resize-final-performance/`.

18 metrics passed; precise vertical-scroll tessellation remains **inconclusive**
(exit 2). Its median p95 rose 3.35%, but paired ratios varied from 0.866 to 1.300,
with relative median absolute deviation above 10%. Only two of five pairs exceeded
10%; there is no repeatable above-threshold regression and no blanket pass.
No thresholds or fixtures were changed. The first run, retained separately in
`target/macos-resize-performance/`, predates the placement fix and had three
inconclusive metrics; it is superseded by this final-code run.

Representative timings below are median per-run values, in milliseconds; the
maximum column is the median of each run's maximum, not a pooled worst case.
Individual maxima and all sorted samples remain in `results.json`.

| Scenario | Base p50/p95/p99/max | Final p50/p95/p99/max |
| --- | --- | --- |
| Vertical precise layout | 0.234416/0.253708/0.293250/0.341542 | 0.239917/0.259042/0.329292/0.347083 |
| Vertical precise tessellation (inconclusive) | 0.042417/0.046042/0.052375/0.052542 | 0.042125/0.047583/0.051875/0.056667 |
| 120-column workspace with inspector | 0.405792/0.430875/0.450000/0.450000 | 0.408083/0.425500/0.440375/0.440375 |

These fixtures do not execute native resize callbacks. They protect existing CPU
layout/tessellation workloads; the user-confirmed native reproduction is the
functional evidence for the resize fix, with the native limitations listed above.
