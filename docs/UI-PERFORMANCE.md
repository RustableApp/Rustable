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

Query editor layout changes can add `--query-editor`. The runner copies the same
`query_layout_qa.rs` fixture into the baseline and measures the production workspace
with 0/4 results at 1280×760, 128 results at 760×760, and 4 results at 500×760 logical
points. Each case uses 20 warmup and 160 measured frames, cycles the selected result,
and includes a synthetic 30-second idle time jump. These are CPU layout/tessellation
measurements at the default headless scale, not native input-to-present latency.
The QA build can also capture the native view with
`RUSTABLE_QUERY_CAPTURE=/absolute/path/capture.ppm`; optional `RUSTABLE_QUERY_LIGHT=1`
and `RUSTABLE_QUERY_WIDTH=760` select the theme and logical window width. This preview
uses disposable in-memory results and never opens a database connection.

### Query footer/result-tab verification (2026-11-12)

Compared against `268c4e674466e55a57c9bbdfb74a1c3a8c8420ef` on Apple M4 Max,
macOS 27.2.1 arm64, Rust 1.99.1, release profile. Five alternating paired runs,
using the query fixtures described above at default headless scale (1×).
All 23 metrics passed. Median per-run CPU milliseconds are below; each cell
is base → candidate. Maximum is the median of per-run maxima, not a pooled maximum.

| Results / width | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| 0 / 1280 | 0.02250 → 0.02237 | 0.02504 → 0.02325 | 0.02792 → 0.02750 | 0.02879 → 0.02912 |
| 4 / 1280 | 0.05233 → 0.05412 | 0.05546 → 0.05658 | 0.06088 → 0.06017 | 0.06187 → 0.06279 |
| 128 / 760 | 0.09450 → 0.05600 | 0.10008 → 0.06008 | 0.10754 → 0.06263 | 0.10788 → 0.06892 |
| 4 / 500 | 0.05212 → 0.05454 | 0.05392 → 0.05733 | 0.06083 → 0.06246 | 0.06183 → 0.06413 |

Raw samples, build logs and environment are retained locally in
`target/query-tabs-style-performance/` (final shared workspace-tab styling). Native dark 1280×760 and light 760×760
captures at 2× scale were visually inspected in `target/query-layout-review/`.
Layout tests cover both themes, 500/760/1280 widths, a four-digit gutter,
128 results, selection, revealing offscreen selections and stable pane height.
Native frame intervals, input-to-present timing, missed frames, 60/120 Hz,
multiple windows and mixed-DPI/monitor reconnect/snapping were not measured.

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

## SQL highlighting memory and CPU — 2026-11-12

The `--sql-highlight` runner option copies the identical fixture into the base
revision and measures uncached highlighting plus production editor layout and
headless tessellation. Fixtures are a 200-statement report (22,000 UTF-8 bytes)
and a statement containing 32,768 spaces. Layout uses 1200×800 logical pixels,
1× default scale, 20 warmup and 160 measured frames. Both unchanged text and
alternating text variants are covered, with a synthetic 30-second idle jump.
Alternating variants exercise changed-text cache lookup, not native key events;
uncached highlighting is measured separately. First-use font initialization is
in warmup, not included in the reported percentiles.

`HIGHLIGHT_MEMORY` records text capacity plus section-vector capacity times the
section size, for one retained highlighting job. This excludes allocator overhead,
font galleys, glyph atlases, other caches and whole-process RSS. The fixture and
all raw measurements are retained in the report directory. No SQL execution or
driver/protocol sequencing changes are involved.

Compared against `06088d111e10b95d96696e69b4dc403dc0ee4009` on Apple M4 Max,
macOS 27.2.1 arm64, Rust 1.99.1, release profile, five alternating serial pairs:

```sh
python3 scripts/ui-performance.py --base 06088d111e10b95d96696e69b4dc403dc0ee4009 --query-editor --sql-highlight --output target/sql-highlight-performance
```

All 29 comparisons passed the unchanged regression gate. Median per-run CPU
milliseconds are below; maximum is the worst single sample across all five runs.

| Workload | Base p50/p95/p99/max | Candidate p50/p95/p99/max |
| --- | --- | --- |
| sql/highlight/query | 0.137417/0.145875/0.149584/0.156833 | 0.087125/0.092166/0.096959/0.108208 |
| sql/layout/query/false | 0.178250/0.200708/0.209417/0.225125 | 0.085250/0.097000/0.105459/0.114625 |
| sql/layout/query/true | 0.608833/0.634208/0.643666/0.899250 | 0.356667/0.375292/0.381500/0.415458 |
| sql/highlight/whitespace | 0.519208/0.561833/0.587917/0.620375 | 0.116083/0.122708/0.125875/0.144667 |
| sql/layout/whitespace/false | 0.919750/0.954708/0.971500/1.151167 | 0.006625/0.007333/0.008500/0.028584 |
| sql/layout/whitespace/true | 4.594208/4.768417/4.855875/5.117083 | 0.889375/0.932583/0.948791/1.117750 |

Retained single-job storage: normal SQL 888,832 → 462,848 bytes (47.9% less;
5,800 → 2,400 sections), whitespace fixture 6,881,280 → 65,952 bytes (99.0%
less; 32,781 → 4 sections). Adjacent identically formatted spans are coalesced;
keyword recognition no longer allocates an uppercase identifier. No cache lifetime,
visual styling, repaint scheduling or database behavior changed. Exact glyph-mesh
and wrapping equality tests cover both themes and Unicode. Existing highlighting,
selection, incomplete-input and preview tests pass.

Raw results, fixture source and environment are in
`target/sql-highlight-performance/`. Native input-to-present, whole-process RSS,
idle wakeups and display/multi-monitor checks were not measured. These results
establish CPU and single-job allocation improvements, not whole-app memory savings.

The full-snapshot undo hotspot identified during this pass was addressed in the
following change. Its effects are not included in the highlighting-only numbers.

## Staged-edit undo history memory and CPU — 2026-11-12

History now keeps one current state and reversible replacement spans. Recording
an action compares the common prefix/suffix, clones only the changed span, and
moves replaced values into history. Undo/redo exchange owned spans, then return
an owned current-state copy as before. A no-op preserves redo; a new edit after
undo drops the abandoned future. Successful commit/discard releases history
storage. No history-depth limit or loss of undo steps is introduced. Arbitrary
large replacement spans can still require large storage; this is not a total
memory cap. Database execution, safety guards and protocol exchanges are unchanged.

The `--change-history` runner copies identical action and memory fixtures into
both revisions. CPU measurement stages 180 distinct cell changes containing
16 KiB text each, then undoes and redoes all 180. Each phase excludes its first
20 operations and measures 160; both versions return and verify the same owned
states. These are completed-action CPU times, not frame layout or physical input
timing. The fixture exercises accumulated changes to different rows, not every
possible bulk/reordering workload. Common-value comparisons still scan the
unchanged prefix/suffix; restoring an owned state still copies its values.

The separate memory workload stages 128 such changes and verifies one undo/redo
in a fresh process. `/usr/bin/time -l` measures whole-test-process peak RSS,
including the fixture, runtime and temporary returned states; it is not live-app
RSS or solely retained history. Both implementations retain all 128 undo steps.

Comparison: base `06088d111e10b95d96696e69b4dc403dc0ee4009`, Apple M4 Max,
macOS 27.2.1 arm64, Rust 1.99.1, release profile; five alternating serial pairs.
The candidate includes the preceding SQL highlighting change. All 32 comparisons
passed the unchanged regression gate, including the existing grid/workspace/sidebar,
query-editor and highlighting fixtures. The history-only timings below isolate
this second optimization from the highlighting work.

```sh
python3 scripts/ui-performance.py --base 06088d111e10b95d96696e69b4dc403dc0ee4009 --query-editor --sql-highlight --change-history --output target/change-history-performance
/usr/bin/time -l <release-test-executable> app::change_history::performance::change_history_memory_benchmark --exact --ignored --nocapture
```

CPU milliseconds: median per-run percentiles; maximum is the worst single sample
across all five runs. The changing state size is intentional and identical on both
versions; these are distributions across the same action sequence.

| Action | Base p50/p95/p99/max | Candidate p50/p95/p99/max |
| --- | --- | --- |
| history/record | 0.095625/0.164292/0.178000/0.193208 | 0.032750/0.057583/0.060167/0.066875 |
| history/undo | 0.027875/0.084000/0.096000/0.114208 | 0.018916/0.036458/0.039500/0.042583 |
| history/redo | 0.024416/0.041875/0.060458/0.085250 | 0.023791/0.041125/0.042167/0.044333 |

Peak RSS in the 128-edit test: median **144.45 → 15.41 MiB (89.3% lower)**
across five fresh processes per version, alternating order. Base range
144.45–144.47 MiB; candidate 15.36–15.45 MiB. Identical retained-value accounting
for this append workload is 8,256 large values in snapshots versus 128 in current
state plus reversible spans; no undo step is evicted. Current application edits
and temporary returned states are additional allocations on both versions.

Raw CPU samples, logs, environment and fixture are in
`target/change-history-performance/`. `memory-results.json` records every peak,
exact release executable paths and SHA-256 hashes; `*-memory-*.log` retain the
complete timing output. The memory runs used the same Cargo executables from the
comparison after the runner removed its temporary copies. No competing builds or
benchmarks ran during either measurement.

Correctness covers 2,000 deterministic mixed operations checked against snapshot
history, stable allocation ownership for unchanged values, releasing history on
clear, and the existing actual cell/bulk/insert/schema, failed-save, intermediate
save and keyboard-ownership tests. Native display/input timing and live-app memory
were not measured; there are no layout, event-loop or repaint changes.

## Search cancellation, shared SQL layout and native profiling (2026-11-12)

Result searches now cancel obsolete requests before starting their replacements.
Closing search, replacing/closing result tabs, reconnecting and dropping a search
signal cancellation; closing also releases match coordinates. Browser cancellation
terminates its worker. Empty searches do not spawn a worker. Native scans check
before compilation, each row, groups of 32 cells, large text cells and completion.
A single regex call is not preempted (the 16 MiB cell fixture takes about 1.5 ms).
Tests cover cancellation and source release, stale identities, match limits,
unicode/navigation and empty searches.

SQL syntax and wrapping now use frame-evicted immutable jobs. A narrow vendored
epaint API shares those jobs with its existing font/atlas/DPI-aware galley cache.
Resizing does not re-tokenize unchanged SQL. Tests cover shared ownership,
fractional width normalization, font/scale/theme/style changes and mesh equivalence.
See `vendor/README.md` for the fork's scope and provenance.

### Final release CPU comparison

Same-machine comparison against `06088d111e10b95d96696e69b4dc403dc0ee4009`,
Apple M4 Max, macOS 27.2.1, Rust 1.99.1, release profile, five alternating pairs.
All builds and benchmarks ran serially. Original 38 workloads and the 10% p95
threshold are unchanged. Final raw samples, exact binary hashes, fixture/build
provenance and reproducible rerun script: `target/performance-goal-chunked`.
The base binary was retained from `target/performance-goal-final`; the candidate
was rebuilt with the final chunked scanner. Unused experimental test code in the
base does not change the executed workloads. The standard rebuild command is:

```sh
python3 scripts/ui-performance.py --base 06088d111e10b95d96696e69b4dc403dc0ee4009 --query-editor --sql-highlight --change-history --result-search --output target/ui-performance
```

**37 comparisons passed; one is inconclusive, not a pass.** History undo's baseline
p95 was 0.11525/0.07617/0.06346/0.03613/0.07950 ms; candidate was
0.05763/0.03663/0.03742/0.03667/0.03621 ms. No pair exceeded +10%, but baseline
variance prevents a reliable quantified undo speedup in this run. Earlier retained
runs passed it. There are no repeatable regressions in the final comparison.

Selected medians across the five runs (milliseconds; p50 / p95 / p99 / maximum):

| Workload | Base | Candidate |
| --- | ---: | ---: |
| SQL cached layout | 0.1782 / 0.1983 / 0.2080 / 0.2130 | 0.0772 / 0.0887 / 0.0968 / 0.0981 |
| SQL editing | 0.6037 / 0.6401 / 0.7074 / 0.7127 | 0.3522 / 0.3718 / 0.3803 / 0.3964 |
| SQL resizing | 0.4741 / 0.5016 / 0.5224 / 0.5428 | 0.2632 / 0.2890 / 0.2950 / 0.2999 |
| Numeric result scan | 1.3029 / 1.3468 / 1.3551 / 1.4558 | 1.3183 / 1.3381 / 1.3589 / 1.4281 |
| Text result scan | 0.9103 / 0.9348 / 0.9473 / 0.9845 | 0.9224 / 0.9430 / 0.9530 / 0.9665 |
| Record staged change | 0.0910 / 0.1610 / 0.1677 / 0.1737 | 0.0357 / 0.0600 / 0.0651 / 0.0663 |
| Undo (inconclusive) | 0.0204 / 0.0762 / 0.0859 / 0.0913 | 0.0199 / 0.0367 / 0.0389 / 0.0424 |

