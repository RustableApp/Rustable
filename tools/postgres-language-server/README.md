# Private PostgreSQL language helper

The normal `rustable-postgres-language-server CONFIG_PATH` mode runs the pinned
upstream 0.28.1 native LSP. Rustable supplies disconnected configuration for
parser/lint diagnostics; this helper does not share the upstream per-user daemon.

`rustable-postgres-language-server --completion-catalog PATH` runs a separate,
offline completion-only LSP surface using the same upstream `pgls_completions`
engine and tree-sitter grammar. It never creates an upstream ServerFactory or
database pool. Its input catalog contains metadata, not connection credentials:

```json
{"tables":[{"schema":"public","name":"customers","columns":[
  {"name":"customer_id","data_type":"integer","not_null":true},
  {"name":"display_name","data_type":"text","not_null":false}
]}]}
```

`data_type` and `not_null` are optional. The parent owns this private temporary
file and recreates the helper when its connection/schema catalog changes.
Catalog size is limited to 16 MiB; JSON-RPC frames to 2 MiB; documents to 1 MiB,
with at most eight open documents. The supported stdio methods are initialize,
shutdown/exit, full-text didOpen/didChange/didClose, and completion. Positions and
TextEdits use UTF-16. Statement splitting and contextual suggestions use upstream
implementations; no handwritten SQL parser is involved.

The scoped column adapter in `completion_scope.rs` uses the upstream parsed tree
and lexer to resolve qualified references through table aliases, CTE output
aliases, chained CTEs, star projections and derived-table aliases. Nested query
aliases remain isolated. Quoted names preserve case and escaped quotes, and
Unicode identifier bytes are masked only in the temporary parser input to work
around grammar limitations; the original text supplies names and edit offsets.
The temporary cursor marker supports `alias.` without changing the user's SQL.

Resolution is limited to 16 levels, 64 bindings per query/CTE group and 2,048
projected columns; at most 50 suggestions are emitted. Inherited CTE column lists
share Arc storage. Complete-token edits preserve UTF-16 offsets and replace a
quoted suffix together with its opening/closing quotes. The previous three
ignored CTE/quoted-name regressions now pass, alongside nested-scope, Unicode,
escaped-quote, protocol lifecycle and application subprocess/acceptance tests.

This is not complete SQL semantic analysis. Recursive
CTE forward references and SEARCH/CYCLE outputs, lateral table functions and mixed join trees, anonymous
expression output names, ambiguous search paths, unterminated quoted tokens,
function catalogs, server-backed typechecking and in-document DDL simulation
remain incomplete. Unknown cases fall back to the upstream/local providers;
local fallback suggestions are not a guarantee of valid SQL.

From the repository root:

```sh
PATH="$PWD/target/language-server/bin:$PATH" mise exec rust@stable node@22.23.2 -- cargo test --locked --manifest-path tools/postgres-language-server/Cargo.toml --release --target-dir target/pg-language-server
PATH="$PWD/target/language-server/bin:$PATH" mise exec rust@stable node@22.23.2 -- cargo build --locked --manifest-path tools/postgres-language-server/Cargo.toml --release --target-dir target/pg-language-server
```

Upstream completion can panic at statement boundaries (e.g. cursor immediately
after a semicolon). Each completion request contains unwinds and returns an empty
list on failure, preserving local fallback and subsequent requests. Regression
coverage includes the boundary failure followed by valid alias completion, plus
a real application LanguageSession lifecycle check and native QA.

Unqualified expression completion resolves columns from the current FROM scope, including CTE and derived-table outputs. Shared names offer alias-qualified edits; nested query sources remain isolated. Unknown/no-match expressions retain upstream keyword/function fallback.

Qualified outer aliases now resolve through expression subqueries (including nested EXISTS and scalar SELECTs), with nearest-alias shadowing and per-scope CTE bindings. This resolver does not cross FROM-derived-table or CTE-body boundaries; Unqualified correlated columns and explicit/comma-form LATERAL subqueries are now supported; table-function and mixed/nested join-tree visibility remain incomplete. Scope rules reference the [PostgreSQL table-expression documentation](https://www.postgresql.org/docs/17/queries-table-expressions.html).

Unqualified correlated columns resolve outward through expression subqueries, with nearest-scope column shadowing and qualified edits for ambiguous outer names. FROM-derived tables and CTE bodies remain visibility boundaries.

Relation column-alias lists now rename leading columns for catalog tables and
derived SELECT/VALUES tables, preserving undeclared names and not inventing
columns beyond known arity. VALUES output defaults to column1, column2, etc.;
comments do not contribute columns. Star projections through CTEs inherit renamed
outputs. Quoted VALUES aliases pass the real editor completion/application test.
Explicit CROSS/INNER/LEFT LATERAL JOIN column alias lists now resolve, including
quoted and partial lists and VALUES outputs. The parser adapter preserves byte
offsets and preceding-source visibility. Table-function output types remain a gap.

From the repository root, run `python3 tools/postgres-language-server/benchmark_protocol.py`
after building the release helper to measure process startup, stdio completion
latency and sampled RSS. It generates a temporary 5,000-table catalog, verifies
500 responses, then opens/completes/closes eight near-1-MiB documents over ten
cycles and checks the ninth-document bound. The helper exits and temporary data
is removed even on failure. No database or native app is used. Requires `ps`
with RSS in KiB (macOS/Linux); results are JSON on stdout. This excludes editor
debounce/rendering and is not a whole-app memory or latency measurement.
