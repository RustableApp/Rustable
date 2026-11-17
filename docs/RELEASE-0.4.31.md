# Rustable v0.4.31

- Indent selected SQL lines with Tab and unindent with Shift+Tab without replacing the selection. Undo and redo restore the whole indentation action.
- Improve editor undo grouping around typing, deletion, selection replacement, cursor movement and composition.
- Right-click sidebar tables to export CSV or JSON. Use Command/Shift selection for bulk export, with one file per table and all saved rows included.
- Add table actions for opening a new tab, viewing definitions, copying names, truncating and dropping selected tables. Destructive actions always require confirmation and respect safe mode and unsaved edits.
- Match sidebar context menus to the table-view style. Review destructive actions with clearly listed tables, connection details and syntax-highlighted SQL.

The website demo is built from the same release tag. Sidebar file export is available in the desktop app.
