# Matrix experiment

Branch: `experiment/matrix-mcp`. This is a working prototype of an MCP client
controlling the **visible Rustable editor**. It is separate from PR #10's headless
database server. It starts with the disposable SQLite studio database and does
not load saved connections or credentials.

## Try the experience

From this branch's checkout on macOS or Linux:

```sh
./scripts/matrix-demo.sh
```

A real Rustable window opens. The scripted tour uses the stdio MCP connector to
open three named SQL tabs, execute customer/revenue queries, and inspect their
results. It pauses between actions so you can watch. This tour is scripted, not
an AI model; any compatible MCP client can use the same tools.

Click **Take over** and continue editing or running any of the queries. This
pauses new AI commands and cancels an outstanding AI query. **Enable AI control**
lets the client resume. Normal Safe mode and destructive-query confirmations
remain in force. SQL and results stay in the live window after the client exits.
The disposable database and this demo's tabs reset when the window closes;
use the normal SQL Save action to keep queries on disk.

## Connect an AI client

```sh
./scripts/matrix-demo.sh --no-tour
```

The launcher prints an MCP configuration with the exact executable and private
socket for that running window. Add that configuration to your MCP client. The
socket changes each launch; this prototype does not install persistent client
configuration. A useful prompt is:

> Use Rustable Matrix to explore revenue by product. Open clearly named query
> tabs, run the analysis, and leave a useful follow-up query for me to edit.

The underlying commands are:

```sh
matrix_dir=$(mktemp -d)
target/debug/rustable matrix --socket "$matrix_dir/workspace.sock"
# In a second terminal, use the actual path printed/created above:
target/debug/rustable mcp --socket /private/directory/workspace.sock
```

The socket's parent directory must be private (0700); the socket is 0600. It is
local to this computer and is not a network server. Treat any client with access
to that socket as able to act in this experiment's workspace.

## Tools and boundaries

- `workspace`: inspect connection, Safe mode, tab IDs, and up to 200 catalog objects.
- `open_query`: create an AI-owned tab with SQL and a title. Never executes it.
- `replace_query`: update an AI-owned tab only when `expected_sql` matches.
- `run_query`: activate and execute the tab using the existing editor dispatch.
  Returns immediately; poll `inspect_query` for completion, without tight loops.
- `inspect_query`: SQL, running/error/confirmation status, and a preview of the
  currently selected result, capped at 20 in-memory rows × 30 columns. Spilled
  rows are not loaded. An empty preview is not proof that the result is empty.
- `cancel_query`: request cancellation of the active AI-owned query.

The Matrix window starts with the row inspector hidden so query tabs and results have room at laptop widths. You can reopen it with the usual toolbar control.

The bridge cannot change Safe mode, approve a dialog, overwrite a human-owned
tab, close a tab, or select a saved connection. Human edits to AI SQL invalidate
stale replacement/execution requests. At most 16 open AI tabs are allowed.
A busy workspace or outstanding edits/dialogs refuse navigation commands.
Requests are capped at 128 KiB, SQL at 32 KiB, and complete response serialization
has a 128 KiB limit. UI admission expires after three seconds. Socket work and
serialization run on a background thread, with a bounded queue and no blocking
send on the UI thread. Reads and accepted commands wake the window on demand;
there is no perpetual repaint timer.

Execution uses `run_query` → `execute_query` → the existing database worker.
There are no new catalog, health-check, prepare, transaction-status, or cleanup
queries in this bridge. `workspace` and `inspect_query` read UI state locally.
The demo currently exercises SQLite; PostgreSQL/MySQL/SSH integration and
multiwindow routing are deliberately outside this first experiment.

## Verification

Tests cover stale-SQL protection, human-tab ownership, paused/expired admission,
message bounds, and real editor execution with Safe mode refusing writes.
The launcher itself is an end-to-end stdio MCP client, rather than a direct
call into the handlers. `--exit-after-tour` closes the window after a QA run.

