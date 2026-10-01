//! A `#[handler]` parameter written as a pattern, `RouteParam(user):
//! RouteParam<User>` (the form the `RouteParam` docs show), binds the route
//! parameter named after the pattern's binding, `user`, exactly as the
//! plain `user: RouteParam<User>` form does.
//!
//! Driven through `handle_request` on a loopback socket, so the route
//! parameter really comes from the matched path.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{
    MiddlewareRegistry, Model, Response, RouteParam, Router, attrs, handle_request, handler, model,
};

use crate::http_wire;

#[model(table = "rd_users", fillable = ["name"])]
pub struct RdUser {
    pub id: i64,
    pub name: String,
}

#[handler]
pub async fn show_destructured(RouteParam(user): RouteParam<RdUser>) -> Response {
    text(format!("destructured {}", user.name))
}

#[handler]
pub async fn show_destructured_mut(RouteParam(mut user): RouteParam<RdUser>) -> Response {
    user.name.push('!');
    text(format!("mut {}", user.name))
}

#[handler]
pub async fn show_plain(user: RouteParam<RdUser>) -> Response {
    text(format!("plain {}", user.name))
}

async fn boot() -> (TestDatabase, SocketAddr) {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rd_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .unwrap();
    RdUser::create(attrs! { name: "Alice" }).await.unwrap();

    let router: Router = Router::new()
        .get("/destructured/{user}", show_destructured)
        .get("/destructured-mut/{user}", show_destructured_mut)
        .get("/plain/{user}", show_plain)
        .into();
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    (db, addr)
}

async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let (status, _headers, body) = http_wire::request(addr, "GET", path, &[]).await;
    (status, body)
}

#[tokio::test]
async fn destructured_route_param_binds_the_parameter_named_by_its_binding() {
    let (_db, addr) = boot().await;

    assert_eq!(
        get(addr, "/destructured/1").await,
        (200, "destructured Alice".to_string())
    );
}

#[tokio::test]
async fn destructured_mut_binding_names_the_parameter_too() {
    let (_db, addr) = boot().await;

    assert_eq!(
        get(addr, "/destructured-mut/1").await,
        (200, "mut Alice!".to_string())
    );
}

#[tokio::test]
async fn destructured_route_param_still_answers_404_for_a_missing_row() {
    let (_db, addr) = boot().await;

    let (status, body) = get(addr, "/destructured/999").await;

    assert_eq!(status, 404, "body: {body}");
}

#[tokio::test]
async fn plain_route_param_binds_as_before() {
    let (_db, addr) = boot().await;

    assert_eq!(
        get(addr, "/plain/1").await,
        (200, "plain Alice".to_string())
    );
}
