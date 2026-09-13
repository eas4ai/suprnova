# One built-in chart, server-rendered SVG through charts-rs; no client charting runtime

Level: Consequential
Decided by: Shawn
Rests on: DATA-004
Would be wrong if: a chart's marks require browser script to appear, a plain GET lacks the chart's data and textual alternative, or a charting library enters a reviewed artifact

## Decision

Chosen 2026-09-13 10:54 after three candidates: charts-rs (Apache-2.0, SVG with the default build, 22 chart types including candlestick) renders on the server into the Askama view, so the chart is in the fetched document, updates ride re-render or streams, and no client runtime ships. Dropped: TradingView Lightweight Charts (Apache-2.0, canvas) as a second built-in - the developer judged charts-rs good enough for the default and dropped the other; ApexCharts as a third - it is revenue-licensed (free under two million USD annual revenue, commercial above), which a framework default must not impose on its users. Live spec 25's visualization-container contract stands for bring-your-own libraries, documented with one manual example. charts-rs enters the workspace as a dependency of the library crate through the audit, feature matrix, and license inventory.

## Realized by

- f241aba8  docs(cairn,live): apply the owner's component-library rulings to the specs
