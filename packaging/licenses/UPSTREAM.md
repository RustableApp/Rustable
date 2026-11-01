# Bundled language-server provenance

Rustable bundles two separate stdio language-server executables. They run in private workspaces; neither binary is the Rustable application itself.

- **syntaqlite 0.13.3** — https://github.com/LalitMaganti/syntaqlite ; crates.io package `syntaqlite-cli`, Cargo `--locked`, features `bundled-sqlite-dialect` with defaults disabled. Analyzer/server are Rust; parser/tokenizer derive from SQLite. The accompanying `syntaqlite-0.13.3-Apache-2.0-and-SQLite.txt` is copied verbatim from the official published crate's LICENSE, including the SQLite public-domain blessing.
- **Postgres Language Server 0.28.1** — https://github.com/supabase-community/postgres-language-server ; upstream commit `e74c8b821abcf781ad3300ec9a43a58d1b26eabd` and all dependencies are pinned in `tools/postgres-language-server/Cargo.lock`. `rustable-postgres-language-server` is Rustable's small private stdio entry point into the unmodified upstream library. The accompanying MIT license is copied verbatim from the pinned upstream checkout, preserving the copyright notice.

`cargo-dependencies.json` inventories the packages resolved by the locked application and both server manifests, including declared license expressions and upstream source identifiers. The `dependencies/` directory contains available upstream license/copyright/notice files copied from those exact Cargo package roots (or the corresponding upstream Git workspace root). Build-only and non-host-platform dependencies may also appear; inclusion does not imply that every listed package is linked into every executable.

The generated inventory is not a claim that all transitive projects use the same license. Consult each dependency's own included notice and license expression. The top-level application and packaging helper's license declarations do not replace upstream licenses.

Tool installation/build is performed only with Cargo using the mise-managed Rust toolchain. The PostgreSQL grammar build additionally uses an already installed mise-managed Node 22.23.2 and Cargo-installed tree-sitter-cli 0.25.9. No Homebrew, npm installation, shared PostgreSQL language-server daemon, or remote database access is used by this packaging workflow.
