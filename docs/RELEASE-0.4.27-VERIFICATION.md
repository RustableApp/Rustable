# v0.4.27 release verification

Published November 13, 2026 at 18:21:03 America/Chicago
(`2026-11-14T00:21:03Z`). GitHub release ID `444600954`.

- [Stable release](https://github.com/RustableApp/Rustable/releases/tag/v0.4.27), immutable tag source `adbfb0c81b727c198607ec3ec52c6da475cd663d`.
- [PR #9](https://github.com/RustableApp/Rustable/pull/9) merged and closed [issue #8](https://github.com/RustableApp/Rustable/issues/8). PostgreSQL has five explicit TLS policies with muted dropdown descriptions and corrected imports. Existing SSL-enabled profiles migrate to Verify full; disabled profiles stay disabled. MySQL, MariaDB and Redis behavior is unchanged.
- Local tests: 830 passed, 218 ignored. Strict QA Clippy, locked browser compilation (79 warnings), and five package-tool tests passed.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/43481300929) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/43482700070) passed all native and browser jobs. An initial PR CI failure exposed a result-snapshot cleanup test race; the test now waits with a bounded deadline for canceled background work to release its reference. Production behavior was unchanged.
- All six payload checksums passed. Mac deep/strict signatures, stable team/bundle/updater identity, Gatekeeper, app/DMG notarization tickets, DMG integrity, Sparkle Ed25519 signature and embedded release notes passed.
- Windows ZIP integrity/x64 PE executables and Linux x64 ELF executables/license manifests passed. The draft demo archive matches the tagged workflow artifact exactly; provenance reports unmodified release-tag source.
- All seven latest-download URLs returned HTTP 200. GitHub asset digests match the verified local files, and the public updater feed matches the verified signed asset. No published versioned asset was replaced.

## Correctness and performance scope

Disposable PostgreSQL 17 TLS fixtures verified encrypted Require/Prefer connections,
verified-mode rejection of an untrusted wrong-host certificate, and refusal of
plaintext by Require. Driver, probe and monitor paths were covered. Existing
connections preserve their SSL policy.

Established-session exchange budgets remain one per ordinary editor statement
and one pipelined exchange for cached/sidebar browsing. Release-mode editor WAN
comparisons and browse regression checks passed at 30 ms simulated RTT. An
initial marginal debug large-result timing failure and passing rerun are retained
in the [performance record](UI-PERFORMANCE.md).

Native dark/light dropdown screenshots were inspected and included in the PR.
The repeated closed-form UI comparisons passed. Expanded-menu comparisons were
inconclusive, with an explicitly accepted approximately 0.006 ms (10–12%) p95
layout cost for displaying five policy explanations; this is not an all-scenarios
performance pass. Raw measurements, alternatives and limitations are documented
in UI-PERFORMANCE.md. Native input-to-present timing, 60/120 Hz, mixed DPI and
monitor/window transitions remain unmeasured. Native Windows/Linux GUI acceptance,
Intel Mac and installed old-to-new Sparkle upgrades remain unverified.

## Matching website

Website content commit `8f091a6cb36fd0d1b46d94bf63a476ee26ab0ce6`; Cloudflare version
`b0086be8-d4da-40cd-b621-c3e1aa311912`.

All 14 website tests passed. All six live landing pages, demo BUILD.txt and WASM
matched deployed source. Live WASM SHA-256:
`f62aefd6dc9ad85a244c50b11efe59aa0c213528ebfc32c834ec4165e2e85d4c`.
COOP/COEP and WASM CSP passed. All 366 main-build demo payloads matched the tagged
artifact, although archive metadata differed; the deployed files came from the
tagged artifact.

Browser access was disabled in Work settings. The user chose to perform the live
startup/SQLite check and was given the demo link and `SELECT 427 AS release_check;`.
The user subsequently explicitly instructed publication without reporting the
check result. Publication followed that instruction; live interactive acceptance
is not claimed as verified.

Package checks, public HTTP results, deployment output and provenance are retained
in `target/release-v0.4.27/`.