Worst individual samples across all pairs: cached SQL 0.217→0.108 ms, editing
0.795→0.409 ms, resizing 0.773→0.317 ms, wide workspace 0.394→0.393 ms
without inspector and 0.455→0.448 ms with inspector. Numeric scan maximum was
1.648→1.797 ms despite unchanged p95; all raw tails are retained.

SQL/search fixtures use 20 warmups and 160 measured frames/scans per run;
SQL viewport is 1200×800 points at 1x, 200 repeated query lines and a separate
32 KiB whitespace fixture. Workspace selection uses 20 warmups, 100 measured
frames, 1440×900 points, 500 rows and 12/120 columns. All fixture definitions and
raw tails remain in the artifacts. The first query-layout frame was 1.015→0.756 ms;
first editing frame 0.677→0.424 ms; first resize frame 0.712→0.414 ms. After the
synthetic 30-second idle jump those were 0.178→0.077, 0.607→0.342 and
0.461→0.251 ms. These are CPU frames, not native wake-to-display latency.

The first combined run (`target/performance-goal-comparison`) exposed numeric-scan
overhead. Checking by groups instead of doing checkpoint bookkeeping per cell
resolved it: final numeric p95 is 1.347→1.338 ms; text scan 0.935→0.943 ms.
An experimental test-only close/render fixture caused repeatable +12–16% wide
workspace timings in `target/performance-goal-final`. Removing that fixture with
the rejected allocator experiment restored timings using the same baseline binary
(`target/workspace-performance-{investigation,without-close}`). This isolates a
compiler-output effect from unused test code; the exact compiler mechanism was
not established. Final wide-workspace p95 changes are −0.1%/+0.3%.
The intermediate rerun `target/performance-goal-verified` exposed remaining numeric
scan overhead; it is retained too. No thresholds were weakened or failing evidence
removed. Whole-process native results below do not imply those CPU-only timings
measure presentation latency.

### Native profiling method

The QA-only `RUSTABLE_NATIVE_PROFILE` fixture uses production SessionWindows,
tab closing and window archive paths with synthetic in-memory results. It reads
no saved connections/recovered documents and isolates preferences in a fresh output
directory. Large fixture construction and logging run off-thread. Background
telemetry requests do not repaint. Weak references track actual Grid release.

The macOS sampler uses libproc RUSAGE_INFO_V4 CPU ticks, interrupt/package-idle
wakeups, RSS and physical footprint. CPU uses `mach_timebase_info` (125/3 ns here),
validated against the OS process CPU clock. Early debug smoke results used an
uncalibrated conversion and unsuccessful optional child-window capture; those are
not performance evidence. Capture hooks were removed before release profiling.

```sh
python3 scripts/native-performance-compare.py --base 06088d111e10b95d96696e69b4dc403dc0ee4009 --output target/native-performance-comparison --runs 3 --seconds 8
```

This copies identical QA fixture adapters to the base, completes both release
builds before sampling, and records binary hashes, OS/hardware/toolchain, fixture
source and raw counters. Results are from three alternating pairs of fresh
processes. Startup means launch-to-first-update notification, not presentation.

### Native results and allocator investigation (2026-11-12)

Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release QA builds, 2x scale.
Root viewport 960×860 points; session windows 1280×820 points. The three-pair
medians below use two seconds settling and eight seconds sampling per phase.
All 48 phase samples reported zero wrapper UI updates during the settled interval;
all expected live-result counts passed (0, 1, 20, 1, 11, 1, 0, 0).
Interrupt wakeups remained approximately 0.37–1.12/s. CPU is percent of one core.
This fixture does not include connected database keepalives or user extensions.

| Phase | CPU % base → head | RSS MiB base → head | Footprint MiB base → head | Interrupt wakeups/s base → head |
| --- | ---: | ---: | ---: | ---: |
| launcher-idle | 0.034 → 0.033 | 138.766 → 139.047 | 141.907 → 141.845 | 1.117 → 1.117 |
| one-tab | 0.014 → 0.014 | 161.625 → 160.922 | 251.126 → 250.486 | 0.869 → 0.868 |
| tabs20 | 0.002 → 0.002 | 373.359 → 372.484 | 450.111 → 444.111 | 0.373 → 0.372 |
| close-tabs | 0.002 → 0.003 | 373.688 → 372.734 | 319.439 → 317.861 | 0.497 → 0.496 |
| three-windows | 0.008 → 0.007 | 377.703 → 375.406 | 454.720 → 456.642 | 0.869 → 0.745 |
| close-windows | 0.004 → 0.005 | 378.016 → 375.672 | 322.392 → 327.408 | 0.622 → 0.621 |
| close-last | 0.001 → 0.006 | 388.156 → 385.844 | 332.486 → 330.579 | 0.373 → 0.497 |
| close-all-windows | 0.008 → 0.005 | 388.125 → 385.797 | 187.611 → 195.251 | 0.620 → 0.620 |

Launch-to-first-update times were 424/135/134 ms for base and 410/130/132 ms
for candidate, in process order. These include first-run loader/cache effects;
OS caches were not flushed and these are not first-presentation measurements.
Whole-process memory changes are small and mixed: no broad RSS reduction is
claimed. Closing all results released every tracked Grid, while allocator and
native graphics allocations kept RSS above startup. Separate untimed vmmap
snapshots in `target/native-memory-diagnostics` identified approximately 213 MiB
resident in empty malloc small regions and about 63 MiB of IOSurface memory.

An experimental debounced background call to macOS's public
`malloc_zone_pressure_relief(NULL, 0)` returned zero bytes on all four closure
requests (1–12 microseconds per call) and did not reduce retained memory. It was
removed rather than adding an ineffective background worker. Raw debug efficacy
results are in `target/memory-reclaim-debug-profile`; their timings are not release
comparison evidence. No allocator replacement or unsafe buffer pooling is shipped.
The experimental close-results benchmark was removed with that implementation;
its raw results and identical fixture source remain in `target/performance-goal-final`.
Native profiling still covers production tab/window closure and result release.

Dark 1280-point and light 760-point native SQL previews were captured and visually
inspected (`target/performance-goal-visual/{dark,light}.png`): editor syntax, result
tabs, table rows and controls render correctly. Native frame intervals,
input-to-present timing, missed frames, 60/120 Hz, mixed-DPI/monitor reconnect and
macOS tiling were not measured; CPU results make no claims about those behaviors.
No database execution/protocol path changed, so network exchange budgets remain
unchanged and database latency fixtures were not rerun for this pass.


### Final checks

806 library tests passed; 211 opt-in tests were ignored. Strict
`cargo clippy --all-targets --features qa -- -D warnings` passed. Locked
browser/WASM compilation passed with the existing target-specific warnings.
Eight UI-runner tests and the native CPU-counter calibration test passed.
Logs: `target/performance-goal-final-{tests,clippy,wasm}.log`.
The native profile precedes the final scan-loop change; its fixture does not
perform result searches. Final scan and rendering measurements use the final code.


## Reused result-search formatting buffer — 2026-11-12

Search now formats scalar values into one reusable String per scan. Text remains
borrowed and NULL uses a static string; blobs format only their byte count.
This removes a temporary allocation/free for every numeric/blob cell and avoids
allocating NULL strings. Cancellation checks, match limits, Unicode matching and
displayed-value semantics are unchanged. Scratch storage depends on scalar
formatting length, never text/blob payload size. There is no persistent cache.

Same-machine incremental comparison uses the exact candidate binary from the
preceding pass as its baseline, preserved before editing. Apple M4 Max, macOS
27.2.1 arm64, Rust 1.99.1, release profile, five alternating serial pairs, unchanged
fixtures: 20 warmups and 160 measured scans each. Numeric fixture: 500 rows ×
120 integers, verifying all 60,000 coordinates. All four search comparisons pass.

| Numeric scan metric | Before (ms) | After (ms) |
| --- | ---: | ---: |
| Median p50 | 1.2929 | 0.8621 |
| Median p95 | 1.3249 | 0.9015 |
| Median p99 | 1.4090 | 0.9079 |
| Median per-run maximum | 1.4642 | 0.9226 |
| Worst individual sample | 1.5178 | 0.9515 |

Numeric p95 improves 32.0%; text/16 MiB-cell scans and navigation remain within
the existing regression gate. These are completed background scan CPU times,
not input-to-display latency or whole-app RSS. Raw samples, binary hashes and
reproducible script: `target/search-buffer-performance`.

The full 38-workload comparison against PR base `06088d111e10b95d96696e69b4dc403dc0ee4009`
also passes, including undo and wide-workspace scenarios that had been noisy or
sensitive to compiler output earlier. Raw samples and provenance:
`target/search-buffer-full-suite`. It reuses the previously built baseline and
runs the unchanged workloads on both implementations. Earlier inconclusive/failing
reports remain retained; their status is not retrospectively changed.

Correctness coverage compares reused formatting with `db::display` for signed
integer extremes, zero, negative zero, subnormal/extreme/nonfinite floats, empty
and large blobs, NULL and Unicode text. It checks buffer reuse, small retained
capacity after large values, and actual match coordinates. No DB protocol, UI
layout, repaint or event-loop path changed; network/native display tests were not
rerun for this search-only change.

Validation: 807 library tests passed (211 opt-in ignored), strict all-target QA
Clippy passed, and locked WASM compilation passed with existing warnings.
Logs are in `target/search-buffer-performance/{all-tests,clippy,wasm}.log`.


## macOS animated zoom and repeated click pairs — 2026-11-12

The final native preview was confirmed by the user: the center grid grows smoothly
through title-bar zoom/restore, and repeated double-click cell editing works without
a cooldown. The renderer flushes the implicit Core Animation transaction only after
a successful swap at a changed framebuffer size. No additional repaint requests,
permanent polling, database operations or ordinary-frame flushes were introduced.
The click helper consumes same-target pairs using the configured double-click delay,
avoiding egui's rolling triple-click classification. Regression tests cover repeated
pairs on the same/different cells, adjacent unpaired clicks, and timeout behavior.

Command: `python3 scripts/ui-performance.py --base 06088d1 --output
target/zoom-click-final-performance`, run in the isolated `rustable-titlebar-zoom`
worktree. Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile, five
alternating serial pairs without competing builds. Unchanged default-scale fixtures:
grid 1200×800, 20 warmup/160 measured frames; workspace/sidebar 1440×900,
20 warmup/100 measured frames. All 19 comparisons pass. Raw samples, build logs,
environment and full comparisons remain in that worktree's report directory.

Representative median per-run timings below are milliseconds, ordered
p50/p95/p99/max (maximum is the median of run maxima).

| Scenario | Base | Candidate |
| --- | --- | --- |
| grid/vertical precise/layout | 0.235584/0.251459/0.297500/0.348708 | 0.243084/0.274834/0.310541/0.367292 |
| grid/vertical precise/tessellation | 0.041667/0.047583/0.052167/0.052708 | 0.042250/0.048875/0.052125/0.062667 |
| workspace/120/true | 0.407042/0.433375/0.448500/0.448500 | 0.407875/0.432750/0.458541/0.458541 |

These CPU workloads do not measure native presentation or the timing of click
activation. The native reproduction and click-sequence tests supply functional
evidence. Native frame intervals, missed frames, input-to-present latency, idle
redraw counts, 60/120 Hz comparisons, secondary windows, mixed-DPI movement,
monitor reconnect and snapping remain unmeasured. Full isolated library tests:
797 passed, 207 ignored; strict all-target QA Clippy and WASM browser check passed.


## v0.4.25 combined release validation — 2026-11-12

The combined performance, resize-presentation and click-pair source passed 808
library tests, strict all-target QA Clippy and the separate locked browser/WASM
check. Release-profile comparisons used the unchanged 38 workloads and thresholds
against `06088d1`, on the same M4 Max/macOS 27.2.1/Rust 1.99.1 machine.
The first five-pair run had 25 passes/13 inconclusive metrics; a second five-pair
run of the exact same binaries had 24 passes/14 inconclusive metrics. Neither
reported a repeatable regression. These are inconclusive comparisons, not blanket
passes. A process snapshot after the second run showed substantial WindowServer,
video-decoder and streaming-app activity; `pmset` reported no thermal warning.
Background contention is a plausible contributor, not a proven explanation for
every noisy metric. No user apps were stopped and no thresholds were changed.

Raw samples, hashes, environment and rerun script are retained under
`target/release-v0.4.25/{performance,performance-repeat}`. Earlier 38-case search/
performance and isolated 19-case resize/click comparisons passed as documented
above; they do not turn the combined noisy runs into passes. The user-confirmed
native resize behavior and unavailable native presentation/display checks remain
as stated in the preceding section. No database sequencing changed.

### PostgreSQL TLS connection form

