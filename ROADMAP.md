# TablePlus parity roadmap

Reference: [TablePlus documented workflows](https://tableplus.com/docs/overview). This is an implementation roadmap, not a claim that the features below are available.

## 1. Harden the SQLite vertical slice (current)

Implemented: connection open, object browser, table pages, filtering, sorting, schema inspection, query tabs/history/favorites, cancellation, safe queries, optimistic single-cell edits, atomic cancellable loaded-result and full matching-table CSV export, local settings, native app bundle. Editor now has current/selection/all execution, independent result tabs, SQLite-aware statement splitting, cached syntax highlighting, stable editor identities, closable/reopenable tabs, and OS-following appearance. Scripts support BEGIN/COMMIT within one run and rollback unfinished transactions.

Next: persistent per-connection workspaces; reorderable tabs; find/replace; schema-aware autocomplete; full query statistics; keyboard grid navigation; accessible names and focus order; native menus; lazy large-cell inspection; horizontal column virtualization.

Acceptance: no data operations on the UI thread; error and cancel paths leave the connection usable; types round-trip; stale edits never overwrite another writer silently; 1M-row indexed SQLite fixtures remain navigable with bounded results.

## 2. Server databases and connections

Introduce a driver interface with engine capabilities and typed values, then implement PostgreSQL and MySQL/MariaDB. Each session owns a worker/runtime, cancellation handle, connection lifecycle, schema cache and independent request IDs. Add TLS certificate validation, connection groups/colors, reconnect behavior, native keychain credentials, and SSH tunneling. Never store secrets in workspace preferences.

Acceptance: container-backed integration suites for each engine, cancellation of long queries, bounded streaming/backpressure, network-disconnect recovery, transaction-state reporting, and secure credential handling.

## 3. Full data workflows

Staged inserts/updates/deletes; change review and atomic commit/rollback; composite-key editing; foreign-key navigation; filter builder; full-table/search scopes; CSV and SQL import/export with preview, mapping, progress and cancellation; indexed/keyset pagination where possible; concurrent data-change detection.

Acceptance: representative real schemas, triggers, generated columns, views, binary/JSON/decimal/timestamp values, rollback on partial failures, and no silent truncation on exports.

## 4. SQL development

Cross-run explicit transaction controls; query parameters; format SQL; autocomplete and diagnostics; split panes; saved query folders; command palette; configurable keyboard shortcuts; execution plans.

## 5. Administration and broader engine parity

Schema/index/constraint editors; users/roles; sessions/process list; server metrics; backup/restore; ER diagrams; additional relational and document/key-value engines. Track each driver's supported capabilities rather than presenting nonfunctional controls.

## 6. Performance and distribution gates

- Measure input-to-paint latency and p50/p95/p99 CPU/GPU frame times on 60/120/144 Hz hardware.
- Target p95 frame time below 6.94 ms at 144 Hz during indexed-grid navigation; report hardware and fixture size.
- Measure idle CPU/wakeups, startup time, peak RSS, large-grid scrolling and result-stream backpressure.
- Stress slow queries, lock contention, million-row results, huge cells, many columns and concurrent sessions.
- Add Windows/Linux CI and native UI smoke tests; signed/notarized macOS distribution; platform installers and updates.
- Review accessibility, keyboard-only workflows, high-DPI scaling and localization before a production release.

- Editor follow-up: CTE/derived column inference, nested alias scopes, engine-specific identifier rules beyond SQLite and large-schema metadata/cache performance. Basic current-statement table/alias column completion is implemented and natively verified. Comma-separated FROM lists now have automated coverage for alias completion and expression/clause boundaries.
- Inspector follow-up: harmonize structure inspector density with the compact single, bulk and insert inspectors; preserve multiline editing for large values.

- CSV import: bounded cancellable UTF-8 preview is implemented and natively verified (100 rows, header/delimiter/quote controls, virtualized grid). Next: source snapshot/change detection, target metadata and explicit column mapping, NULL/type handling, transaction preview and all-or-nothing streaming import with cancellation rollback. Current preview is read-only.

- CSV mapping progress: per-column selectors, explicit exclusion, unique name matching, generated-column exclusion and duplicate validation are implemented for captured current-table metadata. Dedicated target selection/fresh schema validation and actual transactional import remain open.

- Import destination selection now loads worker-side table metadata independently from browsing, rejects views/missing targets and resets on connection changes. Still required: source snapshot, commit-time schema/session revalidation, explicit NULL/type policy, transaction review/execution, cancellation rollback and native destination-selector verification.

- CSV source snapshots implemented: fixed-buffer cancellable temporary copy retained across option changes, released on close; ordinary source-change checks during capture. Next import steps: NULL/type policy, commit-time session/schema validation, transactional streaming execution and review, progress/disk-space UX. Native destination dropdown and snapshot status checks remain pending.

- Transactional SQLite CSV import is implemented with snapshot streaming, explicit empty-to-NULL option, schema revalidation, bound inserts, cancellation/error rollback and a first-ten-insert review modal. Next: native end-to-end QA import, dedicated import menu placement, progress and disk-space UX, richer conversion policies/new-table import, broader driver support, and review-modal shortcut/session tests.

- Native CSV import verified end to end in disposable QA (two rows, quoted comma, Unicode bytes, real affinity, count 2400→2402). Missing CJK font glyphs discovered in preview/review/grid; add suitable font fallback and verify rendered Japanese, without changing stored text.

- CJK rendering defect fixed and natively verified on macOS using shared system Unicode fallback after existing faces. Windows/Linux candidate paths still need platform validation; complete script shaping, emoji, font discovery and startup/RSS/frame-time measurements remain outstanding.
