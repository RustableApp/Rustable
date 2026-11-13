# v0.4.25 release verification

Published November 12, 2026 at 20:07:49 America/Chicago
(`2026-11-13T02:07:49Z`). Release ID `RE_kwDOVeqlPc4acsPg`.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.25), immutable tag source `fbb4569f7754fa192f4946c265be9a0bacc4c811`.
- Includes all branch work: smooth macOS animated window zoom/restore, repeated double-click editing, compact undo history, shared SQL layouts, obsolete-search cancellation and reused scalar search formatting.
- Local locked tests: 808 passed, 211 opt-in tests ignored. Strict all-target QA Clippy and separate locked browser/WASM compilation passed (78 existing WASM warnings). Eight benchmark-runner tests and native CPU calibration passed.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43347711550) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/43347711246) passed all platform and browser jobs.
- Downloaded draft assets passed all six payload checksums. Mac deep/strict signatures, stable team/bundle/updater identity, Gatekeeper, app/DMG notarization tickets, DMG integrity, Sparkle Ed25519 signature and embedded release notes passed. The vendored epaint license is included.
- Windows ZIP integrity/x64 PE executables and Linux x64 ELF executables/license manifests passed. Tagged demo provenance reports unmodified source. Its archive is byte-identical to the browser-tested workflow artifact.
- All seven latest-download URLs returned HTTP 200. GitHub asset digests match the verified local files; the public updater feed matches the verified signed asset. No published versioned asset was replaced.

## Performance and native acceptance scope

Earlier incremental comparisons passed all 38 performance workloads; the isolated
resize/click fix passed 19. The combined release was measured twice against
`06088d1` using five alternating pairs and unchanged fixtures/thresholds. Results
were 25 passes/13 inconclusive and 24 passes/14 inconclusive, with no repeatable
regression. These noisy combined runs are **not** claimed as passes. Substantial
concurrent display/video activity was observed afterward; it is a plausible
contributor, not a proven explanation. Raw results remain in
`target/release-v0.4.25/{performance,performance-repeat}`; see UI-PERFORMANCE.md.

The user confirmed smooth native title-bar zoom/restore and repeated cell editing
in the preview. Native frame intervals, input-to-present timing, missed frames,
60/120 Hz, secondary-window/mixed-DPI/monitor reconnect/snapping coverage remain
unmeasured. Native Windows/Linux GUI acceptance, Intel Mac and installed
old-to-new Sparkle upgrade remain unverified. No database execution/protocol path
changed; database latency fixtures were not rerun for this release.

## Matching website

Website content commit `bf574263f4cdcf9e761aae5a1e9cf9a294dbd067`; Cloudflare version
`afa9dc0c-15f9-4204-963a-285da678ea7b`.

All 14 website tests passed. All six live landing pages, demo BUILD.txt and WASM
match deployed source. Live WASM SHA-256:
`f8f138ff0937b9f89e546faa912f49927048f471ec89d2c269cc107676bd3f8c`.
COOP/COEP and WASM CSP passed. The exact downloaded artifact and deployed demo
both started in the browser and returned 425 for `SELECT 425 AS release_check;`.
The downloaded artifact's numeric result search selected that cell and reported
1 of 1 matches. The live demo uses the same release-tag commit as the desktop.

Local package checks, HTTP results, deployment output and source/binary provenance
are retained in `target/release-v0.4.25/`.
