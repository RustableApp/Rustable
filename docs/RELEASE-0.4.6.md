# Rustable v0.4.6

- Keep only the first SQL editor per connection/database as the persistent scratchpad. Closing and reopening restores that editor; additional SQL tabs are temporary for the connection session. Older multi-tab snapshots recover only the first editor.
- Hide horizontal scrollbars in the workspace and result tab strips to prevent the scrollbar appearing/disappearing and shifting the content below. Overflowing tabs remain horizontally scrollable.
- Faster PostgreSQL change review and saves, particularly over SSH: combine catalog queries and eliminate duplicate metadata reads while preserving transaction, row-conflict and schema-change checks.
- Single-cell and other row edits no longer refresh the entire database catalog after committing. Structure changes still refresh it.
- Show immediate preparation feedback in the review window and progress while saving, with cancellation available.
- Cache native dependencies, pinned language servers and WASM tooling in GitHub Actions to reduce repeat build work.

The website WASM demo is built from this same release tag. The previously reported background OpenGL crash remains unconfirmed; opt-in crash reporting is unchanged.
