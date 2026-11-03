# v0.4.9 release verification

Source tag: `8df1499901eedb360a88e09d490aa0cd2865447b`.
Signed release workflow: `41687798622`; Build and test: `41687796062`.
Both succeeded. Published November 3, 2026 at 17:59:15 UTC.

Local acceptance: 531 enabled native tests passed; 121 optional tests ignored.
Strict all-target Clippy, five packaging tests, and the separate locked browser
package check passed (existing browser warnings remain).
Seven live PostgreSQL integration tests passed, covering transactions, Safe Mode,
setup failure, cancellation, browsing and connection reuse. The new custom-type
stream regression failed with a five-second stall before the fix and passed in
about 0.15 seconds afterward. The monitor listing/selected cancellation test and
unbounded-default query/cancellation test also passed against disposable PostgreSQL.
This reproduces a real deadlock but does not establish the cause of the user's
specific stores query, whose column types were not confirmed.

Packaged and live browser startup and SELECT 49 AS release_check passed in
headless Chrome. Screenshots confirmed the returned value. Three website
embedding tests passed. Live BUILD.txt matches the clean release tag; WASM SHA-256:
`a8b10ed43f2ab9822b59e81cca296d58a45c3b115f5ba49a3135b1600623d7f6`.
COOP/COEP and CSP headers verified.

Website content commit: `427713e`. Cloudflare deployment version:
`f80c3444-1749-4abe-9250-2c704f1db4ff`.

Native visual acceptance was unavailable because the Mac was locked. Native
Windows/Linux GUI and installed old-to-new Sparkle installation remain untested.

All six payload checksums matched SHA256SUMS. Windows ZIP integrity and three
PE executables, Linux archive and three ELF executables verified. macOS deep/strict
codesign, Gatekeeper, app and DMG stapled tickets, DMG integrity and Sparkle Ed25519
signature passed. Updater signature verified against the installed app's public key.
All seven latest-release download URLs returned HTTP 200; the public updater feed
matches the verified release asset.