Use `python3 scripts/ui-performance.py --base <commit> --postgres-connections`
for the PostgreSQL connection editor. This focused mode measures the same default
profile, host changes, wheel input and a 30-second idle gap at 1100×760 and 780×600
logical points on both revisions. It records 160 samples after 20 warmups per
case, in five alternating base/candidate pairs. The candidate adds the TLS mode
explanation and five-option selector; the baseline uses its original two-option
selector. The workload measures layout and tessellation, not native presentation.
The companion `postgres_tls_menu_frame_benchmark` opens the actual picker and
measures its expanded rows at both sizes, including an idle gap. Opening/selection
is also covered by an input-driven UI test and native screenshots; the benchmark
does not claim to measure all five selection paths.

2026-11-13 issue #8 validation against `ee5d224579d77456fdd942df00970216471f7327`:
Apple M4 Max, macOS 27.2.1 arm64, rustc 1.99.1, release profile, 1× headless
scale. Five serial alternating pairs, 20 warmups + 160 samples per size per run.
Median run statistics in milliseconds (base → candidate):

| Viewport | p50 | p95 | p99 | Maximum | p95 change |
| --- | --- | --- | --- | --- | --- |
| 1100×760 | .042667 → .043417 | .045417 → .046292 | .049958 → .050250 | .051750 → .050833 | +1.9% |
| 780×600 | .042666 → .043167 | .045667 → .046084 | .051000 → .048834 | .051959 → .051958 | +0.9% |

Both cases passed. Raw runs, environment and fixture are retained in
`target/issue-8-ui-performance/results.json`. Native dark/light screenshots were
inspected at wide/narrow sizes, and the picker click test selects Require and
checks its explanatory text. CPU numbers do not establish input-to-present
latency. Native 60/120 Hz timing, mixed DPI, monitor reconnect and multi-window
motion were not measured; event-loop/render scheduling was not changed.

Database execution exchanges are unchanged: one per ordinary editor statement,
one pipelined exchange for cached/sidebar initial load, refresh and revisit,
including the existing cold custom-type text path. Metadata fallbacks, transaction
status, bounded draining and cancellation-worker teardown are unchanged. The
existing connector is reused for cancellation. MySQL/MariaDB driver TLS and
execution paths are unchanged. On connection setup, enabled PostgreSQL modes
use the existing one SSLRequest exchange plus TLS/authentication; Disable skips
SSLRequest. Correctly importing TablePlus Prefer (0) now negotiates TLS instead
of incorrectly disabling it. That first-connection negotiation is necessary to
honor the source encryption policy and cannot be replaced by cached server
capabilities; it adds no established-session query exchanges.

At 30 ms simulated RTT on the disposable PostgreSQL 17 fixture, serial release
editor comparisons passed on both revisions: indexed-query p50/p95 43.5/46.4 →
39.2/46.0 ms; 6,000-row disk-backed results 49.5/55.7 → 47.2/53.8 ms. Browse debug
regression passed: refresh p50/p95 43.8/49.8 ms, revisit 44.1/49.5 ms, initial load
48.0/49.6 ms; idle cases also passed. An initial debug editor run failed the
large-result threshold marginally (spooled median 70.2 ms versus reference
45.2 ms plus the 25 ms allowance). Its serial debug rerun passed (71.4 versus
48.4 ms). This marginal result is retained as timing variability, not erased or
claimed to prove a pre-existing baseline failure. Thresholds were unchanged.
Logs are retained under `target/issue-8-*`.

#### Muted descriptions in the expanded TLS picker

The follow-up keeps each option and explanation in one selectable row, uses the
small font and semantic muted color, bounds text layout to the popup width, and
retains a contrasting selected state. Dark 1280×760 and light 780×760 native
captures were inspected; the picker input test and Clippy passed.

The same machine/toolchain/release workload above was rerun with the actual open
menu (20 warmups, 160 samples, five alternating pairs, 1× CPU scale). The base
has two one-line options; the candidate has five options with explanations.
Initial results are in `target/issue-8-ui-descriptions`; a serial repeat of the
same binaries without rebuilding is in `target/issue-8-ui-descriptions-repeat`.
The initial run classified the narrow menu as a regression and the other cases
as inconclusive. The repeat passed both closed-form cases; both menu cases were
inconclusive. No threshold was changed. Median run statistics (milliseconds):

| Scenario | p50 base → candidate | p95 | p99 | Maximum | Repeat status |
| --- | --- | --- | --- | --- | --- |
| postgres/connection-profile/1100x760 | 0.047000 → 0.050208 | 0.049542 → 0.050875 | 0.051959 → 0.055167 | 0.053834 → 0.057959 | pass |
| postgres/connection-profile/780x600 | 0.049583 → 0.051792 | 0.052417 → 0.052958 | 0.056875 → 0.056042 | 0.057958 → 0.056792 | pass |
| postgres/tls-menu/1100x760 | 0.052584 → 0.056208 | 0.053250 → 0.059500 | 0.057417 → 0.065625 | 0.058834 → 0.065958 | inconclusive |
| postgres/tls-menu/780x600 | 0.052833 → 0.058209 | 0.055750 → 0.061584 | 0.059708 → 0.067750 | 0.062958 → 0.077958 | inconclusive |

The expanded-menu p95 cost is consistently about 0.006 ms (roughly 10–12%)
in the repeat. This is an accepted, explicitly recorded cost of presenting the
five security policies and their differences before selection, not a performance
pass. The layout is bounded to five entries and egui caches shaped text. Keeping
only the selected-mode explanation or putting descriptions in hover tooltips
would reduce drawn text but would not provide the requested at-a-glance comparison.
The cost is limited to the open popup; the repeated closed-form comparison passed.
Native input-to-present latency and other display checks remain unmeasured.

### Empty rows-view double-click insertion — 2026-11-15

Double-clicking unused table-body space stages a row through the existing Add Row
path. Cell/header interactions, safe mode, loading and query-result guards remain
intact. Hit testing uses existing viewport/row geometry in constant time; there
are no added database operations or repaint timers. PostgreSQL, MySQL/MariaDB and
other driver sequencing is unchanged, so database latency tests were not rerun.

Comparison: `python3 scripts/ui-performance.py --base cad59dcd6ce9b8016795cc92a2d927d8e9851734 --output target/empty-row-ui-performance`.
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile, five serial
alternating pairs; all 19 comparisons passed. Unchanged fixtures and default 1×
scale: grid 1200×800 (20 warmup/160 samples); workspace/sidebar 1440×900
(20 warmup/100 samples). Median per-run CPU milliseconds, p50/p95/p99/max:

| Workload | Base | Candidate |
| --- | --- | --- |
| Vertical precise grid layout | .23625/.25246/.29996/.33821 | .24350/.26942/.32579/.35642 |
| 120-column workspace with inspector | .40433/.42417/.45338/.45338 | .40683/.43921/.47267/.47267 |

The new `empty_rows_double_click_frame_benchmark` also passed five serial release
runs: 600×400, default 1× scale, 20 warmup/160 measured click pairs, repeated
insertion and a synthetic 30-second idle gap. Median p50/p95/p99/max was
.01054/.01250/.01350/.01367 ms for activation and .01558/.01813/.02075/.02188 ms
for the first editable-row frame. These are candidate-only layout+tessellation
costs; the base has no matching insertion gesture. Raw measurements are retained
in `target/empty-row-review/interaction-{0..4}.log`.

All eight insertion tests passed, including input-driven checks of empty/populated
tables, blank space to the right, cells, headers, safe mode, loading and query
results. Native dark 1280×760 and light 760×760 screenshots at 2× were inspected,
including the focused staged row. The disposable preview adapter, build log and
captures are retained under `target/empty-row-review/`; no preview changes remain
in production source. Native input-to-present timing, frame intervals, 60/120 Hz,
multiple windows and mixed-DPI behavior were not measured. Event-loop and renderer
scheduling are unchanged; CPU results do not establish presentation latency.

### Cell Undo and discard-on-close confirmation — 2026-11-15

Cell editors now keep typing undo/redo locally, then hand exhausted Undo to the
existing staged-change history. An untouched new row can be undone immediately.
Each editing session initializes its own typing history; refocusing after a
validation failure retains it. Undo availability is checked only for the shortcut,
using a vendored accessor that locks the undoer without cloning its history.
SQL and unrelated text fields retain their independent Undo behavior.

Window close, database-workspace close, quit and dirty table-tab close offer
Cancel / Discard. Table discard affects only that table. Session close rechecks
running work and transactions across the affected workspaces before discarding;
it does not commit, roll back, cancel or replay database operations. SQL drafts
remain recoverable. No driver/protocol sequencing changes or additional network
exchanges were introduced; database latency fixtures were not rerun.

`python3 scripts/ui-performance.py --base cad59dcd6ce9b8016795cc92a2d927d8e9851734 --change-history --output target/undo-close-ui-performance`
passed all 22 comparisons, including the preceding empty-space insertion change.
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile, five alternating
serial pairs. The unchanged grid/workspace/sidebar fixtures and 1× scale are
listed above; history uses 20 warmups/160 samples with 16 KiB cell values.
Median per-run CPU milliseconds (p50/p95/p99/max):

| Workload | Base | Candidate |
| --- | --- | --- |
| Staged Undo | .02021/.03767/.03996/.04100 | .02025/.03842/.04058/.04333 |
| 120-column workspace with inspector | .39850/.40163/.41450/.41450 | .39854/.40254/.41292/.41292 |

New candidate-only interaction workloads also passed five serial release runs.
The Undo handoff measures a production inline editor and shortcut handler through
180 insert/type/undo sequences (20 warmups/160 measured handoffs), 600×400 at 1×,
including a synthetic idle gap. Modal rendering uses 760×600 at 1× with
20 warmups/160 frames and an idle gap, for enabled and blocked Discard states.
These include layout+tessellation; they are not comparisons with the former error
or measurements of native input-to-present latency.

| New workload | p50 / p95 / p99 / max ms |
| --- | --- |
| Typing-to-staged Undo handoff | .00221/.00238/.00275/.00279 |
| Discard confirmation | .00721/.00775/.00883/.00954 |
| Running-work confirmation | .00700/.00725/.00833/.00883 |

838 library tests passed (221 opt-in tests ignored). Coverage includes typing
undo/redo, history handoff, immediate new-row Undo, explicit Cancel/Discard button
input, Escape cancellation, no implicit Enter confirmation, scoped table discard,
and rechecking busy parked workspaces. Native dark 1280×760 and light 760×760
captures at 2× were inspected, including the disabled Discard state. Fixtures,
raw interaction samples, tests and captures are in `target/undo-close-review/`;
comparison logs and raw tails are in `target/undo-close-ui-performance/`.
Native multi-window close/quit interactions, frame intervals, input-to-present
latency, 60/120 Hz and mixed-DPI checks were not measured. The existing rendering
and repaint scheduling remain unchanged; these results make no display-latency
claim. The screenshot adapter was restored after building the disposable preview.

Strict all-target QA Clippy passed. The locked browser/WASM check also passed
(with target-specific warnings); logs are retained in the same review directory.

### Neutral background while editing a selected cell — 2026-11-16

The grid restores the active editor's complete cell rectangle to its ordinary
alternating-row color, including gutters and the calendar-button area. Other
selected cells stay blue. Pending insert fields use the same treatment while
editing; their pending fill returns afterward. This adds a rectangle for an
edited cell, with no extra layout, repaint timer, row scan or database operation.

`python3 scripts/ui-performance.py --base cad59dcd6ce9b8016795cc92a2d927d8e9851734 --cell-editor --output target/cell-contrast-ui-performance`
passed all 21 comparisons. The new flag injects the identical focused-editor
fixture into both revisions. Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1,
release profile, five alternating serial pairs, default 1× scale. The added
fixture uses two identical rows, two columns and a selected row with inline
editing at 800×500, 20 warmup/160 measured frames and a synthetic 30-second idle
gap. Existing grid/workspace/sidebar fixtures are unchanged. Median per-run
layout+tessellation milliseconds (p50/p95/p99/max):

| Editor theme | Base | Candidate |
| --- | --- | --- |
| Light | .01325/.01363/.01383/.01496 | .01308/.01358/.01375/.01379 |
| Dark | .01338/.01379/.01633/.01683 | .01350/.01379/.01575/.01671 |

839 library tests passed (222 ignored), as did strict all-target QA Clippy and
all eight performance-runner tests. The geometry test covers both row colors
and both themes, verifies paint order and full-cell height, and checks that
neighboring selection remains visible. Native 2× captures of selected text in
a real grid editor were inspected in dark 900×500 and light 760×500 layouts.
Screenshots, temporary preview adapter and check logs are retained under
`target/cell-contrast-review/`; raw timing samples and environment are under
`target/cell-contrast-ui-performance/`. The preview source was restored after
capture. CPU timings do not measure input-to-display latency; native frame
intervals, 60/120 Hz, multiple windows and mixed-DPI behavior were not measured.
Database/protocol paths and renderer/event-loop scheduling are unchanged.

### Cell Undo continuation and staged-cell reediting — 2026-11-16

