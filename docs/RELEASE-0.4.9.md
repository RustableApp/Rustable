# Rustable v0.4.9

- Fix a PostgreSQL result-stream deadlock involving uncached custom column types.
- Disable default PostgreSQL statement and lock timeouts; report worker and cancellation failures clearly.
- Add a PostgreSQL running-query picker for cancellation through the existing SSH tunnel.
- Use ranked fuzzy matching for SQL completion and sidebar/field filtering.
- Align SQL editor line numbers with the text baseline.
- Let Command+A select results after clicking empty grid space.
- Use “Rustable” for connection-manager and database window titles.

The website demo is built from the same release tag.
