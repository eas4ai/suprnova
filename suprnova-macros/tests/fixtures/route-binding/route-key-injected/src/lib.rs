//! BIND-005: an injected field such as `__eager` is no column, so it cannot
//! be a route key.

use suprnova::model;

#[model(table = "posts", route_key = "__eager")]
pub struct Post {
    pub id: i64,
    pub title: String,
}
