//! Server-rendered charts for the data-display component family.
//!
//! The library owns the one built-in chart: `charts-rs` draws the marks as
//! SVG on the server from typed series, and the result is trusted markup
//! because framework code assembled it from numbers and the labels the
//! application passed as its own literals. No client charting runtime
//! exists; the chart macro renders the SVG beside a text summary and a data
//! table, so the canonical document reads without the picture.
//!
//! The SVG carries no color or font of its own (DATA-007). charts-rs draws
//! each role with a sentinel color, and the renderer turns every sentinel
//! into `currentColor` plus a class that `chart.css` colors from a `--sn-`
//! token, so the chart follows the document's light or dark theme, and the
//! chart's text takes the `--sn-font-sans` font `chart.css` sets on the
//! figure.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Once;

use charts_rs::{BarChart, Color, LineChart, Series, THEME_LIGHT, add_theme, get_theme};

use crate::view::{TrustedHtml, TrustedMarkupError, TrustedMarkupReason};

/// The longest label or series name the chart accepts, in bytes.
const MAX_LABEL_BYTES: usize = 64;
/// The most points one series may carry.
const MAX_POINTS: usize = 512;
/// The most series one chart may carry.
const MAX_SERIES: usize = 12;
/// The largest magnitude a point may carry. `charts-rs` computes its axis
/// steps in `i32` and overflows once a series spans much more than 2e9, so
/// a larger value is refused before it reaches the renderer (DATA-006).
const MAX_VALUE_MAGNITUDE: f32 = 1.0e9;

/// The charts-rs theme the chart renders with: the light theme's layout, a
/// transparent background, and a sentinel color for every role.
const THEME_NAME: &str = "suprnova-live-tokens";

/// A sentinel color: opaque, so charts-rs writes no opacity beside it, and
/// unlike any color charts-rs writes on its own.
const fn sentinel(index: u8) -> Color {
    Color {
        r: 1,
        g: 2,
        b: index,
        a: 255,
    }
}

const TEXT: Color = sentinel(0);
const AXIS_TEXT: Color = sentinel(1);
const AXIS: Color = sentinel(2);
const GRID: Color = sentinel(3);
/// One color per chart palette token. charts-rs picks a series color by the
/// series index modulo the list length, so series 7 takes the first again.
const SERIES: [Color; 6] = [
    sentinel(4),
    sentinel(5),
    sentinel(6),
    sentinel(7),
    sentinel(8),
    sentinel(9),
];

/// The classes that replace each sentinel. Axis labels keep the text class
/// and add their own, so `chart.css` can mute them.
const CLASSES: [(Color, &str); 10] = [
    (TEXT, "sn-chart-text"),
    (AXIS_TEXT, "sn-chart-text sn-chart-axis-text"),
    (AXIS, "sn-chart-axis"),
    (GRID, "sn-chart-grid"),
    (SERIES[0], "sn-chart-series-1"),
    (SERIES[1], "sn-chart-series-2"),
    (SERIES[2], "sn-chart-series-3"),
    (SERIES[3], "sn-chart-series-4"),
    (SERIES[4], "sn-chart-series-5"),
    (SERIES[5], "sn-chart-series-6"),
];

/// Which mark the chart draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartKind {
    /// One bar per point.
    Bar,
    /// One line through the points.
    Line,
}

/// One named series of numeric points.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartSeries {
    name: String,
    values: Vec<f32>,
}

impl ChartSeries {
    /// Names a series and takes its points; the chart rejects it if either
    /// bound is exceeded.
    #[must_use]
    pub fn new(name: impl Into<String>, values: Vec<f32>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }
}

/// Why a chart was not rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartErrorKind {
    /// No series, no label, or a series whose length differs from the labels.
    Shape,
    /// A label or series name is empty or exceeds the bound, or a value is
    /// not finite or its magnitude exceeds 1e9.
    Input,
    /// Too many series or points.
    TooLarge,
    /// The renderer failed or its output exceeded the trusted markup bound.
    Render,
}

/// Rejection of a chart request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChartError {
    kind: ChartErrorKind,
}

impl ChartError {
    const fn new(kind: ChartErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection class.
    #[must_use]
    pub const fn kind(self) -> ChartErrorKind {
        self.kind
    }
}

impl fmt::Display for ChartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ChartErrorKind::Shape => "chart_shape_invalid",
            ChartErrorKind::Input => "chart_input_invalid",
            ChartErrorKind::TooLarge => "chart_too_large",
            ChartErrorKind::Render => "chart_render_failed",
        })
    }
}

impl Error for ChartError {}

impl From<TrustedMarkupError> for ChartError {
    fn from(_: TrustedMarkupError) -> Self {
        Self::new(ChartErrorKind::Render)
    }
}

fn check_label(label: &str) -> Result<(), ChartError> {
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || label
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '&' | '"'))
    {
        return Err(ChartError::new(ChartErrorKind::Input));
    }
    Ok(())
}

/// Registers the sentinel theme once and returns its name. The light
/// theme's font family stays in the theme because charts-rs measures label
/// widths with it; `classify` drops it from the markup.
fn token_theme() -> &'static str {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let mut theme = (*get_theme(THEME_LIGHT)).clone();
        theme.background_color = Color::transparent();
        theme.title_font_color = TEXT;
        theme.sub_title_font_color = TEXT;
        theme.legend_font_color = TEXT;
        theme.series_label_font_color = TEXT;
        theme.x_axis_font_color = AXIS_TEXT;
        theme.y_axis_font_color = AXIS_TEXT;
        theme.x_axis_stroke_color = AXIS;
        theme.y_axis_stroke_color = AXIS;
        theme.grid_stroke_color = GRID;
        theme.series_colors = SERIES.to_vec();
        add_theme(THEME_NAME, theme);
    });
    THEME_NAME
}

