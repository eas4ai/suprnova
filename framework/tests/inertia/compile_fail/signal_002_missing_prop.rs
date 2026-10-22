use suprnova::InertiaProps;
#[derive(InertiaProps)]
#[inertia(live(topic = "posts", only = ["missing"]))]
struct Page { posts: Vec<String> }
fn main() {}
