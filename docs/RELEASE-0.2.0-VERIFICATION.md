# v0.2.0 release verification

## Provenance

Application source is immutable tag `v0.2.0`, commit
`7ffdc801c889125f39b1202c1de350995e179f1e`.

- Linux, Windows and WASM: GitHub Actions run `41317537447`.
- macOS: isolated recovery run `41317706825`, checking out the same tag.
  The original macOS 14 runner was cancelled by GitHub's retirement brownout
  before executing any steps. Workflow-only commits `6c5a440` and `36a303c`
  moved the runner to macOS 15 and isolated recovery concurrency. Successful
  Linux/Windows/WASM jobs were not restarted, and the release tag was not moved.

## Live WASM demo

- Website commit: `6b11228` in RustableApp/website.
- Cloudflare deployment: `be8fcde7-2e9d-4e58-b2c6-c5593621452b`.
- Published at https://rustable.app/demo/ on November 1, 2026.
- `BUILD.txt` reports v0.2.0, the tagged commit above, and no modified tracked files.
- Public WASM SHA-256 matches the exact CI artifact:
  `da5d69ff414160658700830abfe368f527dd5dfc74c00527a4d81331f16485d3`.
- Live UI startup and `SELECT 42 AS release_check;` verified; returned 42.
- COOP, COEP and CSP headers verified on the live response.
- Three website embedding tests passed. The release workflow and release guide
  now require the matching demo for every desktop release.

## Pre-packaging checks

462 native unit tests passed, 107 fixture/manual/benchmark tests ignored.
Strict all-target Clippy, locked WASM compilation, and disposable MySQL,
MariaDB and PostgreSQL browse/export checks passed. Native preview checks
covered compound filtering and preserving multi-selection on right-click.

## Package and publication checks

- Linux: all three binaries have ELF headers and executable permissions;
  2,130 archive entries, with licenses present.
- Windows: ZIP CRC verification passed for 2,129 entries; application and both
  language servers have valid PE headers; licenses present.
- macOS: DMG integrity and stapled ticket validated. Mounted read-only;
  application passed deep/strict codesign verification and Gatekeeper accepted
  it as Notarized Developer ID. Application ticket validated separately.
  Bundle version is 0.2.0; Applications symlink and Finder layout are present.
- The installed v0.1.0 public key independently verified the ZIP's Ed25519
  signature using CryptoKit. The app's public key and updater URL are unchanged.
  Feed URL points to the immutable v0.2.0 archive and byte length matches.
- All six uploaded assets were downloaded back from the draft and matched
  `SHA256SUMS`. The manifest is the seventh release asset.
- Published stable [v0.2.0](https://github.com/RustableApp/Rustable/releases/tag/v0.2.0),
  GitHub release ID `430656450`, on November 1, 2026. Latest-release API confirms
  v0.2.0, not draft or prerelease. All seven public latest-download links return
  HTTP 200; the latest updater feed points to v0.2.0.

## Acceptance limits

Archive inspection and CI tests do not establish native Windows/Linux GUI
acceptance. Those UI checks, Intel macOS, and an end-to-end installed
old-to-new Sparkle update remain unverified. Windows SSH is unavailable;
Windows/Linux are portable packages with manual updates. See release notes
for remaining driver, raw-filter and selected-cell limitations.
