# JSON export

Table Export and SQL-result Export menus offer **Loaded rows as JSON…** alongside CSV. The selected result (including a pinned result) is captured when the dialog opens. Export runs on the existing database/file worker, not the render thread. It exports only loaded rows, not all rows matching an unbounded query or staged GUI edits.

The file is an array of objects, one per row, in result order:

```json
[
  {"id": 42, "label": "Example", "missing": null, "bytes": [0, 255]}
]
```

- NULL becomes JSON null; integers and finite real values become JSON numbers.
- Text remains text, including decimal/timestamp/boolean values delivered as text by a driver. Strings containing JSON are not parsed or transformed.
- Binary values become arrays of byte integers.
- Column names are escaped as JSON object keys. Duplicate column names are rejected with guidance to use query aliases or CSV; they are never silently overwritten.
- Non-finite real values are rejected with guidance to cast to text or use CSV. They are not silently converted to null.
- Integer literals preserve all i64 digits in the file. Consumers must use an appropriate numeric representation for large integers.

Encoding streams through a bounded buffered writer without constructing an entire JSON document in memory. A cancellation-aware writer checks every write and limits each write to 8,192 bytes. Cancellation uses a terminal I/O error (not Interrupted, which write_all retries). Same-directory staging, flush/sync and atomic publication reuse the CSV publication mechanism. Cancellation or encoding failure leaves an existing destination unchanged and removes the temporary file. The worker applies existing live-SQLite-file destination protection.

Validation covers escaped Unicode/control text, NULL, i64 maximum, finite numbers, binary arrays, duplicate headers, non-finite values, cancellation within large text, atomic failure and protected SQLite paths. The original loaded-result implementation passed 352 tests; current verification is recorded in PROGRESS.md. Native table Export→Loaded rows as JSON and the macOS save dialog were verified in isolated QA; the resulting file matched all 500 SQLite fixture rows/columns exactly. Query-result menu native verification remains open. 

## All matching table rows

The table Export menu also offers **All matching rows as JSON…**. It captures the
current table/filter/sort when the destination dialog opens, then streams saved
database values across all pages, ignoring the current page offset. Staged GUI
edits are not included. Arbitrary handwritten SQL export remains limited to the
loaded result; it does not rerun an unbounded query.

SQLite encodes borrowed row buffers before advancing the cursor. PostgreSQL uses
the existing read-only export transaction and server portal in batches of 64 rows.
Neither accumulates the entire table or JSON document. The fixed PostgreSQL batch
bounds row count, not bytes: unusually large individual values and database/client
buffers can still increase memory use. PostgreSQL exports refuse to start inside
an explicit SQL transaction, preserving that transaction.

Full PostgreSQL JSON exports resolve nested domains to their base type. Integer
and floating types become JSON numbers, bytea becomes an array of byte values,
and all other types use PostgreSQL text output. Exact NUMERIC decimals remain
strings, so digits are not rounded; booleans, timestamps, arrays and JSON-valued
columns also remain strings, matching the app's text-valued export convention.
Non-finite floating-point values fail atomically. CSV's existing encodings remain
unchanged.

Regression fixtures compare every row in 6,001-row exports and every ID in 3,000-row
filtered descending exports, despite a 5,000-row page offset. They also cover
empty SQLite output, nested PostgreSQL integer/binary domains, exact decimals,
NULLs, byte arrays, atomic non-finite failures, mid-export PostgreSQL cancellation,
connection recovery, explicit transaction refusal, and protection against
replacing an open SQLite database with full JSON output.


Native full-table JSON QA passed through the menu and macOS save dialog in the
isolated QA app. The resulting 2,400-row file matched every customer row and
column from an independent read-only SQLite query, while the UI had only 500
rows loaded. Screenshot: `screenshots/full-json-export-complete.png`.
The million-row SQLite streaming benchmark and its memory limits are documented
in `PERFORMANCE.md`. Full suite: 353 passed / 38 ignored; both opted-in PostgreSQL
CSV/JSON export regressions passed; strict Clippy passed.
