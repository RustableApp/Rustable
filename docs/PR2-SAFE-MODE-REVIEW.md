# PR #2: Safe mode and destructive-query confirmation

The review branch includes current main plus fixes to confirmation and
MySQL/MariaDB Safe mode. Production connections start in Safe mode on all four
SQL engines; other environments start with it off.

## Confirmation fixes

- Executed PostgreSQL `EXPLAIN ANALYZE` and `EXPLAIN (ANALYZE TRUE)`
  inspect the underlying statement. Plain EXPLAIN does not warn.
- Unparsed procedural SQL containing DELETE/UPDATE warns conservatively,
  even if another statement contains WHERE. Quoted procedural bodies are not
  stripped as though their contents were outer SQL comments.
- MySQL `/*!` and MariaDB `/*M!` executable comments are inspected.
  Their appearance inside a string literal does not cause a warning.
- MySQL parsing respects the connection's ANSI_QUOTES and
  NO_BACKSLASH_ESCAPES settings.

Confirmation remains a mistake-prevention aid, not a proof that arbitrary
functions, procedures, dynamic SQL or triggers cannot modify data. Unknown
procedural syntax can produce conservative extra confirmations.

## MySQL and MariaDB execution

Each validated read is sent as a single COM_QUERY batch:

```sql
START TRANSACTION READ ONLY;
-- exactly one validated user read statement
ROLLBACK AND NO CHAIN NO RELEASE;
```

The driver negotiates multi-statement support already. There is no prepare,
describe, preflight ping, or successful-path cleanup exchange. START/ROLLBACK
results are drained and checked but hidden from the grid and excluded from its
memory/result-set budget. Custom DELIMITER statements and trailing line comments
cannot swallow the cleanup command.

Syntax validation rejects direct writes, implicit-commit operations, CALL,
executable comments, unrecognised statements and SELECT INTO/file output.
The server's read-only transaction also rejects persistent-table writes through
a stored function called by SELECT.

A pre-existing transaction is never implicitly committed or rolled back by
the wrapper. Safe reads require an idle status from an already-consumed server
response; an active or unknown state asks the user to finish the transaction.
Explicit COMMIT/ROLLBACK remain available and intentionally affect that existing
transaction. Turning on Safe mode does not undo previously staged SQL writes.

On error, MySQL stops the batch before its trailing ROLLBACK. After the
cancellation watcher stops, the driver sends a separate rollback for its owned
transaction. Failed cleanup discards the connection. It never replays a query.
NO CHAIN/NO RELEASE preserve behavior even when the session's completion_type
is set to chain or release. Autocommit is not changed.

This is not a security sandbox: read-only transactions have database-defined
exceptions for temporary tables, and reads can still have session-level or
external function side effects. Use least-privilege database credentials for
hard access control. Browse/catalog paths are unchanged.

