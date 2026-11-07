# v0.4.18 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.18.
Published November 6, 2026 at 20:24:30 America/Chicago (November 7, 02:24:30 UTC).
Tag commit: `22c35a85e16e94bd965fd0938d62affa0f4de038`.
Main CI `42328283563` and signed release `42328285509` succeeded on all platforms.

## Scope and validation

Includes `9601b9a`: the refined TablePlus/TablePro import dialog and double-click
opening of saved connections. See [import UI validation](IMPORT-UI-VALIDATION.md)
for prior functional tests, the accepted 0.008–0.015 ms import-dialog CPU cost,
and the inconclusive broader performance comparison.

Per the release request, no app, test, or WASM builds were run locally. Exact-tag
builds and tests ran in GitHub Actions, including the separate locked browser
package. Both lockfiles were updated with the package version before tagging.

- All three platform test/build jobs passed. Exact-tag Linux suite: 607 passed,
  135 opt-in tests ignored. Prior targeted connection/import tests are documented
  in the linked validation record; local benchmarks were not repeated.
- All six payload hashes matched SHA256SUMS. Windows ZIP/three x64 PE binaries
  and Linux archive/x64 ELF binaries passed integrity and license-manifest checks.
- macOS deep/strict code signatures, Gatekeeper, stapled app/DMG notarization,
  DMG integrity and Sparkle Ed25519 signature passed.
- The appcast embeds the exact release notes and immutable v0.4.18 asset URL.
  All seven public latest-download links returned HTTP 200 and the public
  appcast matches the verified asset.
- Local serving of the CI-built browser artifact and the deployed live demo both
  passed startup and returned 418 for `SELECT 418 AS release_check;`.

## Website

Website commit: `91d0841`. Cloudflare version:
`d11ca9ef-c6b1-49a8-bad8-6d89f8ba9ef6`.
Live BUILD.txt reports 0.4.18, the exact tag commit, and clean tracked source.
Live WASM SHA-256 matches the release artifact:
`190ef35e49550994df27beda77f396af632877df1e348e0a97e22bba833ad890`.
Three embedding tests and live COOP/COEP/CSP checks passed. Raw package checks,
public verification logs and live demo screenshot are under `/tmp/rustable-v0418/`.

Native import-dialog visuals and OS file-picker interaction, native Windows/Linux
GUI acceptance, installed old-to-new Sparkle updating, mixed-DPI and display-latency
acceptance were not exercised. This release does not claim to resolve the
previously reported external-monitor resizing issue.
