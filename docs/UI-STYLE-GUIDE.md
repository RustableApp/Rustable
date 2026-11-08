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
- Paint one continuous header across column gutters; avoid separate boxed header cells.
  Header and striped row bounds must match, with 10 px between their outer edges
  and the first/last text. Include TableBuilder’s half-gutter expansion when painting
  backgrounds; verify geometry instead of judging a full-page thumbnail alone.
  Put footer counts and paging inside the table surface with a single divider.
- Paired report panels share a height; use whole-row scroll viewports so the bottom
  does not show a stray strip of the next row. Leave clear spacing after summary text.
- Row hover must persist over child labels and controls, with clipping and overlapping
  window checks retained. Child text must not punch holes in the row highlight.
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
unsaved drafts on failure. Keep value format controls right-aligned on the Value
heading row, vertically centered. Put byte/range metadata on the left and Copy on
the right of one footer row. Place key-level actions next to the key name, value actions
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


## Native pointer conventions

On macOS, use NSEvent's system double-click interval instead of a fixed toolkit
value. Keep semantic same-target checks for opening/editing, and preserve ordinary
single-click selection and Control-click context menus. Test slower valid clicks,
clicks outside the interval, and adjacent targets; do not tune thresholds by feel.
