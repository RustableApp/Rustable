# v0.4.6 release verification

Source tag: `7f6fd646bebe3cf1ee1c1e348a80180e2126f87a`.
Signed release workflow: `41530416875`.

Local acceptance: 518 enabled native tests passed; 118 optional fixture/manual
checks ignored. Five packaging tests and strict all-target Clippy passed. The
separate browser-package locked WASM check passed with existing browser warnings.
Four live PostgreSQL change suites passed on the disposable local fixture,
covering typed values, atomicity, row conflicts, schema/type changes, triggers,
cancellation rollback, and structure edits. Actual SSH-network save latency has
not been measured; this release removes redundant catalog round trips.

Built browser artifact acceptance: startup, single-cell edit/review/commit,
13 overflowing SQL tabs without a tab-strip scrollbar, and
`SELECT 46 AS release_check` passed. Three website embedding tests passed.

Windows ZIP CRC, PE executables, Linux ELF executables, and bundled license
notices passed. macOS strict/deep signature validation, Gatekeeper acceptance,
app and DMG stapled tickets, signed DMG integrity and Sparkle Ed25519 update
signature passed independently.

Native Windows/Linux GUI acceptance and installed old-to-new Sparkle update
acceptance remain unverified. The previously reported background OpenGL crash
has not been reproduced or confirmed fixed; crash-report consent is unchanged.

The browser artifact's BUILD.txt identifies the exact release commit and a clean
tracked tree. WASM SHA-256:
`4783cbb570a7775da9c42ae4da19ddc4076956ef68deefe69fcf71c767580b24`.

## Publication and live demo

Published 2026-11-02 at 22:10:26 UTC. All seven asset checksums match, all public
latest-download links return success, and the public macOS update feed exactly
matches the verified signed feed. Cross-platform Build and test run `41530414879`
and Signed release run `41530416875` both completed successfully.

Website content commit `523401b`; Cloudflare version
`f9a61bdc-7f0a-40cd-a768-51c80a7e3972`. Production startup and
`SELECT 46 AS release_check` passed. Live BUILD.txt, WASM hash, COOP/COEP and
WASM CSP match the verified bundle/configuration.

The cached browser release job took 66 seconds. Linux packaging took 4m46s,
with its pinned language-server step under one second. Windows and macOS had
cold language-server builds this run; main-branch caches are now populated.
