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

- Browser query-library storage now uses IndexedDB transactions with 5,000-item / 32 MiB bounds. Group creation, inline rename and persistence across reload were verified in the full browser UI (`Browser examples`). Three Node tests verify group-child renames, stale-save conflict handling, path/name collisions and oversized SQL rejection. Named save/open UI is now connected: Cmd+S opened the naming dialog, `Revenue by plan` was saved in `Browser examples`, persisted across reload, selected without opening on an explicit single click, and opened on double click with its full SQL. Query listing uses IndexedDB index keys, avoiding document-body reads during sidebar refresh. Five native query-library tests pass; save conflict handling is tested at the storage layer, and multi-browser-tab conflict UI subsequently passed (see below).

- Browser SQLite now has a 64 MiB SQLite allocator limit, 8 MiB SQL/value limits, and no attached databases. This is not a whole-tab or WASM-heap bound. Two native tests verify oversized-value rejection, subsequent query recovery, and explicit rollback after rejection. In the browser, `zeroblob(8388609)` failed cleanly; a billion-step recursive aggregate stopped at 2000.1 ms; Cancel stopped the rerun at 24.2 ms; the same connection then returned 42.

- Loaded-result CSV/JSON export now runs in a separate WASM worker using the desktop streaming encoders, with a 16 MiB encoded-output cap and ten-second worker deadline. Two tests verify exact large integers, NULL, Unicode, binary data, duplicate JSON-name rejection, and bounded writes. Actual browser CSV and JSON downloads were captured and read back from disk. Both preserved integer 9007199254740993, NULL and Japanese text; JSON emitted binary bytes as [0,255], CSV as 00ff. The earlier missing-file observation came from not registering the browser download event before triggering the download.

- Database-worker failures now mark the session disconnected and reject further SQL. A ten-second no-response watchdog terminates the worker and clears the pending operation. Local preview fault injection verified both an immediate worker crash and a worker that never replied (10013.4 ms); `Start fresh demo` restored the sample tables in both cases, and browser-saved queries survived. Fault scripts were removed from preview staging; no fault hooks ship in source.

- SQL import/download verified through the browser filechooser/download events: a BOM-prefixed UTF-8 file containing Japanese text and CRLF imported successfully; the downloaded SQL exactly matched the input after the intended BOM removal. Invalid UTF-8 and an 8 MiB + 1 byte file were rejected without replacing the open document. Browser error messages now omit JavaScript stack traces.
- In the full browser UI, a recursive UPDATE timed out after 2000.4 ms inside an explicit transaction after an earlier UPDATE. The next SELECT returned the original value 7, confirming rollback and same-connection recovery. A native regression test also checks SQLITE_INTERRUPT, autocommit restoration, original data and a subsequent committed write.

- Full-table CSV/JSON export now uses the desktop SQLite streaming path into a bounded 16 MiB download buffer in the database worker. A native test checks 1,200 rows across pages despite a nonzero page offset, descending sort, matching/empty filters, cancellation, active-transaction rejection and oversized-output rejection followed by connection recovery. Existing native full-export tests also pass (three total, one manual benchmark ignored). The actual browser JSON download contained all 2,400 customers, IDs 1–2,400, while the grid loaded 500 rows. Native strict all-target Clippy and the release WASM build passed.

- Browser CSV preview/import now uses an immutable in-memory snapshot (8 MiB cap) and the existing parser, mapping, SQL review and transactional SQLite importer. The browser fixture mapped all eight customer columns, imported successfully, and a subsequent SELECT returned id 90001, name CSV visitor and lifetime_value 42. Five parser/snapshot tests pass, including memory-source reparsing, 150-row traversal beyond the 100-row preview, source identity, and size rejection. Native Clippy and release WASM linking passed. Browser late-row failure recovery also passed: a 151-row CSV showed only 100 preview rows, then failed at data row 151 on a duplicate primary key. The review retained its source/mapping and displayed the row-specific error. After closing the review, a SELECT on the same connection counted zero rows from the preceding 150 inserts. Browser cancellation, parser-option changes and oversized-picker rejection remain to verify.