References:
[MySQL read-only transactions](https://dev.mysql.com/doc/refman/8.4/en/commit.html),
[MariaDB transaction semantics](https://mariadb.com/docs/server/reference/sql-statements/transactions/start-transaction).

## Network budget and measured validation

On established healthy connections with session-state tracking:

| Path | Prior ordinary execution | New ordinary execution | New Safe mode |
| --- | --- | --- | --- |
| First read | 1 exchange | 1 | 1 |
| Repeated read | 1 | 1 | 1 |
| Rejected syntax | n/a | unchanged | 0 |
| Failed safe batch | n/a | unchanged | 2 (query + owned rollback) |

Safe mode previously refused every MySQL/MariaDB query. The ordinary path is the
performance comparator, not a claim that the formerly unavailable feature had
zero latency. Existing session-refresh fallbacks for unsupported tracking/dirty
sessions are unchanged. Cancellation opens a separate control connection on
demand; successful reads do not open one.

The error-only extra exchange is necessary because the server skips remaining
batch statements on error. Sending cleanup in the original batch already handles
success, but cannot handle this case. Closing the session instead would destroy
useful state and require reconnection. Cleanup is never applied to a transaction
that predates the safe batch.

Local debug tests, macOS 27.2 ARM64, Rust 1.99.1, disposable Docker servers,
30 ms simulated network RTT, 12 serial paired samples per engine:

| Server | Ordinary first / median / max | Safe first / median / max |
| --- | --- | --- |
| MySQL 8.4.11 | 40.17 / 41.51 / 45.38 ms | 47.64 / 40.58 / 47.64 ms |
| MariaDB 11.4.13 | 37.52 / 39.40 / 43.64 ms | 41.87 / 37.69 / 43.13 ms |

The proxy counts actual client protocol commands, including hidden pings or
cleanup: exactly one on first/repeated successful reads and two after a failed
safe batch. Timings are noisy local end-to-end measurements, not server timings,
production SSH measurements or UI p95. Failed safe reads including the extra
rollback measured 79.02 ms on MySQL and 79.54 ms on MariaDB (one sample each).
Client-side result errors do not add cleanup when the server already reports idle.

Reproduce against the disposable fixtures only:

```sh
RUSTABLE_MYSQL_TEST_PORT=43316 mise exec -- cargo test --lib db::mysql::safe -- --include-ignored --nocapture --test-threads=1
RUSTABLE_MYSQL_TEST_PORT=43317 mise exec -- cargo test --lib db::mysql::safe -- --include-ignored --nocapture --test-threads=1
```

Live coverage includes direct and function-mediated writes, executable comments,
SELECT INTO, empty results, 128 result sets, custom delimiters, transaction
preservation/recovery, errors, cancellation and reuse, autocommit=0,
completion_type=1/2 and ANSI_QUOTES/NO_BACKSLASH_ESCAPES.

## Regression checks

- Full library suite: 599 passed, 133 opt-in tests ignored.
- MySQL/MariaDB Safe mode checks: four passed on each server, including the
  opt-in delayed-network and cancellation tests.
- Clippy with `--lib --tests -- -D warnings`: clean.
- `git diff --check`: clean.

UI CPU comparison against main `aece507`: five paired release runs, followed
by five more pairs with the exact same cached binaries and no competing builds.
The first comparison passed 18 of 19 metrics; sidebar/20 was noisy
(base/candidate median p95 0.0287/0.0244 ms). The repeat passed sidebar/20 but
other cases became noisy/inconclusive, including workspace/12 with inspector
(0.2837/0.3196 ms) and workspace/120 with inspector (0.4128/0.4606 ms).
No repeatable regression was established, but the overall UI performance result
is **inconclusive**, not a pass. No thresholds were relaxed.

Raw local evidence: `target/pr2-ui-performance/` and
`target/pr2-ui-performance-repeat/`; network/unit/clippy logs:
`target/pr2-verification/`. The later cleanup-condition change only affects
database error handling, not the rendered UI measured by those binaries.
Native presentation latency, mixed-DPI displays, Windows/Linux UI, other server
versions and production SSH behavior were not exercised for this PR.

## Toolbar follow-up

The left toolbar now has three stable groups: sidebar/connections,
discard/preview/save, then Safe mode/database/SQL. Safe mode uses closed/open
lock artwork with its state in the tooltip and accessibility label. The sidebar
toggle persists as a preference. Pending actions moved out of both Data and
query-result footers; right-clicking Preview retains individual change removal.
Save continues to open the existing review step.

Existing shortcuts remain CMD/Ctrl+S for Save, CMD/Ctrl+Shift+P for Preview,
and CMD/Ctrl+Shift+Backspace for Discard. Preview is now consumed before the
less-specific CMD/Ctrl+P palette shortcut, fixing egui's extra-modifier matching.
The result-edit test now checks both Save and Preview with active/staged cells.

Validation: 599 library tests pass; Clippy is clean. Native macOS demo verified
the grouped layout, sidebar hide/show, active actions after editing, and the
Preview shortcut opening Review changes. Screenshot: `target/toolbar-preview.png`.
No user database was used.

Fresh release candidate versus the existing main baseline, five paired runs:
all four toolbar-inclusive workspace cases passed in the first comparison
(p95 changes +5.2%, -5.5%, -0.5%, -5.4%). An isolated grid tessellation metric
flagged +27.5%; that fixture does not render the toolbar. With disposable native
previews closed, its exact-binary repeat changed to -1.4%, so the isolated
flag was not reproducible. Several other metrics remained noisy/inconclusive;
the overall result is still not a clean performance pass. No thresholds changed.
Raw results are in `target/toolbar-ui-performance/` and
`target/toolbar-ui-performance-repeat/`. Both runs compare the same cached
baseline executable against the newly rebuilt candidate, without competing builds.

### Command palette and cancellation follow-up

- Command-P opens the local palette without fetching database names or putting the
  workspace into a loading state. Command-K fetches the database list on first use
  and caches it until a schema refresh or a new connection.
- Cancel buttons in the SQL footer and compact menu show Command-period (Ctrl-period
  on other platforms). The PostgreSQL monitor preselects this workspace's backend,
  preserves an explicitly selected query across refreshes using its full identity,
  and does not select another backend when the current query has ended.
- Cancel selected query uses the shared blue shortcut-button styling. Return or
  Command-Return invokes it when enabled; Escape dismisses the monitor. Existing
  server-side identity checks still protect against cancelling a replacement query.
- The backend PID comes from the existing startup cancellation token. This adds no
  query or exchange to connection setup, execution, table loading, or cancellation.
- Regression tests cover palette dispatch suppression, cached database lists,
  selection identity/reordering, and the startup PID matching PostgreSQL's own PID.

Follow-up verification: 601 unit tests passed (133 ignored); the opted-in disposable
PostgreSQL monitor test passed, including stale-query rejection, selected-query
cancellation, and the startup PID assertion. Clippy with warnings denied and
`git diff --check` passed. Five paired release-mode runs against main `aece507`
passed all 19 existing CPU UI metrics: workspace p95 changed +2.2%, +0.4%, +3.4%,
and -1.4%; all other metrics stayed within threshold. Raw report:
`target/cancel-ui-performance/`. This suite covers workspace/grid/sidebar CPU
rendering, not cancellation-dialog input-to-display latency. Native dialog,
mixed-DPI, and Windows/Linux visual checks were not run for this follow-up.
