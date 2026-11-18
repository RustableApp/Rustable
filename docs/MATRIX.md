# Matrix experiment

Branch: `experiment/matrix-mcp`. Matrix lets an external MCP client work in
Rustable's visible SQL editor using real SQLite, PostgreSQL, MySQL and MariaDB
connections. Queries and results remain in normal tabs so you can continue the
work. This is separate from PR #10's headless database server. A built-in chat
panel and model/provider configuration are still future work.

## Try it with a real connection

```sh
./scripts/matrix.sh
# To use the installed app's saved connections and credential namespace:
./scripts/matrix.sh --release
```

The normal connection manager opens. Open the intended connection, choose your
usual Safe mode setting, then open the **AI** toolbar menu and choose **Enable AI
control** in that connection’s window. Nothing is exposed to MCP until you enable
it. Redis is not supported.
The launcher prints the MCP client configuration for this running process; add
that configuration to your AI client. It does not execute a scripted tour on
real connections.

Debug builds use Rustable Development profiles and credentials. Release builds
use the installed app's profile/credential namespace. Close another instance
using the same profile before starting this experiment to avoid competing
settings writers. The launcher does not copy credentials or save client config.

Example prompt:

> List the Rustable workspaces I enabled. In the sales connection, explore revenue
> by product. Open clearly named query tabs and leave a follow-up query I can edit.

During an AI edit or query, the connection bar becomes an animated Matrix-style
**AI is working** indicator. The editor and results remain visible with a muted
appearance, while background input is disabled. Click **Stop** or press **Esc** to pause AI access
and request cancellation of an active AI query. The working bar remains until the
query worker acknowledges cancellation. When idle, use **AI → Pause AI access**.
It does not undo committed writes or automatically roll back a transaction. Finish any transaction in the normal editor. You can continue editing
all tabs. Enabling AI again grants a fresh target after takeover; stale requests
cannot resume automatically. Switching databases, reconnecting, or changing schema
also revokes the grant. Inactive parked databases are not available through MCP.

## Tools and boundaries

- `list_workspaces`: list only enabled, active SQL workspaces, including their
  connection name, database, engine, Safe mode, production flag and busy state.
- `workspace`: inspect the connection, transaction status, active SQL tab ID, open
  tab IDs, edit state and a bounded preview of the already-loaded catalog.
- `open_query`: create an AI-owned tab with a title and SQL, without executing it.
- `replace_query`: update any requested open SQL tab only when `expected_sql` matches.
- `run_query`: activate and run any requested open SQL tab through the existing editor.
  Requires matching `expected_sql`; returns immediately. Poll `inspect_query`
  for completion without tight loops.
- `inspect_query`: read any open SQL tab, including the user’s text and selection,
  edit state, error/running/confirmation status and selected result
  preview, capped at 20 in-memory rows × 30 columns. Spilled rows are not loaded;
  an empty preview does not prove the result is empty.
- `begin_query_edit`: reserve a character range in an existing tab, with exact
  `expected_sql` matching; returns an edit ID. It does not erase the original SQL.
- `append_query_edit`: insert ordered replacement chunks into that range, each
  appearing immediately in the normal editor. Sequence numbers prevent duplicates.
- `finish_query_edit`: complete the replacement without executing SQL.
- `revert_query_edit`: restore the original text if no later human edit conflicts.
- `cancel_query`: request cancellation of a query started by MCP, including one
  run in an existing user tab.

All tools except discovery require the exact `workspace_id`. IDs change when
access is revoked. The client must ask the user to reenable the intended target;
it must not substitute a different connection. Closing a window invalidates it.

SQL uses the connection's existing database privileges. A workspace ID routes
commands; it is not a database ACL or SQL sandbox. Safe mode and production
protections use the normal Rustable implementation. When Safe mode is off, writes
are possible; destructive-query dialogs still require the user. The MCP bridge
cannot change Safe mode, approve dialogs, open saved connections, or close tabs. It returns query data to the connected AI client,
which may send that data to its model provider.