- SQLite file opening now deserializes a browser-selected standalone database into a writable, disposable copy (8 MiB input cap), validates it with quick_check, and replaces the session only after metadata loading succeeds. Browser verification opened a fixture, browsed its row, updated it, rejected a CSV supplied as a database, then queried the preserved updated copy. Reading the original fixture from disk confirmed it was unchanged. A native regression covers malformed/truncated/oversized/WAL input, writable-copy isolation, failed-replacement preservation and active-transaction protection. Native Clippy and release WASM build passed.

- SQLite allocation exhaustion was exercised in the browser with one atomic INSERT of 100 one-MiB blobs. It failed with out-of-memory after 13.3 ms, left zero partial rows, and the same connection then accepted a 1024-byte blob insert and returned its length. Reproduction: `tools/browser/tests/fixtures/sqlite-memory-recovery.sql`, running each numbered section separately. This verifies the SQLite allocator failure path, not total tab/WASM heap bounds.

- Saved-query conflict UI passed with two independent browser tabs sharing IndexedDB. Both loaded the same baseline; tab A saved SELECT 2, tab B attempted SELECT 3 and received a conflict without losing its dirty SQL. Save As preserved tab B as a separate query. Reloading tab A and reopening the original returned SELECT 2. This exposed a stale conflict banner after recovery; successful browser document operations now clear prior browser-storage errors while leaving unrelated errors alone. Save-dialog errors now belong to the draft rather than the refreshed sidebar listing: a conflict remained visible with stable dialog geometry after waiting beyond the five-second refresh interval. Cancelling the dialog remained functional.

- Full native library regression after the shared browser adapters: 391 passed, 0 failed, 44 existing manual/integration tests ignored. This does not replace live database integration coverage.
- Startup now dynamically imports its JS module inside the error handler. Temporarily removing the built module from local preview produced the visible startup failure and screenshot fallback, rather than an endless loading screen. The module was restored after the check.

- Clean full-app build embedded in a copy of the actual landing page at localhost:8768. Production `_headers` now include COOP same-origin and COEP require-corp, and the preview served these plus the existing CSP. The app rendered without console errors and Cmd+Enter inside the iframe returned 42. A 390×844 responsive check exposed a cramped desktop workspace; mobile presentation remains to fix before publication. Temporary viewport override was reset. Website copy now describes actual file-copy/import/export and browser-local saved-query behavior.

## Required before replacement deployment

- Extend CSV failure-path verification and verify database-copy opening with additional browser engines and larger files. Verify recovery after storage failures, and file transfers in additional browser engines.
- Measure whole-tab memory under sustained use. Manual transactional-write cancellation, transactional timeout rollback, and worker loss after an uncommitted write have passed in the in-app browser (details below). Worker startup crash/no-response recovery also passed. Shared cancellation requires cross-origin-isolated hosting and fallback deadline is two seconds.
- Extend keyboard verification to additional browsers and grid navigation. Cell select-all and current/selected/all query execution passed in the in-app browser with browser-native key events.
- Verify reset/reload isolation, download actions, same-origin embedding headers, responsive layout and additional browser engines. Keep website claims synchronized with tested capabilities.
- Remove stale miniature-demo build assets from the deployment staging directory when publishing the replacement. No desktop release or main push is needed for each iteration.

## Deployment history

The earlier reduced demo was tested and deployed at rustable.app with lazy iframe embedding and screenshot fallback. Its separate JS SQLite worker tests and old size/limit measurements do not validate this replacement. They have been removed from the current source to avoid confusing those results with full-workspace acceptance.

### Compact shared workspace (2026-11-01)

Below 720 egui points, the shared app now switches between workspace, Items,
and Inspector instead of squeezing three panes side by side. Opening a table
or saved query returns to the workspace. Compact pane IDs are separate and
compact layouts never persist desktop pane widths.

Verification: native library suite 391 passed, 44 ignored; strict native
Clippy passed; release WASM build succeeded (existing WASM unused-code warnings
remain). Desktop embedded view retains all three panes. A real 360 CSS-pixel
iframe with normal browser scaling displayed readable controls; Items opened
at full width and selecting orders returned to its populated table. Evidence:
`wasm-compact-workspace.png` in the session visualization directory.

