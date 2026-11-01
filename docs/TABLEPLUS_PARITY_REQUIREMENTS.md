# TablePlus parity requirements and acceptance ledger

Research date: 2026-10-31. Target: macOS TablePlus 27.2.0 (840), plus explicit Rustable requirements in GOAL.md. This is a delivery checklist, not a claim of parity. Status describes inspected source at this date; concurrent implementation may advance it.

## Evidence and confidence

- **U**: explicit user requirement, including supplied reference screenshots. Takes precedence over TablePlus defaults.
- **O-prior**: native fixture observation previously recorded in [TABLEPLUS-REFERENCE.md](TABLEPLUS-REFERENCE.md). Not newly reproduced by this researcher.
- **D**: current official TablePlus documentation linked beside the finding.
- **A**: proposed Rustable acceptance criterion or architecture requirement; not a claim that TablePlus implements it exactly.
- **C**: implementation verified by reading source. Tests named below are present, but their presence alone is not native UX verification.
- **Pending experiment**: must be checked in the installed reference app on `/tmp/rustable-parity.sqlite`.

Native research status — dedicated follow-up pass, 2026-10-31: bundle-ID access and display-name access each failed with `Computer Use server error -10005: timeoutReached`; app inventory still reports TablePlus running. These are **two additional failed attempts**, not successful experiments. No returned window state, fixture opening, reference mutation or fresh native screenshot occurred. Earlier three timeouts remain part of the research record. Current work uses official documentation plus separately attributed prior observations. Do not infer that inventory availability means a functional reference session.

Checklist convention: the existing requirement boxes below represent **full acceptance**, not mere source presence. The implementation/verification ledger at the end separates a delivered subset from native comparison. A checked implementation box cannot close its parent acceptance requirement.

Native research follow-up: access recovered later on 2026-10-31. Cmd+N opened the TablePlus connection chooser and the saved `/tmp/rustable-parity.sqlite` profile was selected for opening. TablePlus refused with “Free Trial limited 2 workspaces.” The alert was dismissed; existing user sessions were not closed. No new fixture interaction or hover experiment was completed. Read-only accessibility observation confirmed the gutter header action is named “Expand the columns to fit the content”. Rustable now implements and natively verifies one-shot fitting to visible cell previews; see `screenshots/fit-table-columns.png`. Exact reference sampling behavior remains unverified.

## Product contract

**U/A:** A compact native-feeling database workspace with predictable keyboard behavior, bounded memory and immediate feedback. Retain staged edits and SQL drafts through navigation, cancellation, failures and restart. Controls must reflect actual engine support. Publish real QA screenshots for mobile review. Rust/toolchain installations use mise; dependencies use Cargo. Full parity is not established by matching one SQLite screenshot.

## Connections and session organization — P0 architecture

