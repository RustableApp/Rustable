# v0.4.30 release verification

Published November 16, 2026 (America/Chicago), at `2026-11-17T03:05:19Z`,
as GitHub release `448041833`. All seven versioned and latest download URLs
returned HTTP 200. The public latest updater feed matches the verified signed feed.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.30), immutable source
  `a891cc1b2ca7583afd0c43a334f51e7861d22351`.
- [Release notes](RELEASE-0.4.30.md): row-tail selection, blank-space clearing,
  Select All, green pending rows, keyboard editing and undo/redo cell selection.
- Local preflight: 855 library tests passed, 228 opt-in tests ignored; strict
  all-target QA Clippy, locked separate browser-package compilation and five
  packaging-tool tests passed. The browser build retains 83 existing warnings.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/44004594313)
  and [signed release](https://github.com/RustableApp/Rustable/actions/runs/44004605922)
  passed on all platforms. The first Windows release attempt hit the existing
  Redis MONITOR loopback transport timeout/500 ms test bound. Main CI passed on
  identical source; retrying only failed jobs passed without changing code or
  thresholds. Successful macOS, Linux and browser artifacts were preserved.

## Package verification

All six payload checksums passed. Uploaded asset sizes and SHA-256 digests match
verified local files. macOS deep strict signing checks, Gatekeeper assessment,
stapled notarization tickets, DMG signature and integrity passed. Bundle identity,
team `624KJV575U`, updater URL and Ed25519 key match the verified v0.4.29 app.
Sparkle's signature verifies against the exact ZIP; bundle/feed versions,
immutable enclosure URL, byte length and embedded notes match v0.4.30.
Draft metadata was explicitly bound to the existing verified tag before publication;
the tag was not moved.

ARM64 Mac app/LSP binaries, x64 Windows PE and x64 Linux ELF binaries have the
expected architectures. Portable archive integrity, bundled licenses and app icon
checks passed. Browser provenance identifies the clean immutable release source.

## Correctness and performance scope

Local native captures and tests cover pending-cell colors, selection, double-click
reediting, arrows across loaded/pending rows, offscreen navigation, Enter editing,
nonselectable display labels, undo/redo field targeting, row tails and blank clearing.
The intermediate navigation CPU comparison had 20 passes and 3 inconclusive metrics.
The final label/history comparison was inconclusive on all 23 metrics under substantial
unrelated CPU load. No performance parity or improvement is claimed. Timings, raw
samples, baseline provenance and limits are in [UI-PERFORMANCE.md](UI-PERFORMANCE.md).
No database execution/protocol sequencing changed; driver latency fixtures were not rerun.

Physical input-to-present latency, 60/120 Hz, mixed-DPI/multi-window transitions,
native Windows/Linux GUI acceptance, Intel Mac and installed old-to-new Sparkle
upgrades remain unverified.

## Matching live website

Website content commit `58eaf2b5706fcce09b4a0c2f2292912a3a8bedca`; Cloudflare version
`602bf848-e477-4198-8fed-8c5acf96dca7`. All 14 website tests passed.
The exact tagged browser artifact started locally and passed row-tail/blank-space
selection, pending-row navigation and editing, and undo/redo field-target checks.
No staged test row was committed.

The deployed demo started and `SELECT 430 AS release_check;` returned 430.
All six landing pages and BUILD.txt match the local website. Public WASM SHA-256
matches the exact release artifact:
`a1a161461eaaae27f3578e52528ad460d5778459af90609691b1b88f87065816`.
COOP `same-origin`, COEP `credentialless`, WASM CSP and demo indexing headers
are preserved. Screenshots: `local-draft-redo.jpg` and `live-demo.jpg`.

Logs, provenance, package checks, public HTTP checks and screenshots are retained
in `target/release-v0.4.30/`.
