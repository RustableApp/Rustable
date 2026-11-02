# Table previews and relationships

Single-clicking a table in Items opens an italic preview. Browsing another table
reuses a preview tab. Double-click the table in Items or its tab to keep it open;
its title becomes non-italic. Starting an edit also keeps the tab, even before
saving the change.

Foreign-key cells show a small arrow. Clicking it always appends a new preview
tab with the referenced columns filtered to the source row's values. It does not
keep the source tab. This also applies to self-references and tables that already
have open tabs: each tab has its own identity, filters, sorting, and page.
Composite foreign keys apply all referenced columns together. Relationship
browsing appears in Activity, not manually executed query history.

Navigation uses declared database foreign keys. Null, binary, and non-finite key
values cannot be followed. PostgreSQL references can cross schemas. MySQL and
MariaDB references currently require the target database to be the current
workspace database; cross-database references display an explanation instead.

Numbers use normal foreground text and right alignment. The arrow, rather than
number color, indicates a relationship. Editing by double-click requires both
clicks to target the same cell.