Cell typing history now starts with a caret at character zero. The handoff checks
text changes independently of cursor-only undo points, then reopens the affected
visible cell as staged Undo/Redo traverses earlier changes. Fresh typing retains
ownership of Redo. Each reopened editor resets its cursor/scroll/IME state, and
staging requests one redraw even when another input took focus, retiring the old
editor's hit-test region. History inspection/navigation runs only on explicit
shortcuts; no network operations, periodic repaint timers or full-result rendering
were added.

`python3 scripts/ui-performance.py --base a427eef --cell-editor --change-history --output target/cell-undo-reedit-performance`
passed all 24 comparisons. Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1,
release profile, five alternating serial pairs, default 1× scale. Existing
workloads and fixtures are unchanged. Selected-editor workloads use 800×500,
20 warmup/160 measured frames, both themes and a synthetic idle gap. Median
per-run CPU milliseconds (p50/p95/p99/max):

| Workload | Base | Candidate |
| --- | --- | --- |
| Selected editor, light | .01342/.01383/.01579/.01675 | .01329/.01358/.01525/.01567 |
| Selected editor, dark | .01342/.01388/.01617/.01708 | .01354/.01488/.01642/.01704 |
| Staged Undo | .01908/.03612/.03804/.03812 | .01942/.03617/.03783/.03963 |

A new candidate-only populated-cell navigation workload passed five serial runs.
Each iteration stages edits in two populated rows, opens a cell, types and undoes
that typing, then measures two Undo/navigation frames including tessellation.
It asserts that staged changes are cleared and the earlier row's editor is open.
Viewport 800×500 at 1×, 20 warmups/160 samples, including a synthetic 30-second
idle gap. Median per-run p50/p95/p99/max: .03117/.03233/.03558/.03621 ms. This is
an absolute interaction measurement, not a comparison with the former behavior.

841 library tests passed (223 opt-in tests ignored), along with strict all-target
QA Clippy and locked browser compilation (target-specific warnings remain).
Regression tests use real grid pointer/key events to reopen and modify a staged
cell, traverse populated cells with Undo/Redo, check caret zero, preserve fresh
typing on Redo, and retain unrelated text input history. Native 2× screenshots
were inspected in dark 900×500 and light 760×500 layouts. Temporary capture source
was restored. Checks, interaction samples and captures are retained under
`target/cell-undo-reedit-review/`; comparison samples/environment are retained
under `target/cell-undo-reedit-performance/`.

These CPU results do not measure input-to-present latency. Native 60/120 Hz,
multiple windows, mixed DPI and monitor transitions were not measured. Database
execution and renderer/event-loop scheduling are unchanged.

### Untouched editors and navigation discard prompts — 2026-11-16

Refresh, table/query navigation and table-view changes distinguish an open editor
from a changed value. Untouched text/numeric/NULL drafts and unchanged inline
renames do not block navigation or close checks. Actual changes use the shared
Cancel/Discard modal; invalid drafts need not parse before they can be discarded.
Cancel retains edits, and Discard resumes the stored action. Table-scoped actions
preserve other tables' staged changes. Connection replacement still checks open
transactions, and confirmation rechecks running work before discarding.

The dirty check avoids formatting/cloning text, and scoped history scans run on
navigation requests. No new timer, background polling, database operation or
protocol sequencing was added: accepted actions dispatch their existing operation
once. Database latency fixtures were not rerun for these UI-only guards.

`python3 scripts/ui-performance.py --base a427eef --cell-editor --change-history --output target/navigation-discard-performance`
passed all 24 comparisons. This includes the preceding local cell Undo/reediting
fixes. Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile, five
alternating serial pairs, default 1× scale and unchanged shared fixtures. Cell
editors use 800×500 with 20 warmup/160 measured frames and a synthetic idle gap.
Median per-run CPU milliseconds (p50/p95/p99/max):

| Workload | Base | Candidate |
| --- | --- | --- |
| Selected editor, light | .01342/.01371/.01504/.01625 | .01350/.01379/.01492/.01633 |
| Selected editor, dark | .01350/.01508/.01658/.01679 | .01346/.01538/.01642/.01717 |
| 120-column workspace, inspector open | .40563/.40971/.42183/.42183 | .40212/.40554/.43767/.43767 |

The new navigation-discard modal workload passed five serial release runs at
760×600, 20 warmup/160 measured frames, with a synthetic 30-second idle gap.
Candidate-only layout+tessellation p50/p95/p99/max: .00817/.00867/.00979/.01063 ms.
It confirms that rendering leaves the draft and pending action intact and does
not dispatch database work. This is an absolute measurement, not a comparison
against the old error banner.

847 library tests passed (224 opt-in tests ignored). Strict all-target QA Clippy
and locked browser compilation passed (existing target-specific warnings remain).
Tests cover grid/inspector untouched editors, Cancel/Discard, invalid values,
staged changes, numeric equivalence, empty text versus NULL, connection switching,
busy-work rechecks and preservation of unrelated table edits. Native dark 900×500
and light 760×500 dialog screenshots at 2× were inspected. The temporary capture
adapter was restored. Logs/captures/interaction samples are retained under
`target/navigation-discard-review/`; comparison raw samples and environment are
under `target/navigation-discard-performance/`.

CPU timings do not establish input-to-present latency. Native 60/120 Hz,
multi-window, mixed-DPI and monitor-transition acceptance remain unmeasured.

Compact-dialog follow-up: reduced the content width from 360 to 244 logical
points, used rounded framing/buttons and shortened the warning/save hint. Removed
the redundant SQL-draft paragraph. Dark/light native captures were inspected in
`target/compact-discard-review/`; existing Cancel/Discard and Escape/Enter tests
passed. This static dialog styling does not change ordinary rendering/input paths;
the timings above precede this copy/layout adjustment and were not rerun.

### Blank-area deselection and table Select All — 2026-11-16

Blank table-body regions now have two bounded hit targets inside the scroll
container. The former outer background response could lose these clicks after
scrolling. A primary blank click finishes a valid edit, clears row/cell and retained
result selection, and gives keyboard focus to the grid. Invalid edits retain their
validation path. Double-click-to-insert remains available. The main table also
accepts Cmd/Ctrl+A when no widget owns focus; focused text fields, editors and
modal controls retain their shortcuts. Selection remains limited to loaded table
rows; disk-backed query results retain their existing logical Select All behavior.
No database or protocol operations changed, and no polling timer was added.

`python3 scripts/ui-performance.py --base deeed9b1498377412b169ccaae521fdc6dd976b8 --grid-selection --output target/grid-selection-performance`
compared five alternating serial pairs on Apple M4 Max, macOS 27.2.1 arm64,
Rust 1.99.1, release profile, default headless scale 1×. The identical new fixture
is copied into both revisions: 500 rows × 3 columns at 1200×800 logical points,
20 warmups/160 measured iterations, a cell click, Select All and a blank click,
including a synthetic 30-second idle jump. Existing scrolling/workspace/sidebar
fixtures are unchanged. All builds finished before serial measurement.

Twenty of 21 comparisons passed. Blank-click p95 increased **16.2%**, from
0.100875 to 0.117167 ms (+0.016292 ms), a retained correctness cost, not a blanket
performance pass. The baseline leaves all 500 rows selected; the candidate clears
them and paints the deselected state in that frame. Keeping the old background
response fails the scrolled-click regression test. Deferring clearing until after
painting would retain stale highlighting for a frame and move the work rather
than remove it. Hit testing adds at most two widgets, clearing runs only on a click,
and row/column virtualization is preserved. No thresholds were weakened.

Median per-run CPU milliseconds (p50/p95/p99), followed by the worst individual
sample across all runs:

| Workload | Base p50 / p95 / p99 / max | Candidate p50 / p95 / p99 / max |
| --- | --- | --- |
| Blank click | .097250 / .100875 / .105250 / .125833 | .113125 / .117167 / .123542 / .286666 |
| Select All | .088250 / .091875 / .098625 / .118375 | .087625 / .090959 / .093833 / .588625 |
| Vertical precise scrolling, layout | .230792 / .241708 / .290667 / .357958 | .233125 / .243167 / .292209 / .395208 |
| 120-column workspace, inspector open | .400125 / .411666 / .415833 / .442375 | .396875 / .401583 / .406541 / .425667 |

The largest selection samples occurred in one candidate run; the other four
Select All maxima were .090–.100 ms. The p95 pass does not establish rare-stall
behavior. All raw samples, environment and fixture source remain in the report.

848 library tests passed (225 opt-in tests ignored), strict all-target QA Clippy
passed, and all eight performance-runner tests passed. Locked WASM library
compilation passed with 83 target-build warnings. Regression coverage includes
blank space below/right of cells, main-table and query-grid focus, a focused text
field's Select All, offscreen disk-backed selection and logical Select All clearing,
and the existing double-click insertion guards. Native screenshots at 2× scale,
dark 900×500 and light 760×500, were inspected after synthetic input for both
selected and cleared states. The temporary capture adapter was restored; captures
and validation logs are in `target/grid-selection-review/`.

These CPU timings and synthetic native captures do not measure physical-key input
to presentation, sustained FPS, GPU/compositor work, 60/120 Hz, mixed-DPI or
multi-window/monitor transitions. Database latency fixtures were not rerun because
execution and protocol sequencing are unchanged.

### Pending-row colors, selection and reediting — 2026-11-16

Pending inserts use green in the grid and inspector; existing-value edits retain
orange. Active inline editors retain neutral backgrounds. Pending-cell selection
uses the shared outline, including clicks on cell padding. Draft IDs remain separate
from loaded-row bulk mutation identities. Double-click activation combines the cell
and label responses and uses the shared same-target/system-interval helper. Blur
requests one redraw to retire the old TextEdit hit target. Generated/binary/oversize
fields retain their editing guards. The footer identifies a pending new row instead
of counting it as a loaded row. No polling timer or database operation was added.

Baseline `5dcee13a51cfa676cda8d9d3861d9c8645eaafc0` is an unreferenced snapshot of
the exact working tree before this change, including the preceding blank-selection
fixes; HEAD and the index were not changed. Command:
`python3 scripts/ui-performance.py --base 5dcee13a51cfa676cda8d9d3861d9c8645eaafc0 --pending-rows --grid-selection --output target/insert-row-performance-final`.
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile, default headless
scale 1×. Five alternating serial pairs, with no overlapping builds or benchmarks.
The identical pending-row fixture is copied to both revisions: 50 draft rows ×
3 columns, 1000×600 logical points, idle and active editing, 20 warmups/160 samples
and a synthetic 30-second idle jump. Existing SQL scrolling/workspace/sidebar and
shared grid-selection workloads are unchanged.

The first attempt (`target/insert-row-performance/`) stopped because the old
version could not complete the double-click reopening assertion. That action is
now measured separately on the candidate, retaining its correctness assertion;
it is not presented as a before/after speedup. Both revisions received the same
revised rendering fixture in the final comparison. Five candidate-only reopening
runs measured p50/p95/p99 .097541/.108541/.127084 ms, maximum .404125 ms.

**Overall performance remains inconclusive.** The first completed comparison had
14 passes and 9 inconclusive metrics. A fresh five-pair repeat of the exact built
binaries had 3 passes and 20 inconclusive metrics. Neither run established a
repeatable regression under the gate, but this is not a performance pass.
Substantial unrelated application, indexing and storage-analysis CPU load was
observed. The repeat retains process snapshots and binary paths, hashes and mtimes
in `target/insert-row-performance-repeat/`; its orchestrator is retained in
`target/insert-row-review/repeat-performance.py`. No thresholds were changed.
A quiet-machine comparison is still needed before asserting performance parity.

Repeat-run CPU milliseconds below are median per-run p50/p95/p99 and the worst
individual sample across all runs; these noisy measurements are descriptive only:

| Workload | Base p50 / p95 / p99 / max | Candidate p50 / p95 / p99 / max |
| --- | --- | --- |
| Pending rows, idle | .092083 / .103292 / .134083 / .187750 | .094375 / .114167 / .168583 / .247625 |
| Pending rows, editing | .093959 / .105291 / .118500 / .210667 | .094666 / .117959 / .129458 / .393459 |
| Vertical precise scrolling, layout | .244291 / .264584 / .341959 / .532042 | .245208 / .336667 / .384500 / .490166 |

850 library tests passed (227 opt-in tests ignored), strict all-target QA Clippy
passed, all eight performance-runner tests passed, and locked WASM library
compilation passed with 83 target-build warnings. Tests cover green versus orange,
selection outline geometry, label/padding activation, typing followed by blur and
reopening the same draft, and generated-field protection. Native 2× screenshots
were inspected in dark 1100×600 and light 900×600 layouts, with selected and active
pending cells and the inspector visible. Capture adapters were restored afterward;
logs, fixtures and images are in `target/insert-row-review/`.

Native physical-input-to-present latency, frame intervals, 60/120 Hz, multiple
windows and mixed-DPI/monitor transitions were not measured. No database driver,
transaction, network or protocol sequencing changed; latency fixtures were not rerun.

