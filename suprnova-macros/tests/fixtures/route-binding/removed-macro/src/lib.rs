//! BIND-001: the `route_binding!` macro is removed. This crate must fail
//! to compile.

pub mod user {
    pub struct Entity;
    pub struct Model;
}

suprnova::route_binding!(user::Entity, user::Model, "user");
