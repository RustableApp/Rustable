# v0.4.37 release verification

Published at `2026-11-24T02:27:36Z`, GitHub release `456232923`:
[v0.4.37](https://github.com/RustableApp/Rustable/releases/tag/v0.4.37).
Release tag points to `69231f9bc0ceb67af30ad36a416bdba5a1c65871`, with matching
root and browser lockfiles. [Release notes](RELEASE-0.4.37.md) describe the Redis
and Valkey TLS modes, local port forwards and independent Sentinel policies.
[Issue #17](https://github.com/RustableApp/Rustable/issues/17) is resolved by this release.

Main CI `45310072935` and signed-release run `45310695179` passed all platforms.
All six payload checksums and seven uploaded asset digests/sizes match. macOS
packages passed deep strict signing, Gatekeeper, app/DMG stapled-ticket validation,
DMG integrity, stable updater identity, Sparkle Ed25519 signature and embedded
release-note checks. The packaged executable reported v0.4.37 and listed 11 MCP
tools without opening a database. Windows x64 PE and Linux x64 ELF app/language
server binaries and bundled licenses were inspected. All seven versioned and
latest download URLs returned HTTP 200; the public latest updater feed matches
the verified signed feed.

Local validation: 946 library tests passed, 255 opt-in tests ignored; strict
all-target QA Clippy, separate locked browser-package check (87 existing warnings),
five packaging tests and 14 website tests passed. Disposable Redis/Valkey TLS,
mutual TLS, SNI, cluster forwarding/redirect/reconnect and Sentinel trust-policy
fixtures passed. Native dark/light connection-form screenshots were reviewed.
Release-mode UI comparisons showed no repeatable regression; standalone and
small-cluster cases remained noisy/inconclusive. See
[UI-PERFORMANCE.md](UI-PERFORMANCE.md) for measurements and limitations.

Website content commit `9b84912` contains the exact tagged browser artifact:
all 366 payload files match the release archive. Cloudflare version
`71aa6b39-a593-4747-8c65-03a3d1a4b7af` serves the matching demo. All six version
pages, BUILD.txt and public WASM bytes match source; existing isolation/CSP and
indexing headers remain intact. The first homepage read during propagation was
stale; a subsequent complete check passed. BUILD.txt reports the release commit
and no modified tracked files. Public WASM SHA-256:
`30ed3457f645081731e5b716826d6ce5fa9dbcb150aa61061c0086427ea51f99`.

Interactive local and live demo startup passed. The live SQLite query
`SELECT 437 AS release_check` displayed one row containing 437 before desktop
publication. Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle
old-to-new upgrade, input-to-present/60–120 Hz, mixed-DPI and monitor hot-plug
acceptance remain unverified. Artifacts and logs are retained under
`target/release-v0.4.37/`.
