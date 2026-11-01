# Query result charts

Data / Message / Chart switches the selected result between the grid, execution
message, and a chart. Charting uses loaded rows only, including pinned results;
it does not run another query or fetch more data.

Bar and line charts support up to eight numeric Y columns. Choose the label/X
column separately. Numeric text is accepted, including PostgreSQL numeric values.
NULL, nonnumeric and nonfinite values are omitted and counted. Lines have gaps
at missing Y values. Numeric line X values use numeric spacing when every X value
is finite; otherwise X is categorical in result order. No sorting or aggregation
is applied. Bar charts group the selected series for each loaded row.

Pie has an independent, single numeric Value selector and the chosen label column.
Its scrolling legend shows labels, original values and percentages, beside the
pie on wide panes and below it on narrow panes. Bar/line series selections are
preserved when switching chart modes. Negative
values produce an explanatory message; all-zero data produces no pie. Hovering a
bar, line point or pie slice shows original values, preserving their exact text
for inspection even though chart coordinates use floating-point approximation.

Chart options remain in the editor session. Numeric series are cached by result
identity and recomputed when the result or Y selection changes; the cache holds
a weak source reference. Closing an editor or changing connections clears cached
chart data. A changed result column layout resets chart selections. Labels are drawn sparsely for large results, and all
loaded rows still contribute geometry. Bars/pie use batched meshes, lines use
paths, and individual line point markers are omitted above 200 rows. Extreme
finite ranges are normalized before subtraction to avoid infinite coordinates.

Remaining parity: chart PNG export, saved chart presets, comment-directed chart
configuration, and Metrics Board. The existing footer Export action still exports
result data as CSV, as its tooltip states. Chart-specific export is not implemented.

Tests cover conversion, source replacement, extreme ranges, negative pie values,
missing values, and finite tessellated geometry for all three modes. A manual
`chart_render_latency` benchmark measures frame generation plus tessellation for
5,000 loaded rows with eight series (one for pie), without GPU/presentation time.

Native macOS QA verified bar/line/pie switching and adding the cost series to
revenue. [Two-series bar screenshot](screenshots/query-result-chart.png).

Release benchmark on the local Apple M4 Max: 20 measured frames after five
warmups, 1200×650 logical viewport, 5,000 rows. CPU frame+tessellation p50/p95:
bar (8 series) 0.63/0.66 ms; line (8 series) 0.77/0.78 ms; pie (1 series)
0.09/0.09 ms. GPU and presentation time are excluded. Command:
`mise exec rust@stable -- cargo test --locked --release chart_render_latency -- --ignored --nocapture`.

Pie refinement verification: 330 regular tests passed, 35 opt-in tests ignored;
strict all-target/all-feature Clippy passed. Render tests verify legend text and
percentages at narrow and wide sizes, and independent pie selection/reset behavior.
Native QA verified switching revenue to cost updates both slices and legend.
[Pie legend screenshot](screenshots/query-result-pie.png). Both app bundles rebuilt.
