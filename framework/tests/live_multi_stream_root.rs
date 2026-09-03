//! A component that declares several streams gets no island-owned
//! `live:stream` directive: the root carries one, so the engine emits it only
//! for a component with exactly one stream and never picks one silently.

mod live_dogfood_support;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use live_dogfood_support::{dispatch, get};
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{
    CanonicalValue, EventPayloadMetadata, LiveBootstrapOptions, LiveComponent, LiveDocument,
    LiveMount, LiveRegistry, MountFlags, live,
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Crypt, EncryptionKey, HttpResponse, MiddlewareRegistry, Request, Router, StatusCode,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

pub struct FeedUpdated;

impl EventPayloadMetadata for FeedUpdated {
    const NAME: &'static str = "feed.updated";
    const VERSION: u16 = 1;
}

pub struct FeedArchived;

impl EventPayloadMetadata for FeedArchived {
    const NAME: &'static str = "feed.archived";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "tests.single-stream",
    view = "live/tests/bootstrap-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "feed", topics("feed"), events(FeedUpdated)))
)]
pub struct SingleStream {
    #[public]
    headline: String,
}

#[live]
impl SingleStream {
    #[action]
    pub fn refresh(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "tests.two-streams",
    view = "live/tests/two-streams.html",
    minimum_protocol_version = 2,
    streams(
        stream(name = "feed", topics("feed"), events(FeedUpdated)),
        stream(name = "archive", topics("archive"), events(FeedArchived)),
    )
)]
pub struct TwoStreams {
    #[public]
    headline: String,
}

#[live]
impl TwoStreams {
    #[action]
    pub fn refresh(&mut self) {}
}

#[suprnova::view(path = "live/tests/bootstrap-document.html")]
struct StreamDocument<'a> {
    bootstrap: &'a TrustedHtml,
    first: &'a TrustedHtml,
    second: &'a TrustedHtml,
    third: &'a TrustedHtml,
}

fn island(html: &str, document_key: &str) -> String {
    let needle = format!("data-suprnova-live-document-key=\"{document_key}\"");
    let position = html.find(&needle).expect("island present");
    let start = html[..position].rfind('<').expect("tag start");
    let end = html[position..].find('>').expect("tag end") + position;
    html[start..=end].to_owned()
}

#[tokio::test]
#[serial_test::serial]
async fn only_a_single_declared_stream_becomes_the_island_directive() {
    static CRYPT: OnceLock<()> = OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    let _container = suprnova::container::testing::TestContainer::fake();
    App::singleton(
        LiveRegistry::builder()
            .register::<SingleStream>()
            .expect("register single")
            .register::<TwoStreams>()
            .expect("register two")
            .build(),
    );
    let single = LiveMount::<SingleStream>::public_seed("/streams", "single", "stream-single")
        .expect("declare single");
    let two =
        LiveMount::<TwoStreams>::public_seed("/streams", "two", "stream-two").expect("declare two");
    let (handler_single, handler_two) = (single.clone(), two.clone());
    let router: Router = Router::new()
        .get("/streams", move |request: Request| {
            let single = handler_single.clone();
            let two = handler_two.clone();
            async move {
                let mut document = LiveDocument::from_request(&request)
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                let parameters = || CanonicalValue::Object(BTreeMap::new());
                let first = document
                    .mount(&single, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                let second = document
                    .mount(&two, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                let bootstrap = document
                    .bootstrap(LiveBootstrapOptions::esm())
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                let empty = TrustedHtml::framework_static(
                    "",
                    suprnova::view::TrustedMarkupReason::new("empty slot").expect("reason"),
                )
                .expect("empty markup");
                document
                    .render(
                        ViewName::parse("live/tests/bootstrap-document.html").expect("view"),
                        &StreamDocument {
                            bootstrap: bootstrap.html(),
                            first: first.html(),
                            second: second.html(),
                            third: &empty,
                        },
                        DocumentResponseIntent::html(StatusCode::OK).expect("intent"),
                        AssetSet::empty(),
                    )
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))
            }
        })
        .into();
    let router = router
        .try_live()
        .expect("install Live")
        .try_live_mount(&single)
        .expect("register single mount")
        .try_live_mount(&two)
        .expect("register two mount");
    prepare_live_router_for_test(&router).expect("prepare runtime");

    let (status, _, body) = dispatch(
        Arc::new(router),
        Arc::new(MiddlewareRegistry::new()),
        get("/streams"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let html = String::from_utf8(body.to_vec()).expect("UTF-8");
    let single_root = island(&html, "stream-single");
    assert!(
        single_root.contains("live:stream=\"feed\""),
        "one declared stream becomes the island directive: {single_root}"
    );
    let two_root = island(&html, "stream-two");
    assert!(
        !two_root.contains("live:stream"),
        "several declared streams get no island-owned directive: {two_root}"
    );
}
