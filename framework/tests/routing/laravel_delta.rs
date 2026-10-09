use hyper::Method;
use std::sync::Arc;
use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Response, Router, any, group, query, routes};

async fn answer(request: Request) -> Response {
    text(format!(
        "{}:{}",
        request.method(),
        request.param("id").unwrap_or("none")
    ))
}

async fn server(router: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("address");
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |request| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            suprnova::server::handle_request(router, middleware, request).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn query_macro_dispatches_an_extension_method() {
    routes! { query!("/delta-query/{id}", answer).name("delta.query.one"), }
    let addr = server(register()).await;
    let (status, _, body) = super::http_wire::request(addr, "QUERY", "/delta-query/7", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body, "QUERY:7");
    let (status, _, _) = super::http_wire::request(addr, "GET", "/delta-query/7", &[]).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn any_macro_accepts_query_at_top_level_and_in_a_group() {
    routes! {
        any!("/delta-any/{id}", answer),
        group!("/delta-group", { any!("/{id}", answer), query!("/only/{id}", answer), }),
    }
    let addr = server(register()).await;
    for path in ["/delta-any/7", "/delta-group/7", "/delta-group/only/7"] {
        let (status, _, body) = super::http_wire::request(addr, "QUERY", path, &[]).await;
        assert_eq!(status, 200, "{path}");
        assert_eq!(body, "QUERY:7");
    }
}

#[tokio::test]
async fn fluent_query_and_method_lists_keep_constraints() {
    let query = Method::from_bytes(b"QUERY").expect("method");
    let router: Router = Router::new()
        .query("/delta-fluent/{id}", answer)
        .where_number("id")
        .group("/delta-fluent-group", |group| {
            group.query("/{id}", answer).any("/any/{id}", answer)
        })
        .into();
    let addr = server(router).await;
    for path in [
        "/delta-fluent/7",
        "/delta-fluent-group/7",
        "/delta-fluent-group/any/7",
    ] {
        let (status, _, body) = super::http_wire::request(addr, "QUERY", path, &[]).await;
        assert_eq!(status, 200);
        assert_eq!(body, "QUERY:7");
    }
    assert_eq!(
        super::http_wire::request(addr, "QUERY", "/delta-fluent/no", &[])
            .await
            .0,
        404
    );
    assert!(
        Router::new()
            .try_methods(&[query], "/query-list", answer)
            .is_ok()
    );
    assert!(
        Router::new()
            .try_methods(&[Method::TRACE], "/trace-list", answer)
            .is_err()
    );
}

#[test]
fn duplicate_query_routes_are_rejected_with_the_method() {
    let router: Router = Router::new()
        .try_query("/delta-duplicate", answer)
        .expect("first route")
        .into();
    let error = match router.try_query("/delta-duplicate", answer) {
        Ok(_) => panic!("duplicate accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("QUERY"));
}

struct GuardHeader;
#[suprnova::async_trait]
impl suprnova::Middleware for GuardHeader {
    async fn handle(&self, request: Request, next: suprnova::Next) -> Response {
        use suprnova::ResponseExt;
        next(request).await.header("X-Query-Guard", "checked")
    }
}

#[tokio::test]
async fn query_routes_and_any_groups_keep_their_method_middleware() {
    routes! {
        query!("/delta-guard/{id}", answer).middleware(GuardHeader),
        suprnova::get!("/delta-guard/{id}", answer),
        group!("/delta-any-guard", { any!("/{id}", answer), }).middleware(GuardHeader),
    }
    let addr = server(register()).await;
    for path in ["/delta-guard/7", "/delta-any-guard/7"] {
        let (status, headers, _) = super::http_wire::request(addr, "QUERY", path, &[]).await;
        assert_eq!(status, 200);
        assert_eq!(
            headers.get("x-query-guard").map(String::as_str),
            Some("checked")
        );
    }
    let (_, headers, _) = super::http_wire::request(addr, "GET", "/delta-guard/7", &[]).await;
    assert!(!headers.contains_key("x-query-guard"));
}
