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

## Outstanding before publication

- Windows repaired package job completion and archive inspection.
- Complete checksum manifest, upload, and download verification including Windows.
- Publish release and verify public direct download links before deploying website buttons.

## Limits and followups

- Windows and Linux native GUI smoke tests remain outstanding; passing compilation/tests and archive inspection do not establish native UI parity.
- Windows packages are unsigned; built-in SSH tunneling is unavailable on Windows.
- macOS Apple Silicon only; Windows/Linux updates are manual.
- Full refresh-rate/input-to-paint benchmarks, cross-platform accessibility and complete administration/driver parity remain outstanding.
- Saved-query single-click selection/double-click opening and history-preview ellipsis fixes were made after the release tag and are not in v0.1.0. Their local regression tests pass. Do not imply these changes are in the tagged binaries.
