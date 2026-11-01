# SQL language tooling evaluation — 2026-10-31

Editor integration is implemented. Real subprocess fixtures verify Syntaqlite 0.13.3 schema-aware SQLite completion/diagnostics and private stdio Postgres Language Server 0.28.1 parser diagnostics plus catalog-backed completion through its upstream Rust completion engine. Native SQLite QA verified unknown-column underlining, schema completion and token replacement; Native PostgreSQL QA verified actual-schema alias suggestions, Down+Tab acceptance and subsequent Cmd+Enter execution; the helper survives statement-end requests. The existing loaded-schema completion remains a fallback. See LANGUAGE_SERVERS.md and SQLITE_LANGUAGE_SERVER.md for exact capabilities and isolation.

## Candidates checked against upstream sources

- [Postgres Language Server](https://github.com/supabase-community/postgres-language-server): Rust-based MIT project, PostgreSQL parser via libpg_query; completion, hover, diagnostics, formatting and type checking. Strong first candidate for PostgreSQL. Its documented LSP interface permits keeping work off egui's render thread. It does not cover SQLite/MySQL; startup cost, incremental edits, credential handling, migration/lint side effects, and schema loading must be measured before adoption. Do not claim integrated.
- [sqls](https://github.com/sqls-server/sqls): Go/MIT SQL language server supporting several database engines, DML/DDL and join completion. Upstream explicitly says unstable interfaces. Useful behavioral reference; a Rust port creates an ongoing maintenance fork and retains license notices.
- [Apache sqlparser-rs](https://github.com/apache/datafusion-sqlparser-rs): Apache-2.0 Rust syntax parser across dialects. Upstream explicitly does not provide semantics; not a complete IntelliSense engine. Useful AST/tokenizer foundation, but incomplete statements and name resolution still require implementation.
- [tree-sitter SQL](https://github.com/DerekStride/tree-sitter-sql): permissive general SQL grammar available via Cargo as tree-sitter-sequel. Candidate incremental parsing foundation for malformed/incomplete editor input. A grammar is not a semantic completion engine; scope/schema resolution still required.

## Recommended evaluation path

Integrate the verified providers through the asynchronous completion interface. PostgreSQL currently runs disconnected without credentials or a database connection; do not claim server-backed catalog/type semantics. SQLite receives bounded cached schema DDL and requires the existing fallback for alias-qualified completion, which the upstream provider does not yet resolve. Keep loaded-schema fallback for unavailable tools and unsupported completion edits. All tool/dependency setup stays through mise/Cargo; no brew/npm toolchain requirement introduced.

Acceptance: relevant columns in joins/aliases/CTEs/nested scopes; correct quoted/Unicode names; suppress suggestions inside comments/strings; tolerate incomplete SQL; latest document version wins; cancellation/stale results cannot move cursor or replace text; no UI-thread blocking; bounded memory/document size; measured keystroke-to-suggestion latency. Ensure providers never execute user's SQL as an autocomplete side effect and never log credentials. Retain upstream license/NOTICE material for any incorporated code.

### PostgreSQL function output names (2026-11-01)

Local derived-table and CTE projection inference now retains the output name of
an unaliased PostgreSQL function call (for example COUNT(*) produces count).
Schema qualification, window calls, unquoted-name folding, quoted function names,
explicit output aliases and partial CTE column renames are covered. Other dialects
keep their existing behavior because their expression labels differ.

Reference: https://www.postgresql.org/docs/15/queries-select-lists.html
The live PostgreSQL fixture test prepares five queries and compares inferred names
to actual server column metadata, without executing mutations. It passes. All 27
ordinary completion tests and strict all-target Clippy pass. This does not fill
remaining unnamed-expression, JOIN USING/NATURAL wildcard or quoted-scope gaps.

### VALUES source completion (2026-11-01)

The scope scanner now recognizes VALUES query blocks, and PostgreSQL/SQLite
projection inference supplies column1, column2, etc., bounded to 1024 columns.
CTEs, derived sources, explicit aliases and a VALUES-leading UNION retain their
visible output names. PostgreSQL partial column renames preserve trailing names.
MySQL/MariaDB generated VALUES labels are not inferred by this change.

The PostgreSQL rule is documented at
https://www.postgresql.org/docs/18/queries-values.html. Actual prepared-statement
column metadata was checked on the local PostgreSQL fixture and in-memory SQLite.
All 29 ordinary completion tests pass; the live PostgreSQL check also passes.
