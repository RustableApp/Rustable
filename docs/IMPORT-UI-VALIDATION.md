# Connection import UI validation — 2026-11-06

The shared TablePro/TablePlus dialog now introduces connection exports before
opening a file picker, provides format guidance, and groups the preview into
cards with a consistent action footer. Cancelling the picker returns to the
introduction; errors allow choosing another file. Import counts exclude skipped
duplicates. Sidebar double-click uses the same validation/password-loading gates
and opening path as Connect; single-click selection and dragging remain intact.

## Functional checks

- 36 connection-manager tests passed, including double-click in normal and
  filtered lists and single-click selection without opening.
- 22 import-related tests passed; nine opt-in tests were not run in this command.
- Added headless checks for picker cancellation and dialog bounds in light/dark
  themes at 800 × 700 logical points.
- Native visual review, mixed-DPI behavior, input-to-display latency, and OS file
  picker interaction were not manually verified. No user database was contacted.

## CPU performance

Baseline: `21b0395bc3f02b22b776fc293719a34f633e5f7b`. Apple M4 Max,
macOS 27.2 arm64, Rust 1.99.1, release profile. Five alternating baseline/candidate
pairs, run serially after both builds completed. Each fixture uses 20 warmup and
100 measured frames at 1200 × 900 logical points, default scale, with a synthetic
120 Hz input timeline. This measures CPU layout plus tessellation, not native FPS.
The identical `import_dialog_frame_benchmark` fixture was added to the baseline
worktree without changing its production code. Preview uses 40 synthetic
connections; the other state is the export-password form.

| Fixture | Base median p95 | New median p95 | Change |
| --- | ---: | ---: | ---: |
| Light password form | 0.0098 ms | 0.0180 ms | +82.6% |
| Light preview | 0.0965 ms | 0.1080 ms | +12.0% |
| Dark password form | 0.0099 ms | 0.0174 ms | +76.4% |
| Dark preview | 0.1017 ms | 0.1170 ms | +15.0% |

These increases exceed the relative 10% investigation threshold. The added
0.008–0.015 ms per frame is accepted here for the requested export guidance,
readable grouping, padding and consistent controls. A text-only layout would be
cheaper but retain the visual problem this change addresses. These costs are
limited to the open import dialog; no new idle repaint loop or database round
trip was added. File reading and decryption remain on background workers.

The standard `scripts/ui-performance.py --base HEAD` comparison completed but
was **inconclusive** due to noisy results in several unrelated grid/sidebar
metrics; it is not recorded as a pass. Raw logs, metadata, p50/p95/p99/max values,
and reports are retained locally in `target/ui-performance-import-dialog/` and
`target/import-ui-comparison/`. The focused comparison helper is retained in
`target/compare-import-ui.py`. No performance thresholds were weakened.