The browser tool's 390px viewport override renders canvas text at roughly half
size despite normal surrounding HTML. This did not reproduce with a narrow
iframe at normal viewport scaling, so no compensating app zoom was introduced.
Actual mobile-device verification remains outstanding. Narrow table controls
wrap into multiple rows and still need layout polish.

### Embedded keyboard and manual cancellation (2026-11-01)

In the actual landing-page iframe, clicking the first customer's name and
pressing Tab selected its email; Shift+Tab then Down selected the second
customer's name. The inspector followed that row (id 2, Noah Williams).

Explicit write cancellation passed using the fixture
`tools/browser/tests/fixtures/sqlite-manual-cancel.sql`. An initial attempt
reached the two-second deadline before the click; this was not counted as
manual cancellation. On the second run, the toolbar cancel button interrupted
the recursive UPDATE after 13.8ms, cleared the open-transaction indicator,
and a subsequent SELECT returned the committed baseline 7 (not the earlier
uncommitted update to 8). The connection remained usable. These observations
are for the in-app browser; only that browser engine is currently exposed by
the available browser-control inventory.

### Rebuilt worker transport and reload isolation (2026-11-01)

After adding active SQL dialect to worker payloads, the browser package's separate
lockfile and vendored mysql patch mapping were synchronized. The release WASM
build passed; mysql remains a native-only dependency. Native library verification
passed 399 tests (53 ignored), with strict all-target Clippy passing.

In the rebuilt actual landing-page iframe at localhost:8768, created reload_probe,
inserted a row and executed all three statements successfully. Reloading the page
removed that table: querying sqlite_schema returned count 0. Saved the check as
`Reload isolation check`, reloaded again, and opened it from Queries by double
click. Its SQL survived intact and executed against the fresh database. This
verifies database disposal independently from persistent browser-local queries
in the in-app browser. Evidence: `wasm-reload-isolation.png` in the session
visualization directory. The public replacement remains undeployed; additional
engine, storage-failure and transactional-worker-failure checks remain open.


### Worker loss after an uncommitted write (2026-11-01)

Verified in the actual landing-page iframe with a preview-only fault inserted
immediately after `database_command(data)` and before `postMessage`. The fault
threw when the command bytes contained `FAULT_WRITE_20261101`. No fault code was
added to repository or production assets; the preview worker was restored from
`tools/browser/web/database-worker.js` before recovery verification.

1. Executed `BEGIN; UPDATE customers SET name='Transaction pending' WHERE id=1;`.
   The UI showed TRANSACTION OPEN and both statements completed.
2. Executed `UPDATE customers SET name='FAULT_WRITE_20261101' WHERE id=1;`.
   The worker executed the command, then failed before acknowledging it. The UI
   showed CONNECTION LOST and explicitly said the failed database was unrecoverable.
3. Tried `SELECT 42;`. The UI rejected it as disconnected and kept Start fresh demo
   available; it did not silently reconnect or replay the write.
4. Restored the clean worker script and clicked Start fresh demo. The transaction
   warning cleared and the sample database returned. Executing
   `SELECT name FROM customers WHERE id=1;` returned `Avery Chen`.

This proves disposal and recovery of the in-memory worker session, not rollback
or recovery of its lost contents. Screenshot evidence:
`wasm-transaction-worker-failure.png` and `wasm-transaction-worker-recovery.png`
in the session visualization directory. The public replacement is still pending.

### Incremental library writes and aborted-save preservation (2026-11-01)

Browser query saves no longer clear and rewrite every library entry. The existing
readwrite transaction still performs conflict and total-size checks, but applies
only changed paths. Renaming a group moves its descendants atomically and leaves
unrelated entries untouched. Unchanged saves issue no writes. Snapshot reads and
size validation still scale with total library size; this is not a constant-time
save claim. Five Node tests pass, including delta application and sibling-prefix
isolation. The release WASM build passes.

In the actual iframe, a preview-only fault aborted the IndexedDB transaction
after scheduling its writes when SQL contained STORAGE_ABORT_20261101. Saving an
edited existing query showed the injected error, retained the SQL in the editor,
and kept the dirty-tab marker. Reloading and reopening the stored query returned
its original SQL, proving the aborted transaction did not replace it. Evidence:
`wasm-storage-abort.png` in the session visualization directory. This simulates
transaction abort, not actual browser quota exhaustion. The clean release build
replaced the injected preview assets afterward. No fault hooks ship in source.

