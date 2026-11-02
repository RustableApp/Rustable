# Rustable v0.4.0

## Connections and navigation

- Import TablePlus connection exports, including password-protected exports and nested groups. Preview imported profiles and review settings before connecting.
- Follow declared foreign keys using cell arrows. Each relationship opens a new preview tab with the matching filters applied, including composite keys.
- Single-click table previews use italic titles. Double-click a table or tab, or edit a value, to keep it open. Relationship navigation preserves the source preview.
- New connection windows start without an unsolicited SQL tab. The toolbar SQL button opens a fresh editor each time.

## Interface improvements

- Inter typography throughout the UI, with medium controls and semibold headings. SQL and data remain monospaced.
- Connection search above the list, bottom-anchored management actions, selectable groups with folder icons, indented connections, and clear context-menu selection.
- Inspector values are directly editable inputs; changes remain staged until Save.
- Right-aligned numbers, cleaner relationship arrows, compact filter controls, and Copy SQL confirmation.
- SQL editor footer spacing and consistent Redis row styling.
- Double-click editing requires both clicks on the same cell.
- Query history contains user-run queries, excluding automatic table browsing.
- Studio demo is hidden in production desktop builds and remains available in the website demo and QA builds.

The website WASM demo is updated from this release's exact source tag.

## Current limits

TablePlus import supports the connection types and settings documented in
[TABLEPLUS_IMPORT.md](https://github.com/RustableApp/Rustable/blob/v0.4.0/docs/TABLEPLUS_IMPORT.md).
MySQL/MariaDB foreign-key navigation across databases is not yet supported.
Binary and null foreign-key values cannot be followed.

macOS Apple Silicon packages are signed/notarized with Sparkle updates.
Windows/Linux x64 packages use manual updates. Built-in SSH tunnels remain
macOS/Linux only. Native Windows/Linux GUI acceptance and installed old-to-new
Sparkle update acceptance remain unverified.
