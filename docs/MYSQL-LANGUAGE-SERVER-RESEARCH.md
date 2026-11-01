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

### Live transport foundation (2026-11-01)

Added native-only mysql 28.0.4 with minimal-rust and rustls-tls-ring. Official
API reference: https://docs.rs/mysql/28.0.4/mysql/. Separate loopback-only,
tmpfs-backed Docker fixtures now run MySQL 8.4 and MariaDB 11.4; existing staging
containers are untouched. The ignored live test passed separately on MySQL
8.4.11 and MariaDB 11.4.13-MariaDB-ubu2404. See tools/database-fixtures/README.md
for reproduction and the explicitly limited verification scope.

Driver integration still needs authoritative transaction-state handling.
mysql 28.0.4 keeps ConnInner.status_flags private, although its no_backslash_escape
accessor reads a protocol flag. SQL-prefix guesses cannot accurately track
implicit commits or errors. Investigate a minimal accessor patch/upstream API
before wiring connection replacement; cancellation and connection-loss semantics
also need live tests. Do not label these engines officially supported yet.

The transaction-status API gap is now addressed by a minimal vendored mysql
patch (documented in vendor/README.md). The first live attempt revealed that
upstream ping discards its OK packet; it now parses the status. A validity flag
avoids stale status after sending a command or receiving a server error. Live
checks cover ordinary errors inside transactions, implicit DDL commits and a
killed connection; integration into the Rustable driver remains outstanding.

### Local nested-query completion (2026-11-01)

MySQL/MariaDB completion now resolves the caret's parenthesized query block
before its enclosing blocks. Inner aliases shadow outer aliases; aliases in
child or sibling subqueries do not leak into the caret's suggestions. Correlated
outer references remain available, including while the inner query is missing
its closing parenthesis. Token quote information is preserved without SQL
string reconstruction, including escaped identifier delimiters.

Verification: 20 completion tests passed, covering shadowing, sibling isolation,
correlation, incomplete subqueries, and the earlier quoted-alias cases. The live
catalog/completion/execution regression passed against both fixture engines;
strict all-target Clippy passed. Full CTE/derived-table projected-column inference,
set-operation result-column inference, and semantic legality of every correlation remain outstanding.
This is local cached-catalog completion, not a completed language server.

### Set-operation completion isolation (2026-11-01)

Local MySQL/MariaDB completion now limits relation lookup to the caret's SELECT
arm for UNION (including ALL), INTERSECT and EXCEPT. Splitting occurs only at the
current query block's top level; nested operators and quoted operator names are
not separators. Correlation lookup in ancestor blocks follows that ancestor's
active arm, preventing unrelated arms from contributing outer aliases.

Verification: 21 completion tests passed, including both sides of each operator,
reused aliases, absent aliases, nested operators and ancestor-arm isolation.
The complete native library suite passed: 416 tests, 73 explicitly ignored
fixture/integration tests. Strict all-target Clippy passed. These are completion
scope checks, not a claim that every server version accepts every set operator.
CTE/derived-table output inference and global set-result ORDER BY completion
remain open semantic work.

### Large-catalog candidate construction (2026-11-01)

Candidate enumeration now borrows catalog names and builds quoted replacement
SQL only for accepted suggestions, stopping at the existing 12-item limit.
Previously every table/schema name and table replacement was allocated before
filtering. Candidate ordering, quoting and duplicate handling are preserved.

Reproducible manual timing: `mise exec -- cargo test --lib
completion::tests::large_catalog_completion_timing -- --ignored --nocapture`
(on one shell line). Synthetic catalog: 20,000 schema-qualified tables with
three columns each, plus the referenced customers table; 50 refreshes per input.
Local debug-build median timings before/after:

| Input | Before | After |
| --- | ---: | ---: |
| `SELECT * FROM ta` | 11.97 ms | 1.81 ms |
| `SELECT c.cu FROM customers c` (caret after cu) | 2.07 ms | 2.01 ms |
| `SELECT nonexistent` | 11.66 ms | 2.32 ms |

These are local debug CPU measurements, not release-build or end-to-end UI
latency guarantees. The manual test checks the candidate cap and expected
qualified-column output but intentionally has no machine-dependent timing
threshold. All 21 ordinary completion tests and strict all-target Clippy passed.

### Qualified completion alias correction (2026-11-01)

Local completion now resolves a column qualifier only against the visible relation
alias (including its implicit table-name alias). Previously it also compared the
catalog table name, so `customers AS orders JOIN orders AS o` caused `orders.`
to suggest columns from both tables. Explicit aliases now hide the original table
name; an inner alias matching an outer table's name does not leak outer columns.
Regression coverage includes SQLite, PostgreSQL and MySQL alias/table collisions,
unaliased relations and MySQL nested scopes. All 22 completion tests passed
(one manual benchmark ignored); strict all-target Clippy passed.

