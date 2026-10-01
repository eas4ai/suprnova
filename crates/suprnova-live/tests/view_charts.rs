//! `render_chart` draws bounded typed series as trusted SVG and rejects
//! everything outside its bounds.

use suprnova_live::view::charts::{ChartErrorKind, ChartKind, ChartSeries, render_chart};

fn series(values: Vec<f32>) -> Vec<ChartSeries> {
    vec![ChartSeries::new("Revenue", values)]
}

#[test]
fn a_bar_chart_and_a_line_chart_render_as_svg_with_the_labels_and_the_series_name() {
    for kind in [ChartKind::Bar, ChartKind::Line] {
        let chart = render_chart(
            kind,
            &["Apr", "May", "Jun"],
            &series(vec![42.0, 47.0, 51.0]),
        )
        .expect("a bounded series renders");
        let svg = chart.as_str();
        assert!(svg.starts_with("<svg"), "{svg}");
        for needle in ["Apr", "Jun", "Revenue"] {
            assert!(svg.contains(needle), "missing {needle} in {svg}");
        }
        assert!(!svg.contains("<script"), "no script in the marks");
    }
}

#[test]
fn a_series_whose_length_differs_from_the_labels_is_a_shape_error() {
    let error = render_chart(ChartKind::Bar, &["Apr", "May"], &series(vec![1.0]))
        .expect_err("two labels and one point");
    assert_eq!(error.kind(), ChartErrorKind::Shape);
    let error = render_chart(ChartKind::Bar, &[], &series(Vec::new())).expect_err("no labels");
    assert_eq!(error.kind(), ChartErrorKind::Shape);
    let error = render_chart(ChartKind::Bar, &["Apr"], &[]).expect_err("no series");
    assert_eq!(error.kind(), ChartErrorKind::Shape);
}

#[test]
fn markup_in_a_label_or_a_name_and_a_non_finite_value_are_input_errors() {
    let error = render_chart(ChartKind::Bar, &["<img src=x>"], &series(vec![1.0]))
        .expect_err("markup in a label");
    assert_eq!(error.kind(), ChartErrorKind::Input);
    let error = render_chart(
        ChartKind::Bar,
        &["Apr"],
        &[ChartSeries::new("a\"b", vec![1.0])],
    )
    .expect_err("a quote in a name");
    assert_eq!(error.kind(), ChartErrorKind::Input);
    let error = render_chart(ChartKind::Line, &["Apr"], &series(vec![f32::NAN]))
        .expect_err("a non-finite value");
    assert_eq!(error.kind(), ChartErrorKind::Input);
    let error =
        render_chart(ChartKind::Line, &[""], &series(vec![1.0])).expect_err("an empty label");
    assert_eq!(error.kind(), ChartErrorKind::Input);
}

#[test]
fn too_many_points_or_series_are_rejected_before_rendering() {
    let labels: Vec<String> = (0..513).map(|i| format!("l{i}")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let error =
        render_chart(ChartKind::Bar, &refs, &series(vec![1.0; 513])).expect_err("513 points");
    assert_eq!(error.kind(), ChartErrorKind::TooLarge);
    let many: Vec<ChartSeries> = (0..13)
        .map(|i| ChartSeries::new(format!("s{i}"), vec![1.0]))
        .collect();
    let error = render_chart(ChartKind::Bar, &["Apr"], &many).expect_err("13 series");
    assert_eq!(error.kind(), ChartErrorKind::TooLarge);
}

#[test]
fn data_006_every_finite_series_renders_or_is_refused_without_panicking() {
    // The magnitudes that stress the renderer's integer axis arithmetic:
    // zero, tiny, fractional, the step boundaries it branches on, and the
    // accepted bound itself, with both signs, as single points and as pairs.
    let magnitudes = [
        0.0,
        f32::MIN_POSITIVE,
        1.0e-30,
        0.05,
        0.5,
        1.0,
        9.0,
        99.0,
        499.0,
        999.0,
        4_999.0,
        9_999.0,
        60_000.0,
        1.0e6,
        3.3e8,
        9.99e8,
        1.0e9,
    ];
    let values: Vec<f32> = magnitudes
        .iter()
        .flat_map(|magnitude| [*magnitude, -*magnitude])
        .collect();
    for kind in [ChartKind::Bar, ChartKind::Line] {
        for first in &values {
            render_chart(kind, &["Apr"], &series(vec![*first]))
                .unwrap_or_else(|error| panic!("{kind:?} [{first}] was refused: {error}"));
            for second in &values {
                render_chart(kind, &["Apr", "May"], &series(vec![*first, *second])).unwrap_or_else(
                    |error| panic!("{kind:?} [{first}, {second}] was refused: {error}"),
                );
            }
        }
    }
}

#[test]
fn data_006_a_value_beyond_the_renderer_range_is_an_input_error() {
    for kind in [ChartKind::Bar, ChartKind::Line] {
        for value in [1.000_001e9, 1.0e12, -1.0e12, 1.0e30, f32::MAX, f32::MIN] {
            let error = render_chart(kind, &["Apr", "May"], &series(vec![1.0, value]))
                .expect_err("a value beyond 1e9");
            assert_eq!(error.kind(), ChartErrorKind::Input, "{kind:?} {value}");
        }
    }
}
