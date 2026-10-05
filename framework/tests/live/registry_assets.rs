//! `registries-serve` (REG-017): a third-party namespace's stylesheets and
//! scripts are served at `/<namespace>-ui/{component}/{file}` from
//! `templates/<namespace>-ui/`, under the contract `/suprnova-ui/` has, and
//! nothing else in that directory, or outside it, is reachable.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, StatusCode};
use hyper_util::rt::TokioIo;
use suprnova::{Crypt, EncryptionKey, MiddlewareRegistry, Router, handle_request};

/// The largest asset the route serves.
const MIB: usize = 1024 * 1024;

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

async fn send(router: &Arc<Router>, method: Method, path: &str, headers: &[(&str, &str)]) -> Reply {
    ensure_crypt();
    let router = Arc::clone(router);
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
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(Full::new(Bytes::new())).expect("request");
    let response = sender.send_request(request).await.expect("response");
    let (parts, body) = response.into_parts();
    let body = body.collect().await.expect("body").to_bytes();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

async fn get(router: &Arc<Router>, path: &str) -> Reply {
    send(router, Method::GET, path, &[]).await
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().expect("a parent directory")).expect("create dir");
    std::fs::write(path, bytes).expect("write fixture file");
}

/// A project as `live:add` leaves it after installing the shipped `field`
/// and `acme/acme-ui/widget`: each library under its own template root,
/// the widget's view, Rust, manifest and install record beside its
/// stylesheet and script, and `suprnova.toml` at the project root, outside
/// both template roots.
fn project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("tempdir");
    let root = project.path();
    write(&root.join("suprnova.toml"), "[registries]\n");
    write(
        &root.join("templates/suprnova-ui/field/field.css"),
        ".sn-field { display: grid; }\n",
    );
    let widget = root.join("templates/acme-ui/widget");
    write(&widget.join("widget.css"), ".acme-widget { color: red; }\n");
    write(&widget.join("widget.js"), "export class AcmeWidget {}\n");
    write(&widget.join("widget.html"), "<p>widget</p>\n");
    write(&widget.join("widget.rs"), "pub struct Widget;\n");
    write(&widget.join("manifest.json"), "{}\n");
    write(&widget.join(".suprnova-installed.json"), "{}\n");
    project
}

fn templates(project: &tempfile::TempDir) -> PathBuf {
    project.path().join("templates")
}

/// The shipped root and the acme root, each through its own call.
fn shipped_and_acme(project: &tempfile::TempDir) -> Arc<Router> {
    Arc::new(
        Router::new()
            .try_live_ui_assets_from(templates(project).join("suprnova-ui"))
            .expect("the shipped root installs")
            .try_live_ui_assets_for_from("acme", templates(project).join("acme-ui"))
            .expect("the acme root installs"),
    )
}

/// REG-017: after the call for the namespace, its stylesheet and script
/// answer at `/acme-ui/widget/...` with the validators and headers the
/// shipped route has, beside the shipped route, and neither root answers
/// for the other.
#[tokio::test]
async fn reg_017_a_namespace_s_stylesheet_and_script_are_served_under_its_own_root() {
    let project = project();
    let router = shipped_and_acme(&project);

    let script = get(&router, "/acme-ui/widget/widget.js").await;
    assert_eq!(
        script.status,
        StatusCode::OK,
        "the namespace route is not served: {}",
        String::from_utf8_lossy(&script.body)
    );
    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        script.header("cache-control"),
        Some("public, max-age=0, must-revalidate")
    );
    assert_eq!(script.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        script.body,
        Bytes::from_static(b"export class AcmeWidget {}\n")
    );
    let etag = script
        .header("etag")
        .expect("an ETag on a namespace asset")
        .to_owned();

    let conditional = send(
        &router,
        Method::GET,
        "/acme-ui/widget/widget.js",
        &[("if-none-match", etag.as_str())],
    )
    .await;
    assert_eq!(conditional.status, StatusCode::NOT_MODIFIED);
    assert_eq!(conditional.header("etag"), Some(etag.as_str()));

    let stylesheet = get(&router, "/acme-ui/widget/widget.css").await;
    assert_eq!(stylesheet.status, StatusCode::OK);
    assert_eq!(
        stylesheet.header("content-type"),
        Some("text/css; charset=utf-8")
    );
    assert_eq!(
        stylesheet.body,
        Bytes::from_static(b".acme-widget { color: red; }\n")
    );

    let head = send(&router, Method::HEAD, "/acme-ui/widget/widget.js", &[]).await;
    assert_eq!(head.status, StatusCode::OK);
    assert!(head.body.is_empty(), "HEAD carries no body");

    let post = send(&router, Method::POST, "/acme-ui/widget/widget.js", &[]).await;
    assert_eq!(post.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(post.header("allow"), Some("GET, HEAD"));

    let shipped = get(&router, "/suprnova-ui/field/field.css").await;
    assert_eq!(
        shipped.status,
        StatusCode::OK,
        "the shipped route still serves"
    );
    assert_eq!(
        shipped.body,
        Bytes::from_static(b".sn-field { display: grid; }\n")
    );

    for crossed in ["/acme-ui/field/field.css", "/suprnova-ui/widget/widget.js"] {
        assert_eq!(
            get(&router, crossed).await.status,
            StatusCode::NOT_FOUND,
            "{crossed} reaches the other library's root"
        );
    }
}

