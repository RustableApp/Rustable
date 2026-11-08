# Redis support

Redis is included in v0.3.1 for standalone servers. Earlier releases do not
include Redis support.

## Connection and browsing

Choose Redis in New connection. Configure a standalone server, logical database
number, optional ACL username/password, and TLS. TLS is enabled by default and
verifies both the certificate chain and hostname; a PEM CA file can be supplied.
Saving a connection stores its password in the operating-system credential store.
Passwords are never serialized into the connection profile.

The connection test distinguishes transport/TLS, authentication, and database
selection failures. Connect and read/write timeouts default to five seconds.
The workspace starts read-only. Network requests and reply decoding run on a
dedicated worker, away from the UI thread.

The workspace uses a database sidebar, a key/value/type/ttl data grid, and a
resizable row inspector on the right. Select a row to inspect or edit it.
The command pane starts closed; toggle it from the toolbar. The database toolbar
button opens the logical database selector.

The key browser uses incremental SCAN with a glob pattern and a Load more caret.
The search icon reveals the key-pattern filter. Grid previews are fetched in
pipelines, in batches of 200 keys, with short string/collection previews.
Grid values are previews, not complete exports: strings show up to 256 bytes,
collections show a small sample, and rendered text is capped at 256 characters.
Select a row for the larger inspector preview. TTL is shown in seconds (-1
means no expiry; -2 means the key disappeared).
It deduplicates results, but a scan is not a consistent snapshot when other
clients change keys. It retains up to 10,000 keys or 16 MiB of key names;
narrow the pattern to browse more. It never issues KEYS automatically.

The inspector supports strings, hashes, lists, sets, sorted sets, and streams,
including binary key names and values. It shows type, length, and TTL at load
time. Refresh retrieves current data. String previews are capped at 64 KiB;
collection requests use range limits or SCAN hints. Retained inspector values
are capped at 256 KiB/1,000 nodes, with partial previews clearly marked.
Redis SCAN COUNT is a hint, not a hard limit, and a single collection element
may itself be large. Replies must be decoded before display limits are applied:
these limits are not a network-memory cap.

### Sentinel connections

Select **Sentinel** under Deployment. Enter the monitored service name and one to
eight Sentinel addresses. The discovery section has its own ACL credentials, TLS
verification and optional CA file. The Primary database section configures the
logical database and credentials/TLS used with the discovered Redis primary.
Both passwords are stored separately in the operating-system credential store.

Test and Connect discover the primary and verify its role before exposing the
connection. MONITOR, subscriptions and channel queries also discover and verify
the primary on their dedicated sockets. Explicit reconnect discovers again. An established workspace makes one automatic
transport recovery attempt after connection loss, preserving drafts and the error
without replaying the operation or refreshing displayed data. If it fails, use
Reconnect. Dedicated MONITOR/Pub/Sub streams still require an explicit restart. No uncertain
write is automatically replayed. Sentinel addresses must be reachable from this
computer, as must the primary address returned by Sentinel.

## Commands and editing

The command pane accepts one command per line, with quoted arguments and
binary escapes such as \x00 and \xff. Semicolons are literal characters,
not SQL delimiters. Up to 100 commands / 1 MiB of input can run at once.

Examples (turn off Read-only before writing):

    SET "demo:greeting" "Hello from Rustable" EX 3600
    GET "demo:greeting"
    HSET "demo:customer" "name" "Alice"
    LRANGE "demo:queue" 0 99

Command output is a bounded preview. Use GETRANGE, LRANGE, HSCAN, SSCAN,
ZSCAN, or bounded XRANGE requests for large values.

Full strings up to 64 KiB can be edited in the inspector. Save uses an atomic
compare-and-set script, refuses a stale/expired value, and preserves its TTL.
Unsaved edits must be saved or discarded before switching keys/databases,
refreshing, running another operation, or closing the workspace.

TTL changes and confirmed key deletion are available after unlocking editing.
Other types can be created/modified through Redis commands. These operations
apply immediately; they do not use SQL's pending-change review workflow.

Read-only mode permits a conservative list of read commands. Unknown commands
and scripts require unlocking. It is an application guard, not a replacement
for Redis ACLs. The server account must allow PING and SELECT, plus the commands
needed for browsing. String saves require EVAL and its TYPE/GET/SET operations.

Protocol-changing commands, transactions, pub/sub, MONITOR and blocking commands
are not supported in the editor. Choose databases through the toolbar.
Batches execute sequentially, not atomically; errors report how many preceding
commands completed. A disconnect or timeout never automatically retries a write.
A timed-out write may have completed: verify it before retrying. Reconnect is
explicit.

