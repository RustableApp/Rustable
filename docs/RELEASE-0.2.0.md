# Rustable v0.2.0

A larger database-workflow and UI update, with the same Rust UI shipped to desktop
and the live WASM/SQLite demo.

- MySQL 8.4 and MariaDB 11.4: connection testing, TLS/SSH, database browsing,
  transactions, cancellation, CSV import/export and reviewed row writes.
- Faster wide-table rendering, viewport culling and more responsive mouse-wheel
  scrolling. Drag scrolling uses the middle mouse button.
- Lucide icons, consistent toolbar controls, full-width workspace tabs, compact
  sidebar search, cleaner database navigation and connection forms.
- Better connection-test diagnostics distinguish SSH and database failures.
- Saved-query selection, double-click opening, inline group naming and rename.
- Row context menus preserve multi-selection; copy rows/cells as plain text, CSV,
  JSON, HTML, Markdown or INSERT statements; duplicate rows and apply quick filters.
- Compound filters with enable toggles, operators, raw comparison expressions,
  per-row Apply, Apply All and export using the same predicates.
- Improved PostgreSQL completion and session handling, browser file/import/export
  workflows, CJK fonts and licensing notices.

## Limits

Early software, not complete TablePlus parity. MySQL/MariaDB reviewed writes need
InnoDB, a primary key and supported effective privileges. Visual schema changes
and a full MySQL language server are not included. Redis remains planned.
Raw filter expressions support comparisons/boolean combinations, not functions
or subqueries. Selected-cell operations target one column across selected rows;
paste edits one value, not a rectangular spreadsheet block.

macOS download is signed/notarized for Apple Silicon with Sparkle updates.
Windows/Linux downloads are x64 portable packages; updates are manual. Built-in
SSH remains macOS/Linux only. Linux requires Ubuntu 24.04-compatible libraries.
Intel macOS and native Windows/Linux UI acceptance remain unverified. The browser
uses local disposable SQLite, not remote server connections.

## Validation before packaging

462 native unit tests passed; 107 fixture/manual/benchmark tests ignored in the
ordinary suite. Strict all-target Clippy and locked WASM compilation passed.
Disposable MySQL, MariaDB and PostgreSQL browse/export checks passed. Full earlier
MySQL/MariaDB fixture evidence and remaining native acceptance limits are recorded
in MYSQL-MARIADB-RELEASE.md and DRIVER_CAPABILITIES.md. Native preview verified
compound-filter layout, numeric filtering and right-click multi-selection.