### Pending-row keyboard navigation and row-tail selection — 2026-11-16

Pending drafts now participate in the grid cursor and its vertical/horizontal
scroll targets, including crossing the loaded/pending boundary. Enter opens the
selected writable draft field; committing or cancelling returns keyboard ownership
to the grid. Consumed arrows cancel egui's queued spatial focus traversal, which
otherwise steals focus when a key arrives immediately after an editor closes.
Draft IDs remain outside loaded-row bulk mutation identities. The right-hand space
beside a row selects that row; only space below all rows clears selection.
Disk-backed results retain their asynchronous row-loading path for tail clicks.
No timers, database operations or protocol changes were introduced.

Baseline `889f26c02dbbc82667b3a2077f8a26986e607be8` captures the working tree
immediately before these changes, including the preceding green/double-click fixes.
HEAD and the index remain unchanged. Comparison command:
`python3 scripts/ui-performance.py --base 889f26c02dbbc82667b3a2077f8a26986e607be8 --pending-rows --grid-selection --output target/insert-navigation-performance`.
The shared right-side click metric is now named `grid/selection/row-tail-click`;
the exact same fixture is installed on both builds (500 rows, 3 columns,
1200×800, 20 warmups, 160 samples). Its intended outcome changes from clearing
selection to selecting the clicked row. Pending idle/editing comparisons retain
50 drafts, 3 columns, 1000×600, 20 warmups and 160 samples. Existing scrolling,
workspace and sidebar workloads remain unchanged. The candidate-only navigation
workload uses 50 drafts at 800×500, alternates Up/Down, verifies Enter opens the
correct draft and Escape closes it, and includes navigation after a synthetic
30-second idle jump. Its 160 measured samples follow 20 warmups; it is not a
before/after speedup because the baseline cannot complete this interaction.

853 library tests passed, with 228 opt-in tests ignored. Strict all-target QA
Clippy, eight performance-runner tests and locked WASM library compilation passed
(the latter with 83 existing warnings). An initial WASM command without `--lib`
attempted to compile the native-only executable and failed at `run_native`; the
correct library target passed. Correctness coverage includes empty/loaded tables,
multiple drafts, generated fields, Enter/typing/commit/Escape, immediately arriving
arrows after editor focus, offscreen rows/columns, row tails and below-row clearing.
Logs and temporary native capture adapters are retained under
`target/insert-navigation-review/`.

The five alternating release pairs on Apple M4 Max / macOS 27.2.1 arm64 /
Rust 1.99.1, headless scale 1×, produced **20 passes and 3 inconclusive metrics**.
All directly affected workloads passed; no regression was reported. Overall this
is not an unconditional performance pass: vertical mouse wheel layout and both
inspector-open workspace scenarios were noisy. Background storage analysis,
browser and streaming-application CPU activity was observed and retained in
`target/insert-navigation-review/process-load.txt`. No thresholds were changed,
and no builds or benchmark workloads overlapped measurement. Raw samples, complete
environment/fixtures and results remain in `target/insert-navigation-performance/`.

Median per-run CPU milliseconds (p50 / p95 / p99), then worst individual sample:

| Workload | Base p50 / p95 / p99 / max | Candidate p50 / p95 / p99 / max |
| --- | --- | --- |
| Pending rows, idle | .094166 / .106250 / .111125 / .152250 | .092417 / .097542 / .102000 / .157584 |
| Pending rows, editing | .096959 / .108708 / .112750 / .161750 | .096167 / .099583 / .108584 / .135666 |
| Row-tail click | .112750 / .126583 / .130584 / .156292 | .088917 / .091917 / .098084 / .212833 |
| Vertical precise scrolling, layout | .238958 / .258916 / .293458 / .430417 | .238750 / .251291 / .294542 / .408792 |

Five candidate-only keyboard runs measured .078458 / .081250 / .086208 ms,
maximum .097875 ms. These measure CPU layout/tessellation for the navigation
frame; Enter/Escape correctness is checked in the same workload but those frames
are not included in that timing. These results do not establish physical-input
latency, native idle process CPU, sustained FPS or GPU/compositor performance.
Native 60/120 Hz, mixed-DPI, monitor and multi-window transitions were not measured.
Database latency fixtures were not rerun because driver/protocol paths are unchanged.

Native 2× screenshots were inspected in dark 1100×600 and light 900×600 layouts:
loaded-row tail selection, pending-row tail selection, the arrow-moved pending-cell
outline and Enter-opened inline editing. The final capture adapter asserts a draft
is selected (and a draft editor is active in the editing case) after synthetic input.
The initial capture targeted the last loaded row; it established loaded-tail behavior
before the input coordinate was corrected for pending-row captures. The temporary
source adapter was restored; native screenshots are not physical-input latency tests.

### Pending labels and history selection — 2026-11-16

Pending-cell display labels explicitly disable text selection, including inspector
labels; inline editors retain normal text selection. Staged undo/redo sets the
exact loaded-cell range and both scroll targets. Draft-field history compares the
changed insert's before/after fields, then selects that field by stable draft ID.
Row deletion history selects the affected loaded row. No database path or repaint
timer changed.

To avoid another redundant baseline build, this iteration reuses the exact prior
candidate release executable, copied and SHA-256 recorded before any source edits.
Its corresponding snapshot is `96986397c35428aaaa536a08bd1b14faa4e0d527`.
`target/history-selection-review/compare.py` uses the build/measure/compare functions
from `scripts/ui-performance.py`, preserves five alternating pairs and the 10% gate,
and rebuilds only the candidate. Existing pending idle/editing, populated-cell
history navigation, typing-to-staged-undo, scrolling, workspace and sidebar fixtures
run unchanged on both binaries. Builds finish before serial measurements.
Baseline provenance, capture adapter and validation logs are retained in
`target/history-selection-review/`; raw CPU samples are in
`target/history-selection-performance/`. Hardware/toolchain/profile are unchanged
from the preceding comparison. Native dark/light captures verify the selector on
the draft name field after undo and redo; the temporary adapter was restored.

855 library tests pass (228 ignored), including text-drag/Copy without entering
edit mode, loaded-cell selection ranges and undo/redo targeting another draft's
changed field. Strict all-target QA Clippy passes. The initial new undo/redo test
omitted key release and therefore generated a held-key repeat; after correcting
the event sequence, the targeted test and complete suite passed. No WASM-specific
code changed; that target was not rebuilt in this iteration. Native idle process
CPU, input-to-present latency, refresh-rate and multi-monitor checks were not run.

The release comparison was **inconclusive for all 23 metrics** under the unchanged
noise gate. Substantial browser/storage-analysis CPU load was present. No parity
or improvement is claimed, and no repeat build was run just to obtain a pass.
Median per-run p50 / p95 / p99 and worst sample, in CPU milliseconds:

| Workload | Base | Candidate |
| --- | --- | --- |
| Pending idle | .113500 / .132125 / .171250 / .629833 | .124042 / .145917 / .226958 / .289084 |
| Pending editing | .114917 / .133458 / .168500 / .779667 | .125917 / .154958 / .187000 / .296917 |
| Typing-to-staged-undo | .002167 / .002375 / .002750 / .020667 | .002292 / .002917 / .003791 / .016917 |
| Populated-cell history navigation | .036083 / .042916 / .067500 / .277042 | .039791 / .048916 / .065709 / .159416 |

The descriptive p95 increases above 10% are not accepted regressions or passes:
run-to-run noise prevents attributing them to this change. A quiet-machine repeat
using the retained binaries is needed to establish performance parity. This run
saved the baseline rebuild; the single candidate release build took 3m07s.

### Editor indentation and text undo — 2026-11-17

Tab/Shift+Tab preserve selected logical lines and selection direction. Text undo
ignores cursor-only changes, preserves redo across navigation, and groups adjacent
typing/deletion with explicit boundaries for paste, cut, indentation, newline and
replacement. Navigation, focus loss and pauses break typing groups. IME composition
remains one step through commit. History retains its existing 100-checkpoint bound.
No database, event-loop, repaint scheduling or renderer path changed.

Compared the working tree against `1b4c0c419da87454f86760fb53004420f57720a0`
using `python3 scripts/ui-performance.py --base <commit> --query-editor
--output target/editor-undo-performance`. Apple M4 Max, macOS 27.2.1 arm64,
Rust 1.99.1, release profile, five alternating serial pairs, default headless 1×
scale. All **24 metrics passed** the unchanged noise and 10% regression gates.
Background macOS storage activity was observed and recorded; the paired samples
nonetheless met the gate. Raw samples, environment and build logs are retained in
`target/editor-undo-performance/`.

The identical query fixture was copied into the baseline. Existing query layout
cases retain 20 warmups/160 samples, 760 logical point height and the existing
result counts and widths. Added history workload: 200 SQL lines at 1200×800,
20 warmups/160 samples per run, type/undo/redo after a synthetic 30-second idle
interval; both revisions assert identical final text. Timings include CPU layout
and tessellation, with fixture reset excluded. Median per-run p50/p95/p99 and the
worst individual sample, milliseconds:

| Workload | Base p50 / p95 / p99 / max | Candidate p50 / p95 / p99 / max |
| --- | --- | --- |
| Typing/undo/redo | .211667 / .217875 / .221875 / .233667 | .213584 / .223958 / .228500 / .277292 |
| Query 0 results / 1280 | .020583 / .022125 / .024583 / .027125 | .020833 / .023750 / .026792 / .034833 |
| Query 4 results / 1280 | .052625 / .055083 / .057750 / .063083 | .052542 / .056375 / .059500 / .065708 |
| Query 128 results / 760 | .055167 / .061125 / .064417 / .069041 | .054584 / .060167 / .063417 / .071084 |
| Query 4 results / 500 | .052459 / .056833 / .060875 / .064791 | .052208 / .056416 / .060042 / .064833 |

The candidate-only block workload performs Tab then Shift+Tab over 40 Unicode SQL
lines and asserts exact restoration on every pair, including after synthetic idle.
20 warmups/160 measured pairs: CPU layout p50 .048208, p95 .054375, p99 .057791,
max .058083 ms. This is not a speedup comparison: the baseline deletes the block.
It uses the test context's default viewport and excludes tessellation.

863 release library tests passed, 229 opt-in tests skipped; the block workload also
passed separately. Strict all-target QA Clippy passed. Focused event tests cover
both selection directions, character limits, partial indentation, Unicode, immediate
undo/redo, ordinary key-plus-text events, navigation, replacement, deletion, pauses,
redo invalidation and IME composition. The performance runner's own tests passed.
Native screenshots at 2× were inspected for dark 1280×760 after Tab/Undo/Redo and
light 760×760 after Tab/Undo; the temporary adapter asserted exact text and was
restored. Captures and adapter sources remain in `target/editor-undo-review/`.
These checks do not establish exact AppKit grouping parity, physical-input latency,
60/120 Hz presentation, idle process CPU, mixed-DPI or multi-window behavior.
Database latency fixtures were not run because database/protocol code is unchanged.

### Sidebar export and table actions (2026-11-17)

Added Command/Shift selection, single and batch CSV/JSON export, definition/name
commands, and reviewed truncate/drop actions. The context menu has no Select all
command. Destructive actions always require review, respect safe mode and pending
edits, and invalidate affected cached table results. No CASCADE or foreign-key
bypass is added. SQLite truncate uses DELETE and retains sequence state. MySQL
review explicitly describes implicit commits, including an open transaction.

Baseline `313137447908abd65250e2de583b5fc6cd95132e` is an isolated snapshot of
HEAD plus the preceding editor fixes, so this comparison isolates sidebar work.
The prescribed runner output is in `target/table-export-performance/` (18 pass,
one noisy/inconclusive). A second five-pair comparison passed all 19 metrics in
`target/table-export-performance-final/`. After the final Query-view selection
guard, all three sidebar metrics passed again in
`target/table-export-sidebar-final/`, using the identical preserved baseline
binary and a rebuilt candidate. Thresholds and fixtures were unchanged.

Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release, headless 1×;
serial runs, five alternating pairs, 100 measured samples per sidebar case.
Viewport, warmups, raw samples and environment are retained in the artifacts and
benchmark source. Background system load was recorded in the review directory.
Median per-run p50/p95/p99 and worst individual sample, milliseconds:

| Catalog | Base p50 / p95 / p99 / max | Candidate p50 / p95 / p99 / max |
| --- | --- | --- |
| 20 | .022500 / .025209 / .026958 / .065750 | .024791 / .026041 / .027875 / .032791 |
| 200 | .033417 / .036042 / .038875 / .043375 | .035250 / .038583 / .043125 / .046083 |
| 2000 | .046959 / .050291 / .054417 / .077667 | .048833 / .052584 / .054958 / .058875 |

New candidate-only interaction workloads use 20 warmups and 160 measured frames,
including layout/tessellation and a synthetic 30-second idle interval. These are
coverage of new behavior, not baseline speedup claims:

