# v0.4.26 release verification

Published November 13, 2026 at 08:35:43 America/Chicago
(`2026-11-13T14:35:43Z`). Release ID `RE_kwDOVeqlPc4afAB2`.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.26), immutable tag source `c7c851615493b7823a3a115285619cd0a13df725`.
- Includes complete desktop SQL results with disk-backed storage, continuous virtualized scrolling, preloading above/below the viewport, stable cache-miss placeholders, full-result search/export and bounded background Copy All.
- Local locked tests: 824 passed, 213 opt-in tests ignored. Strict all-target QA Clippy and separate locked browser/WASM compilation passed (78 existing WASM warnings).
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43441792889) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/43441796797) passed every native and browser job.
- Downloaded draft assets passed all six payload checksums. Mac deep/strict signatures, stable team/bundle/updater identity, Gatekeeper, app/DMG notarization tickets, DMG integrity, Sparkle Ed25519 signature and embedded notes passed.
- Windows ZIP integrity/x64 PE executables and Linux x64 ELF executables/license manifests passed. Tagged demo provenance reports unmodified source; its archive is byte-identical to the browser-tested workflow artifact.
- All seven latest-download URLs returned HTTP 200. GitHub asset digests match the verified files, and the public updater feed matches the verified signed asset. No published versioned asset was replaced.

## Correctness, performance and acceptance scope

See [complete-query-results verification](COMPLETE-QUERY-RESULTS.md) for the
SQLite, PostgreSQL, MySQL and MariaDB fixture coverage, cancellation/errors,
transactions, exact large values, file lifetime and memory bounds. PostgreSQL
retains one established-connection exchange per ordinary editor statement and
the existing browse pipeline; local scrolling adds no database requests.

The final PostgreSQL release measurements at 30 ms simulated RTT passed:
ordinary editor p50/p95 37.3/38.0 ms, complete 6,000-row disk-backed collection
45.9/48.3 ms, validated browse pipeline 40.0/43.1 ms. These include local
result-storage costs; they are not server-only timings.

The final UI comparison against `7910e11` passed 26/27 scenarios, with one
noise-related inconclusive tessellation case (+1.6% median p95). The broader
suite remains inconclusive, not an all-cases pass. The direct before/after
preloading comparison passed all four continuous-scroll workloads under the
unchanged 10% p95 threshold. Raw measurements and limitations are documented in
COMPLETE-QUERY-RESULTS.md and retained under `target/scroll-prefetch-ui/`.

Native dark/wide, light/narrow, forced cache-miss and final-row captures were
reviewed. Native 60/120 Hz frame intervals, input-to-present timing, mixed DPI,
multiple-window load and monitor transitions remain unmeasured. Native
Windows/Linux GUI acceptance, Intel Mac and installed old-to-new Sparkle
upgrade remain unverified. Disk-backed results are read-only; Copy All has a
32 MiB limit and individual rows over 64 MiB report an error. The browser demo
retains its explicit preview bounds.

## Matching website

Website content commit `2382820fefc52a38436a4ad27b5ff4be3f659bd5`; Cloudflare version
`c76b181f-0fdf-47e1-ab47-b61bcf0789bb`.

All 14 website tests passed. All six live landing pages, demo BUILD.txt and WASM
match deployed source. Live WASM SHA-256:
`f86462aff200c4cb342a34b75f7db0c3a577c04bcfa304aa174719a385f2290e`.
COOP/COEP and WASM CSP passed. Both the exact downloaded artifact and deployed
demo started in the browser and returned 426 for `SELECT 426 AS release_check;`.
The live demo uses the same release-tag commit as the desktop.

Local package checks, public HTTP results, deployment output and source/binary
provenance are retained in `target/release-v0.4.26/`.
