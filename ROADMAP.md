# Rustable roadmap

Updated 2026-11-01 for v0.3.0. This distinguishes implemented functionality from
future work; engine-specific limitations are in [the capability audit](docs/DRIVER_CAPABILITIES.md),
[MySQL/MariaDB support](docs/MYSQL-MARIADB-RELEASE.md), and [Redis support](docs/REDIS.md).

## Implemented baseline

- SQLite, PostgreSQL, MySQL and MariaDB connections; standalone Redis in v0.3.0.
- SQL browsing, compound filtering, sorting, query tabs, saved-query groups,
  history, command palette, formatting and basic schema-aware completion.
- Reviewed SQL row edits and CSV workflows, subject to each driver's restrictions.
- TLS, OS credential storage, connection groups/colors, and supported SQL SSH tunnels.
- Virtualized grids, cached rendering, responsive worker-backed operations, and
  smooth desktop scrolling.
- Persistent pane sizes/activity visibility, compact consistent controls, and a
  shared Rust/WASM SQLite demo updated with every release.
- Redis key grid, six core value-type inspectors, guarded string/TTL/delete edits,
  command execution, ACL authentication and verified TLS.

## Next priorities

1. **Measured performance:** reproducible input-to-paint and p50/p95/p99 frame-time
   reports on wide tables, large cells and simultaneous sessions. Report hardware,
   refresh rate, fixture and optimized build. The target is p95 below 6.94 ms at
   144 Hz; this is a target, not a verified cross-hardware result.
2. **Redis workflow depth:** discover logical databases, visual collection editors,
   saved commands/history, and clearer large-value preview/navigation. Cluster,
   Sentinel, SSH, bulk import/export and pub/sub need separate implementation and
   acceptance; they are not implied by standalone support.
3. **SQL tooling:** improve MySQL/MariaDB completion and diagnostics, evaluate a
   reusable Rust language-server implementation, deepen scoped completion, query
   parameters and execution-plan workflows.
4. **Cross-platform acceptance:** Windows/Linux native UI smoke tests, Windows SSH,
   Intel macOS, platform installers/updaters, and installed old-to-new Sparkle tests.
5. **Interaction and accessibility:** keyboard-only navigation/focus order,
   accessible controls, high-DPI and script/font verification across platforms.
6. **Broader database workflows:** richer import conversion/new-table workflows,
   schema/index/constraint administration, foreign-key navigation, sessions,
   server metrics, backup/restore and ER diagrams where supported.

## Release gates

Run native regression tests, strict Clippy and relevant disposable driver suites;
smoke-test the final optimized build. Verify packaged binaries/licenses, macOS
signing/notarization, checksums and update feed. Deploy and verify the WASM demo
from the exact release tag before publishing. Batch milestones rather than
triggering frequent packaging builds. See [release workflow](docs/RELEASES.md).