/// REG-017 through the documented call: `try_live_ui_assets_for("acme")`
/// reads `templates/acme-ui/` under the application base path, and refuses
/// to install, naming the directory and `APP_BASE_PATH` (UI-021), where the
/// base path holds no such directory. The base path is process-wide, so the
/// body runs alone in a child process.
#[test]
fn reg_017_the_documented_call_serves_the_namespace_under_the_base_path() {
    crate::own_process::run_alone(
        "registry_assets::reg_017_the_documented_call_serves_the_namespace_under_the_base_path_child",
    );
}

#[tokio::test]
async fn reg_017_the_documented_call_serves_the_namespace_under_the_base_path_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let project = project();
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let previous = suprnova::base_path("");
    suprnova::set_base_path(project.path());
    let installed = Router::new()
        .try_live_ui_assets()
        .and_then(|router| router.try_live_ui_assets_for("acme"));
    suprnova::set_base_path(elsewhere.path());
    let refused = Router::new().try_live_ui_assets_for("acme");
    suprnova::set_base_path(previous);

    let router = Arc::new(installed.expect("both calls install under the base path"));
    let script = get(&router, "/acme-ui/widget/widget.js").await;
    assert_eq!(script.status, StatusCode::OK);
    assert_eq!(
        script.body,
        Bytes::from_static(b"export class AcmeWidget {}\n")
    );
    assert_eq!(
        get(&router, "/suprnova-ui/field/field.css").await.status,
        StatusCode::OK
    );

    let Err(refused) = refused else {
        panic!("a base path without templates/acme-ui is refused");
    };
    let message = refused.to_string();
    let missing = elsewhere.path().join("templates").join("acme-ui");
    assert!(
        message.contains(&missing.display().to_string()),
        "the refusal names the directory: {message}"
    );
    assert!(message.contains("APP_BASE_PATH"), "{message}");
    assert!(message.contains("templates/acme-ui"), "{message}");
}

/// REG-017's falsifier: the route serves a view, a Rust file, an install
/// record or `suprnova.toml`. Only a closed component name and a closed
/// `.css` or `.js` file name one level below the root are reachable.
#[tokio::test]
async fn reg_017_nothing_but_a_component_stylesheet_or_script_is_served() {
    let project = project();
    let acme = templates(&project).join("acme-ui");
    write(&acme.join("stray.js"), "export {};\n");
    write(&acme.join("widget/nested/deep.js"), "export {};\n");
    write(&acme.join("suprnova.toml"), "[registries]\n");
    let router = shipped_and_acme(&project);

    for closed in [
        "/acme-ui/widget/widget.html",
        "/acme-ui/widget/widget.rs",
        "/acme-ui/widget/.suprnova-installed.json",
        "/acme-ui/widget/manifest.json",
        "/acme-ui/widget/suprnova.toml",
        "/acme-ui/suprnova.toml",
        "/acme-ui/../suprnova.toml",
        "/acme-ui/../../suprnova.toml",
        "/acme-ui/%2e%2e/suprnova.toml",
        "/acme-ui/widget/..%2F..%2F..%2Fsuprnova.toml",
        "/acme-ui/..%2F..%2F/suprnova.js",
        "/acme-ui/widget/widget.js%00.css",
        "/acme-ui/widget/missing.js",
        "/acme-ui/Widget/widget.js",
        "/acme-ui/widget/WIDGET.JS",
        "/acme-ui/widget/widget.js?v=1",
        "/acme-ui/stray.js",
        "/acme-ui/widget/nested/deep.js",
        "/acme-ui/widget/",
        "/acme-ui/",
    ] {
        let reply = get(&router, closed).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{closed} is served");
        assert!(
            !reply.body.starts_with(b"[registries]"),
            "{closed} leaked suprnova.toml"
        );
    }
}