Human changes to AI SQL invalidate stale replacements and executions. At most
16 AI tabs are allowed per workspace. Busy workspaces and outstanding edits or
dialogs refuse navigation commands. The bridge does not replay failed writes;
a client must never retry a write whose commit outcome is unknown automatically.
Credentials stay in Rustable's existing connection system and are not tools output.
SSH/TLS setup uses the normal connection workflow without a separate connection.

## Working together in an existing query

Enabled workspaces share all open SQL tabs with the AI. “What have I written?”
reads the active tab and selection without changing anything. “Fix this query”
should edit that same tab; a separate analysis or alternative can open a new tab.
These choices are documented in the MCP instructions so the client can follow the
user’s intent. Tab titles and file associations survive edits; saving remains a
normal user action. Read access is not limited to AI-created tabs anymore.

Progressive editing replaces a specified Unicode character range. The client sends
useful chunks as it generates SQL; Rustable shows each immediately, marks the
changed lines and offers **Stop** in the working bar. **AI → Undo AI edit**
restores the original after editing stops. Cmd/Ctrl+Z also restores the
original in one step while preserving earlier editor history. A single workspace
has one active stream and one latest-edit restoration control. Ordinary editor
undo retains earlier edits.

While streaming, typing and tab switching are disabled; Stop or Esc returns
control. Defensive conflict checks also stop the stream if the target text or
tab changes through another path. Human changes are never silently overwritten
or undone. Stopped partial
SQL remains visible for review; stopping does not restore the original unless
Undo is requested. Running SQL is blocked during a live edit. Finishing an edit
never executes it; execution requires a separate action with matching SQL. Results
already shown in a tab remain results of its previous execution.

This is actual **MCP chunk streaming**, not an animation replay of a completed
answer. A client that only calls `replace_query` gets an atomic replacement. Direct
token streaming, inline chat, and a full before/after diff review are future UI work.
The model/client supplies the editing decisions; the app enforces target identity,
text conflicts, ordering, bounds and normal database safeguards.

## Local transport and limits

The launcher creates a private temporary directory (0700) and Unix socket (0600)
and prints a per-run stdio MCP configuration. It is local, not a network server.
Any process able to access that socket can act in enabled workspaces. The socket
path changes each launch. macOS and Linux are supported; Windows transport is
not implemented.

```sh
matrix_dir=$(mktemp -d)
target/debug/rustable matrix --socket "$matrix_dir/workspace.sock"
# Use the exact path above in the MCP client's stdio command:
target/debug/rustable mcp --socket /private/directory/workspace.sock
```

Requests and complete serialized MCP responses are capped at 128 KiB; SQL at
32 KiB. Discovery returns up to 50 workspaces. Catalog output has a 48 KiB budget,
up to 200 objects and 50 columns per object, with truncation indicators. Text in
result previews is clipped. Socket IO and final serialization use a background
thread, a bounded queue and deadlines. UI admission expires after three seconds.
Commands wake the root and selected window on demand; no permanent repaint timer.

Execution still follows `run_query` → `execute_query` → the existing worker on
the same live session. Discovery and inspection read local UI state. No extra
catalog, health-check, prepare, transaction-status or cleanup queries are added.
Established healthy PostgreSQL query paths retain one exchange per statement;
normal initial table loads retain their pipelined exchange. Connection startup,
cold type handling, explicit transactions and error recovery retain the existing
fallback costs. MySQL/MariaDB keep their existing execution and read-only batch
paths. Before/after database round-trip counts are unchanged by this bridge.

## Disposable demo and QA

```sh
./scripts/matrix-demo.sh
./scripts/matrix-demo.sh --no-tour
```

The separate `matrix-demo` command opens the disposable SQLite studio and enables
only that workspace. The scripted tour uses actual stdio MCP calls to draft,
replace, run and inspect three queries. It is a script, not an AI model.

For native captures, build with `cargo build --features qa`. The demo accepts
`RUSTABLE_MATRIX_CAPTURE=/absolute/path/demo.ppm`. Real-window QA uses an explicit
SQLite fixture populated from `src/demo.sql`, never a saved connection:

