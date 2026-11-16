# v0.4.29 release verification

Published 2026-11-16T21:41:57Z as GitHub release `447651644`. All seven versioned and latest download URLs returned HTTP 200; uploaded sizes and SHA-256 digests match the verified local files. The public latest updater feed exactly matches the verified signed feed.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.29), immutable source `eb7b501eb7f7e2b3efadc9c0e9c2928325d10644`.
- [Release notes](RELEASE-0.4.29.md): cell Undo continuation, staged-cell reediting, untouched-editor navigation and compact discard confirmations.
- Local preflight: 847 library tests passed, 224 opt-in tests ignored; strict all-target QA Clippy, locked browser compilation and five packaging-tool tests passed.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43955063072) passed on macOS, Windows, Linux and browser. [Tagged release build](https://github.com/RustableApp/Rustable/actions/runs/43956880111) completed Windows, Linux and browser; its macOS 15 job remained queued and was cancelled after the other artifacts were saved. [macOS 26 recovery](https://github.com/RustableApp/Rustable/actions/runs/43958586477) passed tests, packaging, signing and notarization from the same immutable v0.4.29 tag. Successful platforms were not rebuilt.

## Package verification

All six payload checksums passed. macOS app and DMG signatures, deep strict code
verification, Gatekeeper assessment, stapled notarization tickets and DMG integrity
passed. Bundle identity, team `624KJV575U`, updater feed URL and Ed25519 public key
match v0.4.28. The Sparkle signature verifies against the exact ZIP; its immutable
enclosure URL, byte length and embedded release notes match v0.4.29.

The ARM64 Mac app and its two language servers, x64 Windows PE executables and
x64 Linux ELF executables have the expected architecture. Portable archive
integrity, bundled licenses, app icon and browser source provenance passed.
The manual draft combined only same-tag artifacts and its seven uploaded assets
were verified before publication.

## Correctness and performance scope

All 24 release-mode UI comparisons passed against `a427eef`, including selected
cell editors in both themes and staged history. New populated-cell Undo/navigation
and discard-dialog workloads each passed five serial release runs. The subsequent
compact dialog copy/layout adjustment was checked with native dark/light captures
and existing button/keyboard tests; the CPU comparison predates that static styling
adjustment. See [UI-PERFORMANCE.md](UI-PERFORMANCE.md) for timings, raw artifact
locations, environment and limitations.

Tests cover text Undo/Redo handoff, cross-cell focus, fresh typing protection,
reopening staged cells with real pointer/key events, unchanged grid/inspector
editors, Cancel/Discard, invalid values, empty text versus NULL, scoped discard,
connection switching and busy-work rechecks. SQL drafts are retained. Accepted
operations retain their existing database request/protocol sequencing; no driver
latency fixtures were rerun for the UI guards.

Native input-to-present latency, 60/120 Hz, multi-window and mixed-DPI behavior
remain unmeasured. Native Windows/Linux GUI acceptance, Intel Mac and installed
old-to-new Sparkle upgrades remain unverified.

## Matching website

Website content commit `be2758b9cc13987c96c6bcf032e21cb916523d15`, Cloudflare
version `ca6ca507-9cd4-4e5f-a383-7397f6b7266b`. All 14 website tests passed.
All 366 tagged browser payload files match the tested main artifact. Local
acceptance verified untouched-editor refresh, orange staged-cell reediting,
typing Undo followed by staged Undo, and Cancel/Discard preservation and removal.

The deployed demo started and a SQLite SELECT returned the expected 429.
All six landing pages and BUILD.txt match the local website. Live WASM SHA-256
matches the exact release artifact:
`c0b88864fda5bc48663ae90b535e6a1c0025725f2d6a188ec34d77f18f6924e6`.
COOP `same-origin`, COEP `credentialless`, WASM CSP and demo indexing headers
are preserved. The initial check caught brief landing-page propagation lag;
a subsequent complete check passed. Screenshots: `local-discard.jpg` and
`live-demo.jpg` in the release artifact directory.

All release logs, local package checks, public HTTP verification and live demo
screenshots are retained in `target/release-v0.4.29/`.
