# SQL language servers

Rustable's `sql_language_server` worker runs real language servers over isolated
stdio JSON-RPC, off the UI thread. It clears inherited environment variables,
keeps private configuration until process exit, coalesces document submissions,
limits SQL documents to 1 MiB and protocol frames to 2 MiB, and kills/reaps the
child on worker drop. LSP UTF-16 positions are independent of egui character
indices. UI callers must reject events whose document ID/version no longer match.
Each submission uses a new URI; late diagnostics cannot overwrite newer documents.

## PostgreSQL

`tools/postgres-language-server` provides private direct-stdio entry points to the
unmodified upstream [Postgres Language Server](https://github.com/supabase-community/postgres-language-server)
Rust library, pinned to tag **0.28.1**. Its lockfile pins transitive dependencies.
We do not use upstream `lsp-proxy`: that command uses a shared per-user daemon
socket on macOS, which could reuse another editor's configuration.

The diagnostic provider sets `db.disableConnection: true`, disables typechecking,
and passes no connection credentials. It supplies upstream parser/lint diagnostics.
A second private process uses `--completion-catalog PATH` to call upstream
`pgls_completions` against Rustable's already-loaded schema snapshot. The native
LSP has no catalog-injection endpoint, so this completion-only mode adapts its
Rust API to standard LSP requests. It does not create a database pool or run SQL.

The catalog includes table/view names and column names/types/nullability, excluding
DDL, defaults, row values and credentials. It is recreated on connection/schema
invalidation. Completion covers the loaded schema, not every schema on the server.
The offline adapter also resolves qualified CTE output columns and aliases,
chained CTEs, star projections, derived tables and nested alias shadowing using
the upstream parsed tree. Quoted case, escaped quotes and Unicode identifiers
preserve original text and UTF-16 edit ranges. The editor accepts whole-token
quoted edits and rejects unsafe partial replacements. Previously ignored
CTE/quoted-name helper regressions now pass, as does an actual subprocess test
that applies the resulting edits through the application's completion model.

This remains partial semantic coverage: recursive
CTE forward references and SEARCH/CYCLE outputs, lateral table functions and mixed join trees, anonymous expression
names, ambiguous search paths and unterminated quoted identifiers need more work.
Server-backed typechecking, function catalogs and in-document DDL simulation also
remain incomplete. Upstream typechecking uses prepared statement analysis; it
is disabled here, not connected to the user's database. See the helper README
for scope/output limits and fallback behavior.

Build with Cargo and the mise-managed Rust toolchain. Upstream's grammar build
requires tree-sitter CLI and Node; the commands below use an already installed
mise Node version. No npm or Homebrew packages are needed.

```sh
mise exec rust@stable -- cargo install tree-sitter-cli --version 0.25.9 --locked --root target/language-server
PATH="$PWD/target/language-server/bin:$PATH" mise exec rust@stable node@22.23.2 -- cargo build --locked --manifest-path tools/postgres-language-server/Cargo.toml --release --target-dir target/pg-language-server
mise exec rust@stable -- cargo test sql_language_server::live_postgres::disconnected_parser_diagnostics -- --ignored --nocapture
```

The app discovers a sibling `rustable-postgres-language-server` executable, the
workspace build location in debug builds, or an explicit `RUSTABLE_POSTGRES_LSP` override. Package
the sibling executable with distribution builds; developer build paths are not a
distribution strategy.

The real-process fixtures verify initialization, actual catalog column completion
and its applied TextEdit, plus upstream syntax diagnostics without database access.
Run both with `cargo test --locked sql_language_server::live_postgres -- --ignored`.

## SQLite

The [syntaqlite](https://github.com/LalitMaganti/syntaqlite) **0.13.3** provider
sends a bounded, quoted DDL snapshot through `syntaqlite/setSessionContext`.
It needs no SQLite file access or database connection. Real-process tests cover
schema-aware completion and unknown-column diagnostics. Alias-qualified
completion has upstream limitations; preserve local completion when necessary.

```sh
mise exec rust@stable -- cargo install syntaqlite-cli --version 0.13.3 --locked --root target/lsp-tools --no-default-features --features bundled-sqlite-dialect
```

Worker events preserve raw LSP items, including text edits and diagnostic ranges.
The editor adapter must respect edit ranges, reject unsupported snippets and
additional edits, and gate events against current cursor/document state.

## Reproducible app bundling

The Rust helper in `tools/package` owns packaging; it never starts Rustable or connects to a database.

```sh
# Install/build only missing pinned servers; reuse existing binaries.
mise run build-lsps
# Build the app, require both servers, and include dependency notices.
mise run bundle
```

`bundle` deliberately does not silently install missing tools or omit a language server. If a required binary is absent, it fails with the `build-lsps` instruction before modifying the destination app. SQLite must report version 0.13.3. PostgreSQL comes from the locked private wrapper build at `target/pg-language-server/release/rustable-postgres-language-server`.

For an already-built QA app and a QA-specific plist, the same helper supports explicit inputs:

```sh
mise exec rust@stable -- cargo run --locked --manifest-path tools/package/Cargo.toml -- bundle PATH_TO_APP_BINARY PATH_TO_OUTPUT_APP PATH_TO_INFO_PLIST
```

Both servers are placed beside the executable in `Contents/MacOS`, matching runtime discovery. `Contents/Resources/Licenses` contains verbatim upstream license texts, provenance, and a generated inventory/notice directory based on the locked Cargo manifests for the app and both servers. `mise run licenses` generates that inventory independently in `target/bundle-licenses`.

The helper's own tests and lint can be run without building the app:

```sh
mise exec rust@stable -- cargo test --locked --manifest-path tools/package/Cargo.toml
mise exec rust@stable -- cargo clippy --locked --manifest-path tools/package/Cargo.toml --all-targets -- -D warnings
```

This helper creates macOS bundle contents. Signing, notarization, release installation, and native QA remain separate release steps.

Executable updates use a temporary sibling followed by atomic rename, preserving the inode of a running process. Passing the existing destination plist as its own source is safe.

Unqualified expression completion resolves columns from the current FROM scope, including CTE and derived-table outputs. Shared names offer alias-qualified edits; nested query sources remain isolated. Unknown/no-match expressions retain upstream keyword/function fallback.

Qualified outer aliases now resolve through expression subqueries (including nested EXISTS and scalar SELECTs), with nearest-alias shadowing and per-scope CTE bindings. This resolver does not cross FROM-derived-table or CTE-body boundaries; Unqualified correlated columns and explicit/comma-form LATERAL subqueries are now supported; table-function and mixed/nested join-tree visibility remain incomplete. Scope rules reference the [PostgreSQL table-expression documentation](https://www.postgresql.org/docs/17/queries-table-expressions.html).

Unqualified correlated columns resolve outward through expression subqueries, with nearest-scope column shadowing and qualified edits for ambiguous outer names. FROM-derived tables and CTE bodies remain visibility boundaries.

Recursive CTE self-reference uses explicit output columns or names projected by
the anchor SELECT. UNION/INTERSECT/EXCEPT branches use their own FROM bindings,
including correlated expression subqueries. Self-reference is not introduced
inside the non-recursive anchor or ordinary WITH bodies. Helper tests cover
explicit/inferred names, VALUES with declared columns, catalog-name shadowing,
branch isolation and nested expressions. The real LanguageSession test accepts
a quoted recursive output suggestion and applies the complete quoted token.
Forward references, SEARCH/CYCLE generated columns, complex parenthesized set operations are still unverified.

Relation column-alias lists now rename leading columns for catalog tables and
derived SELECT/VALUES tables, preserving undeclared names and not inventing
columns beyond known arity. VALUES output defaults to column1, column2, etc.;
comments do not contribute columns. Star projections through CTEs inherit renamed
outputs. Quoted VALUES aliases pass the real editor completion/application test.
Explicit CROSS/INNER/LEFT LATERAL JOIN column alias lists now resolve, including
quoted and partial lists and VALUES outputs. The parser adapter preserves byte
offsets and preceding-source visibility. Table-function output types remain a gap.

Native QA acceptance: quoted VALUES column alias popup displays the full name;
Tab replaces the quoted token and Cmd+Enter returns `hello`. Recursive CTE
self-reference suggests `step`, accepts with Tab and returns 1,2,3. Comma-form
LATERAL resolves the preceding VALUES alias `Display Name`, accepts with correct
quoting and returns `hello`. Safe mode remained enabled; no database objects were
created. Captures: postgres-values-alias-completion/result.png,
postgres-recursive-completion/result.png and postgres-lateral-completion/result.png
under docs/screenshots (each slash denotes the two separate suffixes).
The initial recursive popup showed stale keywords from the previous caret context.
Caret navigation now clears candidates and replacement edits before new server
results are merged. The regression covers stale keyword removal and preserves
same-position completion. Native follow-up shows only `step`, with successful Tab
insertion and the 1,2,3 result (postgres-recursive-completion-clean.png and
postgres-recursive-completion-clean-result.png). These checks do not establish
full semantic parity or overall completion latency.

Native explicit LATERAL alias-list acceptance also passed in the packaged QA app:
`LEFT JOIN LATERAL (SELECT c.name) AS sub("Display Name") ON true` resolves the
quoted outer reference, displays its full label, accepts with Tab and executes
with Cmd+Enter to return `hello` under `Display Name`. Safe mode stayed enabled.
Captures: postgres-lateral-alias-list-completion.png and
postgres-lateral-alias-list-result.png under docs/screenshots.
