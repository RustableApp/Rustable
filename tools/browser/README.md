# Rustable browser host

This package hosts **the actual Rustable main window** from the root library. It does not implement a second UI. The desktop SQLite driver and command handler run inside a dedicated Web Worker, compiled through rusqlite/sqlite-wasm-rs. JavaScript provides loading, worker transport, browser file transfers and IndexedDB access; SQL execution and the app UI run in Rust/WASM.

This replacement is under development and locally verified. The public site still serves the earlier reduced demo until the remaining browser adapters pass acceptance.

## Build

```sh
mise exec -- rustup target add wasm32-unknown-unknown
mise exec -- rustup component add llvm-tools
mise exec -- cargo install wasm-bindgen-cli --version 0.2.130 --locked
mise exec -- python scripts/build-browser.py --output /tmp/rustable-full-preview/demo
```

Dependencies are locked. The build script uses LLVM's archiver: Apple's system archiver cannot index the SQLite WASM objects. All runtime assets are self-hosted.

Serve the preview with Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp to enable shared-memory cancellation. Database operations have a two-second progress deadline, including when shared-memory cancellation is unavailable. Transport decoding is capped at 32 MiB; existing desktop row/result bounds also apply. Closing/reloading the page discards this sample database.

## Browser adapters

SQL import/download, loaded-result CSV/JSON downloads, and full-table CSV/JSON downloads are connected. Full-table export uses the desktop streaming encoder with the current filter and sort across all pages, capped at 16 MiB; it rejects active transactions and never downloads partial output on failure. Search, formatting and loaded-result encoding run in separate WASM workers. Named queries and groups persist in browser-local IndexedDB, with stale-save checks and storage limits.

## Remaining acceptance

CSV preview/import and database-file import still need browser adapters. Server connections and OS keychains remain desktop integrations. Browser language-server processes are not started; shared catalog completion remains available.

Verify cancellation, error recovery, keyboard shortcuts, browser memory bounds and more browsers before publishing. The full workspace is not a claim that every native OS integration exists in a browser.