## Scope and verification

The tested server is standalone Redis 7.4, including password and ACL
authentication, passwordless ACL users, verified TLS, invalid CA/hostname
rejection, database selection, all six core value types, binary/empty/large
values, scan pagination, expiry, stale edits, read-only guards and timeouts.
RedisJSON and Search integration paths are additionally tested against Redis 8.2.
Vector-set fixtures additionally use Redis 8.4. Other server/version combinations have not been certified.

v0.4.19 adds Cluster and Sentinel connections, bounded Pub/Sub and profiling,
reviewed archives, time-series browsing and Redis 8.4 vector-set inspection.
See the sections below for routing, recovery, module-version and sampling limits.
Built-in Redis SSH tunnels, client-certificate authentication and persistent
automatic command history are not included. Redis server connections are
desktop-only; the browser offers a bounded in-memory Redis-compatible demo.

The browser now supports structured collection mutations, collision-safe creation
and rename, explicit bulk delete/expiry, RedisJSON documents, saved layout and
named snippets, on-demand server/slow-log/memory reports, and read-only Search
index/schema browsing with sortable field results, aggregations and query profiles.
The native Actions menu also offers an explicit, bounded live command profiler
with Start/Stop, follow-latest, retention/drop counts and verified TLS.
Pub/Sub supports exact/pattern subscriptions, active channels and subscriber
counts, highlighted JSON/Text/Hex previews, and text/hex publishing guarded by
read-only mode. Streams use separate connections and bounded message retention.
Open Search from the Actions menu. See
[REDIS-ROADMAP.md](REDIS-ROADMAP.md) for exact limits, protocol budgets and validation.
Snippet persistence is explicit; ordinary command history is session-only.

The database-sidebar width, inspector width, and command-pane height use the app's saved pane preferences.

## Reproduce integration tests

Run `./scripts/test-redis.sh` with Docker, OpenSSL and mise installed. It starts
local Redis 7.4 plaintext/TLS fixtures on ports 16379/16380 and a Redis 8.2
JSON fixture on port 16381, runs the suite,
and removes its containers and generated certificates on exit. All three ports
must be free; it never reuses an existing server.

For manual fixture setup, the plaintext tests
expect Redis 7.4 on 127.0.0.1:16379 with password
'rustable-redis-test-only'. TLS tests expect port 16380 and a CA certificate at
/tmp/rustable-redis-tls/ca.crt, with a server certificate whose SAN includes
127.0.0.1 but not localhost. The fixtures must permit ACL administration.

    mise exec -- cargo test --locked --lib redis_session::tests -- --include-ignored

Tests create their own prefixed keys/users. Run only against disposable fixtures.

Verification on 2026-11-01: 471 native library tests passed; the six-test Redis
suite passed against freshly created Redis 7.4 plaintext/TLS containers.
Strict native all-target Clippy passed. The WASM check passed with its existing
52 browser-only warnings. The optimized desktop build passed. Native UI checks
covered connection testing, key selection, command results, string-save
feedback and protection of unsaved edits. A synthetic pointer-drag regression
test verifies pane-width persistence across UI contexts.

The Redis layout was checked against TablePlus using the same disposable Redis
fixture. Follow-up UI verification covered grid selection, hash previews and the
right-hand inspector; all six integration tests also cover grid metadata reads.

## Browser demo

The website connection manager offers **Studio demo** (real SQLite) and
**Redis demo** (a bounded in-memory Redis-compatible store implemented in Rust,
compiled to WASM). Both retain their state when switching; reload resets them.
The Redis demo reuses the desktop key grid, inspector, TTL controls, database
picker and command editor. It seeds strings, a hash, list, set and sorted set.
Run `INFO` for the supported command subset. Unsupported commands/options return
an explicit error; this is not a full Redis server or a remote Redis connection.
Read-only mode starts enabled. Disable it to try string edits, SET, DEL, EXPIRE
and PERSIST. Key searches support `*` and `?`. The store is capped at 16 logical
databases, 256 keys per database, 64 KiB per value and 4 MiB of payload overall.

## Reviewed Redis archives

Desktop **Actions → Export loaded keys / Export selected keys** captures up to
1,000 keys into a review window. Save explicitly to a `.rredis` file after review.
**Actions → Import / export… → Open archive…** loads an immutable copy, then
**Review target** checks the selected database before enabling import. Read-only
mode blocks import. Existing keys are skipped by default; replacement requires
confirmation. Absolute expiry deadlines are preserved unless you choose removal.

