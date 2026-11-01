# v0.3.1 release verification

## Local checks

On 2026-11-01, 471 native tests passed (111 fixture/manual tests ignored).
The six-test Redis suite passed against disposable Redis 7.4 plaintext and TLS
containers. Strict native all-target Clippy passed. The latest inspector change
also passed four focused layout tests and native visual inspection.
Three website embedding/fallback tests passed. The optimized pre-packaging v0.3.0
binary passed native Redis connection, key-grid selection, string inspection
and command execution (PING → PONG) smoke checks against a disposable fixture.

The corrected v0.3.1 optimized native build and full browser bundle build passed
locally. The browser package locked check passed (52 existing browser warnings).

## Completed release gates

Published stable v0.3.1 on 2026-11-01 at 19:32:39 UTC:
https://github.com/RustableApp/Rustable/releases/tag/v0.3.1
GitHub release ID: 430806410. All seven public latest-download URLs return HTTP
200, and the public macOS update feed points to v0.3.1.

All six content assets were downloaded back from the release draft and matched
SHA256SUMS. The macOS DMG passed integrity verification; the app passed strict
code-signature verification and Gatekeeper accepted it as notarized. Both app
and DMG stapled tickets validated. The Sparkle archive signature verified against
the public key in the installed Rustable app; the feed archive length matched.

Website content commit 9574cab deployed as Cloudflare version
9e07f433-721b-4095-aafd-d0053afb9519. The live demo reports the exact release tag
and source commit with no tracked modifications. Its WASM SHA-256 matches CI:
940dd99f7438a5e17345848d8df42dcc14027ea300863cefd24115922cbe0fd9.
Live startup and SELECT 31 AS release_check returned the expected value 31.
COOP/COEP isolation and WASM CSP headers were verified.

## Acceptance scope

Standalone Redis 7.4 is the tested Redis server. Redis connections are desktop-only;
the shared browser demo uses virtual SQLite. Redis Cluster/Sentinel and built-in
SSH are not included. See [Redis capabilities](REDIS.md).

Native Windows/Linux GUI acceptance, Intel macOS, and an installed old-to-new
Sparkle update remain unverified, as in v0.2.0. Portable Windows/Linux archives
are not native installers or automatic-update implementations.

## Superseded packaging attempt

Tag v0.3.0 was not published. Run 41338582323 failed its WASM build because
the separate browser-package lockfile was stale. Remaining jobs were cancelled
to save runner time. v0.3.1 corrects that lockfile; the earlier tag is not moved.

## CI provenance and recovery

Source tag v0.3.1 is commit 0f1ecab02e239cf3a5e9ea142e62716672ca33a3.
Run 41339074801 passed Linux, Windows and WASM. macOS built and signed the
app and obtained its notarization ticket, then failed staple validation because
Apple CloudKit lost the network connection (NSURLError -1005, stapler error 68).
Isolated recovery run 41340846803 passed using the exact same tag; successful
Linux/Windows/WASM jobs were not repeated.

The Linux archive contains three executable ELF binaries and license notices.
The Windows ZIP passes CRC validation and contains the app and two PE language
servers plus license notices. Twelve browser adapter tests and three website
embedding tests passed. The CI WASM bundle started locally and returned 31 for
SELECT 31 AS release_check. Its provenance reports v0.3.1, the tagged commit,
and no modified tracked files.
