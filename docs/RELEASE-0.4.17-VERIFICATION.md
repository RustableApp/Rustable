# v0.4.17 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.17.
Published November 6, 2026 at 19:15:07 America/Chicago (November 7, 01:15:07 UTC).
Tag commit: `be08af55a74d2a23074e1aec81c0c71564504532`.
Main CI `42321582007` and signed release `42321584886` succeeded on all platforms.

## Validation

This release adds macOS Control-click support throughout existing context menus.
Separate uncommitted connection-import work was excluded. See
[context-click validation](CONTEXT-CLICK-VALIDATION.md) for implementation scope,
regressions, native preview evidence, and local CPU measurements.

- Exact release macOS library suite: 607 passed, 134 opt-in tests ignored.
  Windows/Linux release tests passed. Pre-release strict library/test Clippy passed.
- Separate locked browser-package WASM check from a clean tag checkout passed
  with existing WASM warnings.
- Local CPU comparison: 18 metrics passed and one remained inconclusive due to
  noise; no confirmed regression. This is not an all-metrics performance pass.
  The focused input-frame workload detected no slowdown. Native input-to-display
  latency was not measured.
- Native macOS disposable preview verified Control-click opening the cell menu.
  The exact release browser artifact also opened its cell menu with Control-click.
  Both local and live demos returned 417 for `SELECT 417 AS release_check;`.
- All six payload hashes matched SHA256SUMS. Windows ZIP/three x64 PE binaries
  and Linux archive/x64 ELF binaries passed integrity and license-manifest checks.
- macOS deep/strict code signatures, Gatekeeper, stapled app/DMG notarization,
  DMG integrity and Sparkle Ed25519 signature passed.
- The appcast embeds the exact release notes and immutable v0.4.17 asset URL.
  All seven public latest-download links returned HTTP 200 and the public
  appcast matches the verified asset.
- No database/protocol sequencing changed; no network exchanges were added.

## Website

Website commit: `981159b`. Cloudflare version:
`505a6af1-9baa-4a22-8fca-1f4c2742bc6b`.
Live BUILD.txt reports 0.4.17, the exact tag commit, and clean tracked source.
Live WASM SHA-256 matches the release artifact:
`d9ee9882539bc114fa0f59c7cc8c1f62839730630f6b875087c0d079758a51ed`.
Three embedding tests and live COOP/COEP/CSP checks passed. Raw package checks,
public verification logs and live demo screenshot are under `/tmp/rustable-v0417/`.

Physical trackpad gestures, native Windows/Linux GUI acceptance, installed
old-to-new Sparkle updating, mixed-DPI and 60/120 Hz display acceptance were not
exercised. This release does not claim to resolve the previously reported
external-monitor resizing issue.
