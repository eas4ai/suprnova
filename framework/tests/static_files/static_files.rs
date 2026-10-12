use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::{MiddlewareRegistry, Router, fallback, handle_request};

#[cfg(unix)]
use crate::tree_racer::Racer;

#[tokio::test]
async fn static_files_serves_public_asset_and_blocks_traversal() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("app.css"), "body{color:red}")
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let ok = drive_get(router.clone(), "/app.css").await;
    assert_eq!(ok.status(), 200);
    assert_eq!(header(&ok, "content-type"), "text/css; charset=utf-8");
    assert_eq!(body(ok).await, "body{color:red}");

    let blocked = drive_get(router, "/%2e%2e/Cargo.toml").await;
    assert_eq!(blocked.status(), 404);
}

#[tokio::test]
async fn static_files_rejects_literal_and_decoded_dot_components() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::create_dir(dir.path().join("assets"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("assets/app.css"), "body{color:red}")
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let ok = drive_get(router.clone(), "/assets/app.css").await;
    assert_eq!(ok.status(), 200);

    let literal_dot = drive_get(router.clone(), "/assets/./app.css").await;
    assert_eq!(literal_dot.status(), 404);

    let decoded_dot = drive_get(router, "/assets/%2e/app.css").await;
    assert_eq!(decoded_dot.status(), 404);
}

#[tokio::test]
async fn static_files_head_returns_headers_without_body() {
    let dir = tempfile::tempdir().unwrap();
    let source = "console.log('ok')";
    tokio::fs::write(dir.path().join("app.js"), source)
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let response = drive_request(router, "HEAD", "/app.js").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        header(&response, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        header(&response, "content-length"),
        source.len().to_string()
    );
    assert_eq!(body(response).await, "");
}

#[tokio::test]
async fn static_files_streams_large_get_with_content_length() {
    let dir = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    tokio::fs::write(dir.path().join("large.bin"), &data)
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let response = drive_get(router, "/large.bin").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        header(&response, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(header(&response, "content-length"), data.len().to_string());
    assert_eq!(body_bytes(response).await, data);
}

#[cfg(unix)]
#[tokio::test]
async fn static_files_get_and_head_match_for_unreadable_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("secret.txt");
    tokio::fs::write(&file_path, "secret").await.unwrap();

    let mut permissions = tokio::fs::metadata(&file_path).await.unwrap().permissions();
    permissions.set_mode(0o0);
    tokio::fs::set_permissions(&file_path, permissions)
        .await
        .unwrap();

    if tokio::fs::File::open(&file_path).await.is_ok() {
        let mut restore = tokio::fs::metadata(&file_path).await.unwrap().permissions();
        restore.set_mode(0o600);
        let _ = tokio::fs::set_permissions(&file_path, restore).await;
        eprintln!("skipping unreadable-file regression: current user can read mode-000 files");
        return;
    }

    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let get = drive_get(router.clone(), "/secret.txt").await;
    let head = drive_request(router, "HEAD", "/secret.txt").await;

    let get_status = get.status().as_u16();
    let head_status = head.status().as_u16();
    let head_body = body(head).await;

    let mut restore = tokio::fs::metadata(&file_path).await.unwrap().permissions();
    restore.set_mode(0o600);
    let _ = tokio::fs::set_permissions(&file_path, restore).await;

    assert_eq!(get_status, 404);
    assert_eq!(head_status, get_status);
    assert_eq!(head_body, "");
}

#[tokio::test]
async fn static_files_rejects_post_and_directories() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::create_dir(dir.path().join("assets"))
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let post = drive_request(router.clone(), "POST", "/assets").await;
    assert_eq!(post.status(), 404);

    let directory = drive_get(router, "/assets").await;
    assert_eq!(directory.status(), 404);
}

#[tokio::test]
async fn static_files_sets_cache_control_when_configured() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("logo.svg"), "<svg></svg>")
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(
            suprnova::StaticFiles::from_dir(dir.path())
                .cache_control("public, max-age=31536000")
                .handler()
        )
        .register(Router::new()),
    );

    let response = drive_get(router, "/logo.svg").await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        header(&response, "content-type"),
        "image/svg+xml; charset=utf-8"
    );
    assert_eq!(
        header(&response, "cache-control"),
        "public, max-age=31536000"
    );
}

