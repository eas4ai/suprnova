//! The data-display gallery: every presentational component of the family
//! and the server-rendered chart on one island.
//!
//! The statistics, the plan card, the team card, the keyed activity list
//! and the chart all read state this island holds; `reorder` reverses the
//! activity list so the morph fixture and the browser case can prove that
//! every keyed item keeps its node, and `refresh` re-renders the chart from
//! the same series so a plain re-render updates the marks.

use serde::{Deserialize, Serialize};
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};
use suprnova::live::{LiveComponent, live};
use suprnova::view::TrustedHtml;

/// The checked filters the gallery's views use: the loop-key filter for the
/// activity items and the trusted markup filter for the chart marks.
pub mod filters {
    pub use suprnova::view::filters::{live_key, trusted_html};
}

/// One keyed activity entry with a status badge.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Activity {
    /// The stable domain key.
    pub key: String,
    /// What happened.
    pub text: String,
    /// The badge text.
    pub status: String,
    /// The badge variant.
    pub variant: String,
}

/// One point of the revenue chart's data table.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChartPoint {
    /// The month.
    pub label: String,
    /// The revenue, in thousands.
    pub value: u32,
}

const MONTHS: [&str; 6] = ["Apr", "May", "Jun", "Jul", "Aug", "Sep"];
const REVENUE: [u32; 6] = [42, 47, 51, 49, 58, 64];

#[derive(LiveComponent)]
#[live(
    name = "app.data-display-gallery",
    view = "live/data-display-gallery.html"
)]
pub struct DataDisplayGallery {
    /// Revenue this month, in thousands.
    pub revenue: u32,
    /// Churn this month, in percent.
    pub churn: String,
    /// Seats in use.
    pub seats: u32,
    /// The plan owner.
    pub owner: String,
    /// The owner's initials.
    pub owner_initials: String,
    /// The members line of the description list.
    pub members_text: String,
    /// The keyed activity entries.
    pub activity: Vec<Activity>,
    /// Revenue per month, in thousands; the chart draws these.
    pub revenue_series: Vec<u32>,
}

#[live]
impl DataDisplayGallery {
    /// Seeds the gallery with fixed figures.
    #[mount]
    pub fn mount() -> Self {
        Self {
            revenue: 64,
            churn: "1.8".to_owned(),
            seats: 37,
            owner: "Ada Lovelace".to_owned(),
            owner_initials: "AL".to_owned(),
            members_text: "3 of 5 seats".to_owned(),
            activity: [
                ("act-1", "Invoice 1042 paid", "Paid", "success"),
                ("act-2", "Seat added for Grace", "Change", "info"),
                ("act-3", "Card expires next month", "Warning", "warning"),
                ("act-4", "Export failed", "Failed", "error"),
            ]
            .into_iter()
            .map(|(key, text, status, variant)| Activity {
                key: key.to_owned(),
                text: text.to_owned(),
                status: status.to_owned(),
                variant: variant.to_owned(),
            })
            .collect(),
            revenue_series: REVENUE.to_vec(),
        }
    }

    /// Reverses the activity list; every item keeps its key.
    #[action]
    pub fn reorder(&mut self) {
        self.activity.reverse();
    }

    /// Moves the last month up, so a plain re-render changes the marks.
    #[action]
    pub fn refresh(&mut self) {
        if let Some(last) = self.revenue_series.last_mut() {
            *last += 1;
        }
    }
}

/// The chart marks, drawn on the server from the island's series.
pub fn chart_svg(values: &[u32]) -> TrustedHtml {
    let values = values.iter().map(|value| *value as f32).collect();
    render_chart(
        ChartKind::Bar,
        &MONTHS,
        &[ChartSeries::new("Revenue", values)],
    )
    .expect("a bounded fixed series renders")
}

/// The text summary of the chart.
pub fn chart_summary(values: &[u32]) -> String {
    let first = values.first().copied().unwrap_or_default();
    let last = values.last().copied().unwrap_or_default();
    format!(
        "Revenue rose from {first}k in {} to {last}k in {}, with one dip in {}.",
        MONTHS[0], MONTHS[5], MONTHS[3]
    )
}

/// The chart's data table.
pub fn chart_points(values: &[u32]) -> Vec<ChartPoint> {
    MONTHS
        .iter()
        .zip(values.iter().copied())
        .map(|(label, value)| ChartPoint {
            label: (*label).to_owned(),
            value,
        })
        .collect()
}