```sh
RUSTABLE_MATRIX_FIXTURE=/absolute/path/fixture.sqlite \
RUSTABLE_MATRIX_CAPTURE=/absolute/path/real \
python3 scripts/matrix-demo.py --connections --fixture-tour --exit-after-tour
```

This QA-only path enables the explicit fixture in a real manager session and
runs the stdio tour through multiwindow routing. For editor captures, use the
same environment with `python3 scripts/matrix-demo.py --exit-after-tour` (without
`--connections --fixture-tour`). This hosts the real SQLite connection in a single
native viewport, captures the disabled state, enables Safe mode and fixture
control, and writes `real-disabled.ppm` and `real-active.ppm` after the tour.
`RUSTABLE_MATRIX_NARROW=1` uses a 760px window; `RUSTABLE_MATRIX_LIGHT=1` selects the
light theme for captures. Production builds do not include these fixture hooks.
Child-window screenshot delivery was unavailable on the locked desktop, so these
captures validate the editor view, not native multiwindow layout.

## Real-connection verification (310d581, 2026-11-17)

Real connection tests passed on a temporary SQLite file, PostgreSQL 17.11,
MySQL 8.4.13 and MariaDB 11.4.13 using disposable local containers. Coverage includes
results, empty results, PostgreSQL custom types, Safe mode (including a view with
side effects), ordinary write persistence, destructive confirmations, errors,
cancellation/recovery, schema/reconnection revocation, exact multiwindow routing,
parked targets and stale SQL. No user database was used for testing.

All five PostgreSQL latency checks passed with the disposable fixture. At 30ms
simulated RTT the indexed editor lookup measured p50 43.9ms / p95 45.7ms, versus
39.7ms p50 for the single-exchange reference. Complete 6,000-row results measured
72.6ms / 81.2ms. These validate existing execution paths, not a claim that Matrix
improves database latency. Raw logs: `target/matrix-connections-review/`.

Manual native input checks are unavailable while the Mac is locked. External AI
client integration, SSH/TLS end-to-end connection tests, native input-to-present
latency, missed frames, 60/120Hz, mixed DPI, monitor reconnect and snapping remain
untested. Renderer captures and egui tests do not replace those checks.

The final debug and release library suites each passed **883 tests**, with 238
opt-in tests ignored. Clippy passed with all targets, QA enabled and warnings
as errors. Real-manager stdio tours completed all three queries (3, 4 and 20 rows).
The native editor captures were inspected at 1280×820 dark and 760×760 light,
2× display scale, including disabled and active controls. The manager banner was
captured separately. Takeover/cancellation and idle settling have egui coverage.

UI CPU comparison used `python3 scripts/ui-performance.py --base 7dff16f
--query-editor --output target/matrix-connections-performance`, release profile,
Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1. The initial five alternating pairs
flagged workspace/12/true at +12.9% p95, with multiple inconclusive scenarios.
Investigation used the **same compiled binaries and unchanged fixtures/thresholds**
for nine additional alternating pairs, adding the existing Matrix inspection
workload. The workspace flag did not repeat (+8.9%); 20/26 metrics passed and six
remained inconclusive. System storage services and other applications were active;
there were no concurrent builds or competing benchmark runs. **This is not an
overall performance pass.** A quieter-machine run remains necessary.

Median per-run CPU milliseconds from the repeat (base → candidate):

| Workload | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| Workspace, 12 columns, inspector | .293208 → .288167 | .341625 → .372083 | .409042 → .405041 | .409042 → .405041 |
| Editor history/input after idle | .226875 → .225250 | .258417 → .255167 | .297458 → .296667 | .315167 → .318833 |
| Matrix inspection, 760px | .280250 → .278375 | .326166 → .322125 | .385958 → .364125 | .395375 → .399083 |
| Matrix inspection, 1280px | .324167 → .322875 | .369083 → .366375 | .437667 → .420083 | .468542 → .443625 |

