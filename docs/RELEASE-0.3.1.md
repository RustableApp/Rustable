# Rustable v0.3.1

## Redis joins the workspace

Standalone Redis connections now support optional ACL authentication, verified
TLS and custom CA certificates, logical database selection, and precise connection
test errors. Browse keys with incremental SCAN, inspect strings, hashes, lists,
sets, sorted sets and streams, and run Redis commands in a collapsible editor.

The key/value/type/TTL grid and both sidebars share the SQL workspace's styling.
Connections start read-only. Unlock to edit complete strings up to 64 KiB, change
expiry or delete a key. String saves detect stale values and preserve TTL.
Network work stays off the UI thread; grid previews use pipelined reads.

Tested against standalone Redis 7.4. Cluster/Sentinel, Redis SSH tunnels, client
certificates, visual collection editing, pub/sub and bulk import/export are not
included. Other value types can be modified through commands. Writes apply
immediately, unlike SQL's pending-change review. See [Redis support](REDIS.md).

## Workflow polish

- Clear connection-save feedback, including credential-save status.
- Database logo picker when creating connections.
- Desktop smooth scrolling now follows the browser behavior.
- Activity-pane visibility and SQL editor size persist.
- Removing the final filter clears filtering.
- Compact Run Current/Run All split button with consistent shortcut symbols.
- Command palette dismisses on outside click; the WASM connection view can be exited.
- Removed the obsolete Restore SQL drafts toggle and fixed vendored egui warnings.

## Downloads and demo

macOS Apple Silicon is signed/notarized and includes Sparkle updates.
Windows/Linux x64 downloads are portable packages with manual updates.
Linux requires Ubuntu 24.04 or compatible newer libraries. Windows built-in SSH
remains unavailable; direct connections and external tunnels are supported.

Every release includes the matching shared Rust/WASM application demo, using
virtual SQLite. Redis server connections are desktop-only. JavaScript provides
browser boot/worker glue; the application runs as WASM.

Native Windows/Linux GUI acceptance, Intel macOS, and installed old-to-new
Sparkle update acceptance remain follow-up work. This remains early software.
