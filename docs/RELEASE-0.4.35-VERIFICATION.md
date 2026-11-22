# v0.4.35 release verification

Published at `2026-11-22T19:58:31Z`, GitHub release `454710318`:
[v0.4.35](https://github.com/RustableApp/Rustable/releases/tag/v0.4.35).

Build and test run `45061069940` and signed-release run `45061081867`
passed on every platform. The complete seven-asset draft came from that one
release run; no runner recovery or substitute artifacts were needed.

All six payload checksums and seven uploaded asset digests/sizes match. Final
macOS packages passed deep strict code signing, Gatekeeper, app/DMG stapled-ticket
validation, DMG integrity, stable updater identity, Sparkle Ed25519 signature,
and embedded release-note checks. The packaged executable reported v0.4.35 and
listed 11 MCP tools without opening a database. Windows x64 PE and Linux x64 ELF
binaries, both bundled language servers, and licenses were inspected. The draft
Mac/Linux/browser payloads exactly match the already checked CI artifacts.
An initial local extraction tried to overwrite the previously verified app and
was rejected by macOS; extraction into a fresh directory and all checks passed.
All seven versioned and latest download URLs returned HTTP 200; the public
latest updater feed exactly matches the verified signed feed.

Release tag `v0.4.35` points to `b43135db40538c7e28280ff40e992436106a0f7a`
with matching root/browser lockfiles. [Release notes](RELEASE-0.4.35.md) cover
Solarized Light/Dark, stronger selected rows and active bottom tabs, and readable
primary connection/import buttons. The changes were promoted to main and tagged.

Local library tests: 941 passed, 254 opt-in tests ignored. Strict all-target QA
Clippy, the separate locked browser-package check (87 existing warnings), five
packaging-tool tests, eight benchmark-parser tests and 14 website tests passed.
No database operation or driver sequencing changed; DB latency fixtures were not
run for these theme changes. The final UI comparison passed all 17 p95 gates,
with a largest increase of 4.5%. Earlier coverage passed 27 metrics and a repeated
query-editor comparison. See [UI-PERFORMANCE.md](UI-PERFORMANCE.md) for raw result
locations, the initial inconclusive query run, harness correction, sample counts,
hardware, timing tables and scope. No fixture or threshold was weakened.
Actual native screenshots verified selected rows, active Data tabs, appearance
menus and primary button labels in both Solarized modes. Contrast tests cover
selected backgrounds against adjacent surfaces and selected foreground text.

Website commit `f8c8bdc` includes the exact tagged browser artifact and updated
version/capability text. Cloudflare version
`19d43b27-c737-47a1-a893-27f8c07ee907` serves the matching live demo. Local and
live startup plus `SELECT 435 AS release_check;` passed in the in-app browser.
Six version pages, BUILD.txt and public WASM bytes match site source. BUILD.txt
reports the release commit and no modified tracked files. Preview-only runtime
compatibility overrides did not alter production configuration; existing
isolation/CSP/indexing headers remain intact. Public WASM SHA-256:
`f088440f33ad2bc42eb8b8cc8f677acb00e4c6bec33fdb43b19c928bed2796ce`.

Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle old-to-new
upgrade, 60/120 Hz input-to-present, mixed-DPI and monitor hot-plug behavior remain
unverified. Performance comparisons use existing standard-theme workloads;
Solarized visual checks are not native input-to-present measurements. Artifacts
and verification logs are retained under `target/release-v0.4.35/`.
