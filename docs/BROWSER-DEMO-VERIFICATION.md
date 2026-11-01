# Full workspace WASM verification — November 1, 2026

## Current source versus deployment

The local browser host now creates `rustable::Rustable::new_demo`, using the actual main window in `src/app.rs`. Native and browser builds share the SQLite adapter and command handler. The earlier separate BrowserApp UI has been removed from source. The public website still serves that earlier reduced demo; the replacement has not been deployed.

## Verified locally

- Native all-target strict Clippy passes with the shared application library and target-specific adapters.
- The SQLite dependency is now rusqlite 0.40.2, locked. Its WASM backend links successfully with rustup's LLVM archiver. Apple ar produced an empty WASM archive, which cargo check alone did not expose.
- The full browser host checks, links in release mode, and generates wasm-bindgen assets.
- Two transport tests pass: typed values (i64::MAX, blobs, NULL, Unicode with embedded NUL, infinity) and transaction state survive the binary worker boundary; truncated packets and oversized declared buffers are rejected. They caught and drove a fix for enum discriminant mismatches caused by skipping native-only variants.
- Browser startup exposed native font-loading threads and unsupported SystemTime calls. The browser now uses bundled egui faces, web-time and Chrono's wasmbind clock support.
- In-app browser at http://localhost:8767/demo/: actual native-style toolbar, sidebar, tabs, inspector and 500-row sample table rendered.
- Ran a two-statement script returning exact integer 9007199254740993 and NULL, plus the selected sample row. Both result tabs appeared.
- Staged a customer-name edit, generated the syntax-highlighted SQL review, saved it through the existing transaction path, and observed the changed row with no pending edits. The initial test value was `Avery ChenWASM visitor`. Follow-up isolated that append to the automation key spelling: browser-native `Meta+a` followed by paste correctly replaces the entire cell value (verified as `Replacement` in both grid and inspector).
- Screenshot: `/Users/jatochnietdan/.codex/visualizations/2026/10/31/01a18f82-8ef6-7c12-b1e5-94903450cc37/full-workspace-wasm-local.png`.
- Earlier native database regression run after the SQLite upgrade: 102 passed, 32 existing integration/performance tests ignored. These are not evidence of live PostgreSQL/MySQL/MariaDB verification.

- Browser SQL Beautify now runs the shared formatter in a dedicated WASM worker. Verified multiline output and successful execution of both formatted statements. Result search runs the shared scanner in a worker; searching `800` in the grouped demo result found three cells and selected the first. Both workers have a ten-second UI timeout and terminate when their handles are dropped. Timeout/error paths still need browser fault-injection verification.
- Search tests: 4 passed. Formatter tests: 5 passed, 1 existing ignored. Native strict all-target Clippy passed after adding the worker adapters.

- Verified browser SQL shortcuts with a two-statement document: Meta+Enter at the second statement returned only 22; Meta+Shift+Enter produced both result tabs; selecting the first statement then Meta+Enter returned only 11. Shortcut labels now use egui’s runtime OS detection instead of the WASM build target, with readable Enter/Shift text for symbols absent from bundled browser fonts.

- Browser query-library storage now uses IndexedDB transactions with 5,000-item / 32 MiB bounds. Group creation, inline rename and persistence across reload were verified in the full browser UI (`Browser examples`). Three Node tests verify group-child renames, stale-save conflict handling, path/name collisions and oversized SQL rejection. Named save/open UI is now connected: Cmd+S opened the naming dialog, `Revenue by plan` was saved in `Browser examples`, persisted across reload, selected without opening on an explicit single click, and opened on double click with its full SQL. Query listing uses IndexedDB index keys, avoiding document-body reads during sidebar refresh. Five native query-library tests pass; save conflict handling is tested at the storage layer, while multi-browser-tab conflict UI remains to verify.

## Required before replacement deployment

- SQL import/download adapters now compile in the full WASM build. Imported files use shared UTF-8/BOM/NUL/8 MiB validation and are opened as copies without an owned filesystem path. Download requests do not mark a disk file as saved. Native script-file tests (3) and strict Clippy pass. Actual picker import/download still requires end-to-end verification: the in-app browser picker did not expose an accessible dialog and native Codex UI control is prohibited.
- Finish CSV/database import and data export; those capabilities still report temporary explicit errors. Verify browser saved-query conflict UI and recovery after storage failures.
- Verify cancellation and timeout rollback/recovery, bounds on SQLite memory and SQL input, and worker error recovery. Shared cancellation requires cross-origin-isolated hosting; fallback deadline is two seconds.
- Extend keyboard verification to additional browsers and grid navigation. Cell select-all and current/selected/all query execution passed in the in-app browser with browser-native key events.
- Verify reset/reload isolation, download actions, same-origin embedding headers, responsive layout and additional browser engines. Keep website claims synchronized with tested capabilities.
- Remove stale miniature-demo build assets from the deployment staging directory when publishing the replacement. No desktop release or main push is needed for each iteration.

## Deployment history

The earlier reduced demo was tested and deployed at rustable.app with lazy iframe embedding and screenshot fallback. Its separate JS SQLite worker tests and old size/limit measurements do not validate this replacement. They have been removed from the current source to avoid confusing those results with full-workspace acceptance.
