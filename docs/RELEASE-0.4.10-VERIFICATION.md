# v0.4.10 release verification

Source tag: `4eec90382e1f6b738318a53f1d9bc76cb7100e1c`.
Signed release workflow: `41778900988`; Build and test: `41778901093`.
Both succeeded across macOS, Windows, Linux, and browser jobs.
Published November 3, 2026 at 23:33:13 America/Chicago (November 4, 05:33:13 UTC).

Local validation: 533 enabled native tests passed, 121 optional tests ignored;
strict all-target Clippy and the separate locked browser package check passed.
Coverage includes runtime-platform shortcut labels, keeping Ctrl+C available for
copying, and connection-test backgrounds preserving input geometry.

The user reported the earlier local Windows ARM64 build worked in Parallels after
the hidden-parent redraw workaround. The final release ZIP remains x64; its GUI
has not been manually exercised here. Native Linux GUI, final macOS GUI, and an
installed old-to-new Sparkle update remain untested.

All six payload checksums matched SHA256SUMS. Windows ZIP integrity and three PE
executables, Linux archive and three ELF executables verified. macOS deep/strict
codesign, Gatekeeper, app and DMG stapled tickets, DMG integrity, and Sparkle
Ed25519 signature passed. Updater signature verified against the installed app's
public key. All seven latest-release download URLs returned HTTP 200 and the
public updater feed exactly matches the verified draft asset.

Packaged browser startup and SELECT 410 AS release_check passed under both
macOS and Windows user agents; screenshots confirmed platform-specific toolbar
labels and the returned value. Three website embedding tests passed. Live
startup and query passed in headless Chrome. Live BUILD.txt matches the clean
release tag, isolation/CSP headers passed, and the public WASM exactly matches
the artifact with SHA-256:
`c59a7dd17a6afb5d1d1a0fbdea05121ade93ce5410e17ca9308a11fc64d43e6c`.

Website content commit: `ed74460`. Cloudflare deployment version:
`3645ac6b-d839-4771-b0ce-b372898260f8`.
