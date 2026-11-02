# Rustable v0.4.4

- Cmd+P ranks exact names and prefixes before word-boundary, substring and fuzzy abbreviation matches. Names rank ahead of metadata; ties stay stable. Searching `store` now puts `stores` ahead of `campaign_store_property_values`.
- Existing table tabs restore their cached rows, filters, selection and structure view without querying again. Scroll state is independent for relationship tabs. New tabs and explicit refreshes still load from the database.
- Structure has a cleaner search toolbar and a working Show DDL viewer with scrolling, selectable SQL and copy feedback. MySQL/MariaDB use server-authored definitions; SQLite reads its schema. PostgreSQL reconstructs a structure preview for regular tables and views, including constraints, indexes and triggers. Its preview documents omissions; inherited, partitioned and foreign tables report an explicit limitation.
- NULL values in the field inspector use muted placeholder styling, including multi-row selections. Literal text `NULL` remains ordinary text.

The website WASM demo is built from this same release tag.
