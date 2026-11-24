# Query protection and grouped deletes

The lock menu controls confirmation at execution time. It does not disable local
cell edits, row insertion/deletion, or structure drafts.

| Mode | Execution behavior |
| --- | --- |
| Silent Mode | Send immediately without a dialog |
| Alert Mode 1 | Confirm every user-run query and reviewed save/import |
| Alert Mode 2 | Confirm except for SELECT, EXPLAIN and SHOW scripts |
| Safe Mode 1 | Authenticate every user-run query and reviewed save/import |
| Safe Mode 2 | Authenticate except for SELECT, EXPLAIN and SHOW scripts |

Exemptions inspect the entire script using the active SQL dialect. Mixed scripts,
write CTEs, SELECT INTO, executable MySQL comments, parsing failures and EXPLAIN
of writes prompt conservatively. SELECT functions can have effects: these modes
are confirmation preferences, not a replacement for database permissions.
Background catalog reads, browsing and plan generation do not prompt.

Save prepares and validates changes, then uses the execution gate directly: one
confirmation in Alert modes, one authentication in Safe modes, none in Silent
mode. The explicit Preview button opens the same dialog in every mode, including
Silent. Send displays Cmd+Enter (Ctrl+Enter elsewhere); a fresh press submits,
with password validation in Safe modes. The opening key event and key repeats
cannot approve. Cancel retains drafts; retry generates a fresh plan. The old
Review changes window is removed. Schema checks, concurrency checks and atomic
transactions are unchanged; this UI routing adds no database exchanges.

Production connections start in Alert Mode 2; others start in Silent Mode.
Modes reset on reconnect. Credentials are captured only after successful
connection, retained in zeroizing session memory, and never serialized into
preferences. Password confirmation is local, against that connection's password;
it creates no authentication connection or database round trip. Passwordless
connections can use Alert modes, or Safe modes with Touch ID enabled on macOS.

Settings is available through the macOS application menu and Cmd+, (Ctrl+, on
other platforms). Theme previews apply immediately across windows. The Security
panel enables Touch ID as an alternative to the database password. Each request
uses a fresh LocalAuthentication context with biometric reuse disabled. Cancel,
failure and unavailable biometrics never execute the held operation. Dismissing
the prompt invalidates its biometric context, preventing a late callback from
approving another operation. Authentication is never remembered for subsequent
queries. The password remains available if biometrics fail.

The worker's enforced read-only execution flag remains independent and tested.
MCP requests use the same confirmation gate and cannot approve prompts. Revoking
AI access discards its pending confirmation. Cancelling an editor prompt does
not replace existing result provenance or add unsent SQL to history.

## Delete generation

Adjacent deletes for the same table share a parameterized DELETE with grouped OR
predicates. Composite keys retain parentheses and PostgreSQL parameters are
numbered across the complete statement. Updates for a single row were already
combined; this change targets the redundant per-row DELETE statements.

Each original-row check remains immediately before its corresponding write,
inside the existing transaction. The grouped write must affect exactly the
number of reviewed rows. Stale rows, suppressed writes, conversion warnings,
foreign-key failures and cancellation retain the existing rollback behavior.
There is no write retry. Inserts and operations on different tables are not
reordered. Batches stop at 128 rows, or at the existing statement reaching 512
parameters / 64 KiB of SQL before appending another row.

PostgreSQL falls back to separate deletes when user triggers can fire on the
target or along its DELETE CASCADE / SET NULL / SET DEFAULT paths. This catalog
check is included in an existing metadata response and fingerprint. SQLite
conservatively uses separate deletes when any main/temporary trigger exists,
and rejects a grouped plan if a temporary trigger appears after review. MySQL
and MariaDB retain their existing trigger/cascade validation and reject unsafe
managed-write paths.

## Protocol investigation

Established healthy PostgreSQL connection, built-in/cached metadata types, one
table, N deleted rows and B batches:

- Planning is unchanged: four metadata queries, each prepare then execute
  (eight dependent exchanges).
- Apply: BEGIN (one), table lock (one), four metadata queries (eight), original
  checks (2N), deletes (previously 2N, now 2B), COMMIT (one).
- Eight compatible rows therefore change apply from 43 to 29 dependent exchanges.
  The same count applies on first and repeated saves: these APIs prepare each
  statement from SQL rather than caching the reviewed statement handles.
- Statement Drop enqueues Close+Sync; it is driven alongside the next request,
  without a separate application await. Transaction status comes from the
  existing protocol state. Unknown status retains the existing Sync drain.
  Cancellation-worker teardown unparks/joins locally on successful operations.
- Editor and browse paths are unchanged. Custom-type metadata resolution and
  explicit-transaction refusal retain their existing behavior.

MySQL/MariaDB retain their prepared-statement cache. For identical old per-row
DELETE SQL, the delete portion costs N+1 cold exchanges (one prepare plus N
executions) and N warm; a compatible grouped DELETE costs two cold and one warm.
Original checks, metadata/schema validation, transaction boundaries and cleanup
remain unchanged. SQLite has no network exchanges. The confirmation menu and
password/Touch ID checks add no database traffic.