### Shared PostgreSQL/SQLite local scope resolution (2026-11-01)

PostgreSQL and SQLite local completion now use their sqlparser dialect tokenizers
and the same nested-query/set-operation scope resolver as MySQL. The previous
fallback scanned all relations in a statement and discarded quoted-identifier
identity, allowing inner aliases to leak into outer completions and treating
quoted keyword aliases as syntax. The custom generic token scanner was removed.

Coverage verifies outer/inner alias isolation, correlated references, incomplete
subqueries, UNION branch isolation, quoted keyword/punctuation aliases and
PostgreSQL dollar-quoted strings containing fake FROM clauses. All 23 completion
tests passed (one manual timing benchmark ignored); strict all-target Clippy
passed. Derived-table/CTE output inference remains outstanding; this does not
replace the PostgreSQL language server's deeper semantic analysis.

### Derived-table output completion (2026-11-01)

Local completion now resolves projected columns from complete derived queries in
FROM/JOIN, using the dialect-aware parser. Explicit expression aliases, bare and
qualified column projections, catalog wildcard expansion, nested derived tables,
explicit derived-column aliases, and the first UNION arm's output names are
covered. Parent/sibling query isolation remains intact; a derived table's own
output alias is not visible inside its definition. The same implementation runs
for PostgreSQL, SQLite, and MySQL/MariaDB SQL modes without database requests.

Inference is bounded to 64 KiB per parsed derived query, depth 16, 128 source
relations, and 1024 output candidates (the popup still retains at most 12).
Unknown expression-generated names are omitted. WITH wildcard inference is
conservative to avoid mistaking a CTE for a similarly named catalog table.
Full CTE binding/output inference, partial column-alias list trailing columns,
recursive CTEs, and incomplete derived-query recovery remain outstanding.

Validation: 24 completion tests pass (one manual benchmark ignored), including
seven derived-table scenarios across all three dialects. Strict all-target
Clippy passes. No native release or website deployment is included in this change.

### CTE output completion (2026-11-01)

WITH bindings now participate in local column suggestions across the three SQL
dialects. Chained CTE wildcard projections resolve against preceding definitions;
a CTE shadows the same-named catalog table. Nested WITH declarations override
outer declarations only within their query block. Derived-table wildcard
inference can use visible CTEs, including WITH clauses inside complete derived
queries. Recursive CTEs use declared output columns or the nonrecursive UNION
anchor's projected names, even when the recursive term is unfinished at the caret.
Non-recursive forward references are not inferred.

The environment is bounded to 128 bindings per scope; existing projection depth,
source-count, output-count and parsed-text limits remain in effect. This extends
rather than completes SQL semantic analysis: PostgreSQL recursive forward
references, SEARCH/CYCLE output columns, full quoted-identifier case semantics,
and recovery from arbitrary incomplete declarations remain outstanding. Current
inference also omits engine-generated names for unaliased expressions.

Validation: nine CTE visibility/output scenarios across PostgreSQL, SQLite and
MySQL dialects; all 25 completion tests pass (one manual benchmark ignored).
Strict all-target Clippy passes. Native UI acceptance of these new suggestions
and browser rebuild/deployment remain pending; no release was triggered.

### Completion regression and latency check (2026-11-01)

The full library suite after CTE inference passed: 432 tests, 87 skipped live or
manual tests. Local completion now delays derived-output parsing until the
relation scanner actually identifies a FROM/JOIN source. Scalar subqueries and
WITH declarations no longer undergo redundant derived-table inference; CTE
bindings are still resolved normally. Shadowed outer aliases also skip inference.

Expanded the manual 20,000-table benchmark to include CTEs, derived wildcards and
64 chained CTEs. Debug-build measurements (50 samples per case) after the change:

| Case | Median | p95 |
| --- | ---: | ---: |
| Table prefix | 1.888 ms | 1.943 ms |
| Qualified catalog column | 2.084 ms | 2.169 ms |
| No matching prefix | 2.384 ms | 2.480 ms |
| CTE column | 2.240 ms | 2.280 ms |
| Derived wildcard column | 2.289 ms | 2.399 ms |
| 64 chained CTEs | 5.135 ms | 6.273 ms |

These are local completion timings, not total frame or server latency, and are
not a controlled before/after benchmark. The completion regression suite still
passes all 25 tests; the manual benchmark asserts expected suggestions as well
as recording timings. Strict all-target Clippy passes.

### PostgreSQL partial output aliases (2026-11-01)

Local projection inference now preserves unnamed-expression positions internally
and applies PostgreSQL relation/CTE column aliases as a positional prefix. Trailing
named columns remain available instead of disappearing. Empty internal names never
become suggestions. Added completion coverage for direct/nested CTEs, derived
sources, recursive anchors and an unnamed expression between two named columns.
SQLite/MySQL explicit alias lists retain their existing whole-list behavior.
All 26 completion tests passed (one manual timing test ignored). Before this change,
the full library suite passed 432 tests with 90 fixture/manual tests ignored.

