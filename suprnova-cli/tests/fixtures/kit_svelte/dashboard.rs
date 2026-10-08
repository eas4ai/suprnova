use serde::Serialize;
use suprnova::{handler, InertiaProps, InertiaResponse, Request, Response};

use super::notes::NoteSummary;

#[derive(Serialize)]
pub struct Stats {
    pub notes: u64,
    pub written_today: u64,
}

#[derive(InertiaProps)]
pub struct DashboardProps {
    pub stats: Stats,
    pub recent_notes: Vec<NoteSummary>,
}

#[handler]
pub async fn index(req: Request) -> Response {
    InertiaResponse::new("Dashboard").with_data(DashboardProps { stats, recent_notes })
}
