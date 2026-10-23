//! Live bindings name serialized props, not Rust field identifiers.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[serde(rename_all = "camelCase")]
#[inertia(live(topic = "counts", only = ["post_count"]))]
pub struct DashboardProps {
    pub post_count: usize,
}

fn main() {}
