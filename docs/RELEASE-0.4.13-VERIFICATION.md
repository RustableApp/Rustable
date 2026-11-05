# v0.4.13 release verification

Published stable release: https://github.com/RustableApp/Rustable/releases/tag/v0.4.13.
Published 2026-11-04 at 21:49:50 UTC from tag
`a490ba38355dbc6c4758c51797f7284a73a3cbed`.
Signed-release workflow `41917985193` and CI `41917956682` succeeded on all platforms.

The first pre-tag CI run (`41917048306`, commit `f6f0604`) exposed a Windows
race in the new UI test: it discarded the initial connection's pending state
without waiting for the bounded worker queue. The test now starts without an
initial connection. The corrected test passed in CI and signed packaging.
No tag or draft was created from the failing commit.

## Validation

- Local suite: 538 enabled tests passed, 126 opt-in tests ignored. Strict
  all-target Clippy and the separate locked browser package check passed.
- Seven PostgreSQL integration tests and four browse tests passed, including
  first sidebar loads with cold enums/domains/arrays, exact numeric and bytea
  values, NULLs, empty results, schema changes, explicit transactions,
  cancellation, oversized results and subsequent connection reuse.
- With 30 ms simulated RTT, first sidebar table loads measured 47.5 ms median
  versus 85.9 ms for metadata-first loading; the genuine single-exchange reference
  was 41.1 ms. Refresh measured 44.1 ms and revisit 48.0 ms.
- Editor latency regression passed: 46.1 ms median versus 45.4 ms reference.
- The existing catalog latency regression initially reproduced its prior
  failure (172.9 / 256.1 ms). Removing the sidebar describe exchange improved
  it to 125.7 / 248.6 ms, passing the unchanged 0.65 ratio threshold.
- UI coverage verifies that lazy details load with the inspector hidden and
  failed requests offer a retry rather than looping each rendering frame.

All six payload hashes matched SHA256SUMS. Windows ZIP integrity and three x64
PE executables passed; Linux archive and x64 ELF checks passed. Dependency
license manifests are present. macOS deep/strict codesign, Gatekeeper, app/DMG
stapled tickets, DMG integrity, and Sparkle Ed25519 signature passed. The appcast
embeds the exact Markdown release notes. All seven latest-release download URLs
returned HTTP 200, and the public appcast matches the verified draft feed.

## Matching website demo

Website commit: `e864535`. Cloudflare version:
`7285a7d6-af11-4780-bba8-cf07c779eb60`.
Live BUILD.txt identifies version 0.4.13, the exact source tag, and clean tracked
files. The public WASM SHA-256 matches the release archive:
`76f04fd6b26d06726d3e1bdf559a4b6027433abaa4aab8fa0d096cc3565b6c30`.

Artifact and live demo startup plus `SELECT 413 AS release_check` passed with
no page errors. Three embedding tests and live isolation/CSP headers passed.
The deployed browser archive is byte-identical to the draft release asset.

Native Windows/Linux GUI acceptance and installed old-to-new Sparkle updating
were not exercised; package and feed checks do not establish those behaviors.
