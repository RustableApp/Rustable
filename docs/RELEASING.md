# Releasing Rustable

Keep day-to-day work on development branches. Push to main only when preparing a production release. Package candidates are manual; signed releases run from version tags.

1. Run tests and strict Clippy; review the final changes.
2. Update Cargo version and release notes. Push the consolidated release commit to main.
3. Tag that commit `vX.Y.Z` and push the tag. The Signed release workflow tests and packages macOS, Windows and Linux and creates a draft only when all jobs pass.
4. Verify the assets, notarization, signatures, checksums and download links before publishing the draft. Publish the website download buttons only when assets exist.

The macOS updater reads the signed appcast from GitHub Releases. Windows and Linux updates are currently manual. Never move a published version tag; fix published releases with a new version.

For a packaging-only repair, the manual Package candidates workflow accepts `source_ref` (the existing release tag) and `platform`. It builds application and language-server sources from that ref, using the packaging script from the selected workflow revision. This avoids moving the tag or silently including unreleased app changes. Record both revisions when promoting a repaired package.