#[tokio::test]
async fn static_files_json_and_manifest_mime_types_have_expected_charset() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("data.json"), r#"{"ok":true}"#)
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("site.webmanifest"), r#"{"name":"App"}"#)
        .await
        .unwrap();
    let router = Arc::new(
        fallback!(suprnova::StaticFiles::from_dir(dir.path()).handler()).register(Router::new()),
    );

    let json = drive_get(router.clone(), "/data.json").await;
    assert_eq!(json.status(), 200);
    assert_eq!(
        header(&json, "content-type"),
        "application/json; charset=utf-8"
    );

    let manifest = drive_get(router, "/site.webmanifest").await;
    assert_eq!(manifest.status(), 200);
    assert_eq!(
        header(&manifest, "content-type"),
        "application/manifest+json; charset=utf-8"
    );
}

async fn drive_get(router: Arc<Router>, path: &str) -> hyper::Response<Incoming> {
    drive_request(router, "GET", path).await
}

async fn drive_request(router: Arc<Router>, method: &str, path: &str) -> hyper::Response<Incoming> {
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let io = TokioIo::new(stream);
        let svc = service_fn(move |req: hyper::Request<Incoming>| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(io, svc)
            .await;
    });

    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to test server");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("client handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .expect("build request");

    tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request")
}

fn header(response: &hyper::Response<Incoming>, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn body(response: hyper::Response<Incoming>) -> String {
    String::from_utf8_lossy(&body_bytes(response).await).to_string()
}

async fn body_bytes(response: hyper::Response<Incoming>) -> Bytes {
    response
        .into_body()
        .collect()
        .await
        .expect("collect body bytes")
        .to_bytes()
}

/// How many requests a race test sends while the tree changes under it.
#[cfg(unix)]
const RACE_REQUESTS: usize = 2000;

#[cfg(unix)]
fn static_client(root: &std::path::Path) -> suprnova::testing::TestClient {
    suprnova::testing::TestClient::new(
        fallback!(suprnova::StaticFiles::from_dir(root).handler()).register(Router::new()),
        MiddlewareRegistry::new(),
    )
}

/// HRD-005: a directory under the served root that is swapped for a
/// symlink after the path was checked cannot make the server send a file
/// from outside the root. The handler used to check the canonical path and
/// then open it again by name, so a swap between the two opened whatever
/// the symlink pointed at.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hardening_a_directory_swapped_for_a_symlink_after_the_check_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("public");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("dir")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("dir/data.txt"), "inside").unwrap();
    std::fs::write(outside.join("data.txt"), "SECRET").unwrap();
    let client = static_client(&root);

    let swapped = root.clone();
    let racer = Racer::start(move || {
        let _ = std::fs::rename(swapped.join("dir"), swapped.join("dir.real"));
        let _ = std::os::unix::fs::symlink(&outside, swapped.join("dir"));
        std::thread::yield_now();
        let _ = std::fs::remove_file(swapped.join("dir"));
        let _ = std::fs::rename(swapped.join("dir.real"), swapped.join("dir"));
    });

    let mut served = 0usize;
    let mut leaked = Vec::new();
    for _ in 0..RACE_REQUESTS {
        let response = client.get("/dir/data.txt").send().await;
        match response.status() {
            200 if response.body_text() == "inside" => served += 1,
            404 => {}
            status => leaked.push((status, response.body_text())),
        }
    }
    let swaps = racer.finish();

    assert!(
        leaked.is_empty(),
        "{} of {RACE_REQUESTS} responses were not the file inside the root: {:?}",
        leaked.len(),
        &leaked[..leaked.len().min(5)]
    );
    assert!(swaps > 0, "the racer never swapped the directory");
    assert!(served > 0, "no request was served the file inside the root");
}

