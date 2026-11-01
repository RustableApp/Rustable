# Find in query results

The magnifier beside Export opens a search row for the selected query result.
⌘F opens this search when the result grid or result search field has focus;
when the SQL editor has focus it opens SQL-text search instead.
Search is literal, Unicode case-insensitive by default, with a Match case option.
It searches loaded cell values only, including displayed numbers, NULL and blob
size labels. It neither filters rows nor sends SQL to the server.

The first matching cell is selected and scrolled into view. Enter/Shift+Enter or
the previous/next buttons move through matching cells and wrap at either end.
⌘G / ⇧⌘G navigate next/previous matches while results have focus.
Escape closes the search row and restores keyboard focus to the result grid. Search state is separate for each SQL tab. Changing
the selected result or rerunning SQL invalidates matching coordinates by Arc
identity, preventing matches from an earlier result from selecting new cells.

Scanning runs off the UI thread with one in-flight worker per tab. Input changes
coalesce while a scan runs. The worker borrows strings from an existing Arc<Grid>,
so it does not copy the result payload. At most 100,000 matching coordinates are
retained; a plus sign indicates that more cells matched. Search input is capped
at 1,024 characters. This is cell navigation, not substring highlighting or a
server-side full-table search.

Regression tests cover literal punctuation, Unicode case matching, displayed
numeric/NULL values, match limits, stale result/options rejection, asynchronous
first-match selection and repeated Enter/Shift+Enter navigation with wraparound.

Native macOS QA verified the packaged QA build with a read-only 150-row SQLite
result containing matches at rows 2 and 140. Search selected row 2; Enter
scrolled to row 140 and another Enter wrapped to row 2. An absent value showed
“No matches” with disabled navigation, and Escape dismissed the search row.
Evidence: [selected distant match](screenshots/query-result-search.png).

Focus-routing regression renders the workspace and verifies grid → result search
→ Escape → grid, then editor → SQL-text search. Full suite: 331 passed, 35 opt-in
tests ignored. This keyboard-routing follow-up has automated rendered coverage;
the native evidence above covers the earlier search behavior.
