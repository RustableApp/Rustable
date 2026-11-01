# Rustable browser demo

A real Rust/egui application compiled to WebAssembly, using the desktop SQL highlighter and sample database. This is a limited browser edition, not the full desktop connection manager. It executes real SQL with SQLite's official WASM build in a dedicated worker; there is no database server or query upload. JavaScript provides browser/SQLite bindings, not a replacement HTML/JS application UI.

Build with mise's Rust toolchain, the wasm32-unknown-unknown target, and wasm-bindgen-cli 0.2.130:

```sh
mise exec -- rustup target add wasm32-unknown-unknown
mise exec -- cargo install wasm-bindgen-cli --version 0.2.130 --locked
mise exec -- python scripts/build-browser.py --output ../rustable-website/public/demo
```

Dependencies are locked; the SQLite npm archive is checksum-pinned and self-hosted. The UI uses a separate Cargo package so native drivers, keychain APIs, filesystem dialogs, and the native worker are not pulled into WASM. Shared UI code is referenced from src, not copied. Browser data is in memory and resetting/closing the tab discards it.

Limits: 500 displayed rows per result, 1 MiB result text budget, 4,096-character cell previews, 50 statements, 128 KiB SQL, 64 MiB SQLite heap limit, 2-second SQL progress deadline, and a 15-second worker watchdog. Cancel destroys the isolated worker; the next request recreates the sample database. It does not affect desktop sessions. Desktop downloads remain necessary for PostgreSQL, real database connections, saved files and the complete editing workflows.
