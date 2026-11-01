# Pin query results

The pin button beside the result-search magnifier keeps the selected result
available in the same SQL editor across subsequent runs. Pinned result tabs
appear before the latest run's results. Select one to inspect, copy, search or
export its original values and execution duration. The Message view describes
that result, not a later run's error. Rerunning SQL selects the new result while
retaining the pins.

Click the pin button again to unpin, or right-click a pinned result tab and choose
Close pinned result. Closing the SQL editor, replacing its connection, or closing
the window releases pins. They are session-only and are never serialized into
preferences or recovery drafts.

Each editor retains at most 16 pins and 64 MiB of accounted grid allocations.
Accounting includes string/blob capacity, cell vectors, row vectors, and column
names. Allocator overhead, search workers, the current run, and other editor tabs
are outside that bound; it is not an application RSS limit. Pins share existing
Arc<Grid> payloads, deduplicate the same result, and reject additional pins at
the limit rather than silently evicting earlier ones.

Regression tests exercise a real SQLite worker through successful and failed
reruns, reference sharing/release on close, result-specific messages, removal
selection, memory/count limits, and recovery persistence exclusion. Existing
footer geometry/export clipping tests cover the enlarged action group.

Reference: [TablePlus result-tab pinning](https://api.tableplus.com/docs/query-editor/split-results-into-tabs).
Native macOS QA verified pinning a read-only result containing 42, running a new
query returning 99, selecting the pin to recover 42, and unpinning to return to
99. The pin indicator and result selector are visible in
[the captured screenshot](screenshots/pinned-query-result.png).
