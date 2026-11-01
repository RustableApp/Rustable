# MySQL and MariaDB language intelligence

Initial discovery: November 1, 2026. These are candidates, not verified integrations. No implementation has been adopted or benchmarked yet.

## Existing Rust candidates

- [duck-sqllsp](https://github.com/gentleeduck/duck-sqllsp) advertises MySQL/MariaDB syntax, completion, hover, formatting and introspection. Its README describes PostgreSQL as its deepest target and names tower-lsp/libpg_query. GitHub identifies an MIT license. Audit the MySQL/MariaDB implementation and tests before relying on its feature claims; investigate whether its semantic core can be embedded without PostgreSQL-native dependencies.
- [RainLib/lsp_sqls](https://github.com/RainLib/lsp_sqls) describes a Rust multi-dialect SQL language server including MySQL. GitHub identifies an MIT license. MariaDB-specific behavior and embedded/WASM compatibility remain unverified.

## Potential reference implementations

- [sqls-server/sqls](https://github.com/sqls-server/sqls): Go language server with MySQL support and completion. Candidate reference if Rust options prove inadequate.
- [joe-re/sql-language-server](https://github.com/joe-re/sql-language-server): non-Rust SQL language server supporting MySQL, completion and diagnostics. Candidate reference, not an adopted runtime dependency.

Before translating code, pin the upstream revision, inspect its actual license and transitive notices, and preserve required attribution. A translation is derived work, not a way to remove upstream licensing obligations.

## Evaluation gates

1. Inspect source and tests at pinned revisions. Distinguish keyword completion from semantic resolution using a real catalog.
2. Use the same disposable MySQL and MariaDB fixtures: schemas, quoted identifiers, aliases, joins, nested queries, CTEs, views and stored routines. Test incomplete SQL while typing.
3. Cover server SQL modes, including ANSI_QUOTES and NO_BACKSLASH_ESCAPES, DELIMITER handling and engine/version-specific syntax and functions. Check diagnostic ranges for Unicode text.
4. Measure cold/warm completion and diagnostics, cancellation, stale-response rejection, and memory on large catalogs. Avoid schema network requests on every keystroke; consume cached metadata with explicit invalidation.
5. Assess a pure Rust embeddable core, optional stdio LSP, and WASM build support. Database access should be injected through a catalog interface rather than forcing a second connection or credentials into the language layer.
6. Decide reuse, contribution, fork or separate Rust library based on evidence. Publish only a documented, tested implementation with an honest supported-feature matrix.

Official database support also requires working connection, metadata, query, editing and recovery workflows against both actual servers. A language-server integration alone does not satisfy that milestone.

## Source audit and local checks

- `duck-sqllsp` revision `76a1a75dbb602820e89e339a7617049ea234bb0d`: separate catalog, resolver and completion crates; `dsl-parse` offers a sqlparser-only feature combination. Its dialect enum has MySql but no separate MariaDB variant. The completion crate enables parse defaults transitively, so disabling the PostgreSQL C parser needs dependency-feature work rather than a top-level flag alone. The MySQL connection adapter introspects information_schema.
- `lsp_sqls` revision `5a3c53065701cf74e334d8b70ba548f6657ad4e0`: MySQL completion accepts an injected Schema. MariaDB is explicitly registered as an alias for the same MysqlDialect. Parsing uses a bundled tree-sitter C grammar, and the manifest declares `MIT AND Unicode-3.0`, more specific than the repository license badge.
- Searching both checkouts' Rust sources found no `ANSI_QUOTES`, `NO_BACKSLASH_ESCAPES`, or `sql_mode` references. This is a concrete integration risk, not proof that every mode-dependent case fails.
- Ran `cargo test --locked --test dml_completion_tests mysql` in lsp_sqls: its schema-scoped DML completion test passed. This is an in-memory catalog test, not a live-server test.
- Ran `cargo test --locked -p dsl-parse --no-default-features --features sqlparser` in duck-sqllsp: the parser suite passed. This establishes that the pure-Rust parser configuration builds locally; it does not establish MySQL/MariaDB completeness or WASM compatibility.

Decision remains open. Existing Rust code is substantial enough that a rewrite from another language is not justified yet. Next evaluation should exercise our SQL-mode and MariaDB-specific fixtures before selecting an integration.

### Editor execution preparation (2026-11-01)

Run-current and selected SQL now retain the MySQL/MariaDB client delimiter
active at the slice start. Previously a routine extracted without its earlier
DELIMITER directive could be split at internal semicolons by the execution
scanner. The prepared script carries that directive; the driver-facing scanner
still strips directives and delimiters from server statements. Selection can
change the inherited delimiter again. A caret on an excluded client-directive
line no longer falls back to executing the final unrelated statement.

Verification: 20 filtered SQL/review tests passed, including isolated Unicode
routine bodies, delimiter resets, selected-script overrides, and directive-line
caret handling. Strict native all-target Clippy passed. This is editor/lexical
preparation, not evidence of completed drivers or live MySQL/MariaDB execution.
Session SQL mode propagation remains required when wiring the drivers.
