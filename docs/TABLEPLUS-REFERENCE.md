# Local TablePlus observations

Observed directly on 2026-10-30 in TablePlus 27.2.0 (840), using the disposable `/tmp/rustable-parity.sqlite` fixture. Existing production connections were not modified. The saved fixture connection had an empty path; setting its file-path field to the fixture restored connectivity. All staged reference mutations were discarded, not committed.

## Editing and deletion

- Editable grid cells expose inline text fields. Tab advances the editing focus to the next column.
- A changed cell displays an amber/yellow background when not covered by the blue selection. The table name in the item sidebar also gains an amber background while changes are pending.
- The row inspector shows field name on the left and SQL type on the right above editable fields. Multiple selected rows display “Multiple values.”
- The toolbar exposes Discard (Cmd+Shift+Backspace), Preview SQL (Cmd+Shift+P), and Commit (Cmd+S) when changes exist.
- Preview opens a popover with generated SQL and Discard/Commit buttons; it can open in a separate window. Rustable intentionally differs here per user instruction: Cmd+S should open review, with a second save to execute, and Cmd+Shift+S bypasses review.
- Row-context Delete marks the entire row red and advances selection to the next row. Discard restores it; the row remains visible until execution.
- The observed UPDATE preview matched the primary key. Rustable additionally checks the full original row for optimistic concurrency protection.

## Context-menu and layout parity backlog

The row menu includes Quick Look Editor, Set Value, Refresh, Paste, Paste to Selected Cells, Add Row (Cmd+I), Duplicate (Cmd+D), Copy Rows, Copy Rows As, Copy Selected Cells, Copy Selected Cells As, Copy All Column Values, Quick Filter, Import, Export current page, and Delete.

Bottom controls: Data/Structure/Index, Add Row, selected/total row count, Filters (Cmd+F), Columns, Previous (Cmd+Left), page settings, Next (Cmd+Right). This fixture defaults to pages of 300 rows. Rustable currently uses 500.

Still to verify directly: precise Enter/Escape semantics, inspector edit propagation, save-clears-highlight behavior, undo/redo, insertion/duplication defaults, rectangular selection, bulk editing, and structure/index workflows. A successful connection and selected interactions do not establish full parity.

## Query workspace verified locally

Ran SELECT * FROM customers LIMIT 5 on the disposable fixture. Native screenshot and AX confirm editable SQL at the top, run controls beneath the editor, result grid below, result Data/Message/Chart and timing controls below that, and a separately toggleable console log at the bottom. The colored connection bar spans the top toolbar. Console toggle is Cmd+Ctrl+C. The earlier read-only query attempt timed out at the automation layer but completed successfully; reconnection verified the five rows and logged statement.

## SQL find and replace

Cmd+F in the native SQL editor reveals an inline Find field, previous/next controls, Done and a Replace checkbox. Replace expands a replacement field, Replace (selected match then next) and All. The search menu exposes Ignore Case, Wrap Around, Contains, Starts With, Full Word and Insert Pattern. Opened controls only; no fixture data changed.

## New row workflow verified locally

On the disposable SQLite fixture, the bottom Add Row button (Cmd+I) appends a staged row after the loaded page, scrolls it into view and focuses its first cell. All eight fields initially display DEFAULT, including the integer primary key. The inspector mirrors those DEFAULT values. Preview shows an INSERT without explicit fields. Discard removed the pending row and disabled Commit; no database write was made. Rustable should implement valid SQLite `DEFAULT VALUES` syntax for the all-default case, with omitted fields distinct from explicit NULL. Insertion/duplication interaction details still need further comparison.

## Duplicate row verified locally

Selected customer 300 in the disposable fixture and pressed Cmd+D. TablePlus appended a selected pending row, reset its integer primary key to DEFAULT, copied name/email/plan/status/country/real value/timestamp, and focused the first cell. Discard shortcut removed it without saving. Composite-key and generated-column duplication still need direct reference checks.

## Multiple-row selection

On the disposable SQLite customers fixture, clicking id 298 then Shift-clicking id 300 selects three rows. Cmd-clicking id 299 removes that row and leaves 298/300 selected. The bottom counter updates from three to two. The inspector shows shared field values (Active) and Multiple values for differing fields. Reference data was not changed.

On the same two-row selection, Backspace stages deletion of both records with red backgrounds and clears the selected-row counter/inspector. Cmd+Shift+Backspace discards both pending deletions. No commit was performed.

Editing the shared status value in the multi-row inspector to QA shared value and pressing Tab staged that value in both selected fixture rows. Discard restored Active in both rows; no commit was performed.

### Bulk inspector constants (local fixture)

With two customer rows selected, the inspector field button exposes NULL, DEFAULT, EMPTY, PRETTY, MINIFY and Add a file. Choosing NULL for status changed both selected rows to pending NULL. Cmd+Shift+Backspace restored both original Active values; no reference changes were committed.

### Editing empty inspector values

On the disposable fixture, choosing EMPTY for customer 299 status shows the EMPTY placeholder in the grid and inspector. Clicking the inspector field and entering text followed by Tab stages the replacement and focuses country. Discard restored Active; no reference changes committed.

