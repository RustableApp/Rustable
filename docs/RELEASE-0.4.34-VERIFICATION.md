# v0.4.34 release verification

Published at `2026-11-19T17:00:25Z`, GitHub release `451648947`:
[v0.4.34](https://github.com/RustableApp/Rustable/releases/tag/v0.4.34).

PR #14 merged as `61d44282d4258e44afef83263372bb1f67d25a17` after macOS,
Windows and Linux CI passed (`44540731036`). Release tag `v0.4.34` points to
`aae37f387994ec47562970781987a2f2018c665f` with matching root/browser lockfiles.
[Release notes](RELEASE-0.4.34.md) cover progressive query/table display, the
streaming inspector flashing fix, and external schema refresh.

Signed release workflow `44567527570` passed every platform and produced the
complete seven-asset draft. An initially queued macOS 15 runner prompted a
same-tag macOS 26 recovery run (`44568492127`), which also passed; the original
runner subsequently completed. Published assets are exclusively from the
primary all-platform draft, and were independently reverified after download.

Local debug and release library tests: 938 passed; the final release run ignored
254 opt-in tests. Strict all-target QA Clippy, the separate locked browser-package
check (87 warnings), five packaging-tool tests and 14 website tests passed.
Earlier disposable PostgreSQL/MySQL/MariaDB tests and latency checks for this
change are documented in [COMPLETE-QUERY-RESULTS.md](COMPLETE-QUERY-RESULTS.md).
The fresh release UI comparison passed all 24 p95 gates (−5.9% to +6.7%);
new streaming-inspector workloads passed at both tested widths. See
[UI-PERFORMANCE.md](UI-PERFORMANCE.md) for samples, hardware and limitations.

All six payload checksums and seven uploaded asset digests/sizes match. Final
macOS packages passed deep strict code signing, Gatekeeper, app/DMG stapled-ticket
validation, DMG integrity, stable updater identity, Sparkle Ed25519 signature,
and embedded release-note checks. The signed packaged executable reported
v0.4.34 and listed 11 MCP tools without opening a database. Windows x64 PE and
Linux x64 ELF binaries, bundled language servers, and licenses were inspected.
All seven versioned and latest download URLs returned HTTP 200; the public
latest updater feed is byte-identical to the verified signed feed.

Website commit `2cfb721` includes the exact release browser archive and updated
version/capability text. Cloudflare deployment/version
`75612758-9174-4c9e-bd97-f097fa4b23d0` serves the matching live demo. Local and
live startup plus `SELECT 434 AS release_check;` passed in the in-app browser.
The local Wrangler preview used a preview-only compatibility-date override;
production configuration and existing isolation/CSP/indexing headers remained
unchanged. Six version pages, BUILD.txt, and public WASM bytes match site source.
BUILD.txt reports the release commit and no modified tracked files.
Public WASM SHA-256:
`ade7a3fa4e7faf2df8afb636cac8ddcdd4dad81c8f2b66854a1bb608e16e83b1`.

Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle old-to-new
upgrade, 60/120 Hz input-to-present, mixed-DPI and monitor hot-plug behavior remain
unverified. The browser demo retains its completed, bounded result transport;
progressive desktop display is not advertised as reducing server execution time.
Artifacts and verification logs are retained under `target/release-v0.4.34/`.
