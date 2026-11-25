# Rustable v0.4.41

- Edit PostgreSQL `citext` values directly in SQL query results. Case and Unicode are preserved; `citext` domains and arrays also display as text instead of opaque binary data.
- Make double- and triple-clicking consistent across the SQL editor, connection list and other controls. macOS uses AppKit click counts; Windows uses native double-click events and system settings.
- Select words and paragraphs on mouse-down, and extend selection by whole words or paragraphs when dragging. macOS paragraph selection includes its line ending.
- Select the existing value when entering cell editing, so typing replaces it. Undo restores the original selected value.
- Open the clicked connection reliably on double-click, while preserving unsaved-edit safeguards.
- Make the query confirmation Send button clearly readable in every theme, including its keyboard shortcut. Fixes #19.
- Add native Browse controls for certificate and private-key paths, including Redis and Sentinel credentials and MySQL/MariaDB CA files. Cancelling preserves the current path, and a late picker result cannot modify a different connection draft. Fixes #20.
- Refresh the application icon and its generated desktop assets.

Linux and other backends without native click counts use improved shared timing, distance and target checks; exact desktop preference parity is not yet implemented there.