Archives preserve binary names and opaque Redis DUMP values. This is Rustable's
versioned format, not an RDB backup. Limits are 64 MiB per file, 1 MiB per value,
and 4 KiB per key name. Compatible Redis/module versions are required. Files
contain raw data; the checksum detects corruption but does not authenticate the
source. Export batches are separate snapshots, not a consistent whole-database
backup. Review is also a snapshot; collision/expiry policies are enforced again
when each batch runs.

Import processes up to 16 keys per exchange and supports stopping between batches.
The result list distinguishes restored, existing, expired, failed, unconfirmed,
and unattempted keys. Completed writes are not rolled back; a lost reply is not
retried automatically. Inspect unconfirmed outcomes before deciding to retry.

## Redis Cluster (in progress)

Choose **Deployment → Cluster** in a desktop Redis connection. The host/port is
its preferred seed node, and the database is fixed to zero. Use **Add alternate seed**
to save up to seven additional addresses. Connect and Test try the first available
seed using shared credentials and TLS settings. Certificate and hostname checks
apply to discovered nodes too. The browser scans all primaries from the current
topology, with normal type/pattern filters, bounded paging and key deduplication.
Key previews, inspection and common key commands route to their owning primary.
**Actions → Cluster topology…** shows reported nodes and slot coverage.

Cluster key commands follow explicit MOVED/ASK redirects. A lost connection stops
requests without replaying a possibly completed write. An established workspace
makes one automatic transport-recovery attempt using up to eight previously
discovered primary/replica addresses, starting with the current seed. Reconnect uses the same path.
Successful recovery reloads topology but leaves displayed data stale and retains
drafts and the original error: verify uncertain writes before explicitly refreshing
or retrying. TLS and authentication settings apply to every candidate. Initial
connections use the saved seed list; addresses learned from topology remain only
in the active session. Dedicated MONITOR/Pub/Sub startup also tries saved alternates on connection or
startup transport failure. Authentication/protocol errors stop startup; established
streams require an explicit restart after loss. These tools operate on the selected
node, not a merged view of every Cluster node. Replicas are discovery candidates only; key reads still route
to validated online primaries. Controlled slot migration and planned replica
promotion are covered by fixtures; abrupt loss/election timing, network partitions
and interrupted/concurrent resharding coverage remain open.

Multi-key atomic commands must share a Redis hash slot (for example, keys using
the same `{tag}`). Cross-slot atomic commands are rejected. Archive exports and
target reviews support keys across slots, with atomic capture per key rather than
a database-wide snapshot. Imports also support keys across slots, report each
key's outcome, and stop on uncertain failures without replaying submitted writes.
Server-level commands and
command-line SCAN results are seed-local; use the key browser to scan primaries.
The live profiler and module/Search operations remain scoped to the contacted
node unless the server itself provides cluster-wide semantics. These limitations
are tracked in REDIS-ROADMAP.md.

## Time-series samples

Time-series keys show their latest sample in the key list and open as a paginated,
read-only sample table. Each page contains at most 200 samples. Use From and Through
to select inclusive Unix-millisecond bounds; leave a bound blank for an unbounded
range, then choose Apply range. Previous and Next stay within that applied range.
The text filter searches only the displayed page. Samples in page is not the total
series length. A refresh may be needed to see samples inserted before your current
page while browsing. Whole-key expiry/deletion retain the usual safe-mode guards;
sample editing is not yet available. Choose Table or Chart to switch views of the
loaded page. The chart uses the current text filter, keeps gaps for excluded or
non-finite samples, and shows exact sample timestamps and values on hover. It
never fetches additional data; bounds describe the loaded page, not the whole
series. Chart selection scrolls the plot into view.

Choose **Series info…** to inspect total samples, retention, memory, timestamp
bounds, chunk count, labels and compaction rules. Metadata is fetched on demand
and refreshed explicitly; ordinary sample browsing does not fetch it. The view
requires `TS.INFO` and `EVAL_RO` permissions. A fixed read-only script limits the
response to 64 labels and 64 rules with 256-byte text fields; the footer reports
omitted entries and shortened fields. Redis still constructs the underlying
`TS.INFO` metadata before these response limits are applied. Large numeric values
that pass through Lua are marked approximate. Permission errors are shown without
falling back to an unbounded request. Cluster requests route by the selected key.

### Vector-set element pages

Redis 8.4+ vector sets have an ordered, read-only element inspector with 200-name
pages, Previous/Next navigation, binary-safe names and local page filtering.
The existing virtualized collection table and shared SQL controls are reused.
VCARD supplies the total; a fixed EVAL_RO/VRANGE script limits returned names to
64 KiB each and 1 MiB per page. Redis still constructs up to 201 names inside the
script before applying these output limits. EVAL_RO and VRANGE permissions are
required; older servers and denied permissions produce an error without an
unbounded fallback. Select an element and choose **Inspect element…** to load stored approximate
components and syntax-highlighted JSON attributes. Choose **Find similar…** on a selected element for up to 20 approximate matches,
ranked by score, with an Inspect action for each result. Concurrent changes may alter the set between pages.

