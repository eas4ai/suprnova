//! Real Suprnova server that serves Live documents and artifacts for the browser matrix.
//!
//! The Playwright suite in `crates/suprnova-live/browser` starts this binary
//! and drives it like an application deployment: every document is rendered
//! through `LiveDocument`, every artifact is served by the framework's own
//! immutable asset routes, and the hostile scenarios (duplicate boot tags, an
//! incompatible optional feature, a tampered integrity value) are produced
//! only by what an application could do with the public API.

use std::collections::BTreeMap;

use hyper::header::{HeaderName, HeaderValue};
use suprnova::live::assets::live_asset_catalog;
use suprnova::live::{
    CanonicalValue, EventPayloadMetadata, LiveBootstrapOptions, LiveBootstrapStrategy,
    LiveComponent, LiveDocument, LiveMount, LiveRegistry, MountFlags, MountedIsland, UploadPolicy,
    UploadReplacement, UploadScan, UploadType, live,
};
use suprnova::view::{
    AssetSet, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason, ViewName,
};
use suprnova::{App, HttpResponse, Request, Router, Server, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

const STRICT_CSP: &str = "default-src 'none'; script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'";
const INCOMPATIBLE_PROBE: &str = r#"const surface = globalThis[Symbol.for("suprnova.live.features.v1")];
const result = surface?.register(Object.freeze([Symbol.for("suprnova.live.feature.v1"), 1, 99, 0, Object.freeze({}), () => true]));
globalThis.__suprnovaFrameworkIncompatible ??= [];
globalThis.__suprnovaFrameworkIncompatible.push("async:" + result);
"#;

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

pub struct FeedUpdated;

impl EventPayloadMetadata for FeedUpdated {
    const NAME: &'static str = "feed.updated";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "examples.host-counter",
    view = "live/examples/host-counter.html"
)]
pub struct HostCounter {
    #[public]
    count: u64,
}

#[live]
impl HostCounter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

#[derive(LiveComponent)]
#[live(
    name = "examples.host-uploader",
    view = "live/examples/host-uploader.html"
)]
pub struct HostUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl HostUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "examples.host-feed",
    view = "live/examples/host-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "feed", topics("feed"), events(FeedUpdated)))
)]
pub struct HostFeed {
    #[public]
    headline: String,
}

#[live]
impl HostFeed {
    #[action]
    pub fn refresh(&mut self) {}
}

#[suprnova::view(path = "live/examples/bootstrap-host.html")]
struct HostDocument<'a> {
    bootstrap: &'a TrustedHtml,
    extra: &'a TrustedHtml,
    first: &'a TrustedHtml,
    second: &'a TrustedHtml,
    third: &'a TrustedHtml,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Esm,
    Classic,
    CoreOnly,
    Stimulus,
    Duplicate,
    IncompatibleAsync,
    IntegrityFailure,
    Csp,
}

impl Scenario {
    const ALL: [Self; 8] = [
        Self::Esm,
        Self::Classic,
        Self::CoreOnly,
        Self::Stimulus,
        Self::Duplicate,
        Self::IncompatibleAsync,
        Self::IntegrityFailure,
        Self::Csp,
    ];

    const fn path(self) -> &'static str {
        match self {
            Self::Esm => "/esm",
            Self::Classic => "/classic",
            Self::CoreOnly => "/core-only",
            Self::Stimulus => "/stimulus",
            Self::Duplicate => "/duplicate",
            Self::IncompatibleAsync => "/incompatible-async",
            Self::IntegrityFailure => "/integrity-failure",
            Self::Csp => "/csp",
        }
    }

    fn options(self) -> LiveBootstrapOptions {
        match self {
            Self::Classic => LiveBootstrapOptions::classic(),
            Self::Stimulus => LiveBootstrapOptions::esm().with_stimulus(),
            _ => LiveBootstrapOptions::esm(),
        }
    }
}

#[derive(Clone)]
struct Mounts {
    counter: LiveMount<HostCounter>,
    second_counter: LiveMount<HostCounter>,
    uploader: LiveMount<HostUploader>,
    feed: LiveMount<HostFeed>,
}

impl Mounts {
    fn declare(route: &str) -> Self {
        Self {
            counter: LiveMount::<HostCounter>::public_seed(route, "counter", "host-counter")
                .expect("counter mount"),
            second_counter: LiveMount::<HostCounter>::public_seed(
                route,
                "second-counter",
                "host-second-counter",
            )
            .expect("second counter mount"),
            uploader: LiveMount::<HostUploader>::public_seed(route, "uploader", "host-uploader")
                .expect("uploader mount"),
            feed: LiveMount::<HostFeed>::public_seed(route, "feed", "host-feed")
                .expect("feed mount"),
        }
    }
}

fn empty_markup() -> TrustedHtml {
    TrustedHtml::framework_static("", TrustedMarkupReason::new("empty slot").expect("reason"))
        .expect("empty markup")
}

fn extra_markup(html: String) -> TrustedHtml {
    TrustedHtml::framework_generated(
        html,
        TrustedMarkupReason::new("example-only hostile bootstrap variant").expect("reason"),
    )
    .expect("bounded markup")
}

