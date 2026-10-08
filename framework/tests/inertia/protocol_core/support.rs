//! Helpers the protocol core tests share.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::{
    HttpResponse, InertiaConfig, InertiaRequestExt, InertiaResponse, MiddlewareRegistry, Request,
    Response, Router, handle_request,
};

/// A request the test builds field by field.
pub(super) struct MockReq {
    path: String,
    headers: HashMap<String, String>,
}

impl MockReq {
    pub(super) fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            headers: HashMap::new(),
        }
    }

    pub(super) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }
}

impl InertiaRequestExt for MockReq {
    fn path(&self) -> &str {
        &self.path
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// A config whose manifest does not exist, so the default version source
/// has nothing to hash.
pub(super) fn no_manifest() -> InertiaConfig {
    InertiaConfig::new().manifest_path("this/manifest/does/not/exist.json")
}

/// Serve `router` behind `registry` on a loopback socket for `accepts`
/// connections.
pub(super) async fn spawn_server(
    router: Router,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// A router whose `/page` renders an Inertia page through a real
/// `Request`.
pub(super) fn page_router() -> Router {
    Router::new()
        .get("/page", |req: Request| async move {
            let response: Response = InertiaResponse::new("Page")
                .with_config(no_manifest())
                .with("greeting", "hello")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from);
            response
        })
        .into()
}
