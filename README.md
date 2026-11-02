<p><img src="assets/rustable-mark.svg" width="80" alt="Rustable icon"></p>

# Rustable

A native database client for **PostgreSQL, MySQL, MariaDB, SQLite, and Redis**. Browse your data, write queries, and review changes before saving—all in a fast, focused workspace.

[Try the live demo](https://rustable.app/#demo-title) · [Website](https://rustable.app) · [Latest release](https://github.com/RustableApp/Rustable/releases/latest)

## Download

- [macOS · Apple Silicon](https://github.com/RustableApp/Rustable/releases/latest/download/Rustable-macos-arm64.dmg)
- [Windows · x64](https://github.com/RustableApp/Rustable/releases/latest/download/Rustable-windows-x86_64.zip)
- [Linux · x64](https://github.com/RustableApp/Rustable/releases/latest/download/Rustable-linux-x86_64.tar.gz)

macOS builds are signed and notarized, with automatic updates. Windows and Linux use portable packages and manual updates. Linux requires Ubuntu 24.04 or a compatible newer system.

## A workspace for your databases

- **Keep connections organized.** Groups, environment colors, separate windows, and imports from TablePlus and TablePro.
- **Find what you need.** Ranked fuzzy search, table filters, schema browsing, and clickable foreign-key relationships.
- **Work with SQL.** Multiple editor tabs, completion, query history, saved queries, and result exports.
- **Edit with confidence.** Stage table changes, undo mistakes, and review the SQL before applying them.
- **Browse Redis.** Explore keys and values, run commands, and manage expiration times.
- **Connect securely.** System credential storage, TLS, and SSH tunnels on macOS and Linux.

Built with Rust and egui. You already have a browser; your database client doesn’t need to ship another one.

## Try it without installing

The [browser demo](https://rustable.app/#demo-title) includes disposable SQLite and Redis sample data. Explore, query, and edit without connecting to your own databases. Use the desktop app for real server connections.

## Good to know

Rustable is actively developed and still has rough edges. Support varies by database; see the [driver capabilities](docs/DRIVER_CAPABILITIES.md), [MySQL/MariaDB notes](docs/MYSQL-MARIADB-RELEASE.md), and [Redis guide](docs/REDIS.md).

Exports include loaded results, not automatically the entire database. On Windows, SSH connections require an external tunnel.

## Feedback and contributing

[Report a bug or request a feature](https://github.com/RustableApp/Rustable/issues). Include your OS, database engine, and steps to reproduce the issue.

Want to build or contribute? See [development setup](docs/DEVELOPMENT.md) and [performance notes](docs/PERFORMANCE.md).

[MIT licensed](LICENSE). Same SQL. Less ceremony.