async fn render(
    request: &Request,
    scenario: Scenario,
    mounts: &Mounts,
) -> Result<HttpResponse, String> {
    let catalog = live_asset_catalog().map_err(|error| error.to_string())?;
    let mut document = LiveDocument::from_request(request).map_err(|error| error.to_string())?;
    let parameters = || CanonicalValue::Object(BTreeMap::new());
    let mut islands: Vec<MountedIsland> = Vec::new();
    islands.push(
        document
            .mount(&mounts.counter, parameters(), MountFlags::empty())
            .await
            .map_err(|error| error.to_string())?,
    );
    match scenario {
        Scenario::Esm | Scenario::Classic | Scenario::Csp => {
            islands.push(
                document
                    .mount(&mounts.uploader, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| error.to_string())?,
            );
            islands.push(
                document
                    .mount(&mounts.feed, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        Scenario::IncompatibleAsync => {
            islands.push(
                document
                    .mount(&mounts.feed, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        Scenario::CoreOnly
        | Scenario::Stimulus
        | Scenario::Duplicate
        | Scenario::IntegrityFailure => {
            islands.push(
                document
                    .mount(&mounts.second_counter, parameters(), MountFlags::empty())
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
    }
    let bootstrap = document
        .bootstrap(scenario.options())
        .map_err(|error| error.to_string())?;
    let boot = catalog.boot_script(LiveBootstrapStrategy::Esm);
    let extra = match scenario {
        Scenario::Duplicate => extra_markup(format!(
            "<script type=\"module\" src=\"{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>",
            catalog.url(boot.file()),
            boot.sri()
        )),
        Scenario::IncompatibleAsync => extra_markup(
            "<script type=\"module\" src=\"/incompatible-async.js\"></script>".to_owned(),
        ),
        _ => empty_markup(),
    };
    let empty = empty_markup();
    let mut intent =
        DocumentResponseIntent::html(StatusCode::OK).map_err(|error| error.to_string())?;
    if scenario == Scenario::Csp {
        intent = intent
            .with_header(
                HeaderName::from_static("content-security-policy"),
                HeaderValue::from_static(STRICT_CSP),
            )
            .map_err(|error| error.to_string())?;
    }
    let response = document
        .render(
            ViewName::parse("live/examples/bootstrap-host.html")
                .map_err(|error| error.to_string())?,
            &HostDocument {
                bootstrap: bootstrap.html(),
                extra: &extra,
                first: islands.first().map_or(&empty, MountedIsland::html),
                second: islands.get(1).map_or(&empty, MountedIsland::html),
                third: islands.get(2).map_or(&empty, MountedIsland::html),
            },
            intent,
            AssetSet::empty(),
        )
        .map_err(|error| error.to_string())?;
    if scenario == Scenario::IntegrityFailure {
        // An application that hand-edits its bootstrap markup: the boot script
        // keeps its URL but carries a wrong digest, so the browser refuses it.
        let html =
            String::from_utf8(response.body().to_vec()).map_err(|error| error.to_string())?;
        let tampered = html.replace(
            boot.sri(),
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        );
        return Ok(HttpResponse::html(tampered));
    }
    Ok(response)
}

fn scenario_route(router: Router, scenario: Scenario) -> Router {
    let mounts = Mounts::declare(scenario.path());
    let handler_mounts = mounts.clone();
    let router: Router = router
        .get(scenario.path(), move |request: Request| {
            let mounts = handler_mounts.clone();
            async move {
                render(&request, scenario, &mounts).await.map_err(|error| {
                    HttpResponse::text(format!("live document failed: {error}")).status(500)
                })
            }
        })
        .into();
    router
        .try_live_mount(&mounts.counter)
        .expect("register counter")
        .try_live_mount(&mounts.second_counter)
        .expect("register second counter")
        .try_live_mount(&mounts.uploader)
        .expect("register uploader")
        .try_live_mount(&mounts.feed)
        .expect("register feed")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = std::env::var("SUPRNOVA_LIVE_BOOTSTRAP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4177);
    // `Server::run` installs the key ring itself; a local environment gets a transient key.
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<HostCounter>()
            .expect("register counter")
            .register::<HostUploader>()
            .expect("register uploader")
            .register::<HostFeed>()
            .expect("register feed")
            .build(),
    );
    let mut router: Router = Router::new()
        .get("/health", |_request: Request| async {
            Ok(HttpResponse::text("ok"))
        })
        .get("/identity", |_request: Request| async {
            live_asset_catalog()
                .map(|catalog| HttpResponse::text(catalog.identity().to_owned()))
                .map_err(|error| HttpResponse::text(error.to_string()).status(503))
        })
        .get("/incompatible-async.js", |_request: Request| async {
            Ok(HttpResponse::bytes_body(
                INCOMPATIBLE_PROBE.as_bytes().to_vec(),
                "text/javascript; charset=utf-8",
            )
            .header("Cache-Control", "no-store"))
        })
        .into();
    router = router.try_live()?;
    for scenario in Scenario::ALL {
        router = scenario_route(router, scenario);
    }
    Server::new(router).host("127.0.0.1").port(port).run().await
}
