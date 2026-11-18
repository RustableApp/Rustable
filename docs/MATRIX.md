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
usual Safe mode setting, then click **Enable AI control** in that connection's
window. Nothing is exposed to MCP until you enable it. Redis is not supported.
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

Click **Take over** to stop accepting AI commands and request cancellation of an
active AI query. It does not undo committed writes or automatically roll back a
transaction. Finish any transaction in the normal editor. You can continue editing
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
changed lines and offers **Stop edit** and **Undo AI edit**. Cmd/Ctrl+Z restores the
original in one step while preserving earlier editor history. A single workspace
has one active stream and one latest-edit restoration control. Ordinary editor
undo retains earlier edits.

Typing in the edited tab, closing it, switching away, or taking over stops the
stream. Human changes are never silently overwritten or undone. Stopped partial
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
