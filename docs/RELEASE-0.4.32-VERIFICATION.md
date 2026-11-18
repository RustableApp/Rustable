# v0.4.32 release verification

Published at `2026-11-18T05:44:34Z`, GitHub release `449570756`:
[v0.4.32](https://github.com/RustableApp/Rustable/releases/tag/v0.4.32).
Source: immutable tag `v0.4.32`, commit
`426c69fc82561f7b1eb8bf33aefa3bf5dc19da3d`.
[Release notes](RELEASE-0.4.32.md) cover live-workspace MCP, query/result sharing,
combined SQL and configurable CSV export, SQL editor improvements and aurora
loading activity.

[Main CI](https://github.com/RustableApp/Rustable/actions/runs/44243179057) and
[signed release](https://github.com/RustableApp/Rustable/actions/runs/44244578738)
succeeded across macOS, Windows, Linux and the browser build. No recovery build
or replacement tag was needed. All seven versioned and latest asset URLs returned
HTTP 200; the public latest updater feed exactly matches the verified signed feed.

## Packages and MCP

All six payload checksums passed, and all seven uploaded asset sizes/digests match
downloaded files. macOS deep strict signature validation, Gatekeeper assessment,
stapled app/DMG notarization, DMG signature and archive integrity passed. Bundle
identity, team `624KJV575U`, updater URL and Ed25519 public key match the prior
release. The Sparkle signature verifies against the exact ZIP, with its immutable
v0.4.32 URL, correct length and embedded release notes. Mac ARM64, Windows x64 PE
and Linux x64 ELF architectures, bundled language servers/licenses, and Mac icon
passed package inspection.

The signed Mac package's actual stdio MCP adapter successfully initialized with
version 0.4.32 and returned the expected shared-workspace/edit/query tools. This
probe did not open databases. Native ordinary-launch discovery, explicit grants,
shared-tab editing, stopping, setup dialogs and visual behavior were tested during
implementation; see [MATRIX.md](MATRIX.md).

## Correctness and performance

Local locked native tests: 921 passed, 248 ignored; strict all-target QA Clippy
passed. The separate locked browser package check passed with 87 warnings. All
five packaging-tool tests and 14 website tests passed. Existing native Matrix and
connector previews and aurora dark/light 760/1280 captures were inspected.
CPU measurements, initial noisy cases and native display limitations are recorded
in [UI-PERFORMANCE.md](UI-PERFORMANCE.md) and [MATRIX.md](MATRIX.md).

Preflight found prior Windows SQL bundle tests failing with access denied. The
fix closes temporary destination handles before atomic replacement, retaining
TempPath cleanup ownership. SQL/transaction/cancellation semantics and database
call sequencing remain unchanged: no added network round trips. The existing
two SQL export round-trip tests now pass on Windows CI. Driver latency tests were
not repeated for this filesystem-only fix; prior export/provenance validation is
recorded in [RESULT-SHARING.md](RESULT-SHARING.md).

MCP is macOS/Linux only and uses explicitly enabled live SQL sessions; it is not
intrinsically read-only. Windows transport, Redis MCP and built-in AI chat are not
included. Native Windows/Linux GUI acceptance, Intel Mac, installed Sparkle
old-to-new upgrade and 60/120 Hz/mixed-DPI input-to-present checks remain unverified.

## Matching live website

Website content commit `3fa15b4` contains the exact tagged browser archive and
updated version/capability text. Cloudflare deployment/version
`2d557c67-1b39-4c09-88e6-39ff5651dd21` serves it at https://rustable.app/demo/.
The downloaded draft browser archive exactly matches the locally tested CI
artifact. BUILD.txt identifies v0.4.32 and the release commit with unmodified
tracked source. Local and live browser startup passed; `SELECT 432 AS
release_check;` returned one row containing 432 in each.

All six live version pages, BUILD.txt and the WASM binary exactly match the
committed site content. COOP, COEP, WASM CSP and demo indexing headers were
preserved and verified. Public WASM SHA-256:
`51ffea69fb977004703733716f87c067601b205e6016b69f5cc58dde5011a8dc`.
No Cloudflare credential waiver was needed for this release.

Issues [#11](https://github.com/RustableApp/Rustable/issues/11) and
[#12](https://github.com/RustableApp/Rustable/issues/12) are completed. PR
[#10](https://github.com/RustableApp/Rustable/pull/10) was closed unmerged in favor
of the live-workspace approach, with the requested
[comparison and acknowledgment](https://github.com/RustableApp/Rustable/pull/10#issuecomment-6969786863).
Logs, assets, provenance and validation evidence are retained in
`target/release-v0.4.32/`.
