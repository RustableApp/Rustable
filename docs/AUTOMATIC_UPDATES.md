# Automatic updates

Rustable uses Sparkle 2 for direct macOS distribution. Sparkle does not run on
Windows or Linux. Windows can use WinSparkle with its own appcast and installer;
Linux delivery should follow the selected package format/repository. Those two
backends are not implemented yet. Keep platform feeds separate on the same domain.

## Current integration

The optional `sparkle-updater` Cargo feature links the official Sparkle framework
through pinned Rust bindings (`sparkle-updater = 0.1.0`). Sparkle provides its native
permission, check/download/install UI and archive-signature verification. The app
adds “Check for Updates…” to the macOS application menu. Debug/QA builds never start
the public updater. Unconfigured release bundles do not start update checks.

Rustable postpones Sparkle's requested relaunch while any window has pending work,
staged edits or an explicit SQL transaction. It saves all SQL drafts/preferences
and waits for checked atomic disk persistence before releasing the continuation.
Save failures keep the app open with a retry action. Existing AppKit termination
guards remain active for other quit paths. End-to-end signed installation and
relaunch, including failure and unsaved-work cases, remain mandatory release gates.

## Build and configure

Use the official Sparkle 2.10.1 distribution:
https://github.com/sparkle-project/Sparkle/releases/download/2.10.1/Sparkle-2.10.1.tar.xz

SHA-256: `1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27`.
Extract into `target/sparkle`, preserving symlinks and executable permissions.

```sh
SPARKLE_FRAMEWORK_PATH="$PWD/target/sparkle" mise exec rust@stable -- cargo build --locked --release --features sparkle-updater
mise exec rust@stable -- cargo run --locked --manifest-path tools/package/Cargo.toml -- bundle
mise exec rust@stable -- cargo run --locked --manifest-path tools/package/Cargo.toml -- configure-updates target/Rustable.app target/sparkle/Sparkle.framework https://rustable.app/updates/macos/appcast.xml "$RUSTABLE_UPDATE_PUBLIC_KEY"
```

The configuration command requires an HTTPS feed, a base64 Ed25519 public key,
a Sparkle-linked binary and the production bundle identifier. It copies the
framework with `ditto` and writes Sparkle's plist keys. This invalidates any prior
bundle signature: configure before signing. Do not launch the linked bundle
between `bundle` and `configure-updates`; its embedded framework is not present yet.

Generate the release signing key once with Sparkle's `bin/generate_keys`. Retain
and back up the private key securely; only its public key goes in the app. No real
release keys have been generated as part of this integration.

## Publishing (not yet configured)

1. Host the macOS feed at `https://rustable.app/updates/macos/appcast.xml`, under
   the user-selected `rustable.app` domain. The URL is embedded in the production
   plist; hosting and a real signing public key are still required.
2. Increment `CFBundleVersion` monotonically and set the user-facing release version.
3. Configure the bundle, then Developer ID-sign nested Sparkle services/helpers,
   language servers and the app following Sparkle's signing guidance. Notarize and
   staple. Preserve Sparkle's framework hierarchy throughout packaging.
4. Create the final downloadable archive. Use Sparkle's `generate_appcast` to sign
   that exact archive and produce the feed. Keep release keys away from hosting.
5. Upload immutable versioned archives first, then publish the appcast. Serve both
   via HTTPS; the website download and updater can use the same archive.
6. Verify an older signed installation updates to the new signed installation,
   retains profiles/drafts, handles interrupted downloads, rejects invalid
   signatures, and cannot restart over unsaved edits or open SQL transactions.

Until these gates are exercised, automatic updates are integrated but not
production-ready. No feed or release has been published.

Sources: [Sparkle setup](https://sparkle-project.org/documentation/),
[programmatic integration](https://sparkle-project.org/documentation/programmatic-setup/),
[publishing](https://sparkle-project.org/documentation/publishing/),
[WinSparkle](https://winsparkle.org/).
