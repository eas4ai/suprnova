//! `render_chart` draws bounded typed series as trusted SVG and rejects
//! everything outside its bounds. The SVG carries no literal color or font:
//! every mark carries a class that `chart.css` colors from a `--sn-` token,
//! so the chart follows the light and dark themes (DATA-007).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

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

/// The properties that paint a color, as SVG attributes or CSS declarations.
const COLOR_PROPERTIES: [&str; 8] = [
    "fill",
    "stroke",
    "color",
    "stop-color",
    "flood-color",
    "lighting-color",
    "background",
    "background-color",
];

/// The number of series classes, one per palette token.
const PALETTE: usize = 6;

fn live_file(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// A bar chart and a line chart with the most series a chart takes, so the
/// palette wraps.
fn twelve_series_charts() -> [(&'static str, String); 2] {
    let series: Vec<ChartSeries> = (1..=12_u8)
        .map(|n| ChartSeries::new(format!("s{n}"), vec![f32::from(n), 2.0, 3.0]))
        .collect();
    [("bar", ChartKind::Bar), ("line", ChartKind::Line)].map(|(name, kind)| {
        let chart = render_chart(kind, &["Apr", "May", "Jun"], &series)
            .unwrap_or_else(|error| panic!("a {name} chart with 12 series renders: {error}"));
        (name, chart.as_str().to_owned())
    })
}

/// `none` and `currentColor` paint no literal; a `var(--sn-...)` reads a
/// token.
fn is_token_or_keyword(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("currentColor")
        || (value.starts_with("var(--sn-") && value.ends_with(')'))
}

/// `property: value` pairs of a `style` attribute or a CSS rule body.
fn declarations(text: &str) -> Vec<(String, String)> {
    text.split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(property, value)| (property.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// One opening or self-closing SVG tag, with the classes of the `<g>`
/// groups around it, since a class on a group styles what it holds.
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    group_classes: Vec<String>,
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(attribute, _)| attribute == name)
            .map(|(_, value)| value.as_str())
    }

    fn own_classes(&self) -> Vec<String> {
        self.attribute("class")
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    fn classes(&self) -> BTreeSet<String> {
        self.own_classes()
            .into_iter()
            .chain(self.group_classes.iter().cloned())
            .collect()
    }

    fn has_class(&self, class: &str) -> bool {
        self.classes().contains(class)
    }

    /// The `N` of every `sn-chart-series-N` class the element carries.
    fn series(&self) -> BTreeSet<usize> {
        self.classes()
            .iter()
            .filter_map(|class| class.strip_prefix("sn-chart-series-")?.parse().ok())
            .collect()
    }

    /// Every color the element paints, from its attributes and its `style`.
    fn colors(&self) -> Vec<(String, String)> {
        let mut colors: Vec<(String, String)> = self
            .attributes
            .iter()
            .filter(|(name, _)| COLOR_PROPERTIES.contains(&name.as_str()))
            .cloned()
            .collect();
        colors.extend(
            declarations(self.attribute("style").unwrap_or(""))
                .into_iter()
                .filter(|(property, _)| COLOR_PROPERTIES.contains(&property.as_str())),
        );
        colors
    }

    fn describe(&self) -> String {
        let attributes: Vec<String> = self
            .attributes
            .iter()
            .map(|(name, value)| format!("{name}=\"{value}\""))
            .collect();
        format!("<{} {}>", self.name, attributes.join(" "))
    }
}

/// The elements of an SVG document in document order.
fn elements(svg: &str) -> Vec<Element> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut found = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("every tag closes") + start;
        let tag = &rest[start + 1..end];
        rest = &rest[end + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            if name.trim() == "g" {
                groups.pop();
            }
            continue;
        }
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (name, mut attributes_text) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let mut attributes = Vec::new();
        while let Some((attribute, value_text)) = attributes_text.split_once('=') {
            let value_text = value_text.trim_start();
            let quote = value_text.chars().next().expect("an attribute value");
            let (value, after) = value_text[1..]
                .split_once(quote)
                .expect("a closed attribute value");
            attributes.push((attribute.trim().to_owned(), value.to_owned()));
            attributes_text = after;
        }
        let element = Element {
            name: name.to_owned(),
            attributes,
            group_classes: groups.concat(),
        };
        if element.name == "g" && !self_closing {
            groups.push(element.own_classes());
        }
        found.push(element);
    }
    found
}

