# Platform context clicks

macOS Control-click is normalized to a secondary click before egui processes
pointer state. Existing menus and grid selection receive the same events as a
physical secondary click; Control-click cannot also activate the primary action.
The normalizer is registered once on the shared egui context, covering the
launcher, immediate database viewports, and browser/demo entry points.
Windows/Linux Ctrl-click and macOS Command-click retain their existing behavior.
Native secondary-click events, including OS-configured trackpad gestures, pass
through unchanged. This change does not add keyboard context-menu shortcuts.

The logical button is selected on mouse-down and remembered through release,
including when Control is released first. State is separated by viewport,
cleared on focus loss, and pruned when a window disappears. Work is bounded by
incoming events and live pressed viewports, with no database calls or repaint
requests.

Validation against `d61c95d`:

- Four focused regression tests pass: actual egui popup versus primary action on
  Mac/Windows/Linux, modifier ordering, viewport isolation/cleanup, and unchanged
  secondary/Command-click events. Full working-tree suite: 610 passed, 135 ignored
  (includes separate uncommitted import tests). Strict library/test Clippy passes.
- Standard release-mode comparison ran from an isolated checkout containing only
  this change. Initial five-pair timing was noisy. Exact-binary five-pair repeat:
  18 metrics passed, zero confirmed regressions, one inconclusive metric
  (`grid/vertical precise/tessellation`, baseline 0.045791 ms vs 0.050250 ms,
  +9.7%, noisy paired ratios). This is not an all-metrics performance pass.
- Additional identical input-frame workload alternates disabled/enabled plugin
  across five pairs, 20 warmups and 600 samples per run, 800x600 logical viewport,
  default scale, synthetic 120 Hz timeline, pointer movement, Control press/release
  and idle frames. Median p95: 0.003542 ms disabled, 0.002625 ms enabled. These
  microsecond-scale measurements show no detected overhead, not a claimed speedup.
- Raw initial/repeat data and environment/toolchain metadata are retained under
  `target/context-click-ui-performance/` and
  `target/context-click-ui-performance-repeat/` (including `focused-input.log`).

Native macOS disposable-preview smoke check: Control-click opened the table-cell
context menu with the expected actions. Screenshot: `/tmp/rustable-control-click.png`.

CPU timing does not measure input-to-display latency. Physical trackpad, native
Windows/Linux, mixed-DPI and 60/120 Hz display acceptance are not covered by these
automated checks.