### Candidate completion acceptance probe (2026-11-01)

Ran the repository-owned observation harness
`tools/language-evaluation/mysql_candidate.rs` against the unchanged lsp_sqls
revision `5a3c53065701cf74e334d8b70ba548f6657ad4e0` (only the probe was added to its
checkout). Injected a fictional shop.customers catalog with id, name and 日本語;
no database connection or user SQL execution occurred.

| Input context | Observed completion behavior |
| --- | --- |
| `SELECT c. FROM customers c` | Correct catalog columns |
| Backtick-quoted alias | Correct catalog columns |
| Derived `SELECT name AS label` projected as `d.` | No suggestions; label missing |
| CTE `SELECT name AS label` projected as `c.` | No suggestions; label missing |
| Unicode column prefix with UTF-16 caret | 日本語 present, alongside unfiltered columns |
| Inside a string | Suppressed |
| Inside a block comment | Incorrectly suggests columns and SQL keywords |
| Double-quoted alias | Columns present, without an ANSI_QUOTES mode input |

The parser returned no diagnostics for ordinary SELECT, MariaDB INSERT RETURNING,
a backslash-sensitive literal, or a double-quoted expression. Accepting those
strings without errors is not evidence of mode-correct semantics. The inspected
MysqlDialect completion/parse API takes SQL and Schema, with no SQL-mode input.
The other candidate, duck-sqllsp, selects a fixed sqlparser MySqlDialect in
`dsl-parse/src/backend/sqlparser/dialect_pick.rs`, likewise without session mode.

Decision: do not replace Rustable's mode-aware cached-catalog completion with
lsp_sqls at this revision. It would lose existing CTE/derived completion and
comment suppression. No rewrite from another language is justified by this
probe; Rust candidates exist but need integration/adaptation work. Duck's full
completion acceptance and MariaDB-specific semantic behavior remain unverified.
The harness completed successfully; it intentionally reports capability gaps
rather than asserting a passing acceptance suite. Candidate compilation also
emitted a bundled C grammar signedness warning. No dependency was added.

### Second candidate completion probe (2026-11-01)

Ran `tools/language-evaluation/duck_mysql_candidate.rs` against duck-sqllsp
revision `76a1a75dbb602820e89e339a7617049ea234bb0d`, through its public `complete`
function (which also infers the document's derived catalog), MySql parser and
source-aware resolver. Same fictional catalog and eight caret contexts as the
lsp_sqls probe; no server access.

- Plain alias: exactly the three catalog columns.
- Backtick-quoted alias: 788 candidates, including the correct columns but also
  unrelated PostgreSQL functions such as pg_try_advisory_lock and pg_get_acl.
- Derived-table alias: no candidates; the projected label is missing.
- CTE alias: the projected label is correctly suggested.
- Unicode prefix: 日本語 present alongside the other two columns.
- Inside string and block comment: no candidates, correctly suppressed.
- Double-quoted alias: columns returned, with no session SQL-mode distinction.

The locked build succeeded, including transitive pg_query compilation. This is
not proof of a pure-Rust or WASM-compatible completion dependency configuration.
The probe itself passed because it records observations, not acceptance claims.

Neither inspected candidate is a drop-in improvement for Rustable's existing
MySQL/MariaDB completion. Retain the current mode-aware implementation while
filling its documented gaps. Reconsider a reusable library or upstream adaptation
with the same acceptance matrix; do not launch a Go/TypeScript rewrite solely
because these two snapshots have gaps. Broader routine semantics, server-version
syntax, and a complete MySQL/MariaDB diagnostics provider remain open work.

### Initial MySQL/MariaDB lexical editor warnings (2026-11-01)

The editor now reuses the execution scanner to underline unfinished quotes,
unfinished block comments and invalid DELIMITER directives. Scanner failures
retain byte ranges internally; the editor converts them to character positions
so earlier Unicode text does not shift the underline. Changes to the document,
caret or current session SQL mode invalidate cached results. Corrected input
clears warnings. No language-server subprocess or database request is started.

These are nonblocking warnings, not full syntax/schema diagnostics. Checks use
the current connection mode; they do not simulate prospective SET sql_mode
statements embedded in an unexecuted script. Documents above 64 KiB skip this
local check and the language-service status explains that limit. The check runs
once per document/cursor version or mode change, not every repaint.

Validation: five SQL scanner tests and three ordinary editor-language tests pass;
one opted-in PostgreSQL helper test was skipped. Added coverage includes UTF-8
source spans, character offsets after emoji, SQL-mode-only changes, cursor moves,
custom routine delimiters, corrected SQL and oversized documents. Strict
all-target Clippy passes. Native visual acceptance is still pending; no release
or deployment was performed.
