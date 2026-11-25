# v0.4.41 release verification

Published at `2026-11-25T18:47:27Z`, GitHub release `458510192`:
[v0.4.41](https://github.com/RustableApp/Rustable/releases/tag/v0.4.41).
The immutable tag points to `0d7444b96afb719615ee5eea40850ad8559ed114`,
with matching Cargo version and root/browser lockfiles. The v0.4.40 candidate
was never published: the reported PostgreSQL citext defect was fixed before
publication, and its existing tag was preserved rather than moved.

Signed-release run `45676642763` and main CI `45676642354` passed on every
platform. All packages came from the same release run and tag. Release notes
include native click handling, selected cell editing, certificate browsing,
readable query confirmation, refreshed icons and the citext decoder fix.
GitHub issues #19 and #20 were closed with links to this published release.

All six payload checksums and all seven uploaded asset digests/sizes matched.
macOS passed deep strict code signing, Gatekeeper assessment, app/DMG stapled-ticket
validation, DMG integrity, stable updater identity, Sparkle Ed25519 signature and
embedded release notes. The packaged executable reports v0.4.41 and lists 11 MCP
tools without opening a database. Windows x64 PE and Linux x64 ELF app/language
servers and bundled licenses were inspected. Vendored winit and egui-winit notices
match upstream in every native package (normalizing platform line endings).
All seven versioned and latest asset URLs returned HTTP 200, and the latest
updater feed exactly matches the verified signed feed.

Local checks passed: 955 ordinary tests (273 ignored), strict all-target QA
Clippy, the separate locked browser-package WASM check (85 existing warnings),
five packaging tests and 14 website tests. The opted-in PostgreSQL citext
scalar/domain/array, cold/warm, NULL/empty, transaction and actual cell-editor
save/reread tests passed on a disposable PostgreSQL 17 fixture. The regression
fails with the old decoder. All three custom-type streaming tests and the
release-mode editor/browse WAN regressions passed. The disposable container was
removed after testing; no user database was involved.

Citext decoding adds no protocol exchanges. At 30 ms simulated RTT (seven
samples), editor lookup p50/p95 was 39.0/40.1 ms and browse/refresh was
40.5/42.7 ms. The respective single-exchange reference p50 values were 36.9 and
36.8 ms. See [QUERY-RESULT-EDITING.md](QUERY-RESULT-EDITING.md) for cold/warm
exchange counts, coverage and measurement limits. These checks ran on macOS
27.2.1 arm64 with Rust 1.99.1, using the release profile.

Native click validation includes macOS word/paragraph mouse-down selection,
word dragging, selected cell entry/Undo and one connection-open request per
double click. The native certificate picker was observed opening; field routing,
cancellation, Unicode paths and stale draft delivery are covered by tests.
See [UI-PERFORMANCE.md](UI-PERFORMANCE.md) for hardware, workloads, raw artifacts
and thresholds. Nine broad repeated scenarios passed; cell-entry measurements
were inconclusive (+14.6% p95, only two of five pairs above 10%). Word dragging
increased from 15.541 to 18.917 microseconds p95 (+21.7%, +3.38 microseconds),
accepted for whole-word selection instead of the baseline character selection.
Editor click selection improved from 166.750 to 18.584 microseconds p95.
These are CPU layout/tessellation measurements, not input-to-display latency.

Website content commit `b9cb947` contains the exact tagged browser demo; README
provenance was committed as `d978397`. Cloudflare version
`86a47e41-56d9-40fb-bfbc-ed679d9386a6` serves it. Six public version pages,
BUILD.txt and WASM bytes match source; isolation/CSP and indexing headers remain
intact. Public WASM SHA-256:
`274aa5e7b32bfbb6018b842d818e59255f703042ad5b7b4f1d9fe5cbb5b5d902`.
Local and live startup and SQLite query acceptance passed. The live query
`SELECT 441 AS release_check;` returned one row containing 441 after Cmd+Enter.
Local native typing delivered a shorter alias before execution, still returning
441; live acceptance used paste and verified the complete query before running.
Local Wrangler used compatibility date 2026-10-03 because its installed runtime
predates the production setting; production configuration was unchanged.

Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle old-to-new
upgrades, actual Touch ID authorization, 60/120 Hz input-to-present timing and
mixed-DPI/window-management checks remain unverified. Linux uses the improved
shared click fallback; exact desktop click-preference parity is not implemented.
Raw artifacts/logs are retained under `target/release-v0.4.41/`, with earlier
click/issue verification under the paths recorded in UI-PERFORMANCE.md.
