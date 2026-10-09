//! Route groups: `.name(prefix)` puts a prefix in front of the name of
//! every route in the group, and `controller = module` lets a route name
//! its handler by function alone.
//!
//! Route names are process-wide and a name may be bound once, so every
//! test uses names of its own.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{MiddlewareRegistry, Router, any, get, group, handle_request, post, route};

mod controllers {
    pub mod admin {
        pub mod users {
            use suprnova::http::text;
            use suprnova::{Request, Response};

            pub async fn index(_req: Request) -> Response {
                text("admin users index")
            }
            pub async fn store(_req: Request) -> Response {
                text("admin users store")
            }
            pub async fn anything(_req: Request) -> Response {
                text("admin users anything")
            }
        }
    }
    pub mod health {
        use suprnova::http::text;
        use suprnova::{Request, Response};

        pub async fn check(_req: Request) -> Response {
            text("health check")
        }
    }
}

async fn body_of(router: Router, method: &str, path: &str) -> (u16, String) {
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr: SocketAddr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let service = service_fn(move |request: hyper::Request<Incoming>| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(handle_request(router, registry, request).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("the request failed");
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[test]
fn a_group_puts_its_prefix_in_front_of_every_route_name_in_it() {
    let _router = group!("/gn-admin/users", {
        get!("/", controllers::admin::users::index).name("index"),
        post!("/", controllers::admin::users::store).name("store"),
        any!("/any", controllers::admin::users::anything).name("any"),
        get!("/unnamed", controllers::health::check),
    })
    .name("gn.admin.users.")
    .register(Router::new());

    assert_eq!(
        route("gn.admin.users.index", &[]).as_deref(),
        Some("/gn-admin/users")
    );
    assert_eq!(
        route("gn.admin.users.store", &[]).as_deref(),
        Some("/gn-admin/users")
    );
    assert_eq!(
        route("gn.admin.users.any", &[]).as_deref(),
        Some("/gn-admin/users/any")
    );
    assert_eq!(
        route("index", &[]),
        None,
        "the route is known by its full name, not by the part the group completes"
    );
}

#[test]
fn a_group_inside_a_group_adds_its_prefix_after_the_outer_one() {
    let _router = group!("/gn-nested", {
        get!("/", controllers::health::check).name("home"),
        group!("/users", {
            get!("/", controllers::admin::users::index).name("index"),
            group!("/roles", {
                get!("/", controllers::admin::users::index).name("index"),
            })
            .name("roles."),
        })
        .name("users."),
        group!("/plain", {
            get!("/", controllers::health::check).name("plain"),
        }),
    })
    .name("gn.nested.")
    .register(Router::new());

    assert_eq!(route("gn.nested.home", &[]).as_deref(), Some("/gn-nested"));
    assert_eq!(
        route("gn.nested.users.index", &[]).as_deref(),
        Some("/gn-nested/users")
    );
    assert_eq!(
        route("gn.nested.users.roles.index", &[]).as_deref(),
        Some("/gn-nested/users/roles")
    );
    assert_eq!(
        route("gn.nested.plain", &[]).as_deref(),
        Some("/gn-nested/plain"),
        "a group with no prefix of its own passes the outer one on"
    );
}

#[test]
fn a_group_without_a_name_prefix_leaves_names_as_they_are_written() {
    let _router = group!("/gn-bare", {
        get!("/", controllers::health::check).name("gn.bare.home"),
    })
    .register(Router::new());

    assert_eq!(route("gn.bare.home", &[]).as_deref(), Some("/gn-bare"));
}

#[tokio::test]
async fn a_controller_group_resolves_a_bare_handler_name_in_the_controller() {
    let router = || {
        group!("/gn-controller", controller = controllers::admin::users, {
            get!("/", index).name("index"),
            post!("/", store),
            any!("/any", anything),
            // A handler written as a path is taken as it is written.
            get!("/health", controllers::health::check).name("health"),
            // So is a group, which names its own controller.
            group!("/inner", controller = controllers::health, {
                get!("/", check)
            }),
        })
        .name("gn.controller.")
        .register(Router::new())
    };

    assert_eq!(
        body_of(router(), "GET", "/gn-controller").await,
        (200, "admin users index".to_owned())
    );
    assert_eq!(
        body_of(router(), "POST", "/gn-controller").await,
        (200, "admin users store".to_owned())
    );
    assert_eq!(
        body_of(router(), "DELETE", "/gn-controller/any").await,
        (200, "admin users anything".to_owned())
    );
    assert_eq!(
        body_of(router(), "GET", "/gn-controller/health").await,
        (200, "health check".to_owned())
    );
    assert_eq!(
        body_of(router(), "GET", "/gn-controller/inner").await,
        (200, "health check".to_owned())
    );
    assert_eq!(
        route("gn.controller.index", &[]).as_deref(),
        Some("/gn-controller")
    );
    assert_eq!(
        route("gn.controller.health", &[]).as_deref(),
        Some("/gn-controller/health")
    );
}

#[tokio::test]
async fn a_controller_group_takes_one_item_and_no_trailing_comma() {
    let router = group!("/gn-single", controller = controllers::admin::users, {
        get!("/", index)
    })
    .register(Router::new());

    assert_eq!(
        body_of(router, "GET", "/gn-single").await,
        (200, "admin users index".to_owned())
    );
}
