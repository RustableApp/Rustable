# Release workflow

GitHub Actions has three separate workflows:

- Build and test: macOS, Windows, Linux, on pull requests or manual dispatch.
- Package candidates: manually requested complete app/LSP/license bundles, retained as CI artifacts. macOS candidates are explicitly unsigned.
- Signed release: an existing vX.Y.Z tag matching Cargo.toml. Builds every platform, signs and notarizes macOS, generates its signed Sparkle feed, and creates a GitHub draft only after all platform jobs succeed.

Current stable release: [v0.4.0](RELEASE-0.4.0-VERIFICATION.md), including its matching deployed WASM demo.

The first release, v0.1.0, is published. See [its verification record](RELEASE-0.1.0-VERIFICATION.md) for build provenance and validation limits. Windows SSH transport uses Unix control sockets and needs a Windows implementation before parity can be claimed. Intel macOS, native Windows/Linux acceptance, and end-to-end Sparkle old-to-new installation remain followup verification gates for full platform parity. Windows ZIP and Linux tar.gz are portable packages, not OS installers or automatic-update implementations.

## Signing

GitHub repository secrets:

- MACOS_CERTIFICATE_P12_BASE64 and MACOS_CERTIFICATE_PASSWORD
- MACOS_SIGNING_IDENTITY
- APPLE_NOTARY_KEY_P8, APPLE_NOTARY_KEY_ID, APPLE_NOTARY_ISSUER_ID
- SPARKLE_PRIVATE_KEY and SPARKLE_PUBLIC_KEY

Only the signing step receives these credentials. Its temporary keychain and private files are removed afterward. Signing fails closed; no unsigned macOS fallback is published. Keep protected backups of signing material outside Git and preserve the Sparkle key across releases.

The macOS archive is notarized, its app is stapled and assessed by Gatekeeper, then the archive is recreated before Sparkle signs it. The appcast points to immutable version-specific release asset URLs. Installed macOS builds fetch their architecture's appcast through GitHub's latest-release download URL. Drafts and prereleases do not become the latest stable feed.

Review and test the draft packages before publishing. Never replace a published release asset in place; bump Cargo version and tag a new release. Download buttons must point only to actual published assets, not CI artifacts or missing latest-release links.


## Required for every release: matching live WASM demo

A desktop release is not complete until https://rustable.app/demo/ runs the
shared app from the same release tag. The signed-release workflow builds and
archives `Rustable-wasm-demo.tar.gz` from that tag and includes it in the draft;
a failed demo build blocks draft creation. `demo/BUILD.txt` records version,
commit, and whether tracked source was modified.

For each release:

1. Batch work on the development branch, run local regression/Clippy and relevant
   driver fixtures, update the Cargo version/lockfiles and release notes.
   Check both the root and separate browser package lockfiles before tagging:
   `mise exec -- cargo check --locked --target wasm32-unknown-unknown --manifest-path tools/browser/Cargo.toml`.
   A root-only WASM check does not validate the browser package lockfile.
2. Promote the checked commit to main and push one matching version tag. Ordinary
   development does not push main or trigger repeated package builds.
3. Verify all draft packages, signed/notarized macOS DMG, checksums and updater
   feed. Download and unpack the workflow's matching WASM demo into the website's
   `public/demo`, preserving website isolation/CSP headers.
4. Update website version/capabilities, run its embedding tests, commit the bundle,
   and deploy through the existing private ops checkout. No credentials belong
   in either public app source or website source.
5. Verify live demo startup, a SQLite query, `demo/BUILD.txt` version/commit and
   public WASM SHA-256 against the release archive. Publish the verified desktop
   draft and verify all download links. Record both release and deployment IDs.

Do not mark a release complete with a stale demo. Never overwrite published
versioned assets; fix failures before publishing, or bump the version afterward.


Runner recovery: macOS 14 was retired in November 2026; workflows use macOS 15.
If an infrastructure failure affects only macOS, manually dispatch Signed release
with the original tag and `platform: macos-15`. This builds/signs only macOS and
leaves draft creation to the release operator, who combines only artifacts from
the exact same source tag. Do not rebuild successful Windows/Linux/WASM jobs.
