# v0.4.7 release verification

Source tag: `d37b741ce5d03b706445d53cf569a5740ad772de`.
Signed release workflow: `41565434111`; Build and test: `41565434307`.
Both completed successfully. Published 2026-11-03 at 02:28:27 UTC.

Local acceptance: 522 enabled native tests passed; 118 optional tests ignored.
Five packaging tests, strict all-target Clippy, and the separate browser-package
locked WASM check passed (existing browser warnings remain).

The native macOS context-recovery regression painted all four frames after
clearing the current OpenGL context. Without the fix, this regression reproduces
the reported texture-creation panic and abort. Extended background/sleep-wake
acceptance is still needed to establish that every original trigger is covered.

Browser artifact acceptance: startup, SQL execution, rectangular cell dragging,
full-row gutter dragging, and index-table padding/width passed visual inspection.
Three website embedding tests passed. Production startup and
`SELECT 47 AS release_check` passed.

All six payload checksums matched SHA256SUMS. Windows archive CRC and three PE
executables, Linux archive and three ELF executables, and bundled licenses passed.
macOS strict/deep codesign verification, Gatekeeper, app and DMG stapled tickets,
DMG integrity and Sparkle Ed25519 signature verification passed. The updater
signature was checked against the public key in the installed application.
Native Windows/Linux GUI and installed old-to-new Sparkle updates remain untested.

All seven public latest-release downloads returned HTTP 200; the public updater
feed exactly matches the verified signed feed.

Website content commit: `1245044`. Cloudflare version:
`b70c9d99-4329-4785-adab-19bfe24da9d9`.
Live BUILD.txt reports the exact release source and a clean tracked tree.
Live WASM SHA-256 matches the released artifact:
`0d68b33c56a7e75617c5ff6f00824c413dda6288ed29299946a17f9de6c1da78`.
COOP/COEP and WASM CSP response headers verified.
