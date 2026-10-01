//! Server-rendered charts for the data-display component family.
//!
//! The library owns the one built-in chart: `charts-rs` draws the marks as
//! SVG on the server from typed series, and the result is trusted markup
//! because framework code assembled it from numbers and the labels the
//! application passed as its own literals. No client charting runtime
//! exists; the chart macro renders the SVG beside a text summary and a data
//! table, so the canonical document reads without the picture.

use std::error::Error;
use std::fmt;

use charts_rs::{BarChart, LineChart, Series};

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

/// Renders one chart as trusted SVG markup.
///
/// `labels` name the x axis positions and every series carries one value
/// per label. Labels and names are bounded and may not carry markup
/// characters, so the SVG text nodes hold exactly what the caller passed.
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
    let svg = match kind {
        ChartKind::Bar => BarChart::new(list, axis).svg(),
        ChartKind::Line => LineChart::new(list, axis).svg(),
    }
    .map_err(|_| ChartError::new(ChartErrorKind::Render))?;
    let reason = TrustedMarkupReason::new("charts-rs SVG from bounded typed series")
        .map_err(ChartError::from)?;
    TrustedHtml::framework_generated(svg, reason).map_err(ChartError::from)
}
