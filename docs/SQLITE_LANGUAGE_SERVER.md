# SQLite language-server provider

Research and local fixture verification: 2026-10-31.

## Choice

Use [syntaqlite](https://github.com/LalitMaganti/syntaqlite), pinned to **0.13.3**. Its analyzer and actual LSP server are Rust; its parser/tokenizer derive from SQLite's C grammar. Apache-2.0, with SQLite-derived parts public domain. It remains a 0.x project, so pin its protocol and re-run fixtures before upgrades.

Official sources:

- [Installation](https://docs.syntaqlite.com/latest/getting-started/cli/): supports `cargo install syntaqlite-cli`.
- [LSP transport](https://docs.syntaqlite.com/latest/getting-started/other-editors/): `syntaqlite lsp`, JSON-RPC over stdin/stdout.
- [Schema completion example](https://docs.syntaqlite.com/latest/getting-started/zed/): table and column completion backed by schema definitions.
- [Schema configuration](https://docs.syntaqlite.com/latest/guides/project-setup/): schema-aware diagnostics without opening a database.
- [Published dispatcher source](https://docs.rs/syntaqlite/0.13.3/src/syntaqlite/lsp/dispatcher.rs.html): `syntaqlite/setSessionContext` accepts `{"ddl":"CREATE TABLE ..."}` and republishes diagnostics. This allows in-memory schema updates without writing metadata files.

[Sqruff](https://playground.quary.dev/docs/usage/editors/) is also Rust and supports a SQLite dialect, but its documented LSP supplies lint diagnostics and formatting, not the schema-aware completion required here. It is not a replacement for this provider.

## Local installation

Use the mise-managed Rust toolchain only:

```sh
mise exec rust@stable -- cargo install syntaqlite-cli --version 0.13.3 --locked --root target/lsp-tools --no-default-features --features bundled-sqlite-dialect
```

The local prototype executable is `target/lsp-tools/bin/syntaqlite`. A shipped app must either bundle this binary with its Apache license or explicitly discover a user-installed binary; a developer's target path is not a distribution strategy.

## Adapter contract

`sqlite_language_server::server_spec(objects, program)` runs on the language-server worker. It returns a generic stdio server specification, with an empty private temporary working directory and config discovery disabled. The adapter passes the SQLite version bundled by rusqlite and sends schema metadata through `syntaqlite/setSessionContext` after initialization.

The snapshot contains only cached table/view names, column names, SQLite type affinities, and nullability. It excludes row values, connection paths, credentials, default literals, and view SQL bodies. View result columns are modeled as a table-shaped catalog entry. It is bounded to 10,000 objects / 2 MiB. Missing column metadata rejects schema setup rather than silently asserting that incomplete metadata is authoritative.

Restart/reconfigure the provider on a new schema revision or connection. Documents and responses must retain the generic worker's document-version identity; outdated results must never change the current editor. The provider never connects to or modifies a database.

## Verified capabilities and limits

A real stdio fixture against 0.13.3 verified initialize, schema injection, document open, completion, schema diagnostics, and shutdown:

- `SELECT na FROM customers;` includes the schema column `name`.
- `SELECT missing_column FROM customers;` reports an unknown-column diagnostic.
- **Known upstream gap:** `SELECT c.na FROM customers AS c;` returns no completion items. Inspection of `completion_service.rs` confirms that qualified completion currently looks up the qualifier directly in the catalog instead of resolving an alias. Keep an explicit fallback for this case; do not describe syntaqlite 0.13.3 as covering all alias/CTE completion.

The existing custom completion engine should not be presented as a language server. Conversely, replacing it outright before covering these upstream gaps would regress working alias completion. Contextual fallbacks should be tested separately from LSP results.
