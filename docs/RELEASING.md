# Releasing Rustable

Keep day-to-day work on development branches. Push to main only when preparing a production release. Package candidates are manual; signed releases run from version tags.

1. Run tests and strict Clippy; review the final changes.
2. Update Cargo version and release notes. Push the consolidated release commit to main.
3. Tag that commit `vX.Y.Z` and push the tag. The Signed release workflow tests and packages macOS, Windows and Linux and creates a draft only when all jobs pass.
4. Verify the assets, notarization, signatures, checksums and download links before publishing the draft. Publish the website download buttons only when assets exist.

The macOS updater reads the signed appcast from GitHub Releases. Windows and Linux updates are currently manual. Never move a published version tag; fix published releases with a new version.
