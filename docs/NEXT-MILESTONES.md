# Autonomous development after v0.1.0

User direction, November 1, 2026: continue overnight without routine input, compare and refine the UI against TablePlus, add MySQL and Redis, and publish only meaningful milestones while conserving GitHub minutes. The existing production-readiness/parity goal remains active; this document does not narrow it.

## Release discipline

- Work on development branches. Keep routine changes off main.
- Prefer local mise/Cargo checks during iteration. Do not open/update a PR merely to trigger CI for every small edit.
- Run the cross-platform matrix for a consolidated release candidate, or when a platform-specific change needs evidence unavailable locally.
- Publish only a coherent, verified user-visible milestone; default to no more than one release in 24 hours unless a serious regression or security fix warrants it. Time elapsed alone is not a reason to release.
- Preserve release tags and published assets. Bump versions and generate fresh signed/notarized macOS packages and signed update feeds.
- Keep website support claims, downloads and release notes aligned with actual shipped capabilities. Post-v0.1.0 query/group fixes are not in v0.1.0.

## Next milestones

1. **Native workflow polish.** Verify query/group selection, rename, keyboard focus and persistence; compare library/history, navigation, inspector, editor and grid with supplied TablePlus references and disposable native fixtures. Fix observed differences, including missing semantic icons. Keep the detailed parity ledger open until each acceptance criterion has direct evidence.
2. **MySQL and MariaDB vertical slice.** Add separate connection configuration, redacted credentials, verified TLS and appropriate engine capabilities. Implement persistent worker-owned sessions, metadata browsing, bounded typed query results, dialect-aware editor behavior, cancellation and disconnect recovery. Verify both MySQL and MariaDB on disposable servers before advertising support. Then complete staged edits, transaction handling and import/export with integration coverage.
3. **Redis vertical slice.** Provide a key browser and command interface appropriate to Redis instead of presenting SQL/table controls. Support authentication, TLS, database selection, cursor-based SCAN, TTL and typed values with explicit bounds. Verify strings, hashes, lists, sets and sorted sets using disposable Redis. Add editing and deletion with clear scope, binary-safe data and recovery/error behavior. Never use blocking KEYS to populate the browser.
4. **Performance and platform validation.** Measure connection/query/browse round trips and rendering/memory behavior, retain bounded work on large fixtures, and run native platform acceptance where available. Compilation is not native UI verification.

## Architectural findings to carry forward

`src/db.rs` already owns a worker-side Driver trait, but its required execute/browse/plan/apply methods assume relational SQL. PostgreSQL configuration and profile/session plumbing are engine-specific. MySQL needs dialect and value conversion audits (including unsigned integers and decimals); Redis needs a non-relational capability model and dedicated UI. Do not add selectable engine names before their connection and browsing workflows work.

Use only disposable fixtures for write/cancellation/failure experiments; do not mutate the user's live databases or close their TablePlus workspaces for reference access. Continue independently when a reference window or platform is unavailable, recording the unverified gate rather than claiming parity.