/// HRD-005: the length and the content type a response declares come from
/// the file it sends. Two versions of an extensionless asset are swapped in
/// by rename while requests run; every response must carry one version
/// whole, with that version's length and sniffed type. The handler used to
/// sniff the type by opening the path a second time, so a swap between the
/// two opens labelled one file's bytes with the other's type.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hardening_the_size_and_type_come_from_the_opened_handle() {
    const GIF: &str = "GIF89a, padded so the two versions differ in length";
    const JSON: &str = r#"{"ok":true}"#;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("public");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("version-gif"), GIF).unwrap();
    std::fs::write(root.join("version-json"), JSON).unwrap();
    std::fs::write(root.join("asset"), GIF).unwrap();
    let client = static_client(&root);

    let swapped = root.clone();
    let racer = Racer::start(move || {
        for version in ["version-json", "version-gif"] {
            let _ = std::fs::remove_file(swapped.join("staged"));
            let _ = std::fs::hard_link(swapped.join(version), swapped.join("staged"));
            let _ = std::fs::rename(swapped.join("staged"), swapped.join("asset"));
        }
    });

    let mut served = 0usize;
    let mut mismatched = Vec::new();
    for _ in 0..RACE_REQUESTS {
        let response = client.get("/asset").send().await;
        if response.status() == 404 {
            continue;
        }
        let body = response.body_text();
        let content_type = response.header("content-type").unwrap_or("").to_owned();
        let length = response.header("content-length").unwrap_or("").to_owned();
        let expected_type = match body.as_str() {
            GIF => "image/gif",
            JSON => "application/json; charset=utf-8",
            _ => "a whole version",
        };
        if response.status() == 200
            && content_type == expected_type
            && length == body.len().to_string()
        {
            served += 1;
        } else {
            mismatched.push((response.status(), content_type, length, body));
        }
    }
    let swaps = racer.finish();

    assert!(
        mismatched.is_empty(),
        "{} of {RACE_REQUESTS} responses declared a type or length that is not the body's: {:?}",
        mismatched.len(),
        &mismatched[..mismatched.len().min(5)]
    );
    assert!(swaps > 0, "the racer never swapped the asset");
    assert!(served > 0, "no request was served the asset");
}

/// HRD-005 keeps what the handler served before: a symbolic link that
/// stays inside the root is followed, and one that leads out of it, by an
/// absolute or a relative target, a directory or a file, is a 404. A
/// relative target that climbs above the root is a 404 even when it leads
/// back in, as on a local disk: the walk holds no directory above the root.
#[cfg(unix)]
#[tokio::test]
async fn hardening_a_symlink_that_leaves_the_root_is_refused_and_one_inside_is_served() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("public");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("app.css"), "body{color:red}").unwrap();
    std::fs::write(root.join("sub/real.txt"), "inside").unwrap();
    std::fs::write(outside.join("data.txt"), "SECRET").unwrap();
    symlink(&outside, root.join("out")).unwrap();
    symlink("../outside", root.join("up")).unwrap();
    symlink(outside.join("data.txt"), root.join("secret.txt")).unwrap();
    symlink("app.css", root.join("alias.css")).unwrap();
    symlink(root.join("app.css"), root.join("absolute.css")).unwrap();
    symlink("sub", root.join("linked")).unwrap();
    symlink("../public/app.css", root.join("around.css")).unwrap();
    let client = static_client(&root);

    for escaping in [
        "/out/data.txt",
        "/up/data.txt",
        "/secret.txt",
        "/around.css",
    ] {
        let response = client.get(escaping).send().await;
        assert_eq!(
            response.status(),
            404,
            "{escaping} followed a link out of the root"
        );
        assert!(
            !response.body_text().contains("SECRET"),
            "{escaping} leaked the outside file"
        );
    }

    for (inside, body, content_type) in [
        ("/alias.css", "body{color:red}", "text/css; charset=utf-8"),
        (
            "/absolute.css",
            "body{color:red}",
            "text/css; charset=utf-8",
        ),
        ("/linked/real.txt", "inside", "text/plain; charset=utf-8"),
    ] {
        let response = client.get(inside).send().await;
        assert_eq!(response.status(), 200, "{inside} is a link inside the root");
        assert_eq!(response.body_text(), body, "{inside}");
        assert_eq!(
            response.header("content-type"),
            Some(content_type),
            "{inside}"
        );
    }
}

/// HRD-005: the file is opened without waiting, and its type is checked on
/// that handle. An ordinary open of a FIFO waits until a writer appears, so
/// a FIFO under the root used to hold the request, and a blocking thread,
/// open until one did.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hardening_a_fifo_is_a_404_without_waiting_for_a_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let pipe = tmp.path().join("pipe.txt");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &pipe,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    let client = static_client(tmp.path());

    let answered =
        tokio::time::timeout(Duration::from_secs(5), client.get("/pipe.txt").send()).await;
    if answered.is_err() {
        // A server still waiting on the FIFO is released by a writer, so a
        // failing run ends instead of hanging the runtime's shutdown.
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe);
    }
    let response = answered.expect("the request answered without waiting for a writer");
    assert_eq!(response.status(), 404);
}
