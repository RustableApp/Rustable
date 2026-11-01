# Release workflow

GitHub Actions has three separate workflows:

- Build and test: macOS, Windows, Linux, on main and pull requests.
- Package candidates: manually requested complete app/LSP/license bundles, retained as CI artifacts. macOS candidates are explicitly unsigned.
- Signed release: an existing vX.Y.Z tag matching Cargo.toml. Builds every platform, signs and notarizes macOS, generates its signed Sparkle feed, and creates a GitHub draft only after all platform jobs succeed.

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
