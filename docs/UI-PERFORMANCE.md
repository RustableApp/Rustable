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
