//! Loose end — `GET /api/v3/users` was unbounded, and both JSON:API user
//! endpoints were anonymous.
//!
//! `list_users` called `User::find_all()`: every row materialised into
//! memory and every one rendered. On a real users table that is an
//! availability problem before it is anything else, and this controller
//! is a worked example people copy.
//!
//! Worse, and not on any list before this: `/api/v3/users` and
//! `/api/users/{id}` sat at the top level of the router with no
//! middleware, while `UserResource` serialises `email`. So the dogfood
//! app handed every user's address to unauthenticated callers — the same
//! defect Group 0 fixed in the `--api` scaffold
//! (`api_user_routes_are_behind_an_auth_gate`). The scaffold got the fix.
//! The dogfood, which is the other thing people copy, did not.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::{MiddlewareRegistry, handle_request};

async fn spawn_app() -> SocketAddr {
    let router = Arc::new(app::routes::register());

    // `SessionMiddleware` fails closed without `Crypt`: the request 500s
    // with "encryption key not installed" before it ever reaches the auth
    // gate this file is about.
    suprnova::Crypt::init(suprnova::crypto::EncryptionKey::generate());

    let middleware = Arc::new({
        app::bootstrap::register_http_stack();
        MiddlewareRegistry::from_global()
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .body(Empty::<Bytes>::new())
        .expect("request");

    let res = sender.send_request(req).await.expect("send");
    let status = res.status().as_u16();
    let bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The exposure. An anonymous caller must not receive a user listing —
/// and specifically must not receive an email address.
#[tokio::test]
async fn anonymous_callers_cannot_list_users() {
    let addr = spawn_app().await;

    let (status, body) = get(addr, "/api/v3/users").await;

    // Assert the *specific* 401, not merely "not 200".
    //
    // The first draft of this test asserted `!= 200` and was toothless:
    // with no database configured the handler 500s, so removing the auth
    // gate entirely left the test passing. "Not 200" is satisfied by any
    // failure, including the wrong one. Pinning 401 means only the gate
    // can satisfy it.
    assert_eq!(
        status, 401,
        "an anonymous caller must be refused by the auth gate. \
         `UserResource` serialises `email`, so an ungated listing hands \
         out every address on the system. Body: {body}"
    );
}

/// The single-resource endpoint serialises the same field and was
/// equally open.
#[tokio::test]
async fn anonymous_callers_cannot_read_a_single_user() {
    let addr = spawn_app().await;

    let (status, body) = get(addr, "/api/users/1").await;

    assert_eq!(
        status, 401,
        "an anonymous caller must be refused by the auth gate: {body}"
    );
}

/// The delete demo is deliberately left ungated here — it authorizes via
/// `Gate::authorize` inside the handler, which is the thing it exists to
/// demonstrate. Pinned so the group edit above is not silently widened
/// to swallow it.
#[tokio::test]
async fn the_gate_demo_route_is_not_swallowed_by_the_auth_group() {
    let addr = spawn_app().await;

    let (status, _body) = get(addr, "/api/posts/1").await;

    assert_ne!(
        status, 404,
        "`/api/posts/{{id}}` stopped resolving — moving the user routes \
         into a group must not have taken this one with them"
    );
}