Native QA captures can be generated with a `--features qa` build:

```sh
RUSTABLE_MATRIX_CAPTURE=/absolute/path/matrix.ppm python3 scripts/matrix-demo.py --exit-after-tour
# Add RUSTABLE_MATRIX_LIGHT=1 RUSTABLE_MATRIX_NARROW=1 for light, 760px coverage.
```

Performance and visual verification results are recorded below.

The bridge also refuses actions if the window is manually switched away from
the disposable studio database. This first branch is for exploring the interaction,
not for granting an assistant access to saved or production connections.

### Results (2026-11-17)

- Final library suite: 880 passed, 235 opt-in tests ignored, including seven
  focused Matrix regression tests. The full suite also passed in release before
  the final transport-envelope budget test was added (879 passed).
- Real stdio tour: all three queries completed (3, 4 and 20 rows), including draft
  replacement. Native captures inspected in dark at 1280×820 and light at 760×760,
  2× display scale. The row inspector defaults to hidden in this experiment.
- `cargo clippy --all-targets --features qa -- -D warnings` passed.
- `python3 scripts/ui-performance.py --base 0eadbb8 --query-editor --output
  target/matrix-performance`: all 24 existing metrics passed. Apple M4 Max,
  macOS 27.2.1 arm64, Rust 1.99.1, release profile; five alternating pairs on the
  same machine. Timing was held until competing builds finished. Same fixtures
  on both revisions, default headless scale, unchanged thresholds. Full p50/p95/
  p99/max and raw samples are retained in `target/matrix-performance/results.json`.

Selected median-per-run CPU measurements, milliseconds (base → candidate):

| Workload | p50 | p95 | p99 | Maximum |
| --- | --- | --- | --- | --- |
| Editor history/input after idle | .219667 → .219292 | .223000 → .223333 | .228958 → .229625 | .234083 → .232333 |
| 4 results, 1280px | .053041 → .053167 | .059542 → .056625 | .062542 → .059958 | .062959 → .061209 |
| 128 results, 760px | .055250 → .056000 | .060292 → .061292 | .064250 → .063417 | .065041 → .065708 |

These query fixtures have 20 warmup and 160 measured frames per run, including
an idle time jump; see UI-PERFORMANCE.md for grid/sidebar sample counts and
viewports. They protect existing rendering and input CPU costs, not the new MCP
workflow. The additional `matrix_frame_benchmark` measures the new action and
status bar in a populated workspace, with a 20×30 preview of 210-character cells,
20 warmups and 160 measured frames at both 760×760 and 1280×760. It also prints
cold and first-after-idle frame costs. This is candidate-only evidence because
there was no corresponding MCP action on the base.

The Mac was locked, so direct native mouse/keyboard testing could not run.
Takeover/cancellation and idle settling were verified through egui input tests;
native captures came from the actual renderer. External AI-client integration,
native input-to-present latency, missed frames, 60/120 Hz behavior, multiple
windows, mixed DPI, display reconnect and snapping remain untested. No PostgreSQL
or MySQL latency runs were made for this demo-only connector; driver and protocol
paths are unchanged.

Raw local review artifacts: `target/matrix-review/` and
`target/matrix-performance/`. These generated files are not committed.

Populated Matrix inspection (candidate only, release, milliseconds):

| Viewport | p50 | p95 | p99 | Maximum | Cold frame | First after idle |
| --- | --- | --- | --- | --- | --- | --- |
| 760×760 | .310916 | .376584 | .412958 | .415834 | 4.465833 | .315375 |
| 1280×760 | .315292 | .325500 | .335458 | .353959 | 1.171084 | .315250 |

This single absolute run is retained as `matrix-populated-workload.log`; it is
not a comparative performance gate or an input-to-display measurement. The earlier
empty-result probe is retained separately as `matrix-new-workload.log`.
