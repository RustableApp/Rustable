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

## Release gates in progress

- Tagged CI package builds, archive/signature/checksum verification.
- Matching WASM demo deployment and live query/provenance checks.
- Stable publication and public download/update-feed verification.

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