## Validation

Local artifacts are in `target/protection-review/` and
`target/protection-validation/`. PostgreSQL experiments use only the disposable
port-54329 fixture; MySQL and MariaDB use the disposable 43316/43317 fixtures.
The complete save benchmark compares equivalent eight-row deletes through the
actual driver with 15 ms delay in each direction, alternating five runs of each
plan. See the retained raw log for first-use and repeated samples.

Native captures cover the lock menu, settings, and authentication prompt in dark
and light themes, including a 500-point-wide window. Hardware Touch ID approval,
failure/cancellation, input-to-present timing, 60/120 Hz, mixed-DPI, monitor
reconnection and multiple-window native animation were not measured.

### Recorded results (2026-11-23)

- Final non-ignored library suite: 942 passed, 259 ignored. The obsolete
  destructive-only editor classifier/tests were replaced by whole-script mode
  checks; generated context-action reviews retain their bounded descriptions.
- Strict all-target Clippy with `qa`, and the native QA build, passed.
- Six opted-in PostgreSQL editing/schema tests passed, including the new cascade
  trigger/review-invalidation case. The grouped-delete live test passed on both
  MySQL and MariaDB. Existing editor/browse WAN regressions passed.
- Complete eight-row PostgreSQL apply at simulated 30 ms RTT: separate median
  **1,577.19 ms**, grouped median **1,064.08 ms** (32.5% lower), five alternating
  runs each. Concurrency conflict and explicit-transaction refusal also passed.

Release UI comparison used base `994ee8df60b096676b897a644ff2d16d3dac7926`, Apple
M4 Max, macOS 27.2.1 arm64, Rust 1.99.1, five alternating paired runs. The toolbar
fixtures use 20 warmup / 160 measured frames at 500/760/1280-point widths and
760-point height, light/dark and idle/busy states, default headless scale. Query
fixtures use the same source on base and candidate, 20 warmup / 160 measured
frames, and include a 30-second simulated idle jump. These measure CPU layout
and tessellation, not display latency.

The initial run was noisy. A second five-pair run reused the same executables,
fixtures and thresholds without competing builds: **18 passed, seven
inconclusive, zero repeatable regressions**. This is not an overall performance
pass. Initial increases above 10% did not recur in the same scenarios. The raw
p50/p95/p99/max and sorted samples are retained in
`target/protection-ui-performance/` and `target/protection-ui-repeat/`.

Representative repeated p95 values (milliseconds):

| Workload | Base | Candidate |
| --- | ---: | ---: |
| Light idle toolbar, 1280 | 0.01625 | 0.01608 |
| Dark busy toolbar, 1280 | 0.02379 | 0.02208 |
| Light busy transaction toolbar, 1280 | 0.01904 | 0.01888 |
| Empty editor/results, 1280 | 0.03817 | 0.03833 |
| Four editor results, 1280 | 0.05413 | 0.05442 |

New controls have no equivalent baseline. Five candidate-only runs each use
20 warmup / 160 measured frames at 760×760, including after-idle rendering and
password-entry changes. Median per-run values, in milliseconds:

| New workload | p50 | p95 | p99 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| Settings | 0.02092 | 0.02350 | 0.02600 | 0.03004 |
| Lock menu | 0.00713 | 0.00733 | 0.00750 | 0.00758 |
| Password prompt | 0.01392 | 0.01492 | 0.01783 | 0.02921 |
| Password prompt, bounded 32K-character SQL preview | 0.26025 | 0.27629 | 0.28554 | 0.30529 |

### Unified confirmation follow-up

The follow-up removes the automatic Review changes window and routes explicit
Preview through the execution dialog. Validation: 941 library tests passed,
259 ignored; strict all-target Clippy with `qa` passed. Tests exercise Silent
Save versus explicit Preview, Alert Save, fresh Cmd+Enter, opening-key/repeat
suppression and incorrect-password rejection. Native captures were inspected at
wide/dark and narrow/light sizes in `target/protection-review/confirmation-shortcut*.png`.
Database execution and protocol sequencing did not change in this follow-up.

Five alternating release benchmark pairs compare the previous built protection
UI against this follow-up on the same machine, using the unchanged fixture and
`ui-performance.py` parser/thresholds. Raw logs and comparisons are retained in
`target/confirmation-performance/`. This is a comparison against the preceding
implementation, not a new PR-base comparison (the PR-base checks above remain
separate). Password-dialog CPU layout/tessellation p95 rose from 0.016792 ms to
0.019083 ms (+13.6%, 0.002291 ms); four of five pairs exceeded 10%. This small
regression is accepted for the requested visible shortcut, rendered with the
shared macOS command/return artwork, and fresh-key confirmation handling. Plain
text or omitting the shortcut would reduce rendering work but lose the consistent
shortcut presentation requested here. No continuous redraw was introduced.
Large-preview p95 improved from 0.290375 to 0.277292 ms; settings was unchanged
within threshold, and the untouched menu was noisy/inconclusive. These are CPU
measurements, not input-to-present measurements; native frame timing and actual
Touch ID hardware authentication were not measured in this follow-up.
