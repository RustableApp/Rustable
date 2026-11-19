# Rustable v0.4.34

- See query results as rows arrive, without waiting for the complete result to load. Progressive loading also works in the regular table Data view. Large query results retain disk-backed storage and virtual scrolling.
- Keep the selected row visible in the right-hand inspector while results stream in, fixing repeated flashing during loading.
- Pick up tables and columns changed in another tool without reconnecting. Use **Refresh schema** beside sidebar search or **Cmd/Ctrl+Shift+R**; open table tabs refresh their cached metadata too.

Progressive display preserves your SQL and does not reduce the database's total work. Queries that must sort or aggregate before returning rows can still delay the first visible result. Browser demo results retain their existing bounded loading behavior.
