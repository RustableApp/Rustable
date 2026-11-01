# Browser demo verification — November 1, 2026

This is a working browser edition, not full desktop parity. The Rust/egui UI compiles to WASM and shares `src/sql_highlight.rs` with the desktop editor. SQLite 3.54.1 runs as WASM in a dedicated worker; JavaScript implements browser messaging and SQLite API bindings. No query is sent to a database server.

## Observed and tested

- `cargo check` and strict Clippy pass for the wasm32-unknown-unknown package.
- Six native editor/highlighter tests pass after extracting shared highlighting; native strict all-target Clippy passes.
- Browser UI verified against localhost and the deployed HTTPS website: initial sample tables/results, editable SQL, Cmd+Shift+Enter script execution, separate result tabs and reset.
- Real UPDATE changes the sample customer name to `WASM visitor`; a subsequent SELECT returns it. Reset restores `Avery Chen` without rerunning the edited script.
- `test-worker.mjs` executes the same worker source using SQLite's Node WASM distribution. It verifies writes, NULL, exact 9007199254740993 integer rendering, zero-row column metadata, 500-row cap, current/selected statement behavior, SQL error recovery and a recursive query interrupted by the progress deadline.
- Landing page replaces the primary screenshot with a lazy iframe, full-size link, explicit WASM wording and a screenshot fallback. Desktop screenshots remain available in a collapsed section.
- Browser UI WASM is approximately 3.5 MB (1.36 MB gzip); SQLite WASM approximately 0.88 MB (0.41 MB gzip). Build and deployment ran locally; no desktop release or GitHub build was triggered.

## Limits

- Desktop connection manager, saved files, PostgreSQL/network drivers, full editing workflows, LSP completion and native menus are intentionally absent from this sample-data browser edition and remain desktop functionality.
- Current browser acceptance is the available in-app browser, not a complete Chrome/Firefox/Safari/mobile matrix. Canvas accessibility and touch workflow coverage remain open.
- Displayed results and cell previews are capped; SQL has a progress deadline and worker watchdog. Cancellation resets this disposable database rather than preserving a transaction.
- Browser theme is explicitly dark to match the landing page; this does not change the desktop system-following theme behavior.
