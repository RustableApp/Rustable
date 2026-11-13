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
  statement metadata. Typed queries describe a named statement and resolve its
  types before executing it. Resolving an uncached custom type after Execute
  deadlocked when unread rows filled the bounded response channel and blocked
  the type lookup response. This adds a description round trip, retains bounded
  streaming, and does not execute the user's SQL twice. The live
  `uncached_custom_type_with_large_rows_does_not_stall` regression reproduces
  the original hang with 20 enum-bearing rows containing large text values.
  `query_typed_with_prefix` executes prefixes with Bind/Execute in one Sync
  group so setup failure skips execution. Rustable's two idempotent SET LOCAL
  safeguards also run during description; arbitrary prefixes run only once.
- `query_typed_bounded_with_prefix` is the editor's single-exchange path:
  safeguards, unnamed Parse/Describe/Bind/Execute, then Sync. Built-in and cached
  types stream immediately. Uncached types drain execution first, retaining only
  the configured row/byte budget, then resolve catalog metadata and decode those
  rows. Overflow is exposed by `RowStream::truncated` / `RowIter::truncated`.
  This avoids type-lookup deadlocks without replaying SQL; the general typed API
  retains its describe-first behavior. Live tests cover cold enum results larger
  than the response channel, truncation, INSERT RETURNING without replay, cached
  decoding and empty custom-type results.
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
  preserving the metadata response when the data query fails. Table browsing
  has a separate `query_text_with_metadata` path: parameterized Parse/Bind
  (text result format)/Describe portal/Execute/Sync, pipelined with fresh editing
  metadata. Sidebar column/primary-key hints permit first loads in one exchange.
  `SimpleColumn::type_oid` validates the returned shape without resolving custom
  types. Both responses drain concurrently and retained rows are byte/row bounded
  using `SimpleQueryRow::storage_size`; a truncated page leaves the connection
  usable. Text decoding preserves exact numeric text, NULL and bytea (hex/escape).
  Unknown relations and explicit transactions retain the metadata-first path.
  Index/trigger definitions are loaded separately on demand.
  The metadata query returns only built-in wire types. Fresh schema validation,
  stale-shape retries (autocommit only), and separate Sync boundaries remain;
  an erroneous speculative query cannot discard the fresh catalog response.
- `postgres/src/cancel_token.rs`: additive `cancel_query_with_timeout` wraps the
  entire cancellation connection/TLS/send future in a deadline. `false` means
  timeout; `true` means sent, never confirmed server cancellation. Rustable uses
  two seconds so cancellation-thread cleanup cannot hang in TLS negotiation.

Rustable pipelines SET LOCAL standard_conforming_strings (and, in Safe mode,
SET LOCAL transaction_read_only) with execution of the parameter-free editor query in one
exchange, including description of the editor query. Each prefix is a separate
Parse/Bind/Execute operation, with a single Sync
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

Editor results retain binary typed decoding and bounded streaming (bounded buffering
while resolving uncached custom types). The metadata
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

### macOS resize presentation

`src/native/glow_integration.rs`: after glutin attaches the OpenGL context, set
the layer-backed NSView's contents placement to `TopLeft`. AppKit's default
`ScaleAxesIndependently` stretches the cached framebuffer during title-bar
zoom/fill animations, even when intermediate resize events are painted. Keep
cached text and cells at their original scale between fresh frames. This is a
one-time setting for each viewport; AppKit retains ownership of the backing scale,
layer and redraw policy. Do not disable layer backing (glutin requires it for
context attachment stability) or override the scale for mixed-DPI displays.

`src/native/run.rs`: after a macOS `Resized` event updates the backend surface
and egui input, honor its `RepaintNow` by laying out and presenting synchronously
before returning to AppKit. Previously only Windows painted synchronously;
macOS queued a later redraw. Painting promptly complements the contents-placement
fix; synchronous painting alone did not resolve the reported stretch. The change
is restricted to resize events requesting
an immediate paint. Ordinary input, zero-sized surfaces and other platforms keep
their existing scheduling. Consume any due repaint for that window, preserve
future deadlines, and use
the paint result to schedule further work, rather than adding a redundant redraw
or an animation timer. Both root and secondary viewports use this dispatch path.

