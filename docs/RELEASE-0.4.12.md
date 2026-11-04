# Rustable v0.4.12

- Make PostgreSQL queries and table loads faster over SSH and other remote connections by removing unnecessary network round trips.
- Keep table refreshes fast while validating schema changes and safely handling custom column types.
- Use cleaner `SELECT *` queries for ordinary PostgreSQL, MySQL and MariaDB table browsing, retaining explicit columns where needed for correct results.
- Move the SQL Run button to the right end of the toolbar and show a spinner inside it while a query is running.
- Show release notes in the macOS automatic-update prompt.

The website demo is built from the same release tag.
