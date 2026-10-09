//! RDOC-003 on the HTTP path: a first visit through a root template writes
//! the page JSON into the template's output as it renders, never into a
//! string of its own, and adds no size cap.

use suprnova::{
    HttpResponse, InertiaConfig, InertiaResponse, InertiaRootTemplate, MiddlewareRegistry, Request,
    Response, Router,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::support::{Heap, exchange, exclusive, get_request, serve, status};

/// The smallest root document: the parts and nothing else.
#[suprnova::inertia_root(path = "inertia/memory.html")]
struct MemoryDocument;

/// A route at `path` answering a first visit with `blob` as a prop,
/// rendered through `config`.
fn page(router: Router, path: &str, config: InertiaConfig, blob: String) -> Router {
    router
        .get(path, move |req: Request| {
            let config = config.clone();
            let blob = blob.clone();
            async move {
                let response: Response = InertiaResponse::new("Page")
                    .with_config(config)
                    .with("blob", blob)
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from);
                response
            }
        })
        .into()
}

fn configs() -> (InertiaConfig, InertiaConfig) {
    let default = InertiaConfig::new().development(true).version("v");
    let templated = default
        .clone()
        .root_template(InertiaRootTemplate::of::<MemoryDocument>());
    (default, templated)
}

/// RDOC-003: with a 1 MiB prop, a first visit through a root template
/// allocates less than 1 MiB more than the same visit through the
/// framework's own document, which writes the page into its one buffer.
#[tokio::test]
async fn rdoc_003_a_template_visit_allocates_about_what_the_default_document_does() {
    let _lock = exclusive().await;
    const PROP: usize = 1024 * 1024;
    let blob = "a".repeat(PROP);
    let (default, templated) = configs();
    let router = page(Router::new(), "/default", default, blob.clone());
    let router = page(router, "/template", templated, blob);
    let addr = serve(router, MiddlewareRegistry::new()).await;
    let html = |path| get_request(path, &[("Accept", "text/html")]);
    let (default_request, template_request) = (html("/default"), html("/template"));
    for request in [&default_request, &template_request] {
        let (read, head) = exchange(addr, request).await;
        assert_eq!(status(&head), 200, "{}", String::from_utf8_lossy(&head));
        assert!(read > PROP, "the whole page came back: {read} bytes");
    }

    let heap = Heap::start();
    let before = heap.bytes();
    exchange(addr, &default_request).await;
    let framework = heap.bytes() - before;
    let before = heap.bytes();
    exchange(addr, &template_request).await;
    let template = heap.bytes() - before;
    assert!(
        template < framework + PROP as u64,
        "the template visit allocated {template} bytes, the framework's document {framework}"
    );
}

/// RDOC-003: a 3 MiB prop through a root template, past the 2 MiB a
/// `TrustedHtml` value may hold, returns 200 with the whole page.
#[tokio::test]
async fn rdoc_003_a_three_mib_prop_through_a_template_returns_the_whole_page() {
    let _lock = exclusive().await;
    const PROP: usize = 3 * 1024 * 1024;
    let (_, templated) = configs();
    let router = page(Router::new(), "/large", templated, "b".repeat(PROP));
    let addr = serve(router, MiddlewareRegistry::new()).await;

    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(&get_request("/large", &[("Accept", "text/html")]))
        .await
        .expect("send");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let response = String::from_utf8(response).expect("a UTF-8 response");
    let (head, body) = response.split_once("\r\n\r\n").expect("a head and a body");
    assert_eq!(status(head.as_bytes()), 200, "{head}");

    let open = "<script type=\"application/json\" data-page=\"app\">";
    let start = body.find(open).expect("the page data element") + open.len();
    let end = start + body[start..].find("</script>").expect("its end");
    let page: serde_json::Value = serde_json::from_str(&body[start..end]).expect("the page");
    assert_eq!(page["props"]["blob"].as_str().map(str::len), Some(PROP));
    assert!(
        body.ends_with("<div id=\"app\"></div>\n</body>\n</html>"),
        "{}",
        &body[end..]
    );
}