`src/native/glow_integration.rs` also flushes the implicit Core Animation
transaction after a successful swap whose framebuffer size changed. Intermediate
resize layouts and inspector widths were correct, but AppKit could retain the old
contents until zoom completed. The user verified that this size-change-only flush
makes the center grid grow throughout title-bar zoom/restore. It runs for root and
immediate viewports, skips the first frame and unchanged-size frames, and adds no
repaint requests or timers. The macOS-only QuartzCore dependency uses the existing
objc2 0.5 ecosystem. Native frame timing and mixed-DPI coverage remain outstanding.

Native acceptance requires title-bar double-click zoom/restore, edge dragging,
idle settling, secondary windows, and mixed-DPI move/snap/reconnect checks.
The headless UI suite measures layout/tessellation only, not this AppKit path.
Use `RUSTABLE_UI_PREVIEW=1 mise exec -- cargo run --locked --features qa` for a
disposable SQLite table. Shrink the window, then double-click its title bar to
fill the screen; repeat in both directions and with ordinary edge dragging.
Text and cell heights must retain their scale throughout the transition, not
just after it. Leave it idle afterwards and verify that repaints settle.
See `docs/UI-PERFORMANCE.md` for the November 12 validation scope and limitations.

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

## egui_extras table interaction corrections

`egui_extras 0.33.3` retains upstream MIT/Apache licenses. The table divider
continues to paint through the body, but its resize interaction rectangle covers
only the header, with horizontal grab padding. Existing header drags can continue
outside the header. Rustable's grid interaction regression checks both body
non-resizing and persisted header resizing.

Table row hover capture uses `Response::contains_pointer()` instead of `hovered()`.
Child text may own hover while the enclosing row still contains the pointer; the
row highlight must remain continuous across that text. Clipping/layer hit testing
is preserved. `controls::table_hover_tests::row_hover_survives_pointer_over_child_text`
protects this behavior; the shared SQL-grid and Redis UI benchmarks cover the path.

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

### Compact field caret height

`egui::TextEdit::cursor_vertical_padding` controls the extra caret height above
and below the text row. Its default preserves upstream behavior. The inspector
field editor opts into zero extra height; hover and focused carets use the same
geometry. Text layout, selection and other editors are unchanged.

- `egui/src/style.rs` and `text_selection/visuals.rs`: opt-in
  `Selection::preserve_text_color` skips recoloring selected glyphs, retaining SQL
  syntax colors over the selection background. Disabled by default; Rustable
  enables it only inside the SQL editor. No extra layout or paint pass.

### Bounded PostgreSQL saved-row reads

`postgres::Client::query_text_bounded` wraps the existing `query_text_raw`
single-exchange API for post-save row refreshes. It accepts bound parameters,
returns text-format results without custom-type catalog lookups, and drains rows
beyond the caller's row/byte limit. Rustable rejects truncated refreshes and marks
the result stale; a committed write is never retried because refresh failed.
See `docs/QUERY-RESULT-EDITING.md` for latency costs and regression coverage.

The PostgreSQL cancellation tokens expose `process_id()` from the existing startup
handshake. The query monitor uses it to select this workspace’s running query;
this accessor performs no network I/O and does not expose the cancellation secret.

# Redis native TLS handshake deadline

