# Rustable browser host

This package hosts **the actual Rustable main window** from the root library. It does not implement a second UI. The desktop SQLite driver and command handler run inside a dedicated Web Worker, compiled through rusqlite/sqlite-wasm-rs. JavaScript provides loading, worker transport, browser file transfers and IndexedDB access; SQL execution and the app UI run in Rust/WASM.

The full shared workspace has been deployed to the public site. See `docs/BROWSER-DEMO-VERIFICATION.md` for the deployment history and the exact workflows verified; local source changes are not automatically published. Broader browser acceptance remains open.

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

CSV preview, mapping, review and transactional import accept browser-selected files up to 8 MiB using immutable in-memory snapshots. Cmd/Ctrl+O opens a standalone SQLite file up to 8 MiB as a writable browser-local copy, without modifying the selected file. Invalid files preserve the current session; WAL-mode files require a standalone backup first. Server connections and OS keychains remain desktop integrations. Browser language-server processes are not started; shared catalog completion remains available.

Continue acceptance of cancellation, error recovery, keyboard shortcuts, whole-tab memory bounds and additional browsers before claiming browser parity. The full workspace is not a claim that every native OS integration exists in a browser.

Download object URLs retain at most 32 MiB total for up to 60 seconds. Additional
exports show a wait-and-retry error until that budget is released. Failed launch
releases its buffer immediately. This does not bound browser-managed download
copies or the entire tab's memory.

Browser adapter regression tests (using a mise-managed Node runtime):

```sh
mise exec node@24.22.0 -- node --test tools/browser/tests/*.mjs
```
