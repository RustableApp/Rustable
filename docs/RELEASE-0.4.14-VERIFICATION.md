# v0.4.14 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.14.
Published November 4, 2026 at 20:22:20 America/Chicago (2026-11-05 02:22:20 UTC).
Tag: `ab91cf986c43f3c7c761257d5c2198ab8262c019`.
CI `41952623942` and signed-release workflow `41953628691` succeeded on all platforms.

## Validation

- 540 enabled local tests passed, 126 optional tests ignored; strict all-target
  Clippy passed. The separate locked WASM package check passed with warnings.
- Workspace close tests cover each active/closed position, preserving remaining
  order and selection, and refusing to close a busy workspace.
- Existing toolbar geometry regression passed; packaged and live browser rendering
  visually confirms equal status-bar and button-group heights.
- Added optional idle PostgreSQL browse measurements at 0, 1 and 3 seconds.
  Delayed TCP and SSH-over-delayed-TCP runs passed before the version bump;
  SSH medians were approximately 44, 42 and 44 ms respectively. No production
  database protocol or network sequencing changes are included.

All six payload hashes matched SHA256SUMS. Windows ZIP integrity and three x64
PE executables passed; Linux archive and x64 ELF checks passed. Dependency license
manifests are present. macOS deep/strict codesign, Gatekeeper, stapled app/DMG
notarization tickets, DMG integrity and Sparkle Ed25519 signature passed. The
appcast embeds the exact Markdown release notes. All seven latest-download URLs
returned HTTP 200; the public update feed matches the verified package.

## Website

Website commit: `339cd14`. Cloudflare version:
`dca58a78-bcfd-4752-9a39-002ee170c22f`.
Live BUILD.txt identifies 0.4.14, the exact tag commit and clean tracked source.
Live WASM SHA-256 matches the tagged release artifact:
`37da22c687e96a9a56e5be987b707dfcc7b134fd657779cf74273c535915eecd`.
Artifact and live startup plus `SELECT 414 AS release_check` passed with no page
errors. Three embedding tests and live COOP/COEP/CSP header checks passed.

Native Windows/Linux GUI acceptance and installed old-to-new Sparkle updating
were not exercised. External-monitor reconnection/tiling remains unresolved;
this release does not claim a fix for it.
