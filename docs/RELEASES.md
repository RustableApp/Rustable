# Release workflow

GitHub Actions has three separate workflows:

- Build and test: macOS, Windows, Linux, on pull requests, build-related pushes to main, or manual dispatch. Main also builds the browser demo and warms release dependencies.
- Package candidates: manually requested complete app/LSP/license bundles, retained as CI artifacts. macOS candidates are explicitly unsigned.
- Signed release: an existing vX.Y.Z tag matching Cargo.toml. Builds every platform, signs and notarizes macOS, generates its signed Sparkle feed, and creates a GitHub draft only after all platform jobs succeed.

Current stable release: [v0.4.25](RELEASE-0.4.25-VERIFICATION.md), including its matching deployed WASM demo.

The first release, v0.1.0, is published. See [its verification record](RELEASE-0.1.0-VERIFICATION.md) for build provenance and validation limits. Windows SSH transport uses Unix control sockets and needs a Windows implementation before parity can be claimed. Intel macOS, native Windows/Linux acceptance, and end-to-end Sparkle old-to-new installation remain followup verification gates for full platform parity. Windows ZIP and Linux tar.gz are portable packages, not OS installers or automatic-update implementations.

## Build caches

Native CI, candidate packaging, and releases share Rust dependency caches keyed
by platform and compiler. Main builds the updater-enabled macOS variant so its
release dependencies are warm too. Browser builds separately cache their Rust
dependencies and pinned wasm-bindgen CLI.

Pinned language-server executables use an exact cache key covering platform,
compiler, server sources/lockfile, and packaging tool sources. Never add a loose
restore prefix: the packaging helper trusts an existing PostgreSQL server binary.
This cache also preserves Cargo registry sources required for license generation.
App binaries are rebuilt from the requested source; signing and notarization
still run for each release.

Let the main build finish before tagging when practical. GitHub allows tag jobs
to restore default-branch caches, but not caches from a different tag. The first
run or a compiler/server dependency change can still be cold. Cache eviction is
safe and only costs build time. Documentation-only pushes do not warm caches;
use a manual Build and test dispatch on main if needed.

## Signing

GitHub repository secrets:

- MACOS_CERTIFICATE_P12_BASE64 and MACOS_CERTIFICATE_PASSWORD
- MACOS_SIGNING_IDENTITY
- APPLE_NOTARY_KEY_P8, APPLE_NOTARY_KEY_ID, APPLE_NOTARY_ISSUER_ID
- SPARKLE_PRIVATE_KEY and SPARKLE_PUBLIC_KEY

Only the signing step receives these credentials. Its temporary keychain and private files are removed afterward. Signing fails closed; no unsigned macOS fallback is published. Keep protected backups of signing material outside Git and preserve the Sparkle key across releases.

The update prompt displays Markdown from `docs/RELEASE-X.Y.Z.md`, embedded in
the appcast by `generate_appcast --embed-release-notes`. Missing or empty notes
fail packaging; the generated feed is checked before publication. Keep those
notes user-facing and concise.

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

The signing workflow retries `xcrun stapler` exit status 68 up to three times,
with 15 seconds between attempts, to tolerate transient Apple ticket-service
failures. Other signing/notarization failures stop immediately; final ticket
validation remains required. Recovery dispatches can use the current workflow
while checking out the original immutable release tag for all app artifacts.

For macOS ARM runner capacity failures, recovery also accepts `platform: macos-26`.
It checks out the same immutable release tag and runs the same tests, packaging,
signing, and notarization steps on GitHub's alternate supported macOS image.
Only combine this Mac artifact with successful artifacts from that exact tag.
