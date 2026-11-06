# v0.4.16 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.16.
Published November 6, 2026 at 15:10:42 America/Chicago (21:10:42 UTC).
Tag commit: `600eadd8b25d7d57aea46c408e5857995d7cdc49`.
Main CI `42292230863` and signed release `42292264274` succeeded on all platforms.

## Validation

This release fixes autocomplete opening during caret navigation and offering
redundant completions for complete tokens. Uncommitted connection-import work
was excluded; the release was checked from an isolated, clean tag checkout.

- Exact release macOS library suite: 603 passed, 133 opt-in tests ignored.
  Windows and Linux release tests also passed. Final local completion subset:
  45 passed, 6 ignored. Strict library/test Clippy passed.
- Separate locked browser-package WASM check passed (existing WASM warnings).
- Five alternating release-mode CPU benchmark pairs compared against `21b0395`,
  using the standard UI runner plus the existing completion catalog workload.
  The first run had three noisy cases; an exact-binary five-pair repeat passed
  all 27 metrics. Completion p95 changes ranged from -1.4% to +5.7%; largest
  table-catalog completion was 0.5595 ms baseline versus 0.5598 ms candidate.
  Thresholds were unchanged. Raw samples and environment records are retained
  under `target/completion-navigation-performance/` and
  `target/completion-navigation-performance-repeat/`.
- Browser artifact visual checks confirmed partial-token suggestions, dismissal
  on caret navigation, and no popup for completed SELECT. Local artifact and
  deployed demo both returned 416 for `SELECT 416 AS release_check;`.
- All six payload hashes matched SHA256SUMS. Windows ZIP/three x64 PE binaries
  and Linux archive/x64 ELF binaries passed integrity and license-manifest checks.
- macOS deep/strict code signatures, Gatekeeper, stapled app/DMG notarization,
  DMG integrity and Sparkle Ed25519 signature passed.
- The appcast embeds the exact release notes and immutable v0.4.16 asset URL.
  All seven public latest-download links returned HTTP 200 and the public
  appcast matches the verified asset.
- No database/protocol sequencing changed; no network exchanges were added.

## Website

Website commit: `051b098`. Cloudflare version:
`fc986c75-b221-414f-a0d2-33a3a97a6b1e`.
Live BUILD.txt reports 0.4.16, the exact tag commit, and clean tracked source.
Live WASM SHA-256 matches the release artifact:
`ad4f8d834891a7bc0bd7f706c16e3779ddeaea99a3f5639b31763ba88b853192`.
Three embedding tests and live COOP/COEP/CSP checks passed. Raw package checks,
public verification logs and live demo screenshot are under `/tmp/rustable-v0416/`.

Native Windows/Linux GUI acceptance, installed old-to-new Sparkle updating,
native editor visuals and mixed-DPI acceptance were not exercised. CPU benchmarks
do not measure input-to-display latency. This release does not claim to resolve
the previously reported external-monitor resizing issue.
