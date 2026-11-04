# Rustable v0.4.11

- Reduce PostgreSQL query latency, especially over SSH, by combining prepared-statement cleanup with execution.
- Reconnect lost PostgreSQL sessions and SSH tunnels on the next query or reload without closing the workspace. Failed operations are never automatically replayed.
- Fix workspace resize feedback that could cause flashing when connecting external monitors or changing display scaling.
- Notify you on macOS when a query finishes or fails while its window is unfocused, subject to notification permission.

The website demo is built from the same release tag.
