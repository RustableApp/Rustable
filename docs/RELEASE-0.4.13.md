# Rustable v0.4.13

- Speed up the first PostgreSQL table load by fetching rows and editing metadata in one network exchange.
- Display custom PostgreSQL column types without extra conversion lookups, while preserving exact decimals, binary values and NULLs.
- Load index and trigger definitions only when their views are opened.
- Make the PostgreSQL sidebar catalog faster over remote and SSH connections.
- Adjust the right-hand field editor caret to fit its text more closely.

The website demo is built from the same release tag.
