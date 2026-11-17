# Read-only MCP server

Rustable can run as a [Model Context Protocol](https://modelcontextprotocol.io)
server so that AI assistants such as Claude can explore and query your saved
connections. The server is **read-only**: it cannot insert, update, delete, change
schemas, or start transactions.

```sh
/Applications/Rustable.app/Contents/MacOS/rustable mcp
```

The server talks MCP over stdin/stdout and never opens a window. It uses the
SQLite, PostgreSQL, MySQL and MariaDB connections saved in the desktop app. Redis
connections are not exposed.

## Set up

**Claude Code**

```sh
claude mcp add rustable -- /Applications/Rustable.app/Contents/MacOS/rustable mcp
```

**Claude Desktop** (`claude_desktop_config.json`) and other MCP clients:

```json
{
  "mcpServers": {
    "rustable": {
      "command": "/Applications/Rustable.app/Contents/MacOS/rustable",
      "args": ["mcp"]
    }
  }
}
```

On Linux and Windows, use the path to the `rustable` executable instead.

### Options

| Option | Effect |
| --- | --- |
| `--connection NAME` | Expose only this saved connection (name or id). Repeat it to allow several. By default every supported connection is available. |
| `--timeout SECONDS` | Cancel a tool call that runs longer than this. Default 60, maximum 3600. |
| `--settings PATH` | Read saved connections from a different `app.ron`. Use this for testing. |

To keep production data away from an assistant, list only the connections it
should see:

```sh
claude mcp add rustable -- /Applications/Rustable.app/Contents/MacOS/rustable mcp \
  --connection "Local dev" --connection "Staging replica"
```

## Tools

| Tool | What it does |
| --- | --- |
| `list_connections` | Lists saved SQL connections: name, id, engine, environment, group and address. Never returns passwords. |
| `list_tables` | Lists tables and views. PostgreSQL shows one schema and MySQL/MariaDB one database at a time; pass `schema` to switch. Accepts an optional `name_filter`. |
| `describe_table` | Lists a table's columns: type, nullability, default, primary key and generated columns. Pass `include_definition` to also get the `CREATE` statement. |
| `read_table` | Reads one page of rows (`limit` up to 500, `offset`, `order_by`, `descending`). Only the requested page is fetched. |
| `query` | Runs read-only SQL: `SELECT`, `WITH`, `VALUES`, `SHOW` and `EXPLAIN`, with several statements allowed. Each result set is cut to `max_rows` (up to 1000). |

Each response is limited to about 256 KiB, column names included. Long text
values and column names are shortened to 2,000 characters, and binary values are
summarized. `truncated: true` means rows were left out of the response. A
response that still would not fit (for example a very long view definition)
returns an error asking for a narrower request instead.

## How read-only is enforced

The server uses the same Safe mode as the desktop editor. The database server
enforces it, not just Rustable's statement parsing. This covers `read_table` as
well as `query`, so a view that calls a data-modifying function cannot write:

- **PostgreSQL:** each statement and each table page runs in an implicit
  transaction with `SET LOCAL transaction_read_only = on`, and the session
  starts with `default_transaction_read_only`. Write statements, data-modifying
  CTEs, `nextval()` and attempts to turn read-only off are rejected.
- **MySQL/MariaDB:** the session is opened with `SET SESSION TRANSACTION READ
  ONLY`. Statements are also checked, then wrapped in
  `START TRANSACTION READ ONLY … ROLLBACK`. `SELECT … INTO`, executable comments
  and session changes (including any `SET`) are rejected.
- **SQLite:** the file is opened with `SQLITE_OPEN_READ_ONLY`, `query_only` is
  on, and `ATTACH` is disabled so a session cannot reach other files.

There is no option to turn Safe mode off for MCP. For the strongest guarantee on
server databases, also save a connection that uses a database role with
read-only privileges.

## Credentials and SSH

- Passwords come from the system credential store, where Rustable saves them
  when you connect with the password remembered. If no password is saved, the
  server connects without one and reports how to fix any authentication error.
  macOS may ask you to allow access to the keychain item.
- SSH tunnels use your OpenSSH configuration as in the desktop app. The server
  cannot show a passphrase prompt, so load passphrase-protected keys into
  `ssh-agent` first. Otherwise the connection fails immediately instead of
  waiting.
- Debug builds (`cargo run -- mcp`) read the connections and credentials of
  **Rustable Development**, not those of the installed app, just as the desktop
  debug build does.

## Performance notes

- Connections open on first use and are reused across calls (up to four at a
  time). A connection reopens automatically if you edit its saved settings. A
  lost connection reconnects on the next call, and a call is never replayed.
- `query` uses the editor's execution path: one network exchange per statement
  on PostgreSQL. `read_table` uses the table browser's pipelined page load.
  Catalogs are cached per connection, so `describe_table` usually needs no
  network exchange (pass `refresh` to `list_tables` after schema changes).
- `query` reads the complete result on the server side, as the editor does.
  Large results spill to a temporary file before being cut to `max_rows`. Add
  `LIMIT` to large queries, or use `read_table`, which fetches only one page.
- Calls run one at a time. Up to 32 more can wait; further calls fail at once
  with an error instead of stalling the server.
- Clients can cancel a running or waiting call with `notifications/cancelled`. Calls that
  run longer than `--timeout` are cancelled with the driver's existing
  cancellation path.
