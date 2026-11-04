# v0.4.11 release verification

Source tag: `8b1daad44556a156f672d0fdc1cadc7fd53e61f2`.
Signed release workflow: `41886470946`, attempt 3, succeeded across all platforms.
Earlier attempts and macOS 26 fallback `41889186632` failed before runner
assignment due to GitHub ARM capacity constraints. The third attempt used the
original macOS 15 workflow and passed tests, build, signing and notarization.
Separate CI `41886467961` passed Windows/Linux/browser; its macOS job was cancelled
by runner allocation failure. Equivalent release-workflow macOS tests passed.
Published November 4, 2026 at 13:05:47 America/Chicago (19:05:47 UTC).

Local validation: 536 enabled tests passed, 123 optional tests ignored;
strict all-target Clippy and the separate locked browser package check passed.
Live PostgreSQL checks passed: seven driver integration tests, three transport,
statement-cleanup and reconnection tests, plus the uncached custom-type large-row
regression. At 30 ms simulated network RTT, median query latency improved from
124.4 ms to 82.1 ms. Failed writes are not automatically replayed.

Windows ZIP integrity and all three PE executables verified. Linux archive and
ELF executable checks passed. Native Windows/Linux GUI acceptance, macOS native
notification delivery and external-monitor hotplug remain unverified here.

Packaged browser startup and SELECT 411 AS release_check passed with macOS and
Windows user agents. Three website embedding tests passed. The deployed demo
started and returned the expected query value. Live BUILD.txt matches the clean
release tag; isolation/CSP headers passed. The public WASM matches the artifact:
`c59802d46e907a6be56199e1c0796c9b0915027a1125fe9cebd2bcf1fc2b036b`.
Demo website commit: `912afaf`. Cloudflare deployment:
`420eb815-e966-4714-9ce4-757b17c1576d`.

All six payload checksums matched SHA256SUMS. macOS deep/strict codesign,
Gatekeeper, app and DMG stapled tickets, DMG integrity, and Sparkle Ed25519
signature verified. The updater signature matches the installed app's public
key. All seven latest-release download URLs returned HTTP 200 and the public
updater feed exactly matches the verified asset. The draft WASM archive is
byte-identical to the tested and deployed workflow artifact.

Final website content commit: `2cb75e4`. Cloudflare deployment version:
`d1b4f87a-5ff2-473a-907b-ca7170a7584c`. Public download label is v0.4.11.
Installed old-to-new Sparkle update, native notification delivery and physical
monitor hotplug remain unverified; package/signature checks do not cover these.