| Workload | p50 / p95 / p99 / max (ms) |
| --- | --- |
| Export review, 20 tables | .099416 / .110042 / .116834 / .120167 |
| Export checkbox, 20 | .101209 / .111083 / .119583 / .123208 |
| Export review, 2000 | .097333 / .107334 / .110708 / .116833 |
| Export checkbox, 2000 | .348375 / .381625 / .400709 / .438917 |
| Destructive review, 20 | .018959 / .021667 / .026666 / .027542 |
| Destructive review, 2000 | .019042 / .021250 / .021334 / .021416 |
| Command-click, 20 | .024375 / .027958 / .032042 / .032709 |
| Command-click, 2000 | .045417 / .053000 / .059500 / .063125 |

Command-click first/after-idle CPU samples were .047333/.024750 ms (20) and
.049541/.042125 ms (2000). Tests assert actual selection changes without loading
a table, right-click preservation, review contents, and no hidden prior selection
from a SQL tab. Review lists are virtualized; filename planning happens only on
selection/format changes. Native dark 1120×720 and light 820×720 screenshots at
2× were inspected, with synthetic catalog data and actual pointer events;
`target/table-export-review/` contains single/bulk menus, export and destructive
review. Captures perform no database writes. No physical input-to-present,
60/120 Hz, mixed-DPI, multiple-window, or idle process CPU claim is made.

873 release tests passed, 234 opt-in tests skipped; all three new interaction
benchmarks passed separately. Strict all-target QA Clippy passed. WASM check
passed with 83 existing warnings. Disposable SQLite tests cover batch cleanup,
cancellation, exact output, confirmation and cache invalidation after truncate
and drop. Disposable MySQL/MariaDB batch and existing full-export tests passed;
PostgreSQL CSV/JSON and all five release latency tests passed.

The new worker repeats existing bounded export routines, with zero additional
per-table network exchanges. PostgreSQL healthy built-in-result export remains
8+B exchanges (begin 1, relation metadata 2, columns 2, prepare 1, bind 1,
B portal fetches including final empty fetch, commit 1), plus the existing
conditional Sync when queued cleanup leaves transaction status unknown. Cold and
warm exports prepare per table; custom values are exported as built-in text.
Queued Close/Sync cleanup and cancellation-worker teardown are unchanged.
MySQL/MariaDB remains 3 cold / 2 cached exchanges (combined metadata 1,
SELECT prepare+execute 2 cold / execute 1 cached), with existing permission/error
fallbacks unchanged. SQLite has no network. N-table batch repeats these paths N
times; no preflight or preview queries were introduced. Truncate/drop uses the
existing query/DDL refresh path, with PostgreSQL targets combined in one statement
and MySQL truncate using the same statements as equivalent manual SQL. Ordinary
editor and browse budgets are unchanged.

With 30 ms simulated RTT, PostgreSQL equivalent individual exports had a median
921.455 ms versus batch 908.280 ms, checking exact numeric/Unicode output and an
empty table. The first debug latency suite had one pre-existing large-result
spooling threshold failure (72.7 versus 45.5 ms reference). Investigation retained
the thresholds: the same release test passed on baseline (49.2 versus 42.2 ms)
and candidate (47.4 versus 43.7 ms); all five candidate release tests passed.
Raw debug and release logs, baseline executable provenance and comparison scripts
are retained under `target/table-export-review/`. Tests used disposable fixtures,
never a user database.

### Context-menu and confirmation styling follow-up (2026-11-17)

The sidebar was restoring the global widget style after egui had applied its
menu style. It now reapplies the same `menu_style` used by grid context menus,
retaining normal hover colors while removing boxed row backgrounds and excess
button padding. Destructive confirmation uses a responsive width, connection
subtitle, specific action title/button, aligned affected-table rows and the
shared cached SQL highlighter. Long previews explicitly disclose truncation;
execution, confirmation gates and driver paths are unchanged.

Native captures were inspected in `target/table-menu-style-review/`: single and
bulk menus, dark confirmation at 1120×720 and light confirmation at 820×720,
2× scale. Shared SQL highlighting is cached and operates on the existing bounded
statement previews, not the complete catalog or generated bulk SQL. No extra
repaint loop, network request or database protocol change is introduced.

The focused release comparison uses the preserved pre-styling release executable
(`target/table-export-review/style-base-tests`) and the same runner build/measure/
compare functions, unchanged workloads and thresholds, five alternating pairs.
Raw samples and source hashes are retained in `target/table-menu-style-performance/`
and `target/table-menu-style-review/`. Hardware, toolchain and measurement scope
are unchanged from the preceding sidebar comparison. This measures CPU layout
and tessellation, not physical input-to-present latency.

Median per-run p50/p95/p99 and worst sample, milliseconds:

| Workload | Before p50 / p95 / p99 / max | After p50 / p95 / p99 / max |
| --- | --- | --- |
| sidebar/20 | 0.023417 / 0.026250 / 0.030125 / 0.046000 | 0.024750 / 0.025750 / 0.027958 / 0.045750 |
| sidebar/200 | 0.034583 / 0.038458 / 0.041958 / 0.044166 | 0.035458 / 0.038333 / 0.040958 / 0.088459 |
| sidebar/2000 | 0.048209 / 0.052083 / 0.056375 / 0.062458 | 0.047917 / 0.052458 / 0.061500 / 0.071583 |
| sidebar/action-review/20 | 0.021042 / 0.023833 / 0.025042 / 0.026791 | 0.023916 / 0.026375 / 0.028333 / 0.031916 |
| sidebar/action-review/2000 | 0.021292 / 0.023750 / 0.024417 / 0.053542 | 0.024375 / 0.027250 / 0.027750 / 0.031083 |

All three sidebar p95 comparisons passed. The 20-table confirmation was
inconclusive (+10.7% median); the 2000-table confirmation showed a repeatable
+14.7% regression, from .023750 to .027250 ms (+.003500 ms). This is an accepted
cost of clearer connection/action context, a separated action footer and shared
syntax-highlighted SQL. Plain text would avoid some work but would not meet the
requested SQL preview design. The shared highlighter already caches syntax and
layout; the preview is bounded and table rows remain virtualized. No threshold
was relaxed. The cost is limited to an open confirmation dialog, not typing,
scrolling results or database execution. Worst samples are retained above;
there is no claim of improved native presentation latency.

Final release suite: 873 passed, 234 opt-in tests ignored. Strict all-target QA
Clippy and `git diff --check` passed. Database latency tests were not repeated
for this presentation-only follow-up; no execution/driver path changed.


### Selected-table schema export follow-up

The export dialog now offers Data, Schema, and Schema + data. Schema-only writes
SQL definitions; combined export writes separate SQL and CSV/JSON files. Bulk
exports use a new owned directory, removed on failure/cancellation. SQLite
includes attached indexes/triggers; PostgreSQL uses existing catalog definition
SQL; MySQL/MariaDB use SHOW CREATE. This is selected-object DDL, not a complete
backup: external dependencies, privileges and engine-specific omissions are
explicit in the dialog/SQL. Combined exports do not promise one snapshot spanning
DDL and all row files. Existing data-only export is unchanged.

Opening or editing the dialog makes zero database requests. PostgreSQL schema
export uses bounded parameterized text results and drains through ReadyForQuery:
one exchange per definition with no describe, custom-type lookup, or statement
close wait, on cold and warm healthy connections. The 16 MiB per-definition
retention limit fails explicitly, never silently truncates SQL. An unknown
transaction status retains the existing synchronization fallback before export;
active transactions are rejected. MySQL/MariaDB retain their single SHOW CREATE
exchange and existing cancellation handling; SQLite reads local sqlite_schema.
Schema + data adds this opt-in definition exchange per object to the unchanged
row-export path, necessary to obtain current DDL that is not present in row
results. No additional metadata preview or routine health probe was added.
Normal editor and browse paths are unchanged.

Native dark 1120×720 and light 820×720 captures at 2× were inspected in
`target/schema-export-review/`. The CPU workload is 1000×720 at headless 1×,
20 warmups/160 measured frames, catalogs of 20/2000, idle and checkbox editing,
including a synthetic 30-second idle. Both baseline and candidate receive eight
additional settling frames before locating the checkbox: the larger modal
otherwise moves after the original two-frame setup. No measured workload or
threshold is weakened. Baseline is `0eadbb8`, rebuilt with that same settling
change. Five alternating serial release pairs use the checked-in performance
runner's build/measure/compare functions. Hardware: Apple M4 Max, macOS 27.2.1
arm64, Rust 1.99.1. Raw samples, logs and executables are retained in
`target/schema-export-review/`. These CPU measurements do not measure native
input-to-display latency; 60/120 Hz, mixed-DPI, multiple-monitor/window and
monitor-disconnect checks were not performed for this modal-only change.

Median per-run p50/p95/p99 and worst observed sample, milliseconds:

| Workload | Before p50 / p95 / p99 / max | After p50 / p95 / p99 / max |
| --- | --- | --- |
| sidebar/export-review/20/false | 0.071292 / 0.075084 / 0.079208 / 0.082000 | 0.074166 / 0.078792 / 0.083000 / 0.090250 |
| sidebar/export-review/20/true | 0.085750 / 0.091375 / 0.093375 / 0.105334 | 0.089500 / 0.094417 / 0.098666 / 0.108167 |
| sidebar/export-review/2000/false | 0.093750 / 0.096458 / 0.100833 / 0.110708 | 0.097209 / 0.100166 / 0.103542 / 0.136917 |
| sidebar/export-review/2000/true | 0.360125 / 0.366625 / 0.381417 / 0.404583 | 0.359500 / 0.369791 / 0.383458 / 0.423000 |

All four p95 comparisons passed (0.9–4.9% median increases). Thresholds were unchanged.

The new schema-plus-data workload (candidate only, same fixture) measured:

| Workload | p50 / p95 / p99 / max (ms) |
| --- | --- |
| sidebar/schema-review/20/false | 0.080375 / 0.080875 / 0.081500 / 0.083292 |
| sidebar/schema-review/20/true | 0.106792 / 0.108917 / 0.109291 / 0.109292 |
| sidebar/schema-review/2000/false | 0.100458 / 0.104625 / 0.106625 / 0.107041 |
| sidebar/schema-review/2000/true | 0.746042 / 0.751333 / 0.759958 / 0.774042 |

Validation: 874 release tests passed, 235 opt-in tests ignored; strict all-target
QA Clippy and the separate locked browser WASM check passed (existing WASM
warnings remain). SQLite restoration validates constraints, a unique index and
trigger execution, empty restored data, CSV/JSON pairing, cancellation, active
transactions and failure cleanup. Disposable MySQL and MariaDB tests passed.
Four unchanged PostgreSQL WAN tests passed in release mode. The bulk/schema WAN
test passed after correcting its baseline to perform the same atomic disk writes
(debug build, same machine): at 30 ms simulated RTT, direct schema saves median
89.866 ms versus orchestrated export 89.534 ms for two objects. The initial
in-memory baseline incorrectly counted filesystem sync cost as a network wait;
its failed results are retained, not treated as a pass. No threshold was relaxed.
The initial preview-query implementation also exposed a real extra status/cleanup
wait; the final bounded single-exchange export path removes that dependency.
The final benchmark-only baseline correction does not alter production code or
the UI timings above. PostgreSQL restoration of exported table constraints was
also exercised. Native multi-monitor/input-to-present checks remain untested.


### SQL INSERT export follow-up

Sidebar export now offers SQL inserts for Data and Schema + data. Choosing
Schema + data defaults to SQL: one .sql file per selected table containing DDL,
streamed INSERT statements, then deferred SQLite/PostgreSQL triggers. CSV/JSON
remain available as separate data files. Single-table combined SQL uses Save File;
bulk export uses the existing owned directory and cleanup behavior. The combined
file is atomically published only after its streamed temporary row file completes;
cancellation/error preserves existing destinations. Empty tables retain their DDL.
Views are rejected for INSERT export; schema-only view export remains available.
This remains selected-table export, not a dependency-complete database backup.
Foreign-key/type/function dependencies and PostgreSQL sequence settings have the
same disclosed scope as schema-only export; restore referenced tables first.

The shared row sink emits one INSERT per saved row, omits generated columns,
and quotes identifiers with the owning driver. PostgreSQL uses server text
representations (including custom types, bytea, exact numeric, arrays, JSON,
NaN/infinity), explicit identity override, and restore-time identity sequence
advancement. SQLite handles binary/NUL text with hex literals. MySQL/MariaDB
encode backslash/NUL text independently of quoting mode, preserve zero auto IDs,
and capture/restore the source DDL SQL mode. Restored triggers run only after
saved rows are inserted. These paths retain bounded rows/buffers and run on the
existing database worker, never the UI thread. Combining DDL and rows does not
promise a transaction snapshot spanning schema extraction and data reads.

