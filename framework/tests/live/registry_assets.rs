//! `registries-serve` (REG-017): a third-party namespace's stylesheets and
//! scripts are served at `/<namespace>-ui/{component}/{file}` from
//! `templates/<namespace>-ui/`, under the contract `/suprnova-ui/` has, and
//! nothing else in that directory is reachable.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{Crypt, EncryptionKey, MiddlewareRegistry, Router, handle_request};

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

async fn get(router: Router, path: &str) -> (hyper::StatusCode, hyper::HeaderMap, Bytes) {
    ensure_crypt();
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect to test listener");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(path)
        .header("host", "localhost")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = sender.send_request(request).await.expect("response");
    let (parts, body) = response.into_parts();
    let body = body.collect().await.expect("body").to_bytes();
    (parts.status, parts.headers, body)
}

/// A templates root holding one third-party component beside the shipped
/// root, as `live:add acme/acme-ui/widget` leaves it.
fn templates_with_a_namespace() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("tempdir");
    let shipped = root.path().join("suprnova-ui").join("field");
    std::fs::create_dir_all(&shipped).expect("shipped dir");
    std::fs::write(shipped.join("field.css"), ".sn-field { display: grid; }\n").expect("css");
    let widget = root.path().join("acme-ui").join("widget");
    std::fs::create_dir_all(&widget).expect("widget dir");
    std::fs::write(widget.join("widget.css"), ".acme-widget { color: red; }\n").expect("css");
    std::fs::write(widget.join("widget.js"), "export class AcmeWidget {}\n").expect("js");
    std::fs::write(widget.join("widget.html"), "<p>widget</p>\n").expect("view");
    std::fs::write(widget.join("widget.rs"), "pub struct Widget;\n").expect("rust");
    std::fs::write(widget.join(".suprnova-installed.json"), "{}\n").expect("record");
    root
}

/// REG-017: after the documented call for the namespace, its stylesheet
/// and script answer at `/acme-ui/widget/...`, and nothing else does.
#[tokio::test]
async fn reg_017_a_namespace_s_stylesheet_and_script_are_served_under_its_own_root() {
    let templates = templates_with_a_namespace();
    let router = Router::new()
        .try_live_ui_assets_from(templates.path().join("suprnova-ui"))
        .expect("the shipped root installs");
    let (status, headers, body) = get(router, "/acme-ui/widget/widget.js").await;
    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "the namespace route is not served: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        headers.get("etag").is_some(),
        "no ETag on a namespace asset"
    );
    assert_eq!(body, Bytes::from_static(b"export class AcmeWidget {}\n"));
}
