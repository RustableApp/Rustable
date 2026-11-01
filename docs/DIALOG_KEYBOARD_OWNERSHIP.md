# Dialog keyboard ownership

Workspace shortcuts are checked against dialog state before they inspect input.
This covers the first frame of a newly opened dialog, before egui transfers focus.

Review Changes owns Cmd+S, Cmd+Shift+S and Cmd+Enter confirmation. These still use
the existing save path, including Safe Mode and pending-operation guards.
Cmd+Shift+Enter is consumed without running SQL. Escape returns to editing only
when no operation is pending, retaining staged changes. While applying, Cancel
save remains available inside the dialog and invokes the existing worker cancel
path.

Other connection/history/schema/import dialogs block workspace shortcut dispatch.
Import review retains its existing local keyboard handler. Inline column rename
remains an editing context rather than a separate dialog. Background toolbar,
sidebar, inspector and workspace controls are disabled while dialogs are active.
SQL completion and Beautify explicitly respect that disabled state.

Tests exercise first-frame refresh/tab/run/search/discard blocking, background
SQL typing/formatting suppression, Safe Mode confirmation, Escape retention, and
Cancel save visibility. Full suite: 341 passed, 36 opt-in tests ignored; strict
all-target/all-feature Clippy clean. Verification for this change is automated;
no fresh native interaction capture was made. Remaining shortcut contexts still
need audit; see KEY-03 in TABLEPLUS_PARITY_REQUIREMENTS.md.