Matrix inspection uses 20×30 populated preview cells, 20 warmups and 160 measured
frames at 760×760 and 1280×760, including a time jump for first-after-idle behavior.
The new four-window routing workload (candidate only, 20 warmups / 160 samples)
measured discovery p50/p95/p99/max .002416/.002458/.002500/.002542ms and workspace
inspection .005417/.005500/.007000/.010458ms. This measures local routing only,
not socket round-trip or input-to-display latency. Grid/sidebar fixture details
remain in `docs/UI-PERFORMANCE.md`. Full samples, metadata and original/repeat
results are retained in `target/matrix-connections-performance/`; additional
workload logs and native captures are in `target/matrix-connections-review/`.
Generated review artifacts are not committed.

## Shared-editor verification

The shared-tab extension passed 886 debug library tests (239 opt-in tests ignored).
New coverage includes reading a human tab, active-tab discovery, Unicode range
replacement, ordered chunks, duplicate rejection, the 32 KiB cap, filename and
saved-text preservation, one-step undo, concurrent typing, tab switching/closure,
takeover, manual-run blocking and idle settling while waiting for more chunks.
Existing SQLite execution, Safe mode and multiwindow-routing tests still pass.
Editing adds no database operations or exchanges; execution continues through the
unchanged worker/driver path after the edit finishes.

A real stdio MCP tour read and edited an existing, non-AI-owned tab in four chunks,
kept its title, and verified the complete text without running SQL. Native renderer
captures were inspected during and after streaming at 1280×820 dark and 760×760
light, 2× display scale. The tour uses only a disposable SQLite studio:

```sh
mise exec -- cargo build --features qa
RUSTABLE_MATRIX_EDIT_QA=1 RUSTABLE_MATRIX_CAPTURE=/absolute/path/shared-edit \
python3 scripts/matrix-demo.py --edit-tour --exit-after-tour
```

The QA flag creates a named human-tab fixture; production builds have no fixture
hook. Add `RUSTABLE_MATRIX_LIGHT=1 RUSTABLE_MATRIX_NARROW=1` for the narrow light
capture. Files use `-streaming.ppm` and `-complete.ppm` suffixes. This scripted tour
tests the connector and renderer, not a provider's live token stream. Manual
native keyboard/input-to-present, mixed-DPI, 60/120Hz and display-management checks
remain unverified. Existing user database windows were not restarted for testing.

Final release library suite: **886 passed**, 239 opt-in tests ignored. Clippy
passed with all targets, QA enabled and warnings as errors. The comparison against
`310d581` passed **all 24 metrics**, with no regressions or inconclusive results:

```sh
python3 scripts/ui-performance.py --base 310d581 --query-editor \
  --output target/matrix-shared-edit-performance
```

Same Apple M4 Max / macOS 27.2.1 arm64 / Rust 1.99.1, release profile, five alternating
pairs, unchanged fixtures and thresholds. Builds and benchmarks ran serially;
background OS applications were present. Full environment, p50/p95/p99/max and raw
samples are retained in the output directory. Median per-run CPU milliseconds:

| Workload | p50, base → candidate | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| Editor history/input after idle | .219791 → .221125 | .229250 → .247167 | .235958 → .253875 | .247792 → .256875 |
| Four query results, 1280px | .053166 → .052958 | .059041 → .056000 | .064792 → .060041 | .065666 → .064250 |
| Workspace, 12 columns, inspector | .283459 → .281625 | .302958 → .297041 | .328042 → .314583 | .328042 → .314583 |

The candidate-only `matrix_stream_edit_frame_benchmark` includes chunk admission,
real editor layout/tessellation, change markers and grouped undo while appending
180 SQL comment lines to a human tab. It uses 20 warmups and 160 measured frames
at default headless scale, including a time jump before frame 100:

| Viewport | p50 ms | p95 ms | p99 ms | Max ms | Cold frame ms | First after idle ms |
| --- | --- | --- | --- | --- | --- | --- |
| 760×760 | .179709 | .217166 | .223084 | .450792 | 4.243375 | .219917 |
| 1280×760 | .142333 | .183750 | .186209 | .287583 | .717541 | .153500 |

These are CPU measurements, not sustained FPS or input-to-display latency. The new
streaming interaction has no equivalent on the base. Deep pre-existing undo
histories and very large pre-existing SQL drafts were not benchmarked. Raw new
workload logs, native captures and test results are retained under
`target/matrix-shared-edit-review/`; generated artifacts are not committed.