### Default grid copy format

On the disposable fixture, Edit > Copy with customers 298–300 selected produced three tab-separated rows without headers, verified by pasting into the existing SQL scratch tab without executing. Copy Selected Cells is a separate Edit command. Removed the pasted scratch content afterward. Opening another SQL tab hit the free-trial two-tab limit; dismissed Later and used the existing tab.

### Grid Select All scope

With customers loaded on the disposable fixture, Cmd+A in the grid displays 300 of 2,400 rows selected. Selection is scoped to the loaded page, not every database row. No data changes were made.


## Dedicated research follow-up — 2026-10-31

**Native result: blocked, not observed.** `cua.getApp('com.tinyapp.TablePlus')` and then `cua.getApp('TablePlus')` each returned `-10005 timeoutReached`. Between attempts, native app inventory listed TablePlus as running. Neither attempt returned usable window state. No fixture was opened, no form was dismissed, no SQL ran, no configuration changed and **no fresh screenshot was captured**. The 2026-10-30 observations above remain historical evidence, not re-verification. Rustable and Rustable QA were not operated by this researcher.

### Official documentation checked in this pass

These are documentation findings, not native observations. Current web documentation can describe versions beyond the installed 27.2.0 build; confirm behavior locally before calling it exact UI parity.

- **Connections:** Save and Test are separate from Connect; groups may nest. Group deletion also removes their connections in the documented reference. Rustable currently keeps them, an explicit safer deviation. The connection form supports custom tags, TLS certificate choices and credential-storage policies beyond Rustable's current session-only password implementation. [Connections](https://tableplus.com/docs/gui-tools/manage-connections)
- **Tabs/windows:** Single-click object reuse differs from keeping a tab by double-click. An SQL or dirty tab is preserved. Multiple database workspaces may occupy one window; a workspace can move to another window. Rustable's independent native windows cover only part of that model. [Tabs, workspaces and windows](https://tableplus.com/docs/gui-tools/the-interface/multi-tabs-workspaces-windows)
- **Keyboard:** Reload workspace, reload tab and reconnect are separate commands. The table/edit context matters for Return and Tab. Neither this page nor the row page defines Shift+Tab's complete reverse-edit behavior; that remains an experiment, not an inferred fact. [Shortcut keys](https://tableplus.com/docs/utilities/shortcut-keys)
- **Rows:** The documented header cycle is descending, ascending, then unsorted. Cell middle-click opens a larger value editor; tab middle-click closes the tab. Multi-column sorting, rectangular paste and foreign-key links extend beyond merely displaying sort arrows. [Rows](https://tableplus.com/docs/gui-tools/working-with-table/row)
- **Editor:** The documented run controls lie below the editable area and above results. Result limiting is configurable per connection. SQL folding and persistent file folds are documented; Rustable's wrapped line-numbered editor should not be described as complete editor parity. [Editor basics](https://tableplus.com/docs/query-editor/editor-basics)
- **Suggestions:** Object/column/function/keyword categories and acceptance preferences are documented. The alias-column example supports testing contextual columns; it does not prove exact CTE, nested-scope or malformed-SQL behavior. [Autocomplete](https://tableplus.com/docs/query-editor/autocomplete)
- **Review:** Generated GUI mutations can be previewed, committed, discarded or displayed in a separate window. SQL sent directly from the editor does not enter that pending-change preview. Rustable's Cmd+S-first review remains the user's deliberate override. [Code preview](https://tableplus.com/docs/gui-tools/code-review-and-safemode/code-preview)

### Actionable differences identified by source inspection

1. **Environment identity, P0:** Saved profiles have IDs, but status styles currently map through endpoint labels. Two profiles using the same endpoint can overwrite each other's color/environment. Require active-profile identity all the way to each workspace header.
2. **Sorting, P1:** Current Rustable header handler alternates ASC/DESC and cannot clear its sort on a third click. It also starts ascending, differing from the official row documentation. Confirm with the fixture before claiming exact behavior.
3. **Connections, P1:** Flat groups and native windows are implemented. Search, nested grouping, separate Test, Keychain credentials, duplication and workspace-in-one-window still need their own deliverables.
4. **Native QA remains distinct:** Root-agent Rustable screenshots verify several fixes, including reverse inline navigation. They are not screenshots of the reference app and cannot establish exact TablePlus parity.

The expanded acceptance cases and separate implementation/native-verification boxes are in [TABLEPLUS_PARITY_REQUIREMENTS.md](TABLEPLUS_PARITY_REQUIREMENTS.md). Do not close them based on a documentation read or successful compilation.


### Separately reported Rustable native evidence, 2026-10-31

The root QA agent reports that a saved SQLite fixture opened in an actual second native window, with its own initial tabs (distinct from the demo window's existing drafts) and local green connection status. The child's Connections action focused the root manager. Artifact: [independent-connection-window.png](screenshots/independent-connection-window.png). This is Rustable evidence only; it verifies opening/routing, not native close recovery, concurrent cancellation, or reference parity.