Initial inspection uses TYPE/PTTL followed by a pipelined VCARD/script (two
exchanges); subsequent pages pipeline TYPE/PTTL/VCARD/script (one exchange).

Element details are fetched only on request or explicit refresh, using one
read-only declared-key script routed to the Cluster owner. The response is capped
at 4,096 dimensions (128 bytes per component) and 64 KiB of JSON attributes;
Redis constructs the underlying values before applying these output limits.
Formatting and JSON tokenization happen on the worker; component rows and JSON
lines are virtualized. Copy attributes preserves JSON integer precision. Missing
elements, denied permissions and size limits report errors without an uncapped
fallback. These values are Redis's stored, approximate embedding components;
normalization and quantization can differ from the original input vector.

Similarity search uses direct VSIM ELE/WITHSCORES/COUNT 20/EF 100, preserving Redis's
normal background search behavior. It does not run VSIM in Lua or a transaction.
The existing connection is reused; replies have a 1 MiB transport budget, and
result names are capped at 64 KiB. A response-limit or protocol failure closes the
connection without replay. Cluster MOVED/ASK routing is supported; ASKING and the
query stay pipelined. Source elements may appear among results. Arbitrary input
vectors, attribute filters and adjustable search parameters are not yet exposed.
Changing databases clears old diagnostic windows and their key-scoped actions.

### Command editor

The command pane now follows the SQL editor's execution conventions: Command+Enter
(Ctrl+Enter on other platforms) runs the selected text or the command at the caret;
Command+Shift+Enter runs the full editor. Plain Enter inserts a newline. Shortcuts
are scoped to editor focus and execution is disabled while busy, disconnected, or
reviewing an edit. Existing read-only checks and dirty-value protections remain in
the execution path. Quoted and escaped newlines stay within their Redis command.

The pane uses shared SQL Run controls, line numbers, editor typography and selection
colors. Cached Redis highlighting distinguishes command names, options, quoted
values and numbers. Highlighting falls back to plain text above 64 KiB and caps
section count to keep large drafts bounded; the existing 1 MiB execution limit
remains. Editor and output span the pane width with horizontal dividers and no
side borders. Output has a Copy control in an evenly padded header. The
right-aligned action row sits below the editor. Help, history and saved commands
share the toolbar.

The read-only toggle uses the SQL viewer's lock control in the top toolbar.
Collection rows remain virtualized, use compact spacing within a bordered table,
and place Previous/Next below the value view. Filters for local pages are labelled
as page-only filters.

### Workspace layout

The Redis connection uses Browse, Search, Workbench, Analyze and Pub/Sub tabs
under the shared connection toolbar. Browse puts the type/name filter above a
resizable list/detail split. List/tree controls stay with the key list, metadata
and key actions stay with the detail pane, and Add key opens a draft there with
Cancel/Create actions at the bottom. The bottom utility strip retains the
optional CLI and profiler.

Search has an index catalog and a dedicated query page with a multiline editor,
Command/Ctrl+Enter execution and actions beneath it. Index lists are loaded on
first entry and reused across revisits; Refresh requests current data.
Background key discovery pauses outside Browse and resumes on return.
Workbench gives commands and their results the full page. Analyze displays
explicitly requested reports; its namespace charts describe only the largest
50 sampled keys in the report, not whole-database statistics. Pub/Sub has a
centered subscription state and a separate publish area. Switching tabs retains
the subscription; Stop ends it. Existing read-only and mutation review checks
still apply. No live telemetry polling was added.

### Inspector and tool refinements

JSON documents edit inline with cached syntax highlighting, Save/Discard controls,
existing JSON validation and optimistic conflict checks. A draft prevents navigation
from discarding it. Rename, refresh, delete and close actions live in the key header;
closing the inspector returns the key list to full width. Copy buttons briefly show
“Copied” without polling. Enter retains focus in the key-pattern field.

Workbench has a virtualized, session-only history sidebar (last 100 commands).
Selecting a history entry restores it without execution. Search separates the
query editor, optional query settings and results. Analyze shows top 15 keys and
namespace totals computed across all sampled keys, without additional requests.
Pub/Sub uses a virtualized message table with local receipt times and a publish
composer anchored below the message area; retention and worker drain limits remain
unchanged.