## Matrix activity bar (after `367111d`)

The permanent Matrix control rows are replaced by a compact AI toolbar menu.
During a streamed edit or an AI-started query, a lightly muted workspace stays
visible below the existing connection bar, which becomes a green character-rain
display with readable activity, connection identity, Safe mode and Stop controls.
Below 1000px, the bar uses the full toolbar width to preserve room for Stop.
Background typing, shortcuts,
tab switching and parked-database navigation are disabled. Stop/Esc revokes the
workspace grant, preserves partial SQL and requests existing worker cancellation;
a running query keeps the input lock until cancellation completes. The stop-click
frame stays locked to prevent click-through. Completing work requests one cleanup
frame. Character rain is bounded to 80 columns with three glyphs each and requests
frames at 20 Hz only while AI work is active. Eight cached glyph layouts are
reused, and offscreen characters are skipped. It stops repainting when work ends.

No database driver, SQL execution, metadata or cancellation protocol sequencing
changed. This adds no database exchanges on cold or warm paths for PostgreSQL,
MySQL/MariaDB or SQLite. Existing Safe mode and destructive-query confirmation
paths remain in charge. The PostgreSQL WAN fixture was not rerun for this UI change.

Native QA captures were reviewed at 1280px dark and 760px light, including a live
MCP streamed edit and the completed, fully interactive editor. Artifacts are in
`target/matrix-overlay-review/`. These use a disposable SQLite demo, never a user
database. Native input-to-present timing, 60/120Hz, mixed-DPI and display-management
checks remain unverified. The currently running user session was left open.

Validation: **887 release library tests passed**, 239 opt-in tests ignored. The
14 focused Matrix tests and all-targets QA Clippy (`-D warnings`) also pass. Tests
cover background typing/shortcuts, Stop hit testing, Escape, query cancellation,
lock release after worker completion, partial-text preservation and repaint
settling after work ends.

The standard release comparison was started with:

```sh
python3 scripts/ui-performance.py --base 367111d --query-editor \
  --output target/matrix-overlay-performance
```

For the final Matrix design, the exact retained base binary was reused and the
candidate rebuilt in the same isolated release target. The same script's
`build`, `measure` and `compare` functions measured seven alternating pairs of
all 24 standard cases and four Matrix cases, with unchanged fixtures and the
10% threshold. `--matrix` now includes those four cases for future runs:

```sh
python3 scripts/ui-performance.py --base 367111d --query-editor --matrix --runs 7
```

Apple M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, release profile. Matrix workloads
use 760×760 and 1280×760 headless viewports at default scale, 20 warmups and 160
measured frames per run, including first use and a time jump before frame 100.
They include request admission, editor layout and tessellation. Builds and
benchmarks ran serially with no competing benchmark workloads; normal background
OS applications remained open. Native captures use 2× scale.

Final median per-run CPU milliseconds (base → candidate):

| Workload | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| Inspect, 760px | .281750 → .278542 | .320166 → .318541 | .350500 → .348084 | .358666 → .363209 |
| Inspect, 1280px | .326042 → .327417 | .365833 → .368750 | .381666 → .395167 | .403375 → .426125 |
| Stream edit, 760px | .133958 → .159875 | .186042 → .236417 | .217958 → .245792 | .340708 → .357209 |
| Stream edit, 1280px | .133292 → .169334 | .187875 → .253583 | .207375 → .279208 | .296667 → .318792 |

The stream-edit p95 increases of **27.1% / 35.0%** are repeatable regressions,
accepted on this experimental branch for the requested animated ownership display
and input lock. The cost is **0.050 / 0.066 ms per measured frame**. This is not
called a pass merely because it fits within a display interval. Investigation
found unnecessary per-character layout requests and offscreen glyph work; the
final renderer reuses eight cached layouts and skips invisible glyphs. The
simpler static indicator avoids the character animation but does not provide the
chosen design; the earlier floating card obscures toolbar content. The selected
bar keeps motion within existing chrome and Stop accessible. Animation repaint
requests stop at completion, so the new effect has no continuing idle animation.
Read-only inspection passes the existing relative threshold at both sizes.

