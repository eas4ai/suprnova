//! BIND-005: a route key that is not a column fails the build.

use suprnova::model;

#[model(table = "posts", route_key = "slug")]
pub struct Post {
    pub id: i64,
    pub title: String,
}
