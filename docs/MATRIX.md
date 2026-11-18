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
- `workspace`: inspect the selected connection, transaction status, tabs, and a
  bounded preview of the already-loaded catalog.
- `open_query`: create an AI-owned tab with a title and SQL, without executing it.
- `replace_query`: update an AI-owned tab only when `expected_sql` matches.
- `run_query`: activate and run an AI-owned tab through the existing editor.
  Requires matching `expected_sql`; returns immediately. Poll `inspect_query`
  for completion without tight loops.
- `inspect_query`: SQL, error/running/confirmation status and selected result
  preview, capped at 20 in-memory rows × 30 columns. Spilled rows are not loaded;
  an empty preview does not prove the result is empty.
- `cancel_query`: request cancellation of an active AI-owned query.

All tools except discovery require the exact `workspace_id`. IDs change when
access is revoked. The client must ask the user to reenable the intended target;
it must not substitute a different connection. Closing a window invalidates it.

SQL uses the connection's existing database privileges. A workspace ID routes
commands; it is not a database ACL or SQL sandbox. Safe mode and production
protections use the normal Rustable implementation. When Safe mode is off, writes
are possible; destructive-query dialogs still require the user. The MCP bridge
cannot change Safe mode, approve dialogs, open saved connections, overwrite
human-owned tabs, or close tabs. It returns query data to the connected AI client,
which may send that data to its model provider.

Human changes to AI SQL invalidate stale replacements and executions. At most
16 AI tabs are allowed per workspace. Busy workspaces and outstanding edits or
dialogs refuse navigation commands. The bridge does not replay failed writes;
a client must never retry a write whose commit outcome is unknown automatically.
Credentials stay in Rustable's existing connection system and are not tools output.
SSH/TLS setup uses the normal connection workflow without a separate connection.

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

## Verification (2026-11-17)

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
