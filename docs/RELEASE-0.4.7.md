# Rustable v0.4.7

- Restore the active OpenGL context before rendering on macOS, fixing a reproduced texture-creation crash when a native callback clears the current context.
- Drag across cells to select a rectangular range, or drag the row-number gutter to select complete rows. The red outline shows exactly what Command+C copies as plain text, including staged values.
- Extend the blue row highlight across the full table viewport, including unused space to the right.
- Correct shifted connection-test validation outlines and checkmarks.
- Add consistent padding to the indexes view and fill the unused space at its right edge.

The website demo is built from the same release tag. The rendering regression reproduces the reported panic; extended background and sleep/wake testing is still needed to confirm every trigger is covered.
