# Rustable v0.4.26

- Fetch complete desktop SQL query results instead of stopping at 5,000 rows, with large results stored on disk to keep memory bounded.
- Scroll continuously through large results, with rows preloaded above and below the viewport and stable placeholders during cache misses.
- Search and export the complete result, and copy all selected rows in the background.
- Preserve selected-row details while scrolling and jump directly to search matches or the final row.

Disk-backed results are read-only. Copy All supports up to 32 MiB; use CSV or JSON export for larger results. Individual rows over 64 MiB report an explicit error.

The website demo is built from the same release tag. Its browser-specific result preview limits remain in place.