/// The classes a `#RRGGBB` paint stands for, when it is a sentinel.
fn sentinel_classes(paint: &str) -> Option<&'static str> {
    let digits = paint.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    CLASSES
        .iter()
        .find(|(color, _)| (color.r, color.g, color.b) == (r, g, b))
        .map(|(_, classes)| *classes)
}

fn render_error() -> ChartError {
    ChartError::new(ChartErrorKind::Render)
}

/// Rewrites the charts-rs SVG so it carries no literal color or font: a
/// sentinel paint becomes `currentColor` and the element takes the
/// sentinel's classes, a transparent paint becomes `none`, and the font
/// family goes. Any other color is refused rather than shipped.
fn classify(svg: &str) -> Result<String, ChartError> {
    let mut out = String::with_capacity(svg.len() + svg.len() / 4);
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let end = start + rest[start..].find('>').ok_or_else(render_error)?;
        classify_tag(&rest[start + 1..end], &mut out)?;
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Writes one tag, the text between `<` and `>`, rewritten. charts-rs
/// writes every attribute as `name="value"` with `"`, `<` and `>` escaped,
/// so a value never holds the delimiters this scan relies on.
fn classify_tag(tag: &str, out: &mut String) -> Result<(), ChartError> {
    if tag.starts_with('/') {
        let _ = write!(out, "<{tag}>");
        return Ok(());
    }
    let (body, close) = tag
        .strip_suffix('/')
        .map_or((tag, ">"), |body| (body, "/>"));
    let (name, mut rest) = body.split_once(' ').unwrap_or((body, ""));
    let mut attributes = Vec::new();
    while let Some((attribute, tail)) = rest.split_once("=\"") {
        let (value, after) = tail.split_once('"').ok_or_else(render_error)?;
        attributes.push((attribute.trim(), value));
        rest = after;
    }
    // charts-rs writes a transparent paint as black with a zero opacity.
    let transparent = |opacity: &str| {
        attributes
            .iter()
            .any(|&(attribute, value)| attribute == opacity && value == "0")
    };
    let mut classes: Vec<&str> = Vec::new();
    out.push('<');
    out.push_str(name);
    for &(attribute, value) in &attributes {
        let kept = match attribute {
            "font-family" => None,
            "fill-opacity" | "stroke-opacity" if value == "0" => None,
            "fill" | "stroke" => {
                if value == "none" || transparent(&format!("{attribute}-opacity")) {
                    Some("none")
                } else {
                    classes.extend(sentinel_classes(value).ok_or_else(render_error)?.split(' '));
                    Some("currentColor")
                }
            }
            "class" => {
                classes.extend(value.split(' '));
                None
            }
            "color" | "stop-color" | "flood-color" | "lighting-color" | "style" => {
                return Err(render_error());
            }
            _ => Some(value),
        };
        if let Some(value) = kept {
            let _ = write!(out, " {attribute}=\"{value}\"");
        }
    }
    let mut unique: Vec<&str> = Vec::with_capacity(classes.len());
    for class in classes {
        if !class.is_empty() && !unique.contains(&class) {
            unique.push(class);
        }
    }
    if !unique.is_empty() {
        let _ = write!(out, " class=\"{}\"", unique.join(" "));
    }
    out.push_str(close);
    Ok(())
}

/// Renders one chart as trusted SVG markup.
///
/// `labels` name the x axis positions and every series carries one value
/// per label. Labels and names are bounded and may not carry markup
/// characters, so the SVG text nodes hold exactly what the caller passed.
/// The SVG has a transparent background and no color or font of its own:
/// text carries `sn-chart-text` (axis labels add `sn-chart-axis-text`), the
/// axis `sn-chart-axis`, the grid `sn-chart-grid`, and series `n` (from 1)
/// `sn-chart-series-{(n - 1) % 6 + 1}`, which `chart.css` colors from the
/// `--sn-` tokens.
pub fn render_chart(
    kind: ChartKind,
    labels: &[&str],
    series: &[ChartSeries],
) -> Result<TrustedHtml, ChartError> {
    if labels.is_empty() || series.is_empty() {
        return Err(ChartError::new(ChartErrorKind::Shape));
    }
    if labels.len() > MAX_POINTS || series.len() > MAX_SERIES {
        return Err(ChartError::new(ChartErrorKind::TooLarge));
    }
    for label in labels {
        check_label(label)?;
    }
    let mut list = Vec::with_capacity(series.len());
    for entry in series {
        check_label(&entry.name)?;
        if entry.values.len() != labels.len() {
            return Err(ChartError::new(ChartErrorKind::Shape));
        }
        if entry
            .values
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_VALUE_MAGNITUDE)
        {
            return Err(ChartError::new(ChartErrorKind::Input));
        }
        list.push(Series::new(entry.name.clone(), entry.values.clone()));
    }
    let axis = labels.iter().map(|label| (*label).to_owned()).collect();
    let theme = token_theme();
    let svg = match kind {
        ChartKind::Bar => BarChart::new_with_theme(list, axis, theme).svg(),
        ChartKind::Line => LineChart::new_with_theme(list, axis, theme).svg(),
    }
    .map_err(|_| render_error())?;
    let svg = classify(&svg)?;
    let reason = TrustedMarkupReason::new("charts-rs SVG from bounded typed series")
        .map_err(ChartError::from)?;
    TrustedHtml::framework_generated(svg, reason).map_err(ChartError::from)
}
