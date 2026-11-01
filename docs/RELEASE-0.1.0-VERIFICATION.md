# v0.1.0 release verification

This records evidence for the first release, not completion of TablePlus parity.

## Source and build provenance

- Application source: `v0.1.0` (`ab485a8`). The tag has not moved.
- Linux: successful Ubuntu 24.04 job in GitHub Actions run `41272765556`.
- macOS: the same tagged Rust source built locally with `sparkle-updater`; packaged with the DMG and icon followups. The original CI build passed tests and compilation but failed at signing-key lookup. Local signing success does not prove that the CI signing repair works.
- Windows packaging repair: run `41274333076` builds app/LSP sources from `v0.1.0`, with packaging tooling from `17d22b8`. The original build passed tests and compilation but its ZIP failed on a pre-1980 file timestamp. The repair clamps ZIP timestamps without changing file contents.

## Verified

- macOS app and DMG codesign verification; Apple notarization accepted and tickets stapled/validated.
- Mounted DMG contains the Rustable app icon and `/Applications` symlink; inspected in Finder.
- Sparkle ZIP signature verified with `sign_update --verify`; appcast enclosure byte length matches archive.
- Bundle version is `0.1.0`; update feed points to the repository's latest-release appcast and contains the configured public signing key.
- macOS DMG, macOS ZIP, appcast, and Linux archive downloaded back from the draft release match local SHA-256 hashes.
- Updated website screenshots captured from the native QA app using sample SQLite data, not production data or a mockup.

## Publication — November 1, 2026

- Windows recovery run `41274333076` completed successfully. ZIP CRC/integrity checks pass across 2,025 entries; app and both language servers have PE headers, and bundled licenses are present. This is archive inspection, not a native Windows launch test.
- `SHA256SUMS` covers all five assets. All downloaded release files pass `shasum -a 256 -c SHA256SUMS`.
- Published [v0.1.0](https://github.com/RustableApp/Rustable/releases/tag/v0.1.0) without moving the tag or replacing a published asset.
- Public latest-release links return HTTP 200 for all three OS downloads, the macOS update feed, and checksum manifest.
- Website deployed to `rustable.app` through Cloudflare Workers, version `5d179275-afee-492d-9c22-856505e0c18a`. Live HTML and browser accessibility state confirm the macOS DMG, Windows ZIP, Linux tar.gz buttons and first-release limitations.

## Post-tag development verification

- Isolated macOS QA build: saved query single-click paints the blue selection without opening; double-click opens the selected SQL file.
- Right-click query rename changes the file and visible name. Regression coverage verifies open-tab retargeting, preservation of unsaved SQL and rejection of existing destinations.
- New connection and saved-query groups enter inline naming immediately. Native QA verified creation and right-click inline rename in both lists.
- These changes remain on the development branch and are not in v0.1.0.

## Limits and followups

- Windows and Linux native GUI smoke tests remain outstanding; passing compilation/tests and archive inspection do not establish native UI parity.
- Windows packages are unsigned; built-in SSH tunneling is unavailable on Windows.
- macOS Apple Silicon only; Windows/Linux updates are manual.
- Full refresh-rate/input-to-paint benchmarks, cross-platform accessibility and complete administration/driver parity remain outstanding.
- Saved-query single-click selection/double-click opening and history-preview ellipsis fixes were made after the release tag and are not in v0.1.0. Their local regression tests pass. Do not imply these changes are in the tagged binaries.