`redis/` is the crates.io source of `redis 1.8.1` from
[redis-rs](https://github.com/redis-rs/redis-rs), retaining its BSD-3-Clause license.
The native TLS patch in `src/connection.rs` wraps its
TCP stream during connection establishment with an absolute I/O deadline. Each
handshake read/write receives the remaining socket timeout; TCP address attempts
in the native TLS branch share that deadline. The wrapper disables its deadline
after the handshake, leaving the existing authentication deadline and established
connection read/write timeouts in control. Certificate/hostname verification is
unchanged. No extra command, preflight, retry, or network exchange is added.

Upstream applied the connection timeout only after `ActualConnection::new`, which
could wait indefinitely inside the native TLS handshake. Reproduced while a
cluster connection tried TLS against the disposable plaintext Redis fixture.
Tests: `redis_cluster::tests::stalled_native_tls_handshake_obeys_connection_deadline`
and `discovered_nodes_preserve_tls_identity_and_never_fall_back_to_plaintext`.
DNS resolution remains the platform's synchronous resolver and is not made
interruptible by this patch. The separate streaming transport is unchanged.

### Opt-in bounded synchronous Redis replies

`Connection::req_packed_commands_bounded` accepts a packed RESP2 batch and one
byte budget per expected reply (1–1,000 replies, at most 16 MiB each and 64 MiB
aggregate input budget). The reply-count limit supports a 200-key Analyze metadata pipeline (600
scalar replies) and a full 50-index Search metadata page without serialized
sub-batches. `recv_response_bounded` exposes the same parser/close behavior for
Cluster fan-out: send all node requests first, then receive each bounded reply.
Ordinary Cluster reads retain their existing path; Analyze opts into limits,
including redirected metadata reads. It sends the batch once, consumes every server
error value normally, and closes the connection on parse/transport failure.
Callers must provide a clean request boundary and the exact response count;
RESP3, Pub/Sub and outstanding timed-out responses are rejected. Limits include
decoder read-ahead bytes, so they are conservative. Ordinary request methods are
unchanged. The bounded parser uses the existing nesting restriction and combine's
capped collection preallocation; its byte budget bounds input before full decoding.
Do not retry writes after an uncertain outcome. Initial protocol coverage lives
in `redis_session::tests::bounded_responses_preserve_one_exchange_and_close_on_limit`.
Element-based similarity search now uses this API, including pipelined ASKING
and VSIM. Application sessions discard ordinary parse-failed connections before
a later bounded request. Redirect, overflow and parser-cleanup tests pass; bounded
TLS and further batch-error coverage remain in progress.

### Synchronous Redis over verified SSH

`Client::get_connection_via` overrides only the TCP dial endpoint. TLS SNI and
certificate hostname checks still use the original `ConnectionInfo` host. The
existing setup/authentication deadline and command sequence are retained.
`Connection::retain_transport` owns the tunnel lease through connection drop,
including connections transferred into Sentinel/Cluster routing. Dedicated
streaming connections own the same lease through their transport wrapper.

SSH adds gateway authentication and forwarding setup on cold connection creation;
it adds no Redis protocol exchanges. Warm query paths retain the same connection
and do not create tunnels, ping, or do extra setup. Each newly connected topology
endpoint owns its own tunnel, subject to existing connection bounds. The existing
OpenSSH transport rejects unknown/changed host keys and cleans up private masters.
Coverage: `ssh_tunnel::tests::config_alias_live_forward_and_drop_cleanup` and the
forwarded-hostname cases in `redis_session::tests::live_mutual_tls_client_identity`.

## epaint 0.33.3: immutable shared layout jobs

Source: crates.io epaint 0.33.3, package checksum
`009d0dd3c2163823a0abdb899451ecbc78798dec545ee91b43aff1fa790bab62`, from the same
MIT/Apache-2.0 egui release already vendored here. License files are copied from
that egui release. Both native and browser manifests patch this exact version.

`FontsView::layout_job_shared(Arc<LayoutJob>)` accepts a cached immutable job
without cloning its text/format sections on every hit. Owned `layout_job` and the
new API enter the same generic cache path; owned cache hits still avoid an Arc
allocation. Shared cache misses retain the caller's Arc. Fractional wrap widths
use copy-on-write normalization, leaving the caller's job unchanged. Font/atlas
invalidation, DPI keys, child-paragraph cache liveness and eviction are unchanged.
Rustable keeps separate frame-evicted syntax and prepared-layout caches, so resize
does not tokenize unchanged SQL again. The only unrelated source adjustment is
an explicit `2.0_f32` literal in tessellator debug drawing for Rust 1.99's
float-literal-f32-fallback lint. Tests in `src/sql_highlight.rs` cover owned/shared
cache equivalence, ownership, fractional widths, font replacement, DPI, theme,
editor style, text and preview/wrap changes. Release performance and native
profiling evidence is tracked in `docs/UI-PERFORMANCE.md`.
