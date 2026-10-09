//! A `#[derive(Data)]` DTO with a route-param field compiles in a crate that
//! depends on `suprnova` alone. This fixture crate has no `url` dependency,
//! like a freshly scaffolded application.

#![allow(dead_code)]

#[derive(Debug, suprnova::Data, validator::Validate)]
struct UpdatePost {
    #[data(from_route_param)]
    pub id: i64,

    pub title: String,
}

fn main() {}
