# v0.4.21 release verification

Published November 8, 2026 at 18:20:06 UTC. Desktop release and exact matching live WASM demo verified.

## Publication and provenance

- [Release v0.4.21](https://github.com/RustableApp/Rustable/releases/tag/v0.4.21), GitHub release ID `438430030`; published stable, not a draft or prerelease.
- Immutable annotated tag resolves to `16e53fa793c0d2fd8b67148e5c23ca9acebc3a5a`.
- [Main CI 42507572438](https://github.com/RustableApp/Rustable/actions/runs/42507572438) and [signed release 42508380169](https://github.com/RustableApp/Rustable/actions/runs/42508380169) passed all jobs. Both Cargo lockfiles match version 0.4.21.
- Website commit `a546285`; Cloudflare version `2c456261-d8a6-48ab-bf2d-9a9402af9a23`.
- All six payload checksums passed. macOS deep/strict signature, Gatekeeper assessment, app and DMG notarization tickets, DMG integrity, stable bundle/updater-key identity, embedded release notes and Sparkle Ed25519 signature passed. The read-only mounted DMG's signed application executable and Info.plist match the verified ZIP.
- Windows ZIP integrity and three x64 PE executables passed. Linux archive contains the app and both language-server x64 ELF executables. Both portable packages contain license manifests. These inspections do not claim native GUI acceptance on those platforms.
- All 364 staged website demo files matched the tagged archive byte-for-byte. Live BUILD.txt reports 0.4.21, the immutable commit above, and unmodified tracked files. Live WASM SHA-256: `529f85d41beb36dd5205c175ff6f3a6338caa69404e3aee7967b67e873d89aec`.
- Fourteen website embedding/SEO tests passed. Local exact-artifact and live Chrome checks visually verified `SELECT 421 AS release_check` returning 421, Redis `PING` returning PONG, and direct Redis-first startup. No page errors; cross-origin isolation and CSP headers passed.
- All seven latest-release asset URLs returned HTTP 200. The public updater feed matches the verified draft asset.
- Evidence: `/tmp/rustable-v0421/`, including `release-verification.log`, `mac-verification.log`, `linux-inspection.log`, `deploy.log`, `live-final-verification.log`, `live-browser.log`, `redis-first.log`, `live-query.png`, `live-redis-workbench.png`, and `redis-first.png`.

## Checklist audit

The bounded [release checklist](REDIS-FINAL-RELEASE-PLAN.md) contains implementation,
protocol, screenshot and test evidence for each milestone, not just a completion flag.

| Requirement | Accepted evidence |
| --- | --- |
| SSH and client certificates | Standalone, Cluster and Sentinel mutual-TLS fixtures; seven application-path SSH tests. Verified identity and tunnel ownership; unsupported combinations documented. |
| Search catalog and reviewed create/drop | Catalog fixture, schema/control tests and native screenshots; bounded cached catalog loading and destructive review. |
| Database-wide analysis | Bounded incremental collector tests, cancellation/progress/expiry/report checks, live standalone fixture, native Analyze screenshots and scrolling coverage. |
| History and per-command results | Persistence/privacy/capacity tests, per-command status/error/copy results, transport audit and Workbench screenshots. |
| IntelliSense | Count-dependent clauses, cached capability tests, exact/prefix/inner/fuzzy shared ranking tests, keyboard acceptance and release CPU measurements. |
| Vector controls | Supplied vector/filter/search options, bounded protocol fixtures and matching interaction benchmark. |
| JSON paths and multi-entry creation | Two live atomic editing fixtures plus validation/conflict/ACL/TTL coverage and retained-draft tests. Final pretty JSON-string editing is also verified in actual WASM with exact large-integer retention. |
| Isolated transactions/blocking evaluation | Explicit safety assessment in REDIS-COMMAND-WORKFLOWS.md; workflows remain unavailable. Protocol guards tested before any preceding write can be sent. |
| Focused UI and user follow-ups | Analyze/Search/PubSub table/header snapshots in both themes, hover tests, macOS system double-click timing tests, inline JSON editing and compact inspector alignment/copy feedback. |
| Transport/topology safeguards | 25 standalone tests across 17 invocations; 27 Cluster tests plus six-node failover test; 15 Sentinel tests. Bounded TLS/error alignment/oversized reply closure and no uncertain-write replay exercised. |
| Final local/CI validation | 793 local locked tests passed, 206 opt-in tests ignored; strict QA/all-target Clippy and separate browser release build passed. Main and tagged release CI passed all platforms. |
| Publication and matching demo | Package, live browser, provenance/hash/header, download and updater evidence above. |

Local logs were rechecked at completion: `/tmp/redis-json-final-tests.log`,
`/tmp/redis-json-final-clippy.log`, `/tmp/redis-json-browser-build.log`,
`/tmp/redis-final-{standalone,cluster,cluster-failover,sentinel}.log`,
`/tmp/redis-{mtls-live,sentinel-mtls,cluster-mtls,ssh-integration,search-catalog-live,editing-live-tests}.log`.
The browser build retained 78 known warnings; it was not described as warning-free.

## Performance and limits

[UI-PERFORMANCE.md](UI-PERFORMANCE.md) records the same-machine release comparisons,
raw samples and concrete accepted costs for each milestone. The final seven-pair
repeat passed 36 scenarios and retained four completion regressions of
0.006–0.013 ms p95, justified by requested ranked matching and whole popup rows
after static-name caching removed most initial overhead. They are not threshold
passes. New JSON-string editing has a matching candidate-only workload; existing
inspector comparisons pass. No new database round trips were added by final UI work.

Transactions and blocking commands were evaluated and explicitly deferred for
isolated write lifecycle, cancellation/outcome, selected-database and transaction
node ownership requirements. Windows built-in SSH remains unsupported. Native
Windows/Linux GUI acceptance, Intel macOS, installed old-to-new Sparkle update,
high-refresh input-to-present, mixed-DPI and multiple-monitor interaction checks
remain unverified. Cluster coverage does not assert arbitrary quorum-partition
correctness. CPU timings and screenshots do not establish these untested properties.
This release does not claim exhaustive Redis Insight parity.
