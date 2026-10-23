//! BIND-010: `#[derive(RouteBinding)]` binds unit variants only.

#[derive(suprnova::RouteBinding)]
pub enum Shape {
    Dot,
    Line(u32),
}