Protocol review: row exports retain the previously documented PostgreSQL 8+B
exchanges (plus existing conditional synchronization) and MySQL/MariaDB 3 cold /
2 prepared-cache exchanges. Extra generated/identity flags are read in the same
metadata response; PostgreSQL SQL results remain built-in text, with no custom
type lookup exchanges. PostgreSQL schema pre/post sections share one bounded
text query; MySQL SHOW CREATE and @@SESSION.sql_mode are two result sets in one
COM_QUERY, one exchange. No routine ping or preview query was introduced.
Healthy cold/warm schema + SQL adds one definition exchange per table to the
unchanged data path. Sequence advancement is emitted SQL for restoration, not
an additional source request. Prepare, portal paging, transaction completion,
queued statement cleanup and cancellation-worker teardown are unchanged.

At simulated 30 ms RTT, equivalent individual SQL exports of 130 rows plus an
empty table had median 911.895 ms versus bulk 913.825 ms. JSON comparison was
902.678 versus 953.737 ms; schema file comparison was 88.087 versus 88.282 ms.
All three existing thresholds passed without adjustment. The debug editor
large-result test failed (spooled p95 76.8 ms versus reference 43.9 ms); the
unchanged editor path is checked separately in release mode below. Original raw
results are retained, not relabeled as a passing suite.

UI measurements compare the immediately preceding schema-export executable
(`target/sql-export-review/base-tests`, preserved from the previous review)
against the candidate using the checked-in runner build/measure/compare functions.
Five alternating serial release pairs use identical 20/2000-table dialog
workloads, settled click targets, 20 warmups/160 samples, 1000×720 headless 1×,
and a synthetic 30-second idle. Hardware/OS/toolchain: Apple M4 Max, macOS 27.2.1
arm64, Rust 1.99.1. The additional SQL mode workload covers the new combined-file
interaction. These measure layout/tessellation CPU, not physical presentation
latency; native multi-monitor, mixed-DPI and 60/120 Hz checks were not performed.
Artifacts are retained under `target/sql-export-review/`.

Median per-run p50/p95/p99 and worst observed sample, milliseconds:

| Workload | Before p50 / p95 / p99 / max | After p50 / p95 / p99 / max |
| --- | --- | --- |
| sidebar/export-review/20/false | 0.075041 / 0.080500 / 0.085292 / 0.089875 | 0.076958 / 0.082250 / 0.085375 / 0.129250 |
| sidebar/export-review/20/true | 0.090541 / 0.095500 / 0.100750 / 0.123917 | 0.091167 / 0.096292 / 0.098750 / 0.118750 |
| sidebar/export-review/2000/false | 0.097291 / 0.102625 / 0.107875 / 0.114625 | 0.098000 / 0.101458 / 0.106500 / 0.134959 |
| sidebar/export-review/2000/true | 0.354584 / 0.363375 / 0.371959 / 0.400500 | 0.364833 / 0.376083 / 0.386084 / 0.397750 |

All four p95 comparisons passed (−1.1% to +3.5% median change), with unchanged thresholds.

Candidate-only combined SQL interaction workload:

| Workload | p50 / p95 / p99 / max (ms) |
| --- | --- |
| sidebar/schema-review/20/false | 0.076209 / 0.080209 / 0.088000 / 0.090458 |
| sidebar/schema-review/20/true | 0.092833 / 0.099792 / 0.104541 / 0.109250 |
| sidebar/schema-review/2000/false | 0.099208 / 0.103417 / 0.109625 / 0.111500 |
| sidebar/schema-review/2000/true | 0.509667 / 0.520291 / 0.532958 / 0.556333 |

Final release suite: 875 passed, 238 opt-in tests ignored. Strict all-target QA
Clippy and the separate locked browser WASM check passed (WASM warnings remain).
The PostgreSQL export suite and both MySQL/MariaDB export suites, including SQL
round-trip restoration and existing CSV/JSON/cancellation tests, passed. Both
MySQL/MariaDB bulk tests passed. SQLite coverage restores quoted identifiers,
Unicode/quotes/NUL text, binary, NULL, exact real values, generated columns and
post-data triggers, checks empty and 1200-row bulk exports, and verifies failure
cleanup/cancellation preserves existing files. PostgreSQL checks custom enum,
array, JSON, exact numeric, bytea, non-finite float, identity advancement and
trigger ordering. MySQL/MariaDB checks both quoting modes, unsigned BIGINT,
DECIMAL, generated columns, binary, NUL text and zero auto-increment IDs.

Native dark 1120×720 and light 820×720 screenshots at 2× were inspected in the
artifact directory. Buttons, filenames, mode selection and the complete footer
remain visible in both layouts. No permanent repaint loop was introduced.

All five final release PostgreSQL latency tests passed. The earlier debug-only
large-result failure did not reproduce: release spooled p95 was 49.0 ms versus
46.1 ms for the equivalent single-exchange reference. This does not recast the
debug result as a pass; both logs are retained. No performance thresholds were
changed and no editor execution code was altered for this follow-up.


### Export dialog split-layout follow-up

The supplied TablePlus reference now guides the export dialog: bounded Items
selection on the left, CSV/JSON/SQL tabs and format-specific controls on the
right, followed by a filename preview and Cancel/Export actions. SQL exposes
Data only, Schema only, and Schema + data; CSV/JSON can include a separate SQL
schema. CSV formatting is now configurable as described below; JSON encoding details
remain displayed alongside its schema checkbox. Destination naming still uses the native save
panel. Detailed schema scope is available on hover rather than occupying the
main dialog. Empty/disabled states and bulk selection remain intact.

The list keeps row virtualization. Its taller viewport intentionally exposes
more items, and the single-column list replaces the previous filename column.
The first output filename is cached when selection/format changes, avoiding a
catalog scan during each render. No database, export encoding, network exchange,
file publication or cancellation paths changed in this presentation follow-up;
the preceding database validation remains applicable. No perpetual repaint was
added. Native dark SQL at 1120×720 and light CSV at 820×720 (2×) were captured
and inspected in `target/export-layout-review/`.

The comparison uses the preserved SQL-export release executable from the preceding
review as baseline and the same runner build/measure/compare functions, with five
alternating serial pairs. Workloads and thresholds are unchanged: 20/2000 tables,
idle and checkbox editing, 20 warmups/160 measured frames, 1000×720 headless 1×,
including a synthetic 30-second idle. The SQL interaction workload is also run
on the candidate. Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile.
Raw samples and executables are retained in the artifact directory. These are CPU
layout/tessellation measurements, not input-to-present measurements. Multi-window,
mixed-DPI, monitor reconnect and 60/120 Hz presentation checks were not performed.


### Combined SQL file and configurable CSV follow-up

SQL exports now use a single chosen .sql destination for the entire selection,
with data-only, schema-only and combined modes. All pre-data DDL precedes all
INSERTs; PostgreSQL foreign keys and PostgreSQL/SQLite triggers follow the data.
SQLite uses deferred foreign-key checks inside the generated transaction;
PostgreSQL validates deferred constraint creation after all selected rows exist.
MySQL/MariaDB generated scripts temporarily disable foreign-key checks to allow
child-before-parent DDL, then restore the importing session's prior check flag
and SQL mode. These statements are emitted into the file, not executed against
the source database. External dependencies and permissions remain out of scope.
CSV/JSON plus schema now writes one shared schema.sql beside the individual data
files. Every bundle is staged and published atomically; failures/cancellation
remove owned temporary data and preserve existing destinations.

CSV controls match the reference categories: NULL to empty (off emits unquoted
NULL), CR/LF/CRLF field-break normalization to spaces, header inclusion,
comma/semicolon/tab/pipe delimiter, quote-if-needed/always quoting, LF/CRLF/CR
record endings and point/comma decimal separator. Empty strings are quoted;
literal text NULL is quoted when using the NULL marker. Decimal conversion is
restricted to numeric values, including exact PostgreSQL NUMERIC and MySQL
DECIMAL text; it never rewrites dates or arbitrary strings. Quoting accounts for
the selected delimiter and decimal separator. Field encoding uses fixed buffers,
checks cancellation in both the quoting scan and write pass, and keeps all row
and protocol bounds from existing exports. Legacy CSV menu exports retain their
previous encoder/defaults. Sidebar settings are passed explicitly to the worker.

No source exchanges were added. Generated/identity/base-type metadata already
arrives in existing responses. PostgreSQL column formatting uses built-in text
for configurable CSV numeric values, without custom-type lookups. DDL/FK/post-
trigger sections share the existing single schema query. MySQL SHOW CREATE and
SQL mode remain one COM_QUERY exchange. Source definitions and row reads retain
the previous per-table order while file sections are spooled locally, avoiding
an extra synchronization wait between exports. Healthy cold/warm counts remain
one schema exchange plus the prior PostgreSQL 8+B row exchanges (and conditional
cleanup synchronization) or MySQL 3 cold/2 cached row exchanges. Cancellation,
transaction state, preparation, portal completion and cleanup paths are retained.

At simulated 30 ms RTT, debug comparisons passed unchanged thresholds: separate
SQL files 981.977 ms versus one file 979.311 ms; individual configurable CSV
916.983 ms versus bulk 902.260 ms; schema reference 95.919 ms versus file
95.248 ms. The unchanged large-result editor debug benchmark again failed
(79.7 ms spooled p95 versus 51.1 ms reference); final release checks are recorded
below. Raw failures are retained rather than counted as passes.

The final UI comparison uses the pre-layout SQL-export release executable and
identical existing workloads. It covers both the requested layout and new CSV
controls. The candidate also runs an actual CSV-header checkbox interaction
workload with 20 and 2000 selected tables, plus the SQL-mode workload. Measurement
settings remain those listed above; CSV settings changes do not regenerate all
filenames. Raw results, screenshots and logs are in target/export-config-review/.
Native dark CSV and narrow light SQL captures were inspected; every control and
the single SQL filename remain visible. Strict QA Clippy, browser compilation
and the format-tab/checkbox/filename interaction check passed. Debug full suite:
879 passed, 239 opt-in tests ignored. Restoration covers reverse dependency order,
cyclic PostgreSQL foreign keys, empty tables, SQL-only/schema-only modes,
cancellation and existing-destination preservation. Driver CSV checks preserve
high-precision decimals with a comma separator on PostgreSQL, MySQL and MariaDB.

