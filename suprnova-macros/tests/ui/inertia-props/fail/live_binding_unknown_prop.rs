//! A live binding cannot name a prop the struct does not serialize.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia(live(topic = "posts", only = ["posts", "missing"]))]
pub struct DashboardProps {
    pub posts: Vec<String>,
    pub count: usize,
}

fn main() {}
