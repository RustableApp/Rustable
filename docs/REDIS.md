# Redis support

Redis is available in the development build, for the next release. The published
v0.2.0 download does not include it.

## Connection and browsing

Choose Redis in New connection. Configure a standalone server, logical database
number, optional ACL username/password, and TLS. TLS is enabled by default and
verifies both the certificate chain and hostname; a PEM CA file can be supplied.
Passwords use the existing operating-system credential store when Remember
password is selected, and are never serialized into the connection profile.

The connection test distinguishes transport/TLS, authentication, and database
selection failures. Connect and read/write timeouts default to five seconds.
The workspace starts read-only. Network requests and reply decoding run on a
dedicated worker, away from the UI thread.

The key browser uses incremental SCAN with a glob pattern and Load more keys.
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
Other Redis versions have not been certified.

Cluster/Sentinel discovery, automatic redirects/failover, built-in SSH tunnels,
client-certificate authentication, module-specific inspectors, live pub/sub,
saved command history, and bulk import/export are not included.
Redis server connections are desktop-only; the WASM demo remains SQLite.

The key-pane width and command-pane height use the app's saved pane preferences.

## Reproduce integration tests

Run `./scripts/test-redis.sh` with Docker, OpenSSL and mise installed. It starts
local Redis 7.4 plaintext/TLS fixtures on ports 16379/16380, runs the suite,
and removes its containers and generated certificates on exit. Both ports
must be free; it never reuses an existing server.

For manual fixture setup, the plaintext tests
expect Redis 7.4 on 127.0.0.1:16379 with password
'rustable-redis-test-only'. TLS tests expect port 16380 and a CA certificate at
/tmp/rustable-redis-tls/ca.crt, with a server certificate whose SAN includes
127.0.0.1 but not localhost. The fixtures must permit ACL administration.

    mise exec -- cargo test --locked --lib redis_session::tests -- --include-ignored

Tests create their own prefixed keys/users. Run only against disposable fixtures.

Verification on 2026-11-01: 470 native library tests passed; the six-test Redis
suite passed against freshly created Redis 7.4 plaintext/TLS containers.
Strict native all-target Clippy passed. The WASM check passed with its existing
52 browser-only warnings. The optimized desktop build passed. Native UI checks
covered connection testing, key selection, command results, string-save
feedback and protection of unsaved edits. A synthetic pointer-drag regression
test verifies pane-width persistence across UI contexts.
