# v0.4.8 release verification

Source tag: `8c6373806721ea438d5d8730582f560c5a7e4c85`.
Signed release workflow: `41577703383`; Build and test: `41577701726`.
Both succeeded. Published November 2, 2026 at 10:31:50 PM America/Chicago
(2026-11-03 04:31:50 UTC).

Local acceptance: 527 enabled native tests passed; 118 optional tests ignored.
Strict all-target Clippy, five packaging tests, and the separate locked browser
package check passed (existing browser warnings remain).

Packaged browser acceptance: startup, current-query-only formatting, preserved
starting line, caret at query end, text-bounded shading, SQL execution, stable
editor position, and padded single execution-error message visually verified.
Three website embedding tests passed. Live startup and `SELECT 48 AS release_check`
passed. Live BUILD.txt matches the clean release tag; WASM SHA-256 matches:
`35b310fb9ed329e52e2c44013e36f4861fe64af0556dea0169ac532646b2bcce`.
COOP/COEP and CSP headers verified.

All six payload checksums matched SHA256SUMS. Windows ZIP integrity and three
PE executables, Linux archive and three ELF executables verified. macOS deep/strict
codesign, Gatekeeper, app and DMG stapled tickets, DMG integrity and Sparkle Ed25519
signature passed. Updater signature checked against the installed app's public key.
All seven latest-release downloads returned HTTP 200; public updater feed matches.
Native Windows/Linux GUI and installed old-to-new Sparkle installation remain untested.

Website content commit: `bebf403`. Cloudflare deployment version:
`3c70ec7b-2be9-4495-91c4-2dbcd859f594`.