### Retained download-buffer budget (2026-11-01)

SQL, CSV and JSON downloads now share a 32 MiB budget for retained object-URL
Blobs. Each URL still expires after 60 seconds to allow the browser to consume
it; repeated exports above the shared budget fail with a wait-and-retry message
before allocating another Blob. Failed download launch revokes its URL and
returns its budget immediately, including exceptions during DOM setup.

A Node test uses real typed arrays and Blobs with controlled URL/DOM/timer
adapters to verify budget rejection, expiry, failed-launch cleanup, retry,
filename sanitization and idempotent cleanup. All six browser-adapter tests
pass; release WASM linking passes. This is an object-URL retention bound, not a
whole-tab memory bound or proof that browser-managed download copies are freed.
Actual browser downloads were verified earlier; cross-engine coverage remains
outstanding.

### Validate worker messages before copying (2026-11-01)

Database and background-job reply handlers now require a Uint8Array and check
its length against the shared 32 MiB transport limit before copying into WASM
memory. Previously Uint8Array construction/copy preceded bincode's limit check.
Native transport regressions and the release WASM build pass.

A local preview worker transferred a 32 MiB + 1 byte Uint8Array on startup. The
app displayed the transport-limit error and disconnected cleanly. After restoring
the original worker script, Start fresh demo loaded the sample tables again.
Evidence: `wasm-oversized-response.png` in the session visualization directory.
The fault was preview-only. This avoids an extra oversized WASM allocation; it
cannot prevent the browser from receiving the original structured-clone buffer.

### Complete generated-directory replacement (2026-11-01)

The browser build now stages bindings, web assets and dependency licenses in a
sibling temporary directory before replacing its output. This removes stale
miniature-demo workers, SQLite assets and obsolete licenses instead of merging
new files into an old deployment. Nonempty output directories must carry the
Rustable browser-demo BUILD.txt marker; unrelated directories are rejected.

The current release WASM build succeeded into the local embedded preview.
A sentinel from the previous generated directory was removed. An injected
wasm-bindgen failure preserved every previous output file byte-for-byte, and
an unrelated directory containing user.txt was rejected without modifying it.
Installation uses two directory renames with rollback on an installation error;
it is not a claim of atomic replacement for concurrent HTTP readers. Build into
staging before deploying. Existing WASM warnings remain. No public deployment
or desktop release was performed for this change.

### CSV parser controls and oversized picker (2026-11-01)

Verified the newly rebuilt full app in the localhost:8768 landing-page iframe.
Opened this UTF-8 fixture through Export → Preview CSV file:

```csv
name;email
"Browser; visitor";browser@example.invalid
```

The default comma parser showed one column. Choosing Semicolon reparsed the
same 54-byte snapshot into two columns and one data row, preserving the quoted
semicolon in `Browser; visitor`. Disabling First row is header reparsed it into
two data rows with generic Column 1/Column 2 headings. No import was committed.

A separate 8 MiB + 1 byte CSV selected through the actual file chooser was
rejected with `CSV files are limited to 8 MiB`. Dismissing the error and refreshing
customers succeeded on the same session, with its original sample data visible.
Evidence: `wasm-csv-options.png` and `wasm-csv-oversized.png` in the session
visualization directory. This closes delimiter/header-change and oversized-picker
checks for the in-app browser only; import cancellation, other engines and
whole-tab memory measurement remain outstanding. No public deployment occurred.

### Deterministic shared-import interruption regression (2026-11-01)

Added a native regression against the SQLite importer shared by desktop and
WASM. An SQLite update hook observes at least 100 actual destination inserts
before the progress handler interrupts execution. It covers both interruption
alone (as with a deadline) and interruption with the explicit cancellation flag.
Both cases leave zero imported rows, restore autocommit, clear cancellation,
preserve the interruption error without a spurious rollback-failure message,
and allow the same 1,000-row request to succeed on retry. All three SQLite CSV
import tests and strict all-target Clippy pass. This strengthens transaction
coverage but does not substitute for exercising the browser Cancel import
button and its shared-memory signal; that UI check remains outstanding.