/// A file is served only where it sits: a component directory or a file
/// that is a symbolic link below the root is refused, so a link cannot
/// reach `suprnova.toml`, a view or anything outside the root. The root
/// itself may be a link, as a deployment's templates directory often is.
#[cfg(unix)]
#[tokio::test]
async fn reg_017_a_symbolic_link_below_the_root_is_not_followed() {
    use std::os::unix::fs::symlink;

    let project = project();
    let acme = templates(&project).join("acme-ui");
    write(
        &project.path().join("outside/outside.js"),
        "export const outside = 1;\n",
    );
    symlink(project.path().join("outside"), acme.join("outside")).expect("directory link");
    symlink(
        project.path().join("suprnova.toml"),
        acme.join("widget/config.js"),
    )
    .expect("link to suprnova.toml");
    symlink(
        acme.join("widget/widget.html"),
        acme.join("widget/view.css"),
    )
    .expect("link to view");
    let router = shipped_and_acme(&project);

    for linked in [
        "/acme-ui/outside/outside.js",
        "/acme-ui/widget/config.js",
        "/acme-ui/widget/view.css",
    ] {
        let reply = get(&router, linked).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "{linked} follows a link"
        );
        assert!(reply.body.is_empty(), "{linked} leaked bytes");
    }

    let linked_root = project.path().join("linked-ui");
    symlink(&acme, &linked_root).expect("root link");
    let router = Arc::new(
        Router::new()
            .try_live_ui_assets_for_from("acme", &linked_root)
            .expect("a linked root installs"),
    );
    let script = get(&router, "/acme-ui/widget/widget.js").await;
    assert_eq!(script.status, StatusCode::OK, "a linked root serves");
    assert_eq!(
        script.body,
        Bytes::from_static(b"export class AcmeWidget {}\n")
    );
}

/// The 1 MiB cap the shipped route has holds for a namespace: a file of
/// exactly 1 MiB is served, one byte more is not.
#[tokio::test]
async fn reg_017_the_1_mib_cap_holds_for_a_namespace() {
    let project = project();
    let acme = templates(&project).join("acme-ui");
    write(&acme.join("large/large.js"), vec![b' '; MIB]);
    write(&acme.join("huge/huge.js"), vec![b' '; MIB + 1]);
    let router = shipped_and_acme(&project);

    let large = get(&router, "/acme-ui/large/large.js").await;
    assert_eq!(large.status, StatusCode::OK);
    assert_eq!(large.body.len(), MIB);
    let huge = get(&router, "/acme-ui/huge/huge.js").await;
    assert_eq!(huge.status, StatusCode::NOT_FOUND);
    assert!(huge.body.is_empty());
}

