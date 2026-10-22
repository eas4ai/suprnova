use std::collections::BTreeMap;

use suprnova::live::{
    CanonicalValue, LiveComponent, LiveDocument, LiveMount, LiveRegistry, MountFlags, live,
    testing::{LiveSecurityCheck, record_live_security_pass_for_test},
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Crypt, EncryptionKey, FrameworkError, HttpResponse, Middleware, Next, Request, Response,
    Router, Server, StatusCode, async_trait,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[derive(LiveComponent)]
#[live(name = "fixtures.counter", view = "live/counter.html")]
pub struct Counter {
    #[public]
    count: u64,
}

#[live]
impl Counter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

#[suprnova::view(path = "live/document.html")]
struct DocumentView<'a> {
    island: &'a TrustedHtml,
}

struct LiveSecurityFacts;

#[async_trait]
impl Middleware for LiveSecurityFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        for (check, fact) in [
            (
                LiveSecurityCheck::Session,
                Some(b"fixture-session".as_slice()),
            ),
            (LiveSecurityCheck::Origin, None),
            (LiveSecurityCheck::Csrf, None),
            (
                LiveSecurityCheck::Principal,
                Some(b"fixture-principal".as_slice()),
            ),
            (
                LiveSecurityCheck::Tenant,
                Some(b"fixture-tenant".as_slice()),
            ),
            (LiveSecurityCheck::RateLimit, None),
        ] {
            if !record_live_security_pass_for_test(&mut request, check, fact) {
                return Err(HttpResponse::text("Live security context failed").status(500));
            }
        }
        next(request).await
    }
}

fn routes(mount: &LiveMount<Counter>) -> Result<Router, FrameworkError> {
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/counter", move |request: Request| {
            let mount = handler_mount.clone();
            async move {
                let result: Result<HttpResponse, FrameworkError> = async {
                    let mut document = LiveDocument::from_request(&request)?;
                    let island = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await?;
                    document
                        .render(
                            ViewName::parse("live/document.html")
                                .map_err(|_| FrameworkError::internal("fixture view identity"))?,
                            &DocumentView {
                                island: island.html(),
                            },
                            DocumentResponseIntent::html(StatusCode::OK)
                                .map_err(|_| FrameworkError::internal("fixture response intent"))?,
                            AssetSet::empty(),
                        )
                        .map_err(FrameworkError::from)
                }
                .await;
                result.map_err(|_| HttpResponse::text("Live document failed").status(500))
            }
        })
        .into();
    router.try_live()?.try_live_mount(mount)
}

#[suprnova::main]
async fn main() {
    Crypt::init(EncryptionKey::generate());
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<Counter>()
            .expect("register fixture component")
            .build(),
    );
    let mount = LiveMount::<Counter>::public_seed("/counter", "counter", "fixture-counter")
        .expect("declare fixture mount");
    let router = routes(&mount).expect("install fixture routes");
    let port = std::env::var("SUPRNOVA_LIVE_FIXTURE_PORT")
        .expect("SUPRNOVA_LIVE_FIXTURE_PORT")
        .parse::<u16>()
        .expect("valid fixture port");

    if let Err(error) = Server::new(router)
        .host("127.0.0.1")
        .port(port)
        .middleware(LiveSecurityFacts)
        .run()
        .await
    {
        eprintln!("fixture server failed: {error}");
        std::process::exit(1);
    }
}