/// The first hex color or color function anywhere in the SVG; an
/// `url(#id)` or `href="#id"` reference is not a color.
fn literal_color_in(svg: &str) -> Option<String> {
    for function in ["rgb(", "rgba(", "hsl(", "hsla(", "lab(", "lch("] {
        if svg.contains(function) {
            return Some(function.to_owned());
        }
    }
    svg.match_indices('#').find_map(|(index, _)| {
        let before = &svg[..index];
        if before.ends_with("url(") || before.ends_with("href=\"") {
            return None;
        }
        let digits: String = svg[index + 1..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        (matches!(digits.len(), 3 | 4 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| format!("#{digits}"))
    })
}

/// The innermost `selector { body }` rules of a stylesheet, comments
/// removed; a rule inside `@layer` or `@media` keeps its own selector.
fn css_rules(css: &str) -> Vec<(String, String)> {
    let mut text = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        text.push_str(&rest[..start]);
        let comment = &rest[start..];
        rest = comment.find("*/").map_or("", |end| &comment[end + 2..]);
    }
    text.push_str(rest);
    let mut rules = Vec::new();
    let mut boundary = 0;
    let mut open: Option<(String, usize)> = None;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'{' => {
                let selector = text[boundary..index].rsplit(';').next().unwrap_or("");
                open = Some((selector.trim().to_owned(), index + 1));
                boundary = index + 1;
            }
            b'}' => {
                if let Some((selector, body_start)) = open.take() {
                    rules.push((selector, text[body_start..index].to_owned()));
                }
                boundary = index + 1;
            }
            _ => {}
        }
    }
    rules
}

/// Whether `selector` names `.class` itself, not a longer class it prefixes.
fn selects_class(selector: &str, class: &str) -> bool {
    let needle = format!(".{class}");
    selector.match_indices(&needle).any(|(index, _)| {
        selector[index + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '-' || next == '_'))
    })
}

#[test]
fn chart_tokens_the_svg_carries_no_literal_color_or_font_family() {
    for (name, svg) in twelve_series_charts() {
        for element in elements(&svg) {
            for (property, value) in element.colors() {
                assert!(
                    is_token_or_keyword(&value),
                    "{name} chart: {property}=\"{value}\" is a literal color on {}",
                    element.describe()
                );
            }
        }
        if let Some(literal) = literal_color_in(&svg) {
            panic!("{name} chart: the SVG carries the literal color {literal}");
        }
        assert!(
            !svg.contains("font-family"),
            "{name} chart: the SVG names a font family"
        );
    }
}

#[test]
fn chart_tokens_the_svg_draws_no_background() {
    for (name, svg) in twelve_series_charts() {
        let elements = elements(&svg);
        let root = elements.first().expect("the chart has a root element");
        assert_eq!(root.name, "svg", "{name} chart");
        assert!(
            !root.attribute("style").unwrap_or("").contains("background"),
            "{name} chart: the root {} paints a background",
            root.describe()
        );
        let size = (root.attribute("width"), root.attribute("height"));
        for element in &elements {
            let at_origin = ["x", "y"]
                .iter()
                .all(|axis| element.attribute(axis).is_none_or(|value| value == "0"));
            let covers = (element.attribute("width"), element.attribute("height")) == size;
            let paints_background = element.name == "rect"
                && at_origin
                && covers
                && element.attribute("fill") != Some("none");
            assert!(
                !paints_background,
                "{name} chart: {} fills the whole canvas, so the background is not transparent",
                element.describe()
            );
        }
    }
}

#[test]
fn chart_tokens_text_axis_and_grid_lines_carry_their_classes() {
    for (name, svg) in twelve_series_charts() {
        let elements = elements(&svg);
        for text in elements.iter().filter(|element| element.name == "text") {
            assert!(
                text.has_class("sn-chart-text"),
                "{name} chart: {} carries no sn-chart-text class",
                text.describe()
            );
        }
        // A `<line>` is an axis line, a grid line, or a series mark (part of
        // a legend swatch), and says which.
        let (mut axis, mut grid) = (0, 0);
        for line in elements.iter().filter(|element| element.name == "line") {
            let roles = usize::from(line.has_class("sn-chart-axis"))
                + usize::from(line.has_class("sn-chart-grid"))
                + line.series().len();
            assert_eq!(
                roles,
                1,
                "{name} chart: {} carries {:?}, not exactly one of sn-chart-axis, \
                 sn-chart-grid, sn-chart-series-N",
                line.describe(),
                line.classes()
            );
            axis += usize::from(line.has_class("sn-chart-axis"));
            grid += usize::from(line.has_class("sn-chart-grid"));
        }
        assert!(axis > 0, "{name} chart: no line carries sn-chart-axis");
        assert!(grid > 0, "{name} chart: no line carries sn-chart-grid");
    }
}

