//! Live bindings are repeatable and name serialized props.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[serde(rename_all = "camelCase")]
#[inertia(live(topic = "posts", only = ["posts", "postCount"]))]
#[inertia(live(topic = "counts", only = ["postCount"]))]
pub struct DashboardProps {
    pub posts: Vec<String>,
    pub post_count: usize,
}

#[derive(InertiaProps)]
#[inertia(cacheable)]
#[inertia(live(topic = "articles", only = ["articles", "viewer"]))]
pub struct CachedProps {
    #[inertia(public)]
    pub articles: Vec<String>,
    #[inertia(per_request)]
    pub viewer: String,
}

fn main() {
    let bindings = <DashboardProps as suprnova::inertia::InertiaProps>::live_bindings();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].topic, "posts");
    assert_eq!(bindings[0].only, &["posts", "postCount"]);
    assert_eq!(bindings[1].topic, "counts");
    assert_eq!(bindings[1].only, &["postCount"]);
    assert_eq!(
        <&DashboardProps as suprnova::inertia::InertiaProps>::live_bindings(),
        bindings
    );

    let cached = <CachedProps as suprnova::inertia::InertiaProps>::live_bindings();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].topic, "articles");
    assert_eq!(cached[0].only, &["articles", "viewer"]);
}
