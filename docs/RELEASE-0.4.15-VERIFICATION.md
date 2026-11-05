# v0.4.15 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.15.
Published November 5, 2026 at 09:30:43 America/Chicago (15:30:43 UTC).
Tag commit: `5afe57b7fb1b055f12be719864a84365a304f283`.
Main CI `42053728803` and signed release `42053733356` succeeded.

## Validation

All four reviewed PRs are included: SQL comments, editable query results,
context-aware autocomplete, and cross-database Safe mode. See
[the Safe mode review](PR2-SAFE-MODE-REVIEW.md) for implementation boundaries,
protocol budgets, driver fixtures, and performance evidence.

- Final pre-release library suite: 601 passed, 133 opt-in tests ignored.
  Strict Clippy and separate locked WASM package check passed (WASM has warnings).
- Live disposable MySQL/MariaDB Safe mode and PostgreSQL cancellation tests passed.
- Final five-pair release CPU UI comparison passed all 19 existing metrics.
- Native macOS demo checked toolbar grouping, sidebar toggle, and edit preview.
- All six payload hashes matched SHA256SUMS. Windows ZIP/three x64 PE binaries
  and Linux archive/x64 ELF binaries passed integrity and license-manifest checks.
- macOS deep/strict code signatures, Gatekeeper, stapled app/DMG notarization,
  DMG integrity and Sparkle Ed25519 signature passed.
- The appcast embeds the exact release notes and immutable v0.4.15 asset URL.
  All seven public latest-download links returned HTTP 200 and the public
  appcast matches the verified asset.

## Website

Website commit: `b6a5c83`. Cloudflare version:
`e8277295-a658-41c2-afed-91aea7e327b6`.
Live BUILD.txt reports 0.4.15, the exact tag commit, and clean tracked source.
Live WASM SHA-256 matches the release artifact:
`8cd1c4e0d6b7f43d750ccae25b9eaf1299c7d3e1a8bee7ebf991127932e83290`.
Local artifact and live startup plus `SELECT 415 AS release_check` passed.
Three embedding tests and live COOP/COEP/CSP checks passed. Raw local verification
logs and screenshot are under `/tmp/rustable-v0415/`.

Native Windows/Linux GUI acceptance, installed old-to-new Sparkle updating,
cancellation-dialog native visuals and mixed-DPI acceptance were not exercised.
The CPU benchmarks do not measure input-to-display latency. This release does
not claim to resolve the previously reported external-monitor resizing issue.
