# PostgreSQL streaming and pipeline patches

These are the crates.io sources of `postgres 0.19.14` and `tokio-postgres
0.7.18`, from the upstream [rust-postgres project](https://github.com/rust-postgres/rust-postgres).
Both retain their original MIT and Apache-2.0 licenses in their directories.
Cargo patches crates.io to these local copies; the lockfile still pins all
transitive dependencies.

Local changes:

- `tokio-postgres/src/copy_in.rs`: COPY startup uses Flush after Bind/Execute;
  CopyDone/CopyFail supplies the single Sync boundary. An initial Sync followed
  by CopyFail's Sync produced two ReadyForQuery responses when the server
  rejected COPY before entering copy mode (reproduced with RLS). This desynchronized
  subsequent rollback responses and masked the original server error. The live
  `copy_import_permissions_rls_and_partitions` regression verifies the original
  RLS error, zero partial writes and connection reuse after rejection.

- `tokio-postgres/src/query.rs`: `RowStream::columns()` exposes existing
  statement metadata. `query_typed_with_prefix` encodes non-row-returning
  prefix commands before the typed query, all within one Sync group, then
  consumes their command completions before returning the query stream.
  Existing `query_typed` delegates with an empty prefix.
- `tokio-postgres/src/client.rs`: additive `query_typed_raw_with_prefix` and
  `transaction_status` APIs; tracks outstanding requests alongside status.
- `tokio-postgres/src/codec.rs`: reads the terminal ReadyForQuery status from
  a completed response without scanning or cloning result rows.
- `tokio-postgres/src/connection.rs`: records server status once per completed
  wire response, sharing atomics with the client.
- `tokio-postgres/src/connect_raw.rs`: initializes the shared status state.
- `postgres/src/row_iter.rs`: `RowIter::columns()` forwards the metadata accessor.
- `postgres/src/client.rs`: additive synchronous `query_typed_raw_with_prefix`
  and `transaction_status` APIs. `query_typed_raw_with_metadata` polls a small
  metadata query and a streaming data query together on the same connection,
  preserving the metadata response when the data query fails.
- `postgres/src/cancel_token.rs`: additive `cancel_query_with_timeout` wraps the
  entire cancellation connection/TLS/send future in a deadline. `false` means
  timeout; `true` means sent, never confirmed server cancellation. Rustable uses
  two seconds so cancellation-thread cleanup cannot hang in TLS negotiation.

Rustable pipelines SET LOCAL standard_conforming_strings (and, in Safe mode,
SET LOCAL transaction_read_only) with the parameter-free editor query in one
exchange. Each is a separate Parse/Bind/Execute operation, with a single Sync
after the final query. PostgreSQL's extended-protocol error handling skips the
remaining operations until Sync if any setup command fails. This is essential:
concurrently sending separate setup/query requests, each with its own Sync,
could execute the query without its intended safeguards after setup failure.

The editor uses PostgreSQL autocommit by default. It sends no implicit explicit
BEGIN/COMMIT wrapper. SET LOCAL applies within the implicit transaction covering
this Sync group, and resets when that transaction ends. Unsafe mode permits
user-written transaction commands; explicit transactions remain open across
runs until the user commits or rolls back. Recovery statements bypass the SET
prefix so an aborted explicit transaction can still be rolled back.

Results retain binary typed decoding and bounded streaming. The metadata
accessors preserve headers for zero-row results without re-preparing or
re-running queries after execution. Cancellation follows PostgreSQL semantics:
a cancelled in-flight statement rolls back, but an already completed autocommit
cannot be undone by a later cancellation request.

Table-edit/save and export helpers must not nest their own transaction inside
an explicit SQL transaction. They inspect actual server ReadyForQuery status
(I idle, T active transaction, E failed transaction) and refuse unless idle.
The accessor returns unknown while requests are outstanding, avoiding stale
idle state after a split error response. Only these guarded operations drain
an unknown state using protocol Sync; that does not commit or roll back an
explicit transaction and adds no exchange to editor queries. Queued prepared
statement Close requests are included in this tracking.

When updating these crates, preserve these APIs or remove the patches once
upstream provides equivalent capabilities. Run the opted-in disposable
PostgreSQL tests, particularly
`pipelined_setup_failure_skips_query_and_cancellation_rolls_back` and
`editor_single_exchange_query_preserves_headers_safety_and_recovery`, and
`editor_autocommit_and_explicit_transactions_follow_postgres`.

Repeat table browsing in autocommit uses the paired-query API. Unlike editor
safety prefixes, these are independent read requests with separate Syncs. The
server executes them sequentially, but sending both without awaiting metadata
removes one client/network wait. The previous catalog row is only a speculative
query-shape hint: fresh canonical names and complete column metadata must match
before any rows are published. Changed shapes discard the speculative result
and retry through the fresh sequential path. This fast path is disabled inside
explicit transactions and while transaction status is unknown. It does not open
a second session or cache displayed metadata across refreshes.

## eframe checked persistence

`eframe 0.33.3` retains its upstream MIT/Apache licenses. The additive
`Storage::flush_and_wait` method defaults to an unsupported error. Native
FileStorage joins any earlier writer, writes the latest snapshot to a same-directory
temporary file, syncs it, atomically replaces the state file, and syncs the parent
on Unix. Errors reach the caller; success never means only that a thread started.
Rustable uses this barrier before releasing Sparkle's deferred restart. Normal
periodic autosaves retain upstream behavior. tempfile provides atomic replacement.

Regression checks:

```sh
mise exec -- cargo test --manifest-path vendor/eframe/Cargo.toml --no-default-features --features persistence,glow,x11,wayland,default_fonts checked_persistence_tests
```

## egui_extras header-only column resizing

`egui_extras 0.33.3` retains upstream MIT/Apache licenses. The table divider
continues to paint through the body, but its resize interaction rectangle covers
only the header, with horizontal grab padding. Existing header drags can continue
outside the header. Rustable's grid interaction regression checks both body
non-resizing and persisted header resizing.
