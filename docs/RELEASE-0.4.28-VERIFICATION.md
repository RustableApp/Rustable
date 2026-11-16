# v0.4.28 release verification

Published November 16, 2026 (`2026-11-16T16:29:39Z`). GitHub release ID `447296874`.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.28), immutable tag source `a1777120aa0b1dcf4926d34f26e45d24b23b4054`.
- Adds double-click row creation, typing-to-table Undo, scoped Cancel/Discard prompts, and neutral backgrounds for active cell editors. See [release notes](RELEASE-0.4.28.md).
- Local tests: 839 passed, 222 ignored. Strict all-target QA Clippy, locked browser compilation, and five package-tool tests passed.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43901830887) passed Windows, Linux and browser jobs. Its macOS job expired without acquiring a runner; GitHub reported macOS ARM capacity constraints.
- [Tagged release workflow](https://github.com/RustableApp/Rustable/actions/runs/43903403949) passed Windows, Linux and browser jobs. The remaining queued macOS 15 job was canceled after their artifacts were retained.
- [macOS 26 recovery](https://github.com/RustableApp/Rustable/actions/runs/43903405376) packages the same immutable tag, using the documented recovery path. All native tests, packaging, signing and notarization passed (839 tests passed, 222 ignored).

All six payload checksums passed. Mac deep/strict signatures, stable team/bundle/
updater identity, Gatekeeper, app/DMG notarization tickets, DMG integrity, Sparkle
Ed25519 signature and embedded release notes passed. Windows ZIP integrity,
x64 PE executables, Linux x64 ELF executables and bundled licenses passed.

All seven uploaded asset digests and lengths match verified local files.
All seven versioned and latest-download URLs returned HTTP 200. The public
latest updater feed matches the verified signed asset. No published asset
was replaced.

## Correctness and performance scope

All 21 release-mode UI performance comparisons passed, including identical
focused cell-editor workloads in light and dark themes on both revisions.
Typing undo/redo and handoff, scoped discard and busy-work checks, empty-space
row creation, and active-cell paint geometry have regression coverage. Native
light/dark editor screenshots were inspected before release. Raw measurements,
fixtures and limits are recorded in [UI-PERFORMANCE.md](UI-PERFORMANCE.md).

Database execution/protocol sequencing is unchanged. Native input-to-present
latency, 60/120 Hz, mixed DPI and multi-window close/quit acceptance remain
unmeasured. Native Windows/Linux GUI acceptance, Intel Mac and installed
old-to-new Sparkle upgrades remain unverified.

## Matching website

Website content commit `d50e7f5`; Cloudflare version
`e056a366-5e58-4930-a1d2-e6e62061a87f`.

The deployed demo is the exact tagged browser artifact from workflow
`43903403949`. BUILD.txt reports version 0.4.28, the release commit and no tracked
modifications. All 14 website tests passed. All six live landing pages, BUILD.txt
and WASM match deployed source. Existing COOP same-origin, COEP credentialless,
and WASM CSP headers were preserved and verified.

Live startup and `SELECT 428 AS release_check;` returned 428 in the browser.
Live WASM SHA-256 matches the release artifact:
`1140f4ec17f9298fb67db11578ab4d57eee88ed1589c6e85f739d6a6ca0446cc`.

Package checks, public HTTP results, live browser screenshot, deployment output
and provenance are retained in `target/release-v0.4.28/`.
