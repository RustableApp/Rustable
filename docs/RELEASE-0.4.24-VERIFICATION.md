# v0.4.24 release verification

Published 2026-11-12T17:16:23Z. Release ID `RE_kwDOVeqlPc4aa89i`.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.24), immutable tag `78bcaa79b8d924d3f5bceaf12cb381adbb2a91b9`.
- Fixes query line/column footer alignment and multiple-result tab layout. Result tabs share workspace-tab styling, remain one line, scroll horizontally and render only visible tabs.
- Local locked suite: 796 passed, 207 opt-in tests ignored. Strict all-target Clippy with QA passed. Separate browser-package locked WASM check passed with 78 existing warnings.
- Dark/light native captures inspected. Layout regression checks include narrow/wide windows, four-digit gutters, 128 results, selection and revealing offscreen tabs.
- All 23 release CPU UI comparison metrics passed against `268c4e6`; 128-result p95 decreased 40%. Raw evidence: `target/query-tabs-style-performance/`; scope and timings in UI-PERFORMANCE.md.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43265483854) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/43265488233) passed all jobs.
- Downloaded draft assets: all six payload checksums passed. Mac deep/strict signatures, stable signing team/bundle identity/updater key/feed URL, Gatekeeper, app/DMG notarization tickets and DMG integrity passed. Sparkle Ed25519 signature and embedded release notes passed.
- Windows ZIP integrity/x64 PE executables and Linux x64 ELF executables/license manifests passed. Exact demo provenance matches the tag with unmodified source.
- All seven public latest-download links returned HTTP 200; all GitHub asset digests match verified files. Public updater feed matches the verified signed asset.

## Matching website

Website commit `9373fb7`; Cloudflare version `20509808-0114-4afa-8e57-9ee292db3274`.
All 14 website tests passed. All six live landing pages, demo BUILD.txt and WASM
match deployed source. Live WASM SHA-256: `5706bc68659ad3f0fe688ced964b5a81f20208a1b8807079a1fdd755fd044616`.
COOP/COEP and WASM CSP passed. Exact artifact and live demo startup were visually
verified; two-statement execution displayed both result tabs and returned 424
for `SELECT 424 AS release_check;`.

Local release evidence is retained in `target/release-v0.4.24/`.
Native Windows/Linux GUI acceptance, Intel Mac, installed old-to-new Sparkle
installation, mixed-DPI/multiwindow/monitor reconnect checks and native
60/120 Hz input-to-present/frame-interval measurements remain unverified.
No database or protocol sequencing changed. No published asset was replaced.
