# v0.4.19 release verification

Release candidate prepared November 7, 2026. Publication and deployment are pending.

## Local validation

- Full locked suite: 738 passed, 190 opt-in tests ignored; no failures.
- Strict all-target Clippy passed.
- Separate browser-package locked WASM check passed (61 existing browser warnings).
- Fresh disposable Redis Cluster/vector, Sentinel discovery/failover and standalone
  vector-set fixtures passed. Logs: `target/release-0419-{cluster,sentinel,vectors}.log`.
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
