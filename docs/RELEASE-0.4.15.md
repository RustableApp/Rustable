# Rustable v0.4.15

- Edit eligible SQL query results directly, including partial results containing the primary key. Use Command–S to review and save changes, with dedicated JSON and date/time editors.
- Get smarter SQL completion suggestions based on the current clause, with better fuzzy matching and Unicode handling.
- Toggle SQL comments with Command–/ (Ctrl–/ on Windows and Linux). Comments stay green when selected, and existing comments survive commenting and uncommenting a query.
- Use the new lock control for Safe mode on PostgreSQL, SQLite, MySQL, and MariaDB. Production connections start in Safe mode; potentially destructive SQL requires confirmation when it is off.
- Find sidebar, connections, discard, preview, and save actions in cleaner toolbar groups.
- Open the command palette without triggering database loading. Cancel buttons now show Command–period, and the PostgreSQL cancellation dialog selects the current query with a blue default cancellation action.

The website demo is built from the same release tag.