#[test]
fn chart_tokens_series_marks_take_the_series_classes_in_order_and_wrap_after_six() {
    // Series n of 12 carries sn-chart-series-((n - 1) % 6 + 1), so series 7
    // takes the first palette color again.
    let palette: Vec<usize> = (0..12).map(|index| index % PALETTE + 1).collect();
    for (name, svg) in twelve_series_charts() {
        let mut order: Vec<usize> = Vec::new();
        for element in elements(&svg) {
            // Every shape other than `<line>` is a series mark: a bar, a
            // line chart's path or point, or a legend swatch. A `<line>`
            // may be one too.
            let shape = matches!(
                element.name.as_str(),
                "rect" | "circle" | "ellipse" | "path" | "polyline" | "polygon"
            );
            if !shape && element.name != "line" {
                continue;
            }
            let series = element.series();
            if shape {
                assert_eq!(
                    series.len(),
                    1,
                    "{name} chart: the series mark {} carries {:?}, not one sn-chart-series-N class",
                    element.describe(),
                    element.classes()
                );
            }
            if let Some(&number) = series.first()
                && order.last() != Some(&number)
            {
                order.push(number);
            }
        }
        // The legend and the plot each run through the 12 series in order.
        assert!(
            !order.is_empty()
                && order.len().is_multiple_of(palette.len())
                && order.chunks(palette.len()).all(|run| run == palette),
            "{name} chart: series classes in document order are {order:?}, expected runs of {palette:?}"
        );
    }
}

#[test]
fn chart_tokens_chart_css_colors_every_chart_class_from_a_token() {
    let rules = css_rules(&live_file("components/chart/chart.css"));
    let mut classes: Vec<(String, Option<String>)> = ["text", "axis", "grid"]
        .iter()
        .map(|role| (format!("sn-chart-{role}"), None))
        .collect();
    classes.extend((1..=PALETTE).map(|n| {
        (
            format!("sn-chart-series-{n}"),
            Some(format!("var(--sn-color-chart-{n})")),
        )
    }));
    for (class, series_token) in classes {
        let styled: Vec<(String, String)> = rules
            .iter()
            .filter(|(selector, _)| selects_class(selector, &class))
            .flat_map(|(_, body)| declarations(body))
            .filter(|(property, _)| {
                COLOR_PROPERTIES.contains(&property.as_str()) || property == "font-family"
            })
            .collect();
        assert!(
            styled
                .iter()
                .any(|(property, value)| property != "font-family" && value != "none"),
            "chart.css does not color .{class}"
        );
        for (property, value) in &styled {
            assert!(
                is_token_or_keyword(value) && !value.contains(','),
                "chart.css styles .{class} with {property}: {value}, not a var(--sn-...) token"
            );
        }
        if let Some(token) = series_token {
            for (property, value) in &styled {
                assert!(
                    property == "font-family" || value == &token || !value.starts_with("var("),
                    "chart.css colors .{class} with {property}: {value}, not {token}"
                );
            }
            assert!(
                styled.iter().any(|(_, value)| value == &token),
                "chart.css does not color .{class} from {token}"
            );
        }
    }
}

#[test]
fn chart_tokens_chart_css_gives_the_chart_its_font_from_a_token() {
    let rules = css_rules(&live_file("components/chart/chart.css"));
    let fonts: Vec<(String, String)> = rules
        .iter()
        .flat_map(|(selector, body)| {
            declarations(body)
                .into_iter()
                .filter(|(property, _)| property == "font-family" || property == "font")
                .map(|(_, value)| (selector.clone(), value))
        })
        .collect();
    // The figure holds the title, the SVG and the summary, so a font set on
    // it reaches every piece of the chart's text, whatever font the
    // document or a container around the chart sets.
    assert!(
        fonts
            .iter()
            .any(|(selector, value)| selector == ".sn-chart" && value == "var(--sn-font-sans)"),
        "chart.css does not set font-family: var(--sn-font-sans) on .sn-chart; its font rules: {fonts:?}"
    );
    for (selector, value) in &fonts {
        assert!(
            value.starts_with("var(--sn-font-") && value.ends_with(')') && !value.contains(','),
            "chart.css sets the font of {selector} to {value}, not a var(--sn-font-...) token"
        );
    }
}

#[test]
fn chart_tokens_the_token_stylesheet_defines_the_chart_palette_for_light_and_dark() {
    let rules = css_rules(&live_file("browser/src/styles/suprnova-ui.css"));
    for (scheme, wanted) in [
        ("light", ":root"),
        ("dark, from the system", ":root:not([data-theme=\"light\"])"),
        ("dark, pinned", ":root[data-theme=\"dark\"]"),
    ] {
        let declared: BTreeSet<String> = rules
            .iter()
            .filter(|(selector, _)| selector.split(',').any(|one| one.trim() == wanted))
            .flat_map(|(_, body)| declarations(body))
            .filter(|(_, value)| !value.is_empty())
            .map(|(property, _)| property)
            .collect();
        for n in 1..=PALETTE {
            let token = format!("--sn-color-chart-{n}");
            assert!(
                declared.contains(&token),
                "suprnova-ui.css does not define {token} for {scheme} ({wanted})"
            );
        }
    }
}
