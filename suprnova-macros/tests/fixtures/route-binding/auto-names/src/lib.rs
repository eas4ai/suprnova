//! BIND-001: `AutoRouteBinding` is a second name for `RouteBinding`, through
//! the crate root and through `suprnova::database`, and `from_route_param`
//! still resolves by the route key. This crate must compile.

use suprnova::model;

#[model(table = "pages", route_key = "slug")]
pub struct Page {
    pub id: i64,
    pub slug: String,
}

pub async fn through_both(value: &str) -> Result<(Page, Page), suprnova::FrameworkError> {
    let first = <Page as suprnova::AutoRouteBinding>::from_route_param(value).await?;
    let second = <Page as suprnova::database::AutoRouteBinding>::from_route_param(value).await?;
    Ok((first, second))
}

pub fn route_key_name() -> &'static str {
    <Page as suprnova::RouteBinding>::route_key_name()
}
