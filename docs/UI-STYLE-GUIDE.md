# Rustable UI standards

The SQL viewer is the reference for controls, typography and density. Redis Insight
is a reference for information architecture and workflows, not a second visual theme.
Use the app's semantic theme colors and Lucide icons in both light and dark modes.

## Tables and lists

Every table needs a deliberate visual container, a distinct header, aligned cells,
consistent row height, restrained alternating rows and a clear empty state. Do not
ship raw grids or stacks of ungrouped labels. Use `controls::report_table` for bounded
three-column reports; use the SQL grid or styled `TableBuilder` for interactive data.

- Key/name columns receive the flexible width. Keep numbers right-aligned and give
  units in the header or value; show human-readable memory with exact bytes on hover.
- Use monospace for identifiers and code, regular UI text for labels and metrics.
- Clip long cells and offer the full value on hover or in a detail view. Never let
  long names push actions offscreen. Headers and cells share alignment and padding.
- Make selection and hover intentional. Only give a row a pointer affordance if it
  performs an action. Group lists in surfaces with a heading and count; provide
  loading, empty, error and populated states, not just the happy path.
- Keep pagination below its table. Preserve virtualization and bounded page sizes.
- Review the entire view at narrow and wide sizes, with long names, empty results,
  and both themes. A functional table is not automatically a finished table.

## Actions and feedback

Use `controls::BUTTON_HEIGHT` and shared action/icon buttons. Icon-only actions need
meaningful hover text and accessible labels. Keep related actions together and
match SQL toolbar spacing; avoid oversized flat buttons or ad hoc geometry.

Every copy action confirms the click in the clicked control: clipboard → check and
“Copy” → “Copied” temporarily, with stable dimensions. Use the existing
`redis_value_editor::copy` helper for Redis controls. Do not call `copy_text` directly
from a new Redis button. Feedback must reset and settle at idle; no perpetual redraw.
Clipboard feedback means the app issued the platform copy request, not a clipboard
read-back verification. Errors and unavailable actions must be visible and specific.

Save/edit operations need visible pending, success and failure states. Preserve
unsaved drafts on failure. Place key-level actions next to the key name, value actions
next to the value. Close removes the inspector; no empty reserved right pane.

## Editors

Match SQL syntax colors, font size, line height, selection, shortcuts and controls.
Completion offers command/argument help, keyboard navigation, Tab/Enter acceptance,
Escape dismissal and Ctrl+Space invocation. Command/Ctrl+Enter always executes;
completion must never swallow it. Replacement affects only the current token, keeps
surrounding text and handles Unicode, quoting and binary-safe key insertion.

Never query Redis on each keystroke. Use bounded local metadata and loaded names;
show the scope honestly. No automatic execution on completion selection. Test
first use, after idle, keyboard/mouse acceptance, focus loss and large input.

## Design review is part of implementation

Before calling UI work complete, inspect actual screenshots of every changed view
and its interactions. Compare with SQL and the supplied Redis Insight references.
Check baseline alignment, padding, row density, action placement, copy feedback,
empty states and responsive layouts. Record performance evidence per
[UI-PERFORMANCE.md](UI-PERFORMANCE.md); screenshots alone cannot establish responsiveness.
