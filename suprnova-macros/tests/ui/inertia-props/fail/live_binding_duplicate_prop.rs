//! A live binding lists each affected prop once.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia(live(topic = "posts", only = ["posts", "posts"]))]
pub struct DashboardProps {
    pub posts: Vec<String>,
}

fn main() {}