/// REG-004's closed rule, as the CLI applies it: one segment of lowercase
/// letters, digits and hyphens, starting with a letter, of at most 32
/// bytes, whose module form is not a Rust keyword. Anything else is refused
/// before it reaches a route pattern or a path.
#[test]
fn reg_017_a_namespace_outside_the_closed_rule_is_refused() {
    let project = project();
    let acme = templates(&project).join("acme-ui");
    let too_long = "a".repeat(33);
    for hostile in [
        "",
        "Acme",
        "1acme",
        "-acme",
        "a_b",
        "a.b",
        "a/b",
        "../acme",
        "acme ui",
        "acme\0",
        "{acme}",
        ":acme",
        "*acme",
        "acme%2f",
        "\u{e9}cme",
        too_long.as_str(),
        "self",
        "type",
        "mod",
        "lib",
    ] {
        let Err(refused) = Router::new().try_live_ui_assets_for_from(hostile, &acme) else {
            panic!("{hostile:?} is accepted as a namespace");
        };
        assert!(
            refused.to_string().contains("namespace"),
            "{hostile:?}: {refused}"
        );
    }
    let longest = "a".repeat(32);
    for accepted in ["a", "a1", "acme-ui", "acme-", longest.as_str()] {
        assert!(
            Router::new()
                .try_live_ui_assets_for_from(accepted, &acme)
                .is_ok(),
            "{accepted:?} is refused"
        );
    }
}

/// `suprnova` is the shipped library's namespace, served by
/// `try_live_ui_assets()`, so the namespace call refuses it and names that
/// call; `sn` and `live` are reserved for the shipped library (REG-004),
/// which no library may hold, so they are refused too.
#[test]
fn reg_017_the_reserved_namespaces_are_refused() {
    let project = project();
    let shipped = templates(&project).join("suprnova-ui");
    let Err(refused) = Router::new().try_live_ui_assets_for_from("suprnova", &shipped) else {
        panic!("the shipped namespace is accepted through the namespace call");
    };
    assert!(
        refused.to_string().contains("try_live_ui_assets()"),
        "{refused}"
    );
    for reserved in ["sn", "live"] {
        let Err(refused) = Router::new().try_live_ui_assets_for_from(reserved, &shipped) else {
            panic!("the reserved namespace {reserved} is accepted");
        };
        assert!(refused.to_string().contains("reserved"), "{refused}");
    }
}

/// UI-021 for a namespace: the route refuses to install over a directory it
/// cannot read and names it with the same advice, so an application
/// started away from its templates refuses to start instead of answering
/// 404 for every asset.
#[test]
fn reg_017_an_unreadable_namespace_directory_refuses_to_install() {
    let project = project();
    let missing = templates(&project).join("globex-ui");
    let Err(refused) = Router::new().try_live_ui_assets_for_from("globex", &missing) else {
        panic!("a missing namespace directory is refused");
    };
    let message = refused.to_string();
    assert!(
        message.contains(&missing.display().to_string()),
        "the refusal names the directory: {message}"
    );
    assert!(message.contains("APP_BASE_PATH"), "{message}");
    assert!(message.contains("templates/globex-ui"), "{message}");

    let file = templates(&project).join("file-ui");
    write(&file, "not a directory\n");
    assert!(
        Router::new()
            .try_live_ui_assets_for_from("file", &file)
            .is_err(),
        "a file in place of the directory is refused"
    );
}

/// One explicit call per namespace: a second call for the same namespace is
/// refused as an error, never a panic, while different namespaces sit
/// beside each other and the shipped root.
#[tokio::test]
async fn reg_017_each_namespace_installs_once_beside_the_others() {
    let project = project();
    let globex = templates(&project).join("globex-ui");
    write(
        &globex.join("chart/chart.js"),
        "export class GlobexChart {}\n",
    );
    let three = || {
        Router::new()
            .try_live_ui_assets_from(templates(&project).join("suprnova-ui"))
            .expect("the shipped root installs")
            .try_live_ui_assets_for_from("acme", templates(&project).join("acme-ui"))
            .expect("the acme root installs")
            .try_live_ui_assets_for_from("globex", &globex)
            .expect("the globex root installs")
    };
    assert!(
        three()
            .try_live_ui_assets_for_from("acme", templates(&project).join("acme-ui"))
            .is_err(),
        "a second call for one namespace is refused"
    );

    let router = Arc::new(three());
    for (path, status) in [
        ("/globex-ui/chart/chart.js", StatusCode::OK),
        ("/acme-ui/widget/widget.js", StatusCode::OK),
        ("/suprnova-ui/field/field.css", StatusCode::OK),
        ("/globex-ui/widget/widget.js", StatusCode::NOT_FOUND),
        ("/acme-ui/chart/chart.js", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(get(&router, path).await.status, status, "{path}");
    }
}
