# v0.4.36 release verification

Published at `2026-11-23T22:12:37Z`, GitHub release `455831574`:
[v0.4.36](https://github.com/RustableApp/Rustable/releases/tag/v0.4.36).
[PR #18](https://github.com/RustableApp/Rustable/pull/18) merged and closed
[issue #16](https://github.com/RustableApp/Rustable/issues/16). Release tag
`v0.4.36` points to `7f9c23f1a197611046c020bf6c131fc612266eca`, with matching
root and browser lockfiles. [Release notes](RELEASE-0.4.36.md) describe the
prominent cancellation action and preserved PostgreSQL query-monitor behavior.

PR CI `45233419850` passed all native platforms. Main CI `45235193685` passed
after retrying Windows: its first attempt had nine worker-timeout failures in
existing editing/transaction tests. The unchanged retry passed, as did the
independent signed-release Windows tests; no deadlines or tests were weakened.
The initial failure log is retained. Signed-release run `45237096107` passed
all platforms and produced all seven draft assets. Linux dependency setup was
slow but completed successfully; no substitute artifacts were used.

All six payload checksums and seven uploaded asset digests/sizes match. macOS
packages passed deep strict signing, Gatekeeper, app/DMG stapled-ticket validation,
DMG integrity, stable updater identity, Sparkle Ed25519 signature and embedded
release-note checks. The packaged executable reported v0.4.36 and listed 11 MCP
tools without opening a database. Windows x64 PE and Linux x64 ELF app/language
server binaries and bundled licenses were inspected. Mac/Windows/browser payloads
match the previously checked CI artifacts exactly. All seven versioned and
latest download URLs returned HTTP 200, and the public latest updater feed
matches the verified signed feed.

Local validation: 942 library tests passed, 254 opt-in tests ignored; strict
all-target QA Clippy, the separate locked browser-package check (87 existing
warnings), five packaging tests, eight benchmark-parser tests and 14 website
tests passed. The existing five-pair release-mode comparison against `5445cac`
passed all 24 p95 gates. See [UI-PERFORMANCE.md](UI-PERFORMANCE.md) for p50/p95/
p99/max, environment, raw result locations and scope. Native dark, narrow light,
Solarized Dark/Light and cancellation-requested screenshots were visually
inspected. Driver execution and protocol sequencing are unchanged; database
latency fixtures were not rerun for this UI change.

Website content commit `34016e8` contains the exact tagged browser artifact and
updated version/capability text. Cloudflare version
`e13ec07f-253f-4f12-bd7d-5c7dba0e22ab` serves the matching demo. All six version
pages, BUILD.txt and public WASM bytes match source; existing isolation/CSP and
indexing headers remain intact. BUILD.txt reports the release commit and no
modified tracked files. Public WASM SHA-256:
`5f2e7a0870f4911301fc9167d7eebc5e504d8fddfdb00472e5d24690a644c7f9`.

Interactive local/live demo startup and SQLite query acceptance were explicitly
waived by the user after the verification Mac was unavailable while locked.
These checks were not performed; HTTP/provenance/checksum and automated website
checks passed. Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle
old-to-new upgrade, input-to-present/60–120 Hz, mixed-DPI and monitor hot-plug
acceptance remain unverified. Artifacts and logs are retained under
`target/release-v0.4.36/`.
