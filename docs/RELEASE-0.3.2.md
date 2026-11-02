# Rustable v0.3.2

Fixes the macOS Dock icon: startup now explicitly supplies Rustable artwork,
preventing egui from replacing the bundled icon with its default. Local macOS
bundles now include the icon too.

The matching website WASM demo adds an in-memory Redis demo alongside SQLite.
Use Connections → Redis demo to browse sample keys, inspect values, run supported
commands, edit strings and try expiry controls. Switching demos preserves state;
reloading resets it. This is a bounded Redis-compatible demo subset implemented
in Rust/WASM, not a full Redis server. Run INFO for supported commands.

macOS Apple Silicon downloads are signed/notarized and support Sparkle updates.
Windows/Linux x64 packages use manual updates. Platform support and Redis server
capabilities are unchanged from v0.3.1.
