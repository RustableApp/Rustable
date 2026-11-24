# v0.4.38 release verification

Published at `2026-11-24T04:20:22Z`, GitHub release `456300882`:
[v0.4.38](https://github.com/RustableApp/Rustable/releases/tag/v0.4.38).
The immutable release tag points to `8ccfdf54eb16c874ba5f8616b1eb9349495ceb55`,
with matching Cargo version and root/browser lockfiles.
[Release notes](RELEASE-0.4.38.md) cover grouped deletes, execution protection,
Touch ID, Settings and the unified confirmation/preview workflow.

Main CI `45323497075` and signed-release run `45323522602` passed all platforms.
The Linux release runner spent an unusually long time installing system
libraries. A Linux-only fallback run `45324865114` was started, then cancelled
when the original recovered; every published asset comes from the original
signed-release run and exact tag, with no substituted packages.

All six payload checksums and seven uploaded asset digests/sizes matched.
macOS passed deep strict code signing, Gatekeeper, app/DMG stapled-ticket checks,
DMG integrity, stable updater identity, Sparkle Ed25519 signature and embedded
release-note verification. The packaged executable reported v0.4.38 and listed
11 MCP tools without opening a database. Windows x64 PE and Linux x64 ELF
app/language-server binaries and bundled licenses were inspected. All seven
versioned and latest download URLs returned HTTP 200, and the public latest
updater feed matches the verified signed feed.

Local checks: 941 library tests passed, 259 opt-in tests ignored; strict
all-target QA Clippy, the separate locked browser-package WASM check (85
warnings), five packaging tests and 14 website tests passed. Disposable
PostgreSQL/MySQL/MariaDB edit fixtures and PostgreSQL delayed-network comparisons
were run during implementation. See [QUERY-PROTECTION.md](QUERY-PROTECTION.md)
for protocol counts, trigger/concurrency guarantees and full timing scope.
Eight compatible deletes reduced measured apply latency from 1577.19 to
1064.08 ms at 30 ms RTT. UI comparisons include inconclusive noisy workloads;
the confirmation shortcut increased its small-dialog CPU p95 by 0.002291 ms
(+13.6%), explicitly accepted for the visible shared shortcut artwork and
confirmation handling. Native wide/dark and narrow/light screenshots were
inspected. No blanket performance pass is claimed.

Website content commits `8aa6ace` and `3eb5e6a` include the exact tagged demo
and updated capability descriptions. Cloudflare version
`7fb0c909-4770-465e-920e-9958a3f2c7ae` serves the matching demo. All 368 demo
files were copied from the release artifact. Six public version pages,
BUILD.txt and WASM bytes matched source; isolation/CSP/indexing headers remain
intact. Public WASM SHA-256:
`b7d749688d590deec83fea41fbe84097c309607a2fdef54eb5ebfef2cb718303`.
Interactive local and live startup and `SELECT 438 AS release_check` passed,
displaying one row containing 438 with no prompt in Silent mode. Local Wrangler
used a compatibility-date override because its installed runtime predates the
production configuration; the deployed configuration was unchanged.

Actual Touch ID hardware authorization, native Windows/Linux GUI acceptance,
Intel Mac, installed Sparkle old-to-new upgrades and native input-to-present,
60/120 Hz, mixed-DPI and hot-plug checks remain unverified. Artifacts and raw logs
are retained under `target/release-v0.4.38/`, with UI benchmark artifacts linked
from the query-protection notes.
