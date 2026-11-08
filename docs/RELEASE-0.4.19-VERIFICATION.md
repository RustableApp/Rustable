# v0.4.19 release verification

Published November 7, 2026 (2026-11-08 04:56:36 UTC). Desktop packages and the matching live demo are verified.

## Publication and provenance

- Release: [v0.4.19](https://github.com/RustableApp/Rustable/releases/tag/v0.4.19), GitHub release ID `438041152`.
- Immutable source: `7cccbccd6ea5f8db264e02c18699457918f544a1`.
- [Main CI](https://github.com/RustableApp/Rustable/actions/runs/42448019529) and [signed release](https://github.com/RustableApp/Rustable/actions/runs/42448045988) both succeeded across all jobs.
- Website commit: `f5f85d4`; Cloudflare deployment version: `4828e984-9fe3-4d54-bf01-4d8a3047740e`.
- All six payload checksums passed. macOS app signature, Gatekeeper assessment, app/DMG notarization tickets, DMG integrity, stable bundle/update-key identity and Sparkle Ed25519 signature passed. Embedded updater notes match the release notes.
- Windows ZIP integrity and three x64 PE executables, Linux x64 ELF executables, and bundled license manifests passed inspection.
- The public demo archive matches the exact release artifact. Live BUILD.txt reports version 0.4.19, the source commit above and unmodified tracked files. Live WASM SHA-256: `0f7be91f3449dfbc2c1978d9211b51545278d7209de986248a2af97166e4b924`.
- Website embedding tests: 3 passed. Live Chrome startup, `SELECT 419 AS release_check` returning 419, and Redis demo `PING` returning PONG were visually verified; no page errors. Isolation/CSP headers passed.
- All seven latest-release download URLs returned HTTP 200; the public update feed matches the verified asset.
- Local evidence: `/tmp/rustable-v0419/` contains package verification logs, deployment output, live verification and screenshots (`live-query.png`, `live-redis-workbench.png`).

## Local validation

- Full locked suite: 738 passed, 190 opt-in tests ignored; no failures.
- Strict all-target Clippy passed.
- Separate browser-package locked WASM check passed (61 existing browser warnings).
- Fresh disposable Redis Cluster/vector, Sentinel discovery/failover and standalone
  vector-set fixtures passed. Fresh Sentinel TLS trust-scope validation also passed (`target/release-0419-sentinel-tls.log`). Logs: `target/release-0419-{cluster,sentinel,vectors}.log`.
- Existing standalone Redis 7.4/TLS/RedisJSON, module and transport validation is
  recorded in REDIS-ROADMAP.md. The standalone fixture was not rerun on port 16379
  because the user's demo occupies it; that server was left running and unchanged.
- Native layout screenshots: `target/redis-refinement-final/`; 135 Redis tests
  passed before version preparation, including copy feedback and draft protection.

## Performance and limits

See [UI performance evidence](UI-PERFORMANCE.md) for paired release comparisons,
raw result locations, accepted costs and inconclusive cases. Visible Workbench
history adds approximately 0.0151 ms CPU p95 (+43%) in its fixture; prior syntax
highlighting and richer inspector costs are documented there too. These are not
blanket performance passes or native input-to-display latency measurements.

This incremental release does not claim complete Redis Insight parity. Module
features require compatible servers; analysis samples loaded keys; history is
session-only. Windows built-in SSH, native Windows/Linux GUI acceptance, Intel
macOS, installed old-to-new Sparkle updates, mixed-DPI and display-latency checks
remain outside this release's verified scope. Cluster partition/election and
interrupted concurrent resharding coverage remains incomplete. Bounded Redis
reply TLS and further batch-error coverage remain follow-up work.
