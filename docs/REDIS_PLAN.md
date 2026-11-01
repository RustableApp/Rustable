# Redis integration

Implementation checklist, 2026-11-01. Development availability and limits are documented in [REDIS.md](REDIS.md).

Completion criteria:

- [x] Saved Redis connections, username/password authentication, verified TLS,
  bounded connect/read timeouts, and precise connection-test errors.
- [x] Redis-native key browsing with incremental SCAN, database selection, key type
  and TTL, and bounded inspection of strings, hashes, lists, sets, sorted sets,
  and streams. Binary keys and values must not be corrupted.
- [x] Redis command execution with quoted/binary arguments, useful reply rendering,
  read-only mode, and clear handling of commands that change connection state or
  block indefinitely. No SQL parsing or SQL formatting of Redis commands.
- [x] Useful guarded editing operations, including key deletion and TTL updates;
  no writes merely from browsing or opening an editor.
- [x] Disposable Redis integration tests for auth, invalid databases, disconnects,
  expiry, empty/non-UTF8 values, scan pagination, command parsing and write guards.
- [x] Existing SQL workflows and WASM build remain working. Redis server connections
  are desktop-only; browser demo must explain this without trapping navigation.
- [x] Document exact supported Redis topology/version range and any remaining
  limits. Update icons and support copy only after verification.

Cluster and Sentinel discovery are separate from direct standalone connections.
Do not silently follow redirects or describe unsupported topologies as supported.