### Browser import cancellation and automatic rollback fix (2026-11-01)

A real browser import into a disposable table with an expensive AFTER INSERT
trigger exposed a gap in the earlier native test: interruption inside a write
can make SQLite roll back automatically. Rustable then attempted another ROLLBACK
and incorrectly appended `rollback failed: cannot rollback - no transaction is
active`. The initial browser attempt reached its 2003.7 ms deadline and was not
counted as manual cancellation. Adding a trigger to the native regression
reproduced the same failure at row 100.

The importer now checks autocommit before explicitly rolling back, preserving
the original interruption error when SQLite has already done the cleanup.
All three SQLite import tests, strict all-target Clippy, and the release WASM
build passed. In the rebuilt full-size browser workspace, clicking Cancel import
stopped the 100-row fixture at row 3 after 388.5 ms, well before the deadline.
The review showed only `CSV data row 3: interrupted` and retained its source and
mapping. Closing the review and running SELECT count(*) on the same connection
returned zero rows. This verifies the actual browser cancellation signal and
rollback path in the in-app browser, not additional browser engines.

Reproduction uses a table with id INTEGER PRIMARY KEY and an AFTER INSERT
trigger executing a recursive sum from 1 through 1,000,000; CSV contains header
id and values 1 through 100. Evidence in the session visualization directory:
`wasm-import-trigger-before.png`, `wasm-import-cancel-fixed.png`, and
`wasm-import-cancel-recovery.png`. The public demo remains unchanged.

### Parent-page startup deadline (2026-11-01)

The website embed controller now falls back if the boot script cannot run or
never signals readiness. Its 60-second deadline starts when the lazy iframe
approaches the viewport or loads, not when an offscreen page is first opened.
Only messages from that frame and the same origin can signal readiness/failure.
On failure it removes the iframe and reveals the existing desktop screenshot.

Three Node tests in rustable-website/tests/demo-embed.mjs pass: lazy startup,
missing readiness, ready-before-load ordering, foreign-message rejection and
explicit/frame errors. In local preview only, temporarily removed boot.js and
shortened the deadline to three seconds: the screenshot and explanatory caption
appeared. Restored boot.js and the production 60-second controller; reloading
rendered the live workspace normally. Evidence: wasm-missing-boot-fallback.png
in the session visualization directory. No fault code remains in preview.
This does not close cross-browser, mobile-device or whole-tab-memory checks.

## Production deployment — 2026-11-01

Built app source `8a0babd` into the website bundle using the staged browser builder.
The WASM binary is 11,173,773 bytes uncompressed. Website fallback tests: 3 passed;
Wrangler dry run passed. Deployed Cloudflare version
`e95b9254-52a4-4cfe-93f6-dd41cbb5a7f6`; website local commit `93f2e27`.
No app main push or desktop release was triggered.

Verified public `/demo/` and WASM responses are HTTP 200 with application/wasm,
COOP same-origin, COEP require-corp and the expected WASM CSP. In the production
landing-page iframe, opened SQL Query 1 and ran
`SELECT count(*) AS customers FROM customers;` using Cmd+Enter: result 2400.
Evidence: `wasm-production-query.png` in the existing visualization evidence directory.

Also verified the latest local full workspace renders in installed Firefox,
including the 500-row customers grid and inspector. This is startup/rendering
coverage only, not a claim of complete Firefox workflow verification. Firefox
required raising its test window before the native screenshot refreshed.
Actual mobile hardware, sustained whole-tab memory measurements and deeper
cross-engine file/cancellation workflows remain follow-up work.

### Browser library rename collision checks (2026-11-01)

Group rename preparation now checks every moved record's destination, not only
the group root. A surviving child record beneath a missing destination group
cannot be overwritten by the rename. SQL-file renames move only that file,
never malformed descendant records. Case-only names preserve SQL text and
unchanged names produce no writes.

Validation: all eight pure browser-library transaction-preparation tests passed
with `mise exec node@24.22.0 -- node --test tools/browser/tests/query-library.mjs`.
The tests verify collision failure leaves the input snapshot unchanged and
compare exact puts/deletes and Unicode content. This change has not yet been
rebuilt into or deployed with the production WASM bundle; browser IndexedDB
transaction execution was not rerun in this pass.
