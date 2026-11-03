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

### Native OpenGL context recovery

`src/native/glow_integration.rs`: the context-switch fast path now requires
both `PossiblyCurrentGlContext::is_current()` and `Surface::is_current()`.
In glutin 0.32.3's macOS backend the latter only compares the attached NSView;
it does not establish that the context is current on the calling thread.
An AppKit callback can leave the attachment intact while clearing/changing
the thread's current context. Skipping the rebind then makes `glGenTextures`
return zero, causing egui_glow's texture allocation unwrap to abort in a native
callback. Both ordinary and immediate viewport painting use this helper.
The Windows always-rebind workaround is unchanged.

Run `mise exec -- cargo run --locked --example macos_gl_context_recovery` on
macOS with a graphical session. It queues fresh textures, clears the actual
CGL thread context before paint, and verifies four successful paint callbacks
with a current context and no GL error. Before this patch it reproduces the
exact egui_glow 0.33.3 painter.rs:514 panic and non-unwinding SIGABRT observed
in Sentry RUSTABLE-2; after the patch it exits successfully. This establishes
the missing-context failure and recovery, not the precise OS sleep/wake event
that cleared the context on the affected machine.

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

## MySQL protocol transaction status

`mysql 28.0.4` is copied from crates.io, retaining its upstream MIT/Apache-2.0
licenses and source from https://github.com/blackbeam/rust-mysql-simple.
The additive `Conn::transaction_status() -> Option<bool>` reports the protocol's
`SERVER_STATUS_IN_TRANS` bit after a consumed OK/EOF packet. A separate validity
flag is cleared when sending a command and on server ERR packets, so errors
cannot masquerade as idle or retain an earlier successful state. Read it only
after draining all result sets. Transport errors must still mark the app's
connection disconnected; an unknown status is not permission to replace it.

Upstream `ping()` discards its response. Our patch parses that OK packet so a
caller can refresh unknown transaction status without starting, committing or
rolling back a transaction. This extra exchange is needed after an error, not
on every successful read. A failed refresh must leave connection state unknown
or disconnected. The existing status flags and result-set parsing are unchanged.

The opted-in `sql::mysql::server_tests` test verifies both MySQL and MariaDB:
idle/read, explicit transaction, failed statement -> unknown -> ping -> active,
rollback, implicit DDL commit, and server-killed connection -> unknown. Run via
tools/database-fixtures/README.md. Preserve these semantics when updating the
vendored crate, or remove the patch when upstream exposes an equivalent API.

### MySQL TCP destination separate from TLS peer

The additive `OptsBuilder::tcp_destination(Option<SocketAddr>)` overrides only the
TCP address/port, retaining `ip_or_hostname` for TLS peer verification. This is
needed for SSH forwarding: connecting to loopback must not verify the database
certificate against loopback or disable verification. Setting an override disables
implicit local socket probing; an explicitly configured socket still wins.
Default options and direct connections are unchanged. The patch does not expose
URL parsing for the override or alter TLS verification settings.

Rustable's opted-in `connection::mysql_tests::live_tls` tests cover direct and
transport-override connections against both fixture engines: a deliberately wrong
configured TCP port proves the override is used, a localhost-only certificate
proves the peer name is retained despite the IP destination, wrong-peer.invalid
must fail hostname verification (not DNS), and an untrusted CA must still fail.


## egui 0.33.3: same-frame table wheel input

`vendor/egui` is the crates.io egui 0.33.3 source, retaining the upstream
MIT/Apache-2.0 licenses. Changes to `src/containers/scroll_area.rs`:
`ScrollArea::wheel_before_layout` is an opt-in path that applies wheel input
before creating the content UI, using the previous measured content extent.
It clamps offsets, clears animated targets/drag momentum, respects clipping,
disabled UI and dragging, and consumes only the enabled axis. The ordinary
end-of-layout wheel path remains the default and the first-layout fallback.
Do not opt in for a parent whose children consume wheel input on the same axis.

`egui_extras::TableBuilder::wheel_before_layout` forwards this option to the
vertical body. Rustable enables it only for its data grid's horizontal area
and vertical table body. It replaces the temporary repeated `request_discard`
approach, eliminating the extra layout pass and egui's red performance warning.
Production-grid tests cover both axes, isolated/continuous line and precise
point events, same-frame painted movement, one pass and no residual motion.

`src/input_state/mod.rs`: additive `set_scroll_delta_without_smoothing` replaces
native deltas and clears the otherwise private interpolation accumulator. Zoom
smoothing is left unchanged. Replacing only the public smooth delta stopped
visible animation but left the old accumulator requesting invisible repaint
frames; the idle-repaint regression reproduced that and now passes.

Compiler compatibility cleanup: stroke-width literals explicitly use `f32`
(including the debug painting paths), avoiding the newer compiler's
`float_literal_f32_fallback` future-incompatibility warning. The scene module
no longer imports the legacy `core::f32` module, so `f32::EPSILON` resolves to
the primitive's associated constant. Numeric values and rendering behavior
are unchanged; no warning lints are suppressed.