**D:** TablePlus supports named saved connections, separate Test/Save/Connect actions, status colors/tags, credential-storage choices, bootstrap SQL, connection duplication and search. Groups can nest and accept drag/drop. SSH supports keys or SSH configuration; TLS offers modes and certificate files. Connection export/import is supported. [Connection management](https://tableplus.com/docs/gui-tools/manage-connections)

**C (updated during implementation):** `src/connection_profiles.rs` and `src/connections.rs` implement persisted profiles, flat groups, reorder/move interactions, per-profile environment/color, and SSH config/key/agent settings. Passwords default to session-only; optional native credential storage is implemented with isolated QA/development identities, but real keychain and cross-platform verification remain open. Background Test reports successful and failed connection fields; live disposable PostgreSQL authentication/database/success fixtures passed. Nested groups, connection import/export and full TLS certificate/options remain open. Each `Rustable` owns one worker; `session_windows.rs` hosts independent boxed workspaces in native viewports. Native QA has verified opening the saved SQLite fixture and Cmd+N returning to the separate launcher; broader multiwindow failure/recovery QA remains required.

**C (identity isolation update):** Live/pending sessions now carry a stable profile ID and presentation snapshot. A successful connect changes the active identity; failed replacement/reconnect preserves the existing endpoint and identity. Same-endpoint profiles retain independent names/environments/colors. Root-owned metadata revisions update only matching child snapshots, without cloning the profile store each frame. Regression covers two such profiles, independent window, rename, persistence/reopen, failed replacement/reconnect, deleted-profile snapshot retention, and ad hoc legacy fallback. Native verification remains pending. Status-bar click now reveals the matching saved profile by identity, clearing search and expanding its group. Native child-window manager requests forward the active profile ID to the root manager. Unsaved forms or nonempty session passwords block replacement with a save/discard message; the currently selected draft is retained. Removed profiles produce an explanation without altering the live connection. All 32 connection-manager tests and strict all-target Clippy pass; native click-through verification remains pending. Its tooltip retains the actual endpoint.

- [ ] **CON-01 / A:** Introduce stable profile IDs separate from labels. Rename host/user/display name without orphaning appearance, history or workspace state. Save metadata without connecting; Test must not replace the active workspace.
- [ ] **CON-02 / A:** Credential references use macOS Keychain; no secret values in serialized preferences, logs, screenshots, crash messages or connection labels. Verify ask-every-time and no-password workflows. Handle missing/locked credentials explicitly.
- [ ] **CON-03 / U,D,A:** Searchable groups with creation/rename, nested groups, drag/drop reparenting and visible drop targets. Persist order; prohibit group cycles. Dragging must not connect. Keyboard alternative and clear delete scope required. Test duplicate names and search while moving.
  Non-drag context-menu controls now move groups/connections up or down and move
  connections to a group or Ungrouped. Reorder controls are disabled during search;
  boundary moves are disabled. Headless UI regression verifies ordering, root
  transfer, persisted IDs and no connect request. Native keyboard-only access,
  nested groups and the full CON-03 matrix remain unverified/incomplete.
  Native profile Move down/up and restart persistence passed; destination submenu
  disables the current group. Added Shift+F10 menu access from focused rows and
  the selected search result; headless open/Escape/no-connect regression passes.
  Native Shift+F10 now focuses an enabled action, Up/Down changes focus and Enter
  reorders the profile. Original QA ordering restored. Screenshot:
  `docs/screenshots/connection-keyboard-menu-focused.png`. Full keyboard-only
  group/submenu/search matrix and nested groups still remain open.
- [ ] **CON-04 / U,A:** Environment and color edit in profile; immediate matching top status bar; independent styling across same-host profiles. Custom environment labels must not be coupled to display name.
- [ ] **CON-05 / D,A:** SSH connection setup with separate database/SSH identities, key passphrase handling and known-host verification. Validate tunnel lifecycle, cancellation, disconnect and reconnect using disposable SSH infrastructure. Never silently downgrade transport or ignore host-key changes.
- [ ] **CON-06 / D,A:** TLS mode/certificate UI must truthfully map driver capabilities; certificate failures retain editable connection details and useful redacted errors.
- [ ] **CON-07 / A:** Profile migration/import/export preview, collision handling and explicit secret inclusion. A cancelled import or test preserves current sessions. URI parsing never logs credential-bearing URI text.

## Tabs, workspaces and windows — P0

**D:** TablePlus distinguishes object/query tabs, per-database workspaces and windows. A clean object tab can be reused on single click; double-click keeps it. Dirty tabs and SQL tabs are preserved. Tabs support middle close and context close operations. Connections/databases can open in another workspace or window; a workspace can move to its own window. [Tabs/workspaces/windows](https://tableplus.com/docs/gui-tools/the-interface/multi-tabs-workspaces-windows)

**C (updated):** `open_table`, `close_table_tab`, `new_tab`, `close_tab`, `reopen_tab` support basic lifecycle. `session_windows.rs` now provides independent native session windows, with root-owned profile editing and child SQL documents archived into a persistent recovery pool. Window arrangement/reconnection restore and multiple workspaces within one window remain open. Query documents preserve filename, draft and baseline. Tests cover worker independence and child recovery. Root native QA verified separate initial tabs and the child Connections action focusing the root manager; close/relaunch and concurrent-error behavior remain unverified.

- [ ] **TAB-01 / U,A:** Middle close works on active/inactive table and SQL tabs; closing the selected table clears sidebar selection. Last tab closes cleanly; reopen restores draft, source association and caret where available.
- [ ] **TAB-02 / D,A:** Implement preview-versus-kept table tabs. Single click reuses only an eligible clean preview. Double click keeps it. Never replace a dirty or SQL tab. Check details natively before changing behavior.
- [ ] **TAB-03 / D,A:** Close others/right/all, rename SQL tab and reorder tabs. Preserve tab identity while worker results arrive; closing an unrelated tab cannot redirect a result.
- [ ] **TAB-04 / U,D,A:** Independent sessions per workspace/window: worker, connection, schema cache, cancel token, staged mutations, results and tab state. A slow query in one session must not block another.
- [ ] **TAB-05 / A:** Move workspace to window without reconnecting or losing transactions/drafts. Close-window decisions operate only on affected workspaces. Restore arrangement without automatically executing SQL.

## SQL editor and IntelliSense — P0/P1

**D:** TablePlus places results below the editor; current execution uses selection or highlighted statement, all execution gives separate results. It supports result limits, folding, comment commands and file saving. [Editor basics](https://tableplus.com/docs/query-editor/editor-basics)

**D:** Completion covers database objects, columns, functions, procedures, keywords and saved-query bindings. Options control categories, schema prefixes, whitespace, keyword case, paired delimiters and acceptance key. [Autocomplete](https://tableplus.com/docs/query-editor/autocomplete)

**C:** `editor.rs` caches syntax layout; `sql.rs` shares dialect-specific execution boundaries with the PostgreSQL backend. `completion.rs` remains a hand-built bounded suggestion implementation with simple relation extraction. It is not a complete semantic language service. Query document recovery and conflict-aware save exist. Dollar-quoted PostgreSQL suppression/execution routing and real SQLite/PostgreSQL language helpers are implemented. PostgreSQL qualified CTE/derived-table columns, quoted names and alias shadowing now have parsed-tree resolution; full semantic coverage remains open.

- [ ] **SQL-01 / U,A:** Full-height editable pane, logical line numbers and soft wrap; no horizontal scrollbar. Wrapped lines preserve cursor, selection and execution highlight. Empty editor uses available height immediately.
- [ ] **SQL-02 / U,A:** Cmd+Enter selection/current; Cmd+Shift+Enter all. Caret exactly at/after semicolon targets preceding statement. Highlight and backend must share dialect boundaries. Cover comments, quoted separators, SQLite triggers, PostgreSQL dollar bodies and incomplete input.
- [ ] **SQL-03 / U,A:** Evaluate a maintained SQL language/parser library for IntelliSense rather than extending ad hoc parsing indefinitely. Gate adoption with Rust integration, licensing, incomplete-input recovery, SQLite/PostgreSQL dialect support, latency and memory measurements. Library usage alone does not satisfy scope resolution.
- [ ] **SQL-04 / U,A:** Completion resolves aliases, qualified schemas, quoted identifiers, CTEs, subqueries and shadowing. Metadata loads off the UI thread; changes invalidate only relevant caches. Tests must include cursor before FROM, joins, same table names in different schemas, huge schemas and malformed SQL.
- [ ] **SQL-05 / U,A:** Compact suggestion popup sized to text, no help footer; stable arrow/Enter/Tab/Escape behavior, no double acceptance, correct undo grouping. Avoid suggestions inside strings/comments; preserve engine-appropriate quoting.
- [ ] **SQL-06 / A:** Explicit find/replace scope, undoable formatting, indentation/outdent, multi-caret operations and folding. Library-backed parsing must not stall cursor movement on large documents.
- [ ] **SQL-07 / A:** Script save and database commit remain distinct commands. External file changes cannot silently overwrite disk or editor. Restore dirty drafts without replaying SQL or discarding their conflict baseline.

**D:** Independent side-by-side or vertically split editors/results, pane focus shortcuts and result visibility are documented. [Split panes](https://tableplus.com/docs/query-editor/split-panes)

- [ ] **SQL-08 / D,A:** Resizable split panes with distinct focus/results/cancel state; closing a pane preserves recoverable drafts. Query parameters, explain plans, query-folder organization and richer result modes remain separate deliverables.

## Keyboard map and focus routing — P0

**D:** Cmd+R reloads workspace, Cmd+Option+R reloads current tab, Cmd+Shift+R reconnects. Cmd+E opens SQL; Cmd+W closes tab. Table Return/Tab edits a cell; editor Cmd+Enter runs current/selection. Cmd+I is context-sensitive (new row versus SQL formatting). [Shortcut keys](https://tableplus.com/docs/utilities/shortcut-keys)

**C (corrected after implementation):** Cmd+R in Query view now dispatches metadata refresh without rerunning SQL; a regression uses a destructive SQL draft and verifies it is not executed. Current cell editor consumes Shift+Tab from the event's modifiers and requests previous-cell navigation; regression tests exist. Exact native refresh and Shift+Tab reference behavior still require observation.

- [ ] **KEY-01 / A:** Resolve Cmd+R semantics using reference UI before claiming parity. Reload must not unexpectedly execute writes. Preserve filters, sort, page and pending edits or explicitly explain why an operation cannot proceed.
- [ ] **KEY-02 / U,A:** Shift+Tab commits valid active-cell draft then edits previous eligible cell; Tab moves forward. Boundary, deleted/readonly/generated columns and invalid values must be tested. Do not let window focus traversal steal the chord.
- [ ] **KEY-03 / A:** One context owns each shortcut: modal review/import, inline cell, SQL editor, grid, global workspace. No action fires behind a modal; no completion acceptance also executes SQL.
- [ ] **KEY-04 / U,A:** Preserve requested deviation: Cmd+S opens generated-change review; second Cmd+S/Cmd+Enter confirms; Cmd+Shift+S bypasses review. TablePlus documents Cmd+Shift+S as SQL file save, so Rustable must expose script save elsewhere and make this intentional conflict visible in menus/tooltips.

## Data editing, structure and safety — P0/P1

**O-prior:** Fixture experiments recorded inline editing, Tab progression, amber changed cells/sidebar item, red pending deletion, multi-row inspector shared values, explicit NULL/EMPTY/default actions, default-valued insertion, duplication with regenerated integer key, page-scoped selection and TSV copy. All staged reference mutations were discarded. See [native record](TABLEPLUS-REFERENCE.md).

**D:** Relational table modes place Data/Structure/Index below content and defer GUI changes until commit. [Table workflow](https://tableplus.com/docs/gui-tools/working-with-table)

**C (updated 2026-10-31):** SQLite implements staged mutation planning/application, structure/index UI and CSV workflows. PostgreSQL staged row insert/update/delete and full-table export are implemented and live-tested, including primary-key writes with optimistic original-row checks, catalog-derived complex type casts, and schema revalidation. PostgreSQL structure changes are now being integrated into the same review workflow; implementation alone does not prove native structure parity. PostgreSQL mapped CSV import now streams through COPY in one transaction; disposable-server coverage checks rollback, cancellation, type conversion and schema validation. Native import UI verification remains open.

- [ ] **DATA-01 / U,A:** Inline grid and inspector editing share one staged value. Yellow covers full pending cell box, including padding; selected state must not falsely imply saved state. After successful commit clear pending paint; after failure retain draft and error.
- [ ] **DATA-02 / U,A:** Selection changes on primary press, without hover tint (latest user correction supersedes release-only behavior). Support rectangular and multi-row selection, clear focus indication and correct copy/paste scopes. Native verify Shift/Cmd combinations and keyboard navigation.
- [ ] **DATA-03 / A:** NULL, empty text, DEFAULT and unchanged must be distinct typed states. Numeric precision, Unicode, binary, timestamp and JSON values round-trip. Generated/readonly columns are visibly noneditable.
- [ ] **DATA-04 / A:** Insert/delete/duplicate support keys, composite keys, defaults and generated columns per engine. Review displays inline escaped values while execution uses bound parameters. Preview and commit must share the exact plan.
- [ ] **DATA-05 / A:** Atomic mutation batches, stale-original conflict detection, trigger/constraint errors, cancellation and disconnected-server failures preserve recoverable pending edits. Test via real drivers, including externally modified rows.
- [ ] **SCHEMA-01 / U,A:** Dense editable structure grid and properties inspector for names/types/default/nullability/keys/engine-specific fields. Unsupported changes cannot look actionable. Migration previews describe reconstruction/destructive effects accurately.
- [ ] **SCHEMA-02 / A:** Indexes, constraints, foreign keys, triggers, views/functions and DDL require per-engine capability and integration coverage; a static DDL panel is not an editor.

## Visual and interaction acceptance — P0 continuous

**U/O-prior:** Use supplied screenshots and native fixture as the visual baseline. Do not invent pixel measurements from resized screenshots. Match at equal logical scale and font size in light/dark mode.

- [ ] **VIS-01:** Bottom segmented controls have visible inset separators, contained selection corners and common height with adjacent actions. Keep padding between grid and toolbar. Paging carets are normal-size button targets; Filters has an icon.
- [ ] **VIS-02:** Inspector uses compact search-with-magnifier, field/type labels and tight single-line value boxes. No redundant Row heading. Long content expands intentionally without making ID fields huge. Actions sit at right edge; copy feedback shows a checkmark temporarily.
- [ ] **VIS-03:** Optional activity log lies beneath the central table, between sidebars. Text occupies its pane width. It is read-only. SQL tabs instead have editor above results.
- [ ] **VIS-04:** Top status communicates connection/environment/color, timing, running state and errors. No fake percentage for unknown-duration work. Repaint animations only while active.
- [ ] **VIS-05:** Tab close icons, sidebar Items/Queries/History, search, toolbar groups and disabled actions must be consistent. Test narrow widths, overflow, display scale, large fonts and light appearance.

**D:** TablePlus's toolbar exposes discard/preview/commit, safety, session controls and panel toggles; its colored status bar shows environment and running progress. Safe Mode has several warning/password levels. [Toolbar](https://tableplus.com/docs/gui-tools/the-interface/toolbar)

**A:** Rustable's server-enforced read-only safe mode is a deliberate stronger/different policy, not a claim of reproducing every TablePlus warning mode. Document exact behavior per engine.

## Broad remaining parity and release gates

**A:** These remain open audit tracks, not implied completed features: MySQL/MariaDB and other engines; database switching; advanced filter builder; database-wide search; foreign-key navigation; streaming result paging; SQL/JSON import/export; backup/restore; users/roles; process management; server metrics; ER diagrams; native menus; command palette; customizable keymap; accessibility; packaging/signing/updates; Windows/Linux UX.

- [x] Record current driver capabilities, unsupported operations and transaction boundaries in [the driver capability matrix](DRIVER_CAPABILITIES.md). This is a source audit with focused transaction checks, not implementation or release acceptance.
- [ ] Measure UI input-to-paint and p50/p95/p99 frame times at actual 60/120/144 Hz; report hardware and fixture. Do not equate egui renderer support with sustained application performance.
- [ ] Test bounded memory with million-row results, large cells, wide tables and concurrent sessions; workers must not block render or leak stale results across sessions.
- [ ] Record idle CPU/wakeups, startup time, cancellation latency and failure recovery; maintain regression budgets.
- [ ] Native screenshot evidence and keyboard-only accessibility checks accompany visual acceptance. Unit tests complement rather than replace these.

## Next native experiment matrix

Only fixture connection; no production edits or credentials. Last successful research saw a PostgreSQL form; current window state is unknown because access timed out. Inspect before acting, and dismiss any unrelated form without Save/Connect.

| Experiment | Actions | Evidence and acceptance to capture |
|---|---|---|
| Connection manager | Open fixture context menu; inspect group controls without changing real groups | Exact commands, group/drop visual feedback; use an isolated QA-only group if needed |
| Tab lifecycle | Single click orders, double click customers, revisit, middle-close selected tab | Reuse/pin behavior, selected sidebar clearing, last-tab behavior |
| Refresh | Run harmless SELECT; Cmd+R versus Cmd+Option+R; inspect log | Which queries actually rerun and which metadata refreshes; never test with writes |
| Reverse editing | Edit disposable cell, Tab then Shift+Tab, Escape; discard | Focus destination, commit-to-pending timing and paint coverage |
| Completion | Fixture table alias, quoted identifier, joins and incomplete CTE | Popup sizing, rank, acceptance, suppression, schema prefix; no mutation |
| Multiwindow | Open fixture in second workspace/window | Ownership, focus, restore, query independence and close prompts |
| Inspector density | Narrow/wide inspector with ID, timestamp, JSON and multiline text | Equal-scale screenshots in light/dark, value box height/spacing |
| Schema staging | Inspect column editor; stage reversible QA-only change; preview then discard | Editable fields, generated SQL, validation and pending highlights |

Do not mark a row accepted until its native evidence, code implementation and meaningful error-path validation are recorded. Keep this ledger synchronized with GOAL.md and PROGRESS.md as implementation advances.


## Dedicated research pass: implementation versus verification

Audit snapshot: 2026-10-31; code can advance after this snapshot. **I** means the narrowly named behavior is present in source/tests. **V** means the named Rustable native behavior has evidence; it does **not** mean complete TablePlus parity. `O-prior` evidence remains dated 2026-10-30. This pass produced no new TablePlus screenshot.

| Slice | Implementation | Native Rustable verification | Reference evidence / remaining acceptance |
|---|---|---|---|
| Stable saved profile IDs, secret-free metadata persistence | [x] I | [ ] V restart of saved profile | `connection_profiles.rs`, serde tests; complete CON-01 still needs profile-bound workspace identity and Test |
| Flat group CRUD and ordered move | [x] I | [x] V creation/move reported in PROGRESS.md, [manager screenshot](screenshots/saved-connections.png) | Nested groups/search and keyboard move remain open; exact native drop feedback not compared |
| Open saved SQLite in independent native window; child manager routes to root | [x] I | [x] V root-reported [independent window](screenshots/independent-connection-window.png) | Distinct initial tabs from demo; fixture header and local green status verified; child Connections focuses root manager |
| Dirty SQL recovery and concurrent failure isolation across windows | [x] I | [ ] V full close/relaunch/isolation matrix | `session_windows.rs` and session tests; initial-window screenshot does not close TAB-04a |
| Shared appearance/favorites/history revisions | [x] I | [ ] V cross-window propagation | `publish_shared_preferences` / revision tests; prevent stale windows replacing newer shared state |
| SSH lifecycle and key/agent configuration | [x] I | [ ] V complete native connection/error flow | `ssh_tunnel.rs`; transport tests do not prove form behavior, passphrase UX or reconnect parity |
| Reverse inline editing and gapless pending color | [x] I | [x] V root-reported [Shift+Tab screenshot](screenshots/inline-edit-shift-tab.png) | Exact TablePlus reverse-navigation/boundary semantics remain pending; user behavior contract controls implementation |
| Compact suggestions, persistent segment separators | [x] I | [x] V root-reported [completion](screenshots/compact-completion.png) / [structure](screenshots/structure-dividers.png) | Full light/dark/narrow-width comparison pending |
| Cmd+R in SQL view refreshes schema without running draft | [x] I | [x] V root-reported [refresh screenshot](screenshots/refresh-schema-shortcut.png) | Docs name reload scope but do not prove exact SQL result/metadata behavior; native reference experiment remains open |
| Pending actions fit beside paging/filter controls | [x] I menu consolidation | [ ] V pending state at narrow widths | Previous screenshot exposed overlap. Do not close VIS-01 from source-only layout change |
| Duplicate endpoint profiles retain independent environment/color | [x] I | [ ] V | Active/pending profile identity now binds on successful connection; worker-backed tests cover same-endpoint styles, metadata edits, persistence and failed reconnect. Native comparison pending. |
| Three-state header sort cycle | [x] I | [x] V basic fixture cycle | DESC → ASC → default order; SQLite fixture checks actual rows, filter preservation and page reset. Explicit context actions and multi-column sort remain open. |
| Contextual CTE/subquery/schema-qualified completion | Partial I | Partial V | Qualified CTE/derived-table columns and alias shadowing pass helper and application subprocess tests; full SQL-04 scope remains open |

## New acceptance slices from reference research

These are Rustable engineering tests (**A**) linked to source findings in the [follow-up reference record](TABLEPLUS-REFERENCE.md#dedicated-research-follow-up--2026-10-31). They are not claims of freshly observed TablePlus behavior.

### P0 — environment identity and session ownership

- [ ] **CON-04a:** Save profiles A and B pointing to the same disposable database, assign visibly distinct names, colors and environments, then open both. Each header must show its own profile. Editing B must never relabel A. Restart and retest; also test rename and endpoint edits. Profile ID—not endpoint text—is the association key.
- [ ] **TAB-04a:** In separate fixture windows, stage an edit in A while running a slow harmless query in B. B's cancel, close and refresh must not clear A's draft. Verify closing A is blocked while DB changes are staged, while an unsaved SQL document is retained in recovery. Exit/relaunch and inspect recovered filename, dirty baseline and source association without running it.
- [ ] **TAB-04b:** Change theme in either window, add a favorite, run unique harmless SQL, then interact with the other window. Confirm consistent shared preferences and newest-first bounded history after autosave, another window opens, one closes, and restart. No per-frame preference cloning while unchanged.
- [ ] **KEY-03a:** Exercise every chord with focus in a field, grid, editor, completion popup, review window and import modal. A query run must never leak behind a modal or occur from a refresh chord. Give native menus/tooltips the same ownership map as handlers.

### P1 — connection manager completeness

- [ ] **CON-08:** Add non-connecting Test with explicit running/success/error state. A test does not replace the active session, execute bootstrap SQL or save a password. Retry after correction; closing the form must stop/release test work.
- [ ] **CON-09:** Search profiles/groups without destructive filtering; retain search while editing and restore focused row. Add duplicate-profile action that creates a new ID and retains nonsecret transport choices. Test duplicate display names and keyboard-only selection.
- [ ] **CON-10:** Introduce nested grouping only with cycle-safe reparenting, visible insertion/parent targets and a keyboard destination chooser. Invalid drops are no-ops. Reload the profile store and verify exact order and membership.
- [ ] **CON-11:** Keep group deletion scope explicit. **Current Rustable policy preserves connections**, unlike the documented reference behavior. Do not silently make this destructive in the name of parity. Label the outcome, and require a separate deliberate operation for deleting contents if added later.
- [ ] **CON-12:** Form layout must retain name/environment/color, database host/port/user, transport settings and actions at common widths. An invalid field should explain why Open/Test is unavailable. Keep credential-storage policy visible; do not portray session-only passwords as Keychain support.

### P1 — sort, editor and review detail

- [ ] **DATA-06:** Use the documented three-state sort cycle as the target, subject to native confirmation. Ensure a new sort resets page, the indicator matches SQL and clearing sort removes its generated clause. Explicit ascending/descending/clear context actions avoid reliance on cycling. Then add independently tested multi-column ordering.
- [ ] **SQL-09:** Run controls now sit between editor and results, verified in `screenshots/query-results-footer.png`; continue checking narrow widths and shortcut ownership. Keep full editor height, wrapping and result visibility while changing control placement. A button's default action must agree with its tooltip and keyboard shortcut.
- [ ] **SQL-10:** Separate engine result truncation from query rewriting. The UI must state what was loaded, whether the result is incomplete and how to request more. A limit setting must never silently change a write statement, nested query or existing limit. Persist any future limit preference per profile.
- [ ] **SQL-11:** Exercise completion acceptance with both mouse and keyboard; changing its acceptance preference must not also insert a newline/tab. Add category/schema-prefix/case preferences only when each maps to implemented behavior. Native-check ordering/ranking instead of guessing.
- [ ] **DATA-07:** Review contains only pending GUI mutations, with inline values and a bound execution plan. Manually run SQL must not appear as an uncommitted UI change. After a review is opened, editing again must invalidate the old plan. Copying review SQL must not execute it. A future detached preview must remain tied to its session and revision.
- [ ] **DATA-08:** Treat middle-click context separately: tab close versus cell Quick Look. Add a larger value view for long text/JSON/binary only after native observation; retain the user's inline editing default. Test Escape and focus return without losing an active draft.

## Reference-dependent experiments still blocked

1. TablePlus **Shift+Tab**: official shortcut and row pages do not specify reverse editing details. Record direction, whether the value becomes pending before focus moves, last/first-column behavior, multiline cells and validation failure before declaring exact parity.
2. **Reload**: documented names distinguish workspace, tab and reconnect. They do not establish which SELECT/result caches rerun. Use only harmless SELECT and the console log to observe this. Never use a write statement to probe the reference.
3. **Sort**: verify first-click direction and third-click clearing on a previously unsorted fixture; compare column context menu and multi-sort indicator.
4. **Tabs**: capture reuse versus pinned object tabs, SQL-tab preservation, selected sidebar clearing, middle-click last-tab behavior and trial-limit interruptions. Existing trial limit must not be mistaken for a product bug.
5. **Windows**: create a second fixture session from the manager, inspect connection/workspace distinction and quit prompts. No production session is an acceptable substitute.

When native access returns, capture screenshots only after the visible fixture identity is confirmed. Give each artifact a date, logical window size, appearance and exact action; do not reuse a Rustable screenshot as TablePlus evidence. This researcher has not altered any application preference or database during this follow-up pass.

Native Rustable QA follow-up (2026-10-31): saved SQLite profile opened in child window and header displayed its profile name, Local environment and green color. Three sequential ID-header clicks visibly produced descending IDs/down arrow, ascending IDs/up arrow, then default IDs/no arrow; activity log matched all states. Screenshot: screenshots/profile-header-sort-descending.png. Same-endpoint dual-profile persistence/failure isolation is covered by worker-backed tests; its full native dual-window matrix remains pending.

## Connection form refinement implementation (2026-10-31)

- [x] Implement compact aligned Profile/Database/Security/SSH fields with bordered inputs and fixed footer actions.
- [x] Adapt form height to available screen; expanded SSH fits 1440×952 in layout regression.
- [x] Reserve scrollbar gutter on constrained screens; light/dark 1000×650 geometry regressions.
- [x] SSH agent checkbox reflects authentication source; choosing key unchecks it, checking it clears chosen key, cancelled picker preserves state.
- [x] Saved connection search and Duplicate implemented with pointer/menu and stable-width regressions.
- [ ] Complete native verification/screenshots of connection search/duplicate workflows. QA automation is available again; opening the saved PostgreSQL form and connecting with a session-only password were verified on 2026-10-31. Search/duplicate coverage remains outstanding.
- [x] SQL run/file controls moved below editor; native harmless SELECT 42 confirmed results below controls (screenshots/sql-controls-below-editor.png).

SSH config reference: TablePlus documents using ~/.ssh/config when key authentication is enabled without an explicit private key: https://tableplus.com/docs/gui-tools/manage-connections . Rustable follow-up delegates config parsing to system OpenSSH, preserving Host/Include semantics. Implementation and fixture verification underway; live configured-alias tunnel verification not yet claimed.

### Native quit safety checkpoint
- Implemented/nativeverified: menu Quit / CmdQ routes through pending-edit and running-work checks across sessions, focusing the blocking window. Clean quit persists SQL draft recovery.
- Implemented: synchronous AppKit applicationShouldTerminate guard using the published end-of-frame readiness state. A QA native-menu probe of NSApplication terminate verified pending-edit cancellation and clean exit with persisted SQL recovery. Actual Dock interaction was denied by computer-use permission; no OS logout/shutdown was triggered, so those end-to-end checks remain unverified.

### PostgreSQL structure editing acceptance (2026-10-31)

Reference: [TablePlus column workflow](https://tableplus.com/docs/gui-tools/working-with-table/column). The reference supports staged inline attribute changes and an editable properties inspector. The existing Rustable dialogs are an intermediate UI, not full visual/workflow parity.

- [x] Review and atomically apply add/rename column, type/default/nullability changes and create/drop ordinary indexes (live driver fixtures).
- [x] Revalidate table/type/index definitions and reject stale reviews; live failure/rollback tests passed. Dedicated cancellation during a long DDL operation remains to be tested.
- [x] Preserve safe-mode and view restrictions; structure changes do not require a primary key (capability regression + live no-PK fixture).
- [x] Native PostgreSQL fixture: add column, inspect generated SQL, save, refresh and verify metadata; undo/redo before save. Other property/index native flows and discard remain open.
- [ ] Dense inline structure cells, editable inspector, pending highlight, column removal and constraints (still outstanding).
- [ ] Mixed data/schema change batches and engine-specific expressions/defaults beyond typed constants (still outstanding).

### Inline structure names (2026-10-31)
- Double-click a column name to edit within the structure grid; Enter or focus loss stages the rename, Escape cancels the active draft.
- Pending names retain a yellow cell background and participate in the existing reviewed SQL and change-history workflow. Re-editing uses the staged name; restoring the original removes that rename action.
- Save and table-navigation completion validate the inline name. Empty/whitespace-only names and NUL are rejected without losing the draft. Active drafts also guard window/application closure.
- Remaining structure editing: inline type/default/nullability, editable property inspector, column deletion and broader constraints. These are not covered by the inline-name implementation.

### Inline nullability (2026-10-31)
- Structure Nullable cells expose YES/NO selection for supported editable table columns; primary-key/generated fields, Safe mode and busy connections are disabled.
- Selection merges with any staged type/default edits for the same column. Returning to the loaded value removes only the nullability delta (or the entire alteration if otherwise empty).
- The cell receives pending background feedback and the action uses the shared change history and reviewed schema planner. Database constraints and existing values are validated by the existing SQLite/PostgreSQL apply paths.


## Latest interaction and activity requirements (2026-10-31)
- [x] Source/tests: select sidebar items and table rows on primary press; preserve modifier selection without re-toggling on release. No hover tint.
- [x] Source/tests: previous/next tab uses Cmd+Shift+[ / Cmd+Shift+]; plain Cmd+[ / Cmd+] is unbound.
- [x] Source/tests: persist data column widths by saved profile, qualified table and column; query results use output shape, without storing SQL literals. Restore after restart/reorder and synchronize windows. Ad-hoc sessions remain ephemeral.
- [x] Source/tests: remove extra egui smoothing of native wheel/momentum input. Programmatic scrolling is unchanged; subjective native comparison remains open.
- [x] Native QA verified: replace connection underline with full-height rounded-clipped block activity behind text; show honest elapsed time and decoded-result observations. No fabricated query percentage or socket throughput. Raw socket byte telemetry remains separate work.
- [ ] Release: stable macOS developer signing/notarization for predictable Keychain identity across updates; do not weaken Keychain ACLs.

- [x] Source and regression tests; native QA verified centered headers and compact sizing. Latest table polish: full-header sorting hit area; centered header label and adjacent sort icon; content-based initial widths unless explicitly resized; avoid horizontal-scrollbar flash when selecting rows.
- [x] Source and regression tests for loaded rows; pending inserted rows remain a separate navigation path. Latest navigation: active-cell outline with corner handles, four-arrow cell traversal and automatic reveal; preserve editor keyboard ownership. Orange/yellow indicates a committed staged value, not merely entering edit mode.
- [x] Source/tests: pagination uses visible row range and comma-separated totals, with ~ for catalog estimates and an ellipsis when the total is unknown. Avoid costly full-table scans just to display a count.

- [x] Source/test correction: active-cell outline follows the entire cell background rectangle, with no interior inset; opposing corner dots remain on its boundaries.
- [x] Source/tests latest footer refinement: compact horizontal button padding; left table actions, centered row range/selection count, right-anchored filtering/export/paging; retain nonoverlapping narrow-layout behavior.

- [x] Source/interaction regression: Enter edits the active writable data cell; Escape cancels the draft and restores four-arrow grid navigation without staging it.
- [x] Source: Items/Queries/History section tabs highlight on hover; database object rows retain quiet hover behavior.
- [x] Source/state regression: successful connection dismisses the independent Connections picker; failed attempts keep it open; Cmd+N may reopen it.

- [x] Source/tests: automatic widths cap at155 points; migrate legacy uniform155 saved defaults while retaining custom dimensions and future explicit155 widths. Verify actual profile-backed rendered geometry.
- [x] Source/toggle regression: compact top-right bottom-pane and right-pane icons; appearance choices in Settings rather than exposed toolbar selector.

- [x] Source/tests: only user-dragged columns save width overrides; automatic rendered changes stay temporary. Sidebar, inspector, and activity sizes only update on intentional resize, retaining chosen sizes through temporary window clamps.
- [x] Source/tests: Tab/Shift+Tab follow active-cell Right/Left navigation. Editing Tab maintains active-cell selection so Escape returns navigation ownership.
- [x] Source/tests: schema catalog completions resolve PostgreSQL qualified identifiers, unqualified tables, aliases, and qualified prefixes. Catalog currently limited to the loaded/selected schema.
- [x] Source/real-process tests: upstream PostgreSQL completion engine consumes loaded schema metadata offline and resolves unquoted table aliases/columns; no duplicate database connection.
- [ ] Full PostgreSQL semantic intelligence: Qualified CTE output aliases, quoted names and nested alias shadowing now have offline parsed-tree resolution and regressions. Unqualified correlated references and explicit JOIN LATERAL / CROSS JOIN LATERAL now have helper regressions; quoted CTE references through CROSS JOIN LATERAL also pass real editor-session completion/application. Comma-form LATERAL subqueries now have visibility/projection regressions and real editor-session quoted-CTE completion/application coverage. Recursive self-reference now resolves declared or anchor SELECT output columns, with branch-local FROM scope and real editor quoted-name coverage. Recursive forward references, SEARCH/CYCLE outputs, table functions, mixed/nested join trees and in-document DDL remain incomplete or unverified. Native QA verified alias suggestions/Tab acceptance/query execution with the completion helper confirmed alive; screenshots: postgres-schema-completion.png and postgres-completion-result.png.

- [x] Source/render tests: suggestion popup measures full text without phantom shortcut-column spacing; SELECT remains visible, compact20-point rows, bounded360-point width and full-label tooltip for longer entries.

- [x] Source/tests: server suggestions do not crowd out matching keywords/catalog entries; preserve server text edits and explicit empty-prefix ordering.
- [x] Source/layout tests: SQL actions right-aligned; SQL and table-footer action buttons share pagination-style rounded corners.

- [x] Source/live tests: PostgreSQL SQL editor defaults to autocommit using one request exchange and reused connection. Safe-mode settings share the query's Sync group; staged table edits retain transactions.
- [x] Source/live tests: explicit PostgreSQL SQL transactions/savepoints persist across runs, errors remain aborted until user rollback; table-save/export guards preserve user transaction ownership.
- [x] Source/worker tests: ordinary query completion avoids catalog reload; schema changes conservatively refresh.
- [x] Controlled latency: 30ms simulated RTT indexed lookup median38.2ms, equal to bare singleexchange reference38.2ms; legacy187.7ms. Production-server timing remains unmeasured.
- [x] Source/style/layout tests: SQL editor reference colors, neutral background and one bottom baseline for status/actions.

- [x] Readable, syntax-highlighted precommit SQL with inline typed values and matching clipboard output; bound execution unchanged.
- [x] Query-result footer: Data/Message, duration, centered row count, CSV Export, and preserved multiple-result selection. Basic bar/line/pie Chart view implemented and native QA verified; chart export, presets, comment configuration and Metrics Board remain open. Session-only result pins implemented with bounded retention; native pin/rerun/select/unpin QA verified. Loaded-result search implemented and native QA verified (selection, scroll, wrap, no matches, Escape). Focus-aware Cmd+F, Cmd+G/Shift+Cmd+G navigation, and grid focus restoration have rendered regression coverage.

- [x] PostgreSQL table-open/refresh reduced to two exchanges with fresh metadata, persistent connection and full elapsed measurement; controlled-WAN benchmark and metadata regression coverage.
- [x] Connection browser/setup visual cleanup with engine badges, endpoint subtitles, tags, transparent groups, rounded form cards and aligned actions.
- [x] Remove redundant editor separator/gap above results; query-result footer stays below rows.

- [x] PostgreSQL object catalog refresh combines relation/column metadata into one streamed typed query; schema switching avoids duplicate schema-list reload. Controlled 30ms RTT median 234.0ms → 82.9ms; live zero-column/generated/dropped-column/transaction regressions pass.
- [x] Repeat autocommit table loads pipeline fresh metadata and data on the existing session; validate the cached projection before display and retry changed schemas. Controlled 30ms RTT median 75.5ms → 41.3ms. First loads and explicit transactions remain sequential; see `PERFORMANCE.md`.

- [x] DATA-06 subset: header context menu explicitly selects ascending/descending or restores default order; click cycle retained. Rendered pointer-interaction regression verifies order and page reset. Multi-column ordering remains open.

Reference clarification (2026-10-31): TablePlus's documented loaded-result search filters rows by column/operator and supports staged replacement. Rustable currently provides literal matching-cell navigation only; column/operator filtering and replacement remain open. See https://api.tableplus.com/docs/query-editor/working-with-query-results.

SQL-06 progress (2026-10-31): added background Beautify (Cmd+I) for selection or whole document, preserving keyword case, with one-step undo/redo and stale-result rejection. Independent dialect token comparison rejects unsafe rewrites. Native QA verified format/undo/redo and toolbar; docs/SQL_FORMATTING.md describes bounds and known dialect limitations. Multi-caret/folding and complete dialect formatting remain open.

SQL-06 formatter follow-up: opaque-token preservation now supports PostgreSQL dollar bodies, nested/CRLF comments, escaped strings and SQLite quoted identifiers; adjacent literal separators preserved. Required-success regressions and SQLite value-equivalence test pass. Procedural-body internals remain untouched and unsupported token transformations still reject. Release worker benchmark: 105,780 bytes/2,000 statements p50 28.30ms, max-of-seven29.05ms.

KEY-03 progress: review/schema/history/import dialogs now own workspace shortcut routing. Review accepts explicit save confirmation and Escape-to-edit while blocking Run All, refresh, tab/file/search commands and staged-history actions behind it. Background toolbar/sidebar/inspector/workspace controls are disabled; editor-local completion/Beautify respect disabled state. Review retains an in-dialog Cancel save action while applying. Tests cover first-frame routing, disabled editor input, Safe Mode confirmation guard, retained staged edits on Escape and visible cancellation. Full suite341 passed/36 ignored; Clippy clean. This does not establish all shortcut contexts as audited.

KEY-03 progress: table Data view now routes Cmd/Ctrl+F to the existing Filters value field, preserving applied/draft filters until explicit application. Button opening focuses the same field. Rendered regression covers dialog ownership, actual field focus and no query dispatch/search leakage; existing SQL/result focus routing tests pass. Full keyboard audit remains open.

Editing/filter consistency: Apply and Clear validate the active cell draft before changing the applied filter or page. Invalid edits retain the exact displayed grid/filter/page and draft; regression covers both actions and successful Clear recovery. Advanced filter-builder parity remains open.

Grid transition audit: explicit/header sorting already guards invalid drafts. Pagination now validates before mutating offset, so rejected edits retain matching rows/page controls. Next-page offset uses checked arithmetic, previous at page zero does not reload. Regression covers both directions and recovery; full suite344 passed/36 ignored.

KEY-03 progress: Cmd/Ctrl+Left/Right now page backward/forward only with enabled table-grid focus, using the same guarded transition as the page buttons. Text input, SQL results, active drafts, modal dialogs and popups retain ownership. Rendered routing regression and full suite345 passed/36 ignored; native network-load focus restoration remains to verify.

Native keyboard QA follow-up: isolated SQLite fixture verifies Cmd+Right/Left page sequence1→2→3→2→1 without repeated clicks; Cmd+F focuses filter and Enter applies it. Grid explicitly locks arrow ownership even without selected rows after paging. Screenshots table-keyboard-page.png and table-keyboard-filter.png; full suite346 passed/36 ignored. Slow-network PostgreSQL focus testing remains open.

CON-09 progress: search-field Up/Down selects visible connections in displayed group order and scrolls the selected row into view. Search stays focused; dirty forms block switching through the existing guard; no connection opens from navigation. Rendered regression covers traversal/filter changes/dirty preservation. Duplication already has independent-ID/password-clearing coverage. Full suite347 passed/36 ignored; native keyboard-only workflow and scroll visibility still require QA before closing CON-09.

CON-09 native QA follow-up: isolated QA launcher confirms filtered Down selection, search focus retained for subsequent typing, no-match navigation preserving the current form, and unsaved form blocking Up selection with visible error. Temporary draft discarded; no profile saved or database connected. Evidence screenshots/connection-keyboard-dirty-guard.png. Long-list scrolling, duplicate-name navigation and restart coverage remain open.

CON-09 verification expanded: rendered60-profile regression proves ID-based traversal across duplicate names and final-row visibility inside the clipped list viewport, while retaining search focus. Serialized/restored store regression proves display-group order, collapsed skipping and group-name search revealing collapsed members. Full suite349 passed/36 ignored; Clippy clean. This closes automated long-list/duplicate-name/serialization coverage, not native long-list or full process-restart QA.

CON-09 keyboard completion: Enter from connection search uses the Connect button's validity/pending-password/pending-operation guard and existing Save/Open flow, only for the visible selected saved profile. It never opens a hidden search result or collapsed member. Rendered regression validates guards and requested profile identity. Full suite350 passed/36 ignored.

Native Enter-to-connect verified: search SQLite, Down, Enter opened the disposable fixture workspace and closed the manager; screenshot connection-keyboard-open.png. No SQL/data writes.

Export progress: loaded rows as JSON now available for table/query results, with captured grid snapshot, streamed typed encoding, cancellation, atomic publication and SQLite destination guards. Duplicate headers/non-finite numbers fail explicitly; binary is byte arrays. See JSON_EXPORT.md. Full-table JSON is now implemented for SQLite/PostgreSQL with filters/sorts across all pages, borrowed-row encoding and existing atomic publication. See JSON_EXPORT.md and PROGRESS.md for current verification.

Native table JSON export verified through format menu/macOS save dialog; exported500 fixture rows compare exactly with independent read-only SQLite values. Screenshot json-export-complete.png. Query-result menu native QA remains open.

Unqualified expression completion resolves columns from the current FROM scope, including CTE and derived-table outputs. Shared names offer alias-qualified edits; nested query sources remain isolated. Unknown/no-match expressions retain upstream keyword/function fallback.

Qualified outer aliases now resolve through expression subqueries (including nested EXISTS and scalar SELECTs), with nearest-alias shadowing and per-scope CTE bindings. This resolver does not cross FROM-derived-table or CTE-body boundaries; Unqualified correlated columns and explicit/comma-form LATERAL subqueries are now supported; table-function and mixed/nested join-tree visibility remain incomplete. Scope rules reference the [PostgreSQL table-expression documentation](https://www.postgresql.org/docs/17/queries-table-expressions.html).

Native QA on 2026-10-31 verified unqualified quoted CTE completion and qualified outer-column completion in a scalar correlated subquery. Both popups displayed the complete Display Name label without stale SELECT candidates; Tab accepted the quoted replacement, Cmd+Enter executed it, and each query returned 42. Evidence: screenshots/postgres-unqualified-completion.png, postgres-unqualified-result.png, postgres-correlated-completion.png and postgres-correlated-result.png.

Native connection identity verification (2026-10-31): searched SQLite, duplicated the fixture profile from its context menu while filtered, retained group/file, assigned a distinct name, staging environment and blue color, opened original and copy as simultaneous windows, and renamed the copy while both were open. Original remained local/green; duplicate updated its own title/header. QA restart preserved both profiles; searching the renamed copy and Down/Enter reconnected with correct styling. Screenshots: profile-identity-a.png, profile-identity-b.png, profile-identity-a-after-b-rename.png, profile-identity-b-renamed.png, profile-identity-restart.png and profile-identity-b-reconnected.png. CON-04a remains partial pending native endpoint-edit and full post-restart two-window retest; CON-09 duplicate display names and complete keyboard selection matrix remain open.

Native recovery regression verification (2026-10-31): the rebuilt QA launcher showed 24 retained drafts after filtering legacy blank scratch tabs. Opening an untouched SQLite session, quitting and relaunching kept the count at 24. Evidence: `screenshots/recovery-blank-migration.png` and `screenshots/recovery-blank-restart.png`. The automated recovery test separately preserves empty file-backed edits and their dirty saved baseline. Full TAB-04a concurrent failure/source-association coverage remains open.

PostgreSQL completion update (2026-10-31): unqualified correlated expression columns now resolve through enclosing scalar/EXISTS query scopes, respecting nearest column names and qualified ambiguity edits. Helper regression verifies inner alias shadowing cannot produce an invalid bare ambiguous replacement. Native acceptance for this new case remains open; full SQL-03 semantics are not complete.

Correlated-completion verification follow-up (2026-10-31): the real editor/helper subprocess test now accepts quoted bare outer columns and ambiguous qualified edits through the editor completion model, checking exact final SQL. Offline resolver latency is recorded in PERFORMANCE.md. Native popup acceptance for these specific new cases remains open.

PostgreSQL CSV import (2026-10-31): enabled the existing mapping/review workflow for PostgreSQL. Preview shows the COPY command and bounded mapped source samples. The driver validates fresh destination columns under a table lock, streams escaped COPY text from the immutable source snapshot, and rolls back on errors/cancellation. Tests cover 6,001 rows, reordered/omitted columns, exact numeric text, Unicode, multiline/tab/backslash values, NULL versus empty, default values, constraints, schema changes, explicit transaction rejection, cancellation before/during execution and connection reuse. Native wizard verification, permissions/RLS and partitioned-table cases remain open; broad import parity is not complete.

PostgreSQL import edge-case verification (2026-10-31): real-driver tests confirm range-partition routing, all-or-nothing missing-partition failure, restricted-role permission denial, active-RLS rejection and connection reuse. The RLS test uncovered and now guards a COPY-startup protocol cleanup fix in the vendored driver. RLS applies normally; COPY rejection is surfaced with the server hint to use INSERT. An INSERT-based RLS import path and column-only privileges remain open. Native wizard verification and large-file memory measurement remain open.

PostgreSQL RLS import mode (2026-10-31): CSV mapping now offers explicit bounded INSERT batches alongside default COPY. The reviewed request preserves the method; all batches execute atomically under normal row policies. Real-driver tests verify permitted rows, policy rejection after a prior batch with full rollback, precise numeric text, NULL/empty/backslash text, arrays/defaults and cancellation. This implements the earlier open INSERT-based RLS path; native wizard acceptance, column-only privileges and memory measurement remain open.

Import memory verification (2026-10-31): isolated release processes imported 1,000 and 1,000,000 rows through both PostgreSQL methods with approximately 11 MB peak client RSS and full server-side value verification. See PERFORMANCE.md for timings, source sizes and limitations. This closes the narrow-row scaling measurement gap; worst-case wide-record memory and native wizard verification remain open.

### Native PostgreSQL import verification — 2026-10-31

Verified small-file COPY and explicit INSERT through the isolated QA wizard on
the disposable PostgreSQL fixture. Reordered source columns map by name; quoted
and Unicode labels survive; an omitted column keeps its database default.
Cmd+Enter confirms the review and visible results refresh to four total rows.
Evidence: `screenshots/postgres-import-copy-mapping.png`,
`screenshots/postgres-import-copy-result.png`,
`screenshots/postgres-import-insert-result.png`.
Review sample formatting now uses named source strings/null rather than Rust
debug wrappers (unit tested; new presentation native verification pending).
Native error/cancel flows and column-only privileges remain unverified.

### PostgreSQL column-only import grants — 2026-10-31

Closed the driver-level column-only INSERT privilege gap: both COPY and INSERT
now pass live tests with INSERT granted only on mapped columns. Permission
denials, RLS and rollback tests remain passing. Lock acquisition is verified
through pg_locks; preparing the lock statement creates no rows and does not
evaluate a sequence default. Native restricted-role UI coverage remains open.

### Import review presentation and refresh follow-up — 2026-10-31

Native QA confirms readable named PostgreSQL source values and highlighted COPY
SQL, separated by labels, with selectable text/copy action. Screenshot:
`screenshots/postgres-import-highlighted-review.png`. No import was executed
during this visual check. Three modal keyboard/error tests pass.
An observed empty-Data-workspace refresh no-op is fixed for both shortcut and
toolbar with regression coverage; final native recheck is still pending.

### Empty-workspace refresh native acceptance — 2026-10-31

Closed the pending native verification: with no selected table and Safe mode
enabled, Cmd+R discovered a table created after connection; the toolbar refresh
then discovered a second newly created table. No SQL tab selection or query
execution was required. Evidence: `screenshots/postgres-empty-workspace-refresh.png`.
Complete regular suite: 357 passed, 41 ignored (live/benchmark opt-in cases).

### Import dialog keyboard guard follow-up (2026-10-31)

Grid navigation, selection, copy and deletion now share the workspace dialog
ownership guard, including CSV preview and review. Event-level regression covers
retained grid focus on dialog opening with ArrowDown, Tab, Cmd+A, Backspace and
native Copy. Events are not consumed and grid state/clipboard output remain
unchanged. KEY-03a remains open for the full native focus/context matrix.

### Native connection search/duplicate acceptance (2026-10-31)

Isolated QA, dark appearance, 1920x1784 screenshot: group search retained both
members and excluded unrelated profiles; arrow navigation selected each matching
profile without clearing search. Duplicate inherited group, environment, color
and SQLite path; renaming/saving did not rename the source and survived a
confirmed process restart. No-match search retained the form; clearing restored
the full list. Evidence: screenshots/connection-search-keyboard.png,
screenshots/connection-duplicate-saved.png, screenshots/connection-search-cleared.png,
and screenshots/connection-duplicate-restarted.png. CON-09 remains open for
same-name profiles and full keyboard-only context actions. Zero matches visibly
shrinks the sidebar/form split; layout stability needs follow-up.

Search split stability follow-up (2026-10-31): fixed the empty-result sidebar
shrink described above. Regression failed on the old layout and now checks form
position across matching, empty and cleared searches. All 23 connection tests
pass. Rebuilt native QA confirms unchanged separator/form positions in dark mode:
screenshots/connection-empty-search-stable-split.png and
screenshots/connection-cleared-search-stable-split.png. This closes that specific
layout issue, not the remaining CON-09 keyboard/same-name acceptance matrix.

Connection Test cancellation follow-up (2026-10-31): PostgreSQL handshake and
readiness futures now have per-phase ten-second deadlines and 20 ms cancellation
checks; the readiness transport task is aborted/awaited on return. Local stalled
protocol tests verify prompt cancellation and socket closure before authentication
and during SELECT 1. Live fixture confirms existing auth/database error field
classification and success. Native close-during-Test, TLS stalls and OS resolver
behavior remain separate acceptance gaps; do not close full CON-08 from these tests.

Readiness reporting follow-up (2026-10-31): authentication no longer marks the
Database field green prematurely. It stays running until the readiness query
returns, and readiness timeout marks Database alone red, preserving successful
network/authentication fields. Local wire-protocol regression verifies event order,
deadline failure and socket closure; live success/auth/database cases pass. Native
field-color acceptance remains outstanding.

### SQL transaction parity gap from driver audit (2026-10-31)

- [ ] **SQL-12:** Support explicit SQLite transactions across separate editor runs
  with visible transaction state and coordinated safe-mode, apply/import/export,
  cancel, close and reconnect behavior. Cross-run retention is now implemented;
  file-backed app/worker tests cover commit visibility, errors, cancellation,
  savepoints, safe-mode recovery and apply/import/export guards. Native SQLite
  acceptance verifies BEGIN, a separate SELECT, visible transaction state,
  blocked Cmd+Q and last-tab Cmd+W, and ROLLBACK recovery from a new SQL tab.
  Quit succeeds after rollback. Native reconnect and transport cases remain open. See
  [driver transaction audit](DRIVER_CAPABILITIES.md).

SQL-12 prerequisite (2026-10-31): transaction state now crosses the driver worker
boundary. Open/failed PostgreSQL transactions appear in the connection bar and
block close/quit/replacement, with COMMIT/ROLLBACK guidance; owner-thread guard
also rejects replacement. Live app/worker regression covers BEGIN, error,
ROLLBACK/COMMIT and subsequent replacement. Native acceptance and disconnected
transport cases remain open. SQLite cross-run retention is implemented as described
above; SQL-12 remains incomplete pending the remaining acceptance cases.

Transport recovery follow-up: live PostgreSQL app/worker coverage now verifies
backend termination during an open transaction and in autocommit. The next
operation publishes a distinct connection-lost state, permits reconnect/close,
and retains pending-edit guards. No automatic replay or inferred commit outcome.
Lost COMMIT acknowledgement is now verified with a real-server protocol proxy:
the server commits once while the client receives no success response, reports
unknown outcome, skips subsequent statements and never replays the write.
Native autocommit loss/close/reopen/query acceptance passed in isolated QA;
explicit-transaction native loss, idle detection and other timings remain open.
Safe-mode COMMIT/ROLLBACK recovery is now live-tested; parsed recovery grammar
keeps transaction settings, prepared-transaction commands and writes blocked.

### Native completion follow-up

- [x] Quoted VALUES alias: full popup label, Tab insertion and successful read-only query.
- [x] Recursive CTE self-reference: suggestion/Tab insertion and bounded 1,2,3 result.
- [x] Comma LATERAL preceding-source reference: quoted suggestion/Tab insertion and result.
- [x] Explicit LEFT JOIN LATERAL column alias list: full quoted `Display Name`
  suggestion, Tab insertion and `hello` result with safe mode enabled. Evidence:
  postgres-lateral-alias-list-completion.png and postgres-lateral-alias-list-result.png.
- [x] Clear candidates and replacement edits when the caret leaves its completion
  context. Native recursive-column popup now contains only `step`; Tab insertion
  and execution return 1,2,3. Evidence: postgres-recursive-completion-clean.png
  and postgres-recursive-completion-clean-result.png.

Evidence in docs/screenshots: postgres-values-alias-completion.png,
postgres-values-alias-result.png, postgres-recursive-completion.png,
postgres-recursive-result.png, postgres-lateral-completion.png,
postgres-lateral-result.png. Broader function and query-scope semantics remain open.

CON-03 keyboard destination follow-up: Move to group now receives initial keyboard focus when appropriate, and opening it transfers focus to its first enabled destination across the submenu sizing pass. Rendered regression covers Shift+F10 → Enter → Down → Enter, preserved profile identity and no connection request. Native destination-submenu verification and nested groups remain open.

### Table context menu and compound filters (2026-11-01)

- Right-click inside the selected set preserves all selected rows and moves the
  focused cell; right-click outside selects the clicked row.
- Context menu exposes inspector editing, NULL/empty values, refresh, paste into
  a reviewed value editor, add/duplicate, row/cell copy formats, quick filters,
  CSV import, current-page export, and staged deletion/restoration.
- Copy formats: plain text, CSV with/without headers, JSON, escaped HTML and
  Markdown, and dialect-aware INSERT. Copies include staged cell values.
- Compact filter rows support enable toggles, column/operator/value, Raw SQL,
  per-condition Apply, add/remove, Clear, Apply All, WHERE-clause copy, and export.
  Shared predicates cover SQLite, MySQL/MariaDB, and PostgreSQL browse/export.
- Raw SQL currently accepts comparison expressions, boolean combinations, LIKE,
  IN, BETWEEN and NULL checks. Functions/subqueries/comments are rejected rather
  than allowing a filter to execute side effects. IN values use JSON arrays.
- Selected cells currently means the focused column across selected rows. Paste
  assigns one value via the editor; rectangular multi-column TSV paste is not
  implemented by this change. Quick Look uses the existing inspector.
- Validation: 462 native unit tests passed, 107 environment/benchmark tests
  ignored; strict all-target clippy passed (existing vendor warnings remain).
  MySQL 43316, MariaDB 43317, PostgreSQL 54329 fixture browse/export tests passed.
  Locked WASM check and 5 packaging tests passed. Native isolated preview verified
  single numeric filtering, adding/clearing rules, and preserving three selected
  rows on right-click. No main push or release performed.
