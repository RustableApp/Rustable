# Sharing query results

Use **Share…** in the query result toolbar or **Share Results…** in a result
cell's context menu. **Command–Shift–J** opens the dialog on macOS
(**Control–Shift–J** elsewhere). **Command–Enter** copies from the dialog
(**Control–Enter** elsewhere); Escape closes it. The dialog owns these shortcuts,
so copying never executes the editor behind it.

The dialog remembers format (Markdown, plain text, JSON), row scope, query
inclusion and execution details in the app preferences, including across windows
and launches. Selected rows is the initial scope; without a selection it falls
back to the current page. The menu's **Copy Query + Rows** skips the dialog,
reuses the saved format/scope/details, and always includes the query without
changing the dialog's saved query checkbox. Copy confirmation appears in place.

The source is the exact statement attached to the result by the database adapter,
not the current editor contents. Pinned results and cached table pages retain
that source. Table queries inline captured parameter values as dialect-escaped SQL literals in both the preview and clipboard output. Quoted text and comments are preserved. Query/result preparation never reruns SQL or fetches more
rows from the database. Stored desktop result values are read locally in a
background task, including values shortened in the grid preview.

Partial selections, truncated results and included unsaved cell edits are labeled
in the copied text. New unsaved rows are not query results and must be saved
before sharing. Plain text includes a header-bearing tab-separated table;
Markdown includes a SQL code block and a table; JSON includes the query, a
column-name list, row arrays and scope metadata. Row arrays preserve duplicate
column names from joins. Binary cells are byte arrays; non-finite numbers use
an explicit `non_finite_number` object. Execution details currently include elapsed milliseconds.
Connection addresses and credentials are not added to the payload.

Desktop preparation uses one background task per open dialog. Sharing is bounded to
4 MiB of row data and 32 MiB of formatted output, with a 1 MiB query/parameter
limit; use Export for larger selections. The preview shows at most 12 KiB and
labels shortening; copying retains the complete prepared payload. Browser
sharing is limited to 256 rows / 64 KiB to bound synchronous browser work.
Settings can change while preparation is running; obsolete output is discarded.
Closing the dialog cancels pending local result reads.

## Verification

Core release tests: 888 passed, 240 opt-in tests ignored. Final menu adjustments
were then checked with the full debug suite and native context-menu capture. The targeted share tests cover
formatting, selected/current-page scope, bindings, Markdown
fences, empty/truncated results, stored values, staged-edit disclosure, saved
preferences, stale-preview replacement and Command–Enter ownership.
Strict all-targets QA clippy and the locked browser build also pass (the browser
build retains existing warnings). Native dark/wide and light/narrow captures are
under `target/share-design-review/`.

Query provenance is attached after result collection, and browse provenance is
copied from the SQL/bindings already returned in `TableData`. Inspection of the
SQLite, PostgreSQL and MySQL/MariaDB paths found no new driver calls, health
checks, preparations, cleanup waits or catalog lookups. Added sequential network
exchanges are **zero on both cold and warm paths**. PostgreSQL established editor
statements and validated sidebar loads retain their one-exchange budgets;
existing schema/type/transaction fallbacks are unchanged. Sharing itself makes
zero remote requests and reads stored desktop rows locally.

All five PostgreSQL latency tests passed against the disposable loopback fixture
at port 54329 with 30 ms simulated RTT; the fixture was removed afterward. Editor
p95 was 47.0 ms (single-exchange reference p50 38.0 ms); browse/refresh p95 was
47.8 ms; complete 6,000-row spooled results p95 was 55.1 ms versus reference
44.9 ms, within the existing threshold. These are regression checks against the
suite's protocol references, not a separate pre-change latency A/B measurement.
MySQL/MariaDB provenance attachment was inspected and compiled; their live
integration suites were not rerun for this change. Full logs remain in
`target/share-design-review/latency.log`.

See `docs/UI-PERFORMANCE.md` for the five-pair release UI comparison, new dialog
measurements, raw artifact locations and native measurement limitations.
