# render_chart panics inside charts-rs for values from 1e12

Surfaced from: DATA-004
Captured: 2026-09-17T11:53:47.808Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/src/view/charts.rs checks labels and finiteness, then charts-rs 1.0.0 overflows in src/charts/util.rs line 410 (attempt to multiply with overflow). Reproduced in a debug build: a bar chart whose largest value is 1e10 renders; 1e12, 1e15, 1e20, 1e30 and f32::MAX panic. render_chart returns Result, and public-surface code must not panic. A release build, where overflow checks are off by default, was not run.