Median cold stream frames were 1.107 → .704 ms (760px) and .745 → .644 ms
(1280px); first after simulated idle was .177 → .160 and .141 → .168 ms.
Worst measured candidate stream-frame stalls across all seven runs were .402 ms
and .367 ms respectively (cold frames are reported separately).

General UI measurements remain partly **inconclusive**, not a clean performance
pass. The final repeat using unchanged binaries produced 16 passing cases and
eight noisy cases: vertical precise scrolling (three metrics), four-result views
at 1280px and 500px, 200/2000-entry sidebars and the 12-column workspace with
inspector. Their median p95 changes ranged from −0.8% to +3.3%, but paired spread
exceeded the unchanged noise limit. No general case met the repeatable-regression
criterion; this does not establish equivalence for the noisy cases.

Raw results, environment, source patch and execution recipe are retained in
`target/matrix-bar-performance-final/`, including both general runs. Earlier
measurements remain in `target/matrix-bar-performance/` and
`target/matrix-overlay-performance/`. These are CPU checks, not GPU presentation,
input-to-display latency or sustained FPS. The native checks listed above remain
unverified; no performance thresholds or fixtures were weakened.

## Toolbar spacing and Stop emphasis (after `44a8328`)

The connection pill previously reserved only the two original right-hand control
groups. Adding the AI menu without reserving its width made those controls overlap
the pill in both idle and working states. The reservation now includes the shared
32-point AI button width and its 6-point gap. Stop is a fully rounded, bold,
high-contrast pill: pale mint on the dark bar, deep green on the light bar, with a
brighter hover/focus treatment from the button interaction visuals.

A rendering-geometry regression test reproduces the old overlap (26 logical
points at 720px) and now passes for idle/working states, light/dark themes,
720/760/1000/1280/1920px widths and 1×/2× scale. It compares painted bar and button
group bounds, not a second copy of the width calculation. The 15 focused Matrix
tests and all-targets QA Clippy pass. Updated native wide dark and narrow light
captures, including the completed idle toolbar, are under
`target/matrix-overlay-review/toolbar-fixed-*`.

Release library suite: **888 passed**, 239 opt-in tests ignored. The four Matrix
CPU workloads were compared against the retained `44a8328` release binary using
the same performance runner helpers, unchanged fixtures and 10% threshold, and
seven alternating pairs. The first inspection run at 760px was noisy/inconclusive;
a focused repeat with unchanged binaries passes both inspection widths. Both runs
are retained. Stream-edit cases pass on the original seven-pair run.

Same Apple M4 Max / macOS 27.2.1 arm64 / Rust 1.99.1 release environment, default
headless scale, 760×760 and 1280×760, 20 warmups and 160 samples per run, including
the existing first-use/after-idle workloads. Builds and benchmarks ran serially.
Median per-run CPU milliseconds, base → candidate:

| Workload | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| matrix/inspect/760 | 0.275500 → 0.277542 | 0.290041 → 0.311541 | 0.298208 → 0.328541 | 0.305083 → 0.351000 |
| matrix/inspect/1280 | 0.327792 → 0.323917 | 0.337917 → 0.355167 | 0.352292 → 0.398625 | 0.390375 → 0.467542 |
| matrix/stream-edit/760 | 0.156958 → 0.162917 | 0.226667 → 0.246250 | 0.232083 → 0.283833 | 0.350666 → 0.372458 |
| matrix/stream-edit/1280 | 0.167709 → 0.173708 | 0.236500 → 0.259042 | 0.242125 → 0.275458 | 0.307375 → 0.336916 |

Final p95 increases range from 5.1% to 9.5%; these meet the existing threshold,
rather than demonstrating zero cost. Raw logs, environment, binary fingerprint,
source patch and recipe are retained in `target/matrix-toolbar-spacing-performance/`.
This layout/style correction changes no database exchanges. Native presentation,
mixed-monitor and 60/120Hz timing checks remain unverified as noted above.