Future spreadsheet formats were investigated but not implemented: a streaming
XLSX writer is a practical first step. rust_xlsxwriter's constant-memory mode
flushes rows to temporary files, fitting the existing worker/streaming design
(https://docs.rs/rust_xlsxwriter/latest/rust_xlsxwriter/performance/index.html).
Numbers imports XLSX and delimited text, so XLSX provides a supported Numbers
workflow without a separate native .numbers serializer
(https://support.apple.com/en-mk/guide/numbers/tan9f3c54bdc/mac).
Direct Google Sheets export is a separate authenticated, batched network workflow
with quota, retry, cancellation and explicit destination handling, not simply
another file extension (https://developers.google.com/workspace/sheets/api/guides/batch
and https://developers.google.com/workspace/sheets/api/limits). Future XLSX work
must preserve long IDs/precise decimals as text where necessary, encode strings
as values rather than formulas, and define sheet/row/column overflow behavior.

Final five-pair UI comparison, median per-run p50/p95/p99 and worst sample (ms):

| Workload | Before p50 / p95 / p99 / max | After p50 / p95 / p99 / max |
| --- | --- | --- |
| sidebar/export-review/20/false | 0.069041 / 0.077792 / 0.080792 / 0.101375 | 0.078084 / 0.085459 / 0.087875 / 0.093000 |
| sidebar/export-review/20/true | 0.087250 / 0.092917 / 0.095833 / 0.106292 | 0.088292 / 0.093500 / 0.097334 / 0.103500 |
| sidebar/export-review/2000/false | 0.095875 / 0.100083 / 0.103833 / 0.118542 | 0.102416 / 0.108125 / 0.110667 / 0.120542 |
| sidebar/export-review/2000/true | 0.342333 / 0.358667 / 0.372791 / 0.404750 | 0.344208 / 0.360209 / 0.361833 / 0.380292 |

The first five pairs had three passes and one inconclusive small-catalog idle
comparison (+11.8%, including a noisy first pair). A fresh serial comparison
without builds or database workloads passed all four cases: +9.9%, +0.6%, +8.0%
and +0.4% median p95, respectively. Small-catalog idle is close to the 10%
threshold; this is not a claim of zero CPU cost. Additional CSV controls and the
taller list add visible work, while selection editing remains near baseline.
No threshold or measured fixture was weakened; both sets of raw results remain.

CSV-header checkbox workload (candidate only):

| Case | p50 / p95 / p99 / max (ms) |
| --- | --- |
| sidebar/csv-options-review/20/false | 0.082542 / 0.094500 / 0.101458 / 0.102042 |
| sidebar/csv-options-review/20/true | 0.085542 / 0.091708 / 0.098833 / 0.104083 |
| sidebar/csv-options-review/2000/false | 0.104917 / 0.105666 / 0.108500 / 0.108625 |
| sidebar/csv-options-review/2000/true | 0.107875 / 0.108500 / 0.111041 / 0.114375 |

SQL-mode workload (candidate only):

| Case | p50 / p95 / p99 / max (ms) |
| --- | --- |
| sidebar/schema-review/20/false | 0.076500 / 0.081083 / 0.089500 / 0.091625 |
| sidebar/schema-review/20/true | 0.084375 / 0.087500 / 0.087792 / 0.090250 |
| sidebar/schema-review/2000/false | 0.098958 / 0.102250 / 0.106375 / 0.106708 |
| sidebar/schema-review/2000/true | 0.506875 / 0.516916 / 0.522166 / 0.522625 |

Final release verification: 879 tests passed, 239 opt-in tests ignored. PostgreSQL
and MySQL/MariaDB export suites and both MySQL/MariaDB bulk suites passed. All
five release PostgreSQL latency tests passed, including configurable CSV and
single-file SQL. Release single-file SQL median was 956.355 ms versus 971.899 ms
for separate files at simulated 30 ms RTT. The unchanged editor large-result
release check passed (53.1 ms spooled p95 versus 47.1 ms reference), while its
debug failure remains recorded above. Native QA Clippy, browser compilation,
format selection and effective CSV option forwarding checks passed. Disposable
fixtures were removed. Spreadsheet formats remain researched, not implemented.

### Query/result sharing (2026-11-17)

Compared the current working tree with `0eadbb8e531e979aa3dc6e574861b53acc6ba4f0` using
`python3 scripts/ui-performance.py --base 0eadbb8e531e979aa3dc6e574861b53acc6ba4f0 --query-editor --output target/share-results-performance`.
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile; five alternating
pairs, identical fixtures and default headless scale (1×). All 23 comparisons
passed the existing 10% p95 gate. Builds and timings ran serially.

The affected query-result fixtures retain 20 warmup / 160 measured frames per run.
Values below are median per-run CPU milliseconds (base → candidate), not
input-to-present timing. Full grid/sidebar/workspace results and raw samples are
retained in `target/share-results-performance/`.

| Fixture | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| query/editor-results/0/1280 | 0.02079 → 0.02121 | 0.02325 → 0.02263 | 0.02479 → 0.02554 | 0.02488 → 0.02592 |
| query/editor-results/4/1280 | 0.05262 → 0.05371 | 0.05600 → 0.05679 | 0.05913 → 0.05950 | 0.05996 → 0.06042 |
| query/editor-results/128/760 | 0.05513 → 0.05621 | 0.05987 → 0.06025 | 0.06246 → 0.06350 | 0.06338 → 0.06608 |
| query/editor-results/4/500 | 0.05321 → 0.05412 | 0.05667 → 0.05779 | 0.05850 → 0.06137 | 0.06063 → 0.06379 |

The new share dialog has a separate production-rendering workload
(`share_dialog_frame_benchmark`): 20 warmups and 200 measured frames at
500×720 and 1120×720, with a simulated 30-second idle gap. These absolute costs
have no baseline equivalent and are not counted as comparative passes.

| Dialog width | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| share/dialog/500 | 0.02267 | 0.02300 | 0.02333 | 0.02529 |
| share/dialog/1120 | 0.02221 | 0.02254 | 0.02521 | 0.02592 |

Native dark/wide and light/narrow screenshots are inspected under
`target/share-design-review/`; the context menu is also inspected natively but
is not part of the comparative closed-menu timings. Tests separately cover shortcut ownership, early
copy while preparation is pending, stale-preview rejection, persistence, complete
stored values, bounded output, bindings and result/statement provenance.
The new dialog benchmark measures cached rendering, not background formatting,
clipboard service latency or opening a very large selection. Native 60/120 Hz
input-to-present timing, missed frames, multiwindow mixed-DPI, monitor reconnect
and macOS snapping were not measured.

### SQL Enter indentation and parenthesis pairing (2026-11-17)

Compared with `0eadbb8e531e979aa3dc6e574861b53acc6ba4f0` using
`python3 scripts/ui-performance.py --base 0eadbb8e531e979aa3dc6e574861b53acc6ba4f0 --query-editor --output target/editor-assist-performance`.
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release builds, five alternating
pairs with identical fixtures at default headless scale (1×). All 23 comparisons
passed the unchanged 10% p95 gate. Builds and benchmark workloads ran serially.

Query result fixtures use 20 warmups / 160 measured frames. Below are median
per-run CPU milliseconds (base → candidate).

| Fixture | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| query/editor-results/0/1280 | 0.02087 → 0.02096 | 0.02150 → 0.02171 | 0.02367 → 0.02538 | 0.02475 → 0.02592 |
| query/editor-results/4/1280 | 0.05262 → 0.05342 | 0.05558 → 0.05608 | 0.05583 → 0.05900 | 0.05829 → 0.06100 |
| query/editor-results/128/760 | 0.05492 → 0.05550 | 0.05983 → 0.05925 | 0.06300 → 0.06288 | 0.06383 → 0.06400 |
| query/editor-results/4/500 | 0.05225 → 0.05338 | 0.05508 → 0.05613 | 0.05871 → 0.05883 | 0.06171 → 0.06171 |
| query/editor-history | 0.21633 → 0.21633 | 0.22217 → 0.22092 | 0.22679 → 0.22725 | 0.22921 → 0.22996 |

The new `editor_assist_input_benchmark` exercises the production TextEdit event
handler: type `(`, Enter, undo the newline, undo the pair, and assert exact text
restoration. It measures four input/layout passes and tessellation of the two
insertion passes, with 20 warmup / 100 measured cycles. Fixtures repeat a two-line
SQL block 10 or 1,000 times (220 / 22,000 ASCII characters). These are absolute
costs for new behavior, not a comparison against the baseline that lacks it.

| Two-line blocks | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| 10 | 0.04975 | 0.05829 | 0.06100 | 0.06100 |
| 1000 | 1.15246 | 1.35054 | 1.43229 | 1.43229 |

Raw comparisons are in `target/editor-assist-performance/`; input samples, test
logs and native captures are in `target/editor-assist-review/`. Focused event tests
cover tabs/spaces, first Enter after a simulated 30-second idle gap, Unicode
selection wrapping, closing-character overtyping, empty-pair deletion, literal
typing groups, undo/redo, paste, IME, limits and dialect-specific lexical guards.
Native captures inject `(`, Enter and `SELECT 1` into the actual query editor and
assert the resulting text, including the literal tabs and aligned closer; dark
1280-wide and light 760-wide views were inspected.

This measures CPU layout/input handling, not end-to-end key-to-present latency,
native 60/120 Hz frame intervals, mixed-DPI, monitor changes or snapping. The new
input benchmark uses a plain code TextEdit; complete production workspace typing
latency and worst-case multi-megabyte scripts were not measured. No database
execution/protocol paths changed, so network latency suites were not rerun.

### Query toolbar divider spacing (2026-11-17)

Added a 4-point bottom inset to the query editor panel. Native captures at 1280 and 500 logical pixels (2× scale), dark/light, confirm spacing above the results divider; multi-result strips were also inspected. Artifacts: `target/query-toolbar-spacing/`.

Ran `python3 scripts/ui-performance.py --base 0eadbb8e531e979aa3dc6e574861b53acc6ba4f0 --query-editor --output target/query-toolbar-spacing/performance` on Apple M4 Max, macOS 27.2.1, rustc 1.99.1, release profile, five alternating serial baseline/candidate runs. Query layout fixtures use 20 warmups and 160 measured frames per run; unchanged fixtures and viewport/scale metadata are retained in the report. This compares the complete pending changes against the release base.

Median run statistics in ms (base → candidate):

| Query workload | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| query/editor-history | 0.2256 → 0.2246 | 0.2420 → 0.2352 | 0.2809 → 0.2424 | 0.3001 → 0.2474 |
| query/editor-results/0/1280 | 0.0213 → 0.0215 | 0.0240 → 0.0238 | 0.0249 → 0.0263 | 0.0249 → 0.0290 |
| query/editor-results/128/760 | 0.0569 → 0.0572 | 0.0633 → 0.0646 | 0.0693 → 0.0747 | 0.0711 → 0.0899 |
| query/editor-results/4/1280 | 0.0543 → 0.0549 | 0.0638 → 0.0590 | 0.0661 → 0.0632 | 0.0675 → 0.0636 |
| query/editor-results/4/500 | 0.0540 → 0.0549 | 0.0621 → 0.0610 | 0.0736 → 0.0628 | 0.0962 → 0.0634 |

The overall comparison is **inconclusive**, not a pass: seven noisy scenarios, including the 500-pixel query layout, failed the consistency criterion. Query layout median p95 changes ranged from −7.5% to +2.2%; the narrow layout was −1.7%. Grid precise tessellation (+13.2%) and the 20-item sidebar (+10.9%) were also inconclusive; no repeatable regression was established. Raw paired runs, p50/p95/p99/max and samples are retained in `performance/results.json`. No thresholds were relaxed. These are CPU measurements, not native input-to-present or FPS measurements; mixed-DPI and multi-monitor checks were not performed.

## Aurora connection activity (#12)

The ordinary busy connection pill now uses cyan, blue and violet shaded ribbons
instead of scrolling square blocks. AI work retains the separate green Matrix
rain and takeover controls. This is an indeterminate activity animation, never a
query completion estimate. Connection identity and elapsed time keep their prior
layout and hover details; the normal connection color returns when work finishes.
The busy surface uses theme-specific contrast. No input locking or cancellation
behavior changes accompany this visual update.

The mesh is bounded to three ribbons, 96 segments each (873 vertices maximum),
with no textures, file I/O, network calls or catalog/result scans. Geometry stays
inside the pill, including its rounded ends. Animation uses the existing pending
operation redraw cadence; no new timers or idle polling were added. Database
protocol/round-trip paths are unchanged, so DB latency tests were not rerun.

Native screenshots at 760×760 and 1280×760, dark and light, were inspected from
`target/aurora-review/`. QA uses a synthetic pending query with no DB execution.
60/120 Hz input-to-present timing, mixed-DPI displays, moving/snapping and monitor
reconnection were not measured. These screenshots validate layout, not FPS.

The repeatable performance command is:

```sh
python3 scripts/ui-performance.py --base 1c1688f --connection-activity --runs 7 --output target/aurora-performance
```

The runner copies the same new toolbar workload into the base revision. It
measures the actual production toolbar, idle and busy, both themes and viewport
widths, with 20 warmups and 160 measured frames, including first-use and a
simulated 30-second idle gap. Raw CPU layout/tessellation p50/p95/p99/max and frame
samples are retained with environment metadata in the output directory.

Measured on Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile,
default headless scale (native captures at 2×). Builds and benchmarks ran
serially. After an edge-case geometry adjustment, the candidate was rebuilt;
measurement resumed using the runner's unchanged helpers and the retained
baseline binary. The first seven-pair batch passed every busy case but had two
inconclusive idle light-theme cases. A second seven-pair batch with the same
binaries passed all eight cases, with the unchanged 10% threshold. Both batches
are retained; the following table reports the second batch's medians of per-run
CPU timings, in milliseconds (p50 / p95 / p99 / maximum).

| Width / theme / state | Base | Aurora | p95 change |
| --- | --- | --- | ---: |
| 1280 / light / idle | 0.015500 / 0.015958 / 0.018417 / 0.019750 | 0.015459 / 0.015916 / 0.018208 / 0.019542 | -0.3% |
| 1280 / light / busy | 0.023916 / 0.027416 / 0.030209 / 0.030875 | 0.020417 / 0.022958 / 0.025208 / 0.025792 | -16.3% |
| 1280 / dark / idle | 0.015333 / 0.016042 / 0.019083 / 0.019750 | 0.015375 / 0.016333 / 0.018291 / 0.019542 | +1.8% |
| 1280 / dark / busy | 0.023958 / 0.027875 / 0.031250 / 0.034833 | 0.020292 / 0.022583 / 0.025708 / 0.026042 | -19.0% |
| 760 / light / idle | 0.015292 / 0.015833 / 0.018166 / 0.019334 | 0.015333 / 0.016625 / 0.018791 / 0.019666 | +5.0% |
| 760 / light / busy | 0.019000 / 0.021125 / 0.022042 / 0.023125 | 0.017333 / 0.019667 / 0.022375 / 0.023292 | -6.9% |
| 760 / dark / idle | 0.013917 / 0.015708 / 0.017500 / 0.017917 | 0.014917 / 0.015542 / 0.017708 / 0.019000 | -1.1% |
| 760 / dark / busy | 0.019000 / 0.021541 / 0.023750 / 0.024084 | 0.017250 / 0.020000 / 0.022209 / 0.022916 | -7.2% |

Validation: 921 library tests passed in both debug and release (248 ignored),
and strict all-target QA Clippy passed. Coverage includes bounded geometry,
rounded-cap containment, motion, and restoration of the idle bar without ongoing
redraws. First-use and after-idle measurements are in the raw logs. These CPU
measurements are not a claim about native presentation latency or sustained FPS.
