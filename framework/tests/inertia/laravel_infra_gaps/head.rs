//! The document head (PAR-160, PAR-161): `Head` resolves a page's head
//! from five layers (defaults, route group, route, run time, error status)
//! field by field, as Laravel Head does; renders it as escaped tags, as
//! data, and as the Inertia `head` prop with stable `data-inertia` keys;
//! and writes it once into the first visit's `<head>`.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;
use suprnova::head::{
    HeadBuilder, ImageType, Media, OgMedia, OgType, OpenGraph, Schema, TwitterCard, TwitterCardType,
};
use suprnova::testing::{TestClient, TestContainer, TestResponse};
use suprnova::{
    FrameworkError, Head, HttpResponse, InertiaConfig, InertiaErrorPageMiddleware,
    InertiaRequestExt, InertiaResponse, MiddlewareRegistry, Request, Response, Router, SsrConfig,
    SsrGateway, SsrResponse, async_trait,
};

/// Every page renders with this configuration, so the document does not
/// depend on the process environment.
fn config() -> InertiaConfig {
    InertiaConfig::new()
        .development(false)
        .ssr_disabled()
        .manifest_path("/nonexistent/manifest.json")
}

async fn page(request: &Request, component: &str) -> Response {
    InertiaResponse::new(component)
        .with_config(config())
        .resolve(request)
        .await
        .map_err(HttpResponse::from)
}

mod routes {
    use super::*;
    use suprnova::{get, group, routes};

    async fn about(request: Request) -> Response {
        Head::title("About");
        page(&request, "About").await
    }

    async fn plain(request: Request) -> Response {
        page(&request, "Plain").await
    }

    async fn titled(request: Request) -> Response {
        InertiaResponse::new("Titled")
            .with_config(config())
            .title("From the response")
            .resolve(&request)
            .await
            .map_err(HttpResponse::from)
    }

    async fn hostile(request: Request) -> Response {
        Head::title("<script>alert(1)</script>").description("\"quoted\" & <b>bold</b>");
        page(&request, "Hostile").await
    }

    async fn missing(_request: Request) -> Response {
        Head::title("Run time title");
        Err(HttpResponse::from(FrameworkError::not_found(
            "No such post",
        )))
    }

    async fn canonical(request: Request) -> Response {
        Head::canonical();
        page(&request, "Canonical").await
    }

    async fn layered(request: Request) -> Response {
        Head::description("Run time description");
        page(&request, "Layered").await
    }

    routes! {
        get!("/about", about),
        get!("/plain", plain),
        get!("/titled", titled),
        get!("/hostile", hostile),
        get!("/canonical", canonical),
        get!("/route-head", about).with_head(|head| head
            .title("Route title")
            .description("Route description")),
        get!("/missing", missing).with_head(|head| head.title("Route title")),
        group!("/admin", {
            get!("/layered", layered).with_head(|head| head.title("Dashboard")),
        }).with_head(|head| head.robots("noindex, nofollow").description("Group description")),
    }
}

fn client() -> TestClient {
    TestClient::new(
        routes::register(),
        MiddlewareRegistry::new().append(InertiaErrorPageMiddleware::new("Error")),
    )
}

/// The `<head>` element of a first visit.
fn document_head(response: &TestResponse) -> String {
    let body = response.body_text();
    let start = body.find("<head>").expect("a <head> element");
    let end = body.find("</head>").expect("a </head> end tag");
    body[start..end].to_string()
}

/// The `head` prop of an Inertia visit, as strings.
fn head_prop(response: &TestResponse, prop: &str) -> Option<Vec<String>> {
    let props = response.inertia_props(None);
    props.get(prop).map(|value| {
        value
            .as_array()
            .expect("the head prop is a list")
            .iter()
            .map(|tag| tag.as_str().expect("a rendered tag").to_string())
            .collect()
    })
}

fn laravel_defaults() {
    Head::defaults(|head| head.title("Laravel").title_suffix(" - Laravel"));
}

// ---- Layers ---------------------------------------------------------------

#[tokio::test]
async fn a_run_time_title_inherits_the_default_suffix_once() {
    let _container = TestContainer::fake();
    laravel_defaults();
    let client = client();

    let first = client
        .get("/about")
        .header("Accept", "text/html")
        .send()
        .await;
    first.assert_ok();
    let head = document_head(&first);
    assert_eq!(head.matches("<title").count(), 1, "{head}");
    assert!(
        head.contains("<title data-inertia=\"title\">About - Laravel</title>"),
        "{head}"
    );

    let visit = client.get("/about").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.contains(&"<title data-inertia=\"title\">About - Laravel</title>".to_string()),
        "{tags:?}"
    );
}

#[tokio::test]
async fn the_default_title_renders_as_it_is_without_a_higher_title() {
    let _container = TestContainer::fake();
    laravel_defaults();
    let visit = client().get("/plain").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.contains(&"<title data-inertia=\"title\">Laravel</title>".to_string()),
        "{tags:?}"
    );
}

#[test]
fn an_exact_title_ignores_the_inherited_suffix() {
    let base = HeadBuilder::new().title("Base").title_suffix(" | Site");
    let html = base.clone().render_html();
    assert!(
        html.contains(">Base</title>"),
        "the defining layer renders as it is: {html}"
    );
    let html = base
        .clone()
        .merge(HeadBuilder::new().title("Page"))
        .render_html();
    assert!(html.contains(">Page | Site</title>"), "{html}");
    let html = base
        .merge(HeadBuilder::new().exact_title("Exact"))
        .render_html();
    assert!(html.contains(">Exact</title>"), "{html}");
}

#[tokio::test]
async fn a_route_description_survives_a_run_time_title() {
    let _container = TestContainer::fake();
    let visit = client().get("/route-head").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.contains(&"<title data-inertia=\"title\">About</title>".to_string()),
        "the run time title replaces the route's: {tags:?}"
    );
    assert!(
        tags.contains(
            &"<meta data-inertia=\"description\" name=\"description\" content=\"Route description\">"
                .to_string()
        ),
        "the route description stays: {tags:?}"
    );
}

#[tokio::test]
async fn group_route_and_run_time_layers_merge_field_by_field() {
    let _container = TestContainer::fake();
    let visit = client().get("/admin/layered").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    for expected in [
        "<title data-inertia=\"title\">Dashboard</title>",
        "<meta data-inertia=\"description\" name=\"description\" content=\"Run time description\">",
        "<meta data-inertia=\"robots\" name=\"robots\" content=\"noindex, nofollow\">",
    ] {
        assert!(
            tags.contains(&expected.to_string()),
            "{expected} in {tags:?}"
        );
    }
}

#[tokio::test]
async fn error_metadata_wins_for_its_status() {
    let _container = TestContainer::fake();
    laravel_defaults();
    Head::errors(|errors| {
        errors
            .defaults(|head| head.robots("noindex, follow"))
            .status(404, |head| head.title("Page Not Found"))
    });
    let client = client();

    let first = client
        .get("/missing")
        .header("Accept", "text/html")
        .send()
        .await;
    first.assert_status(404);
    let head = document_head(&first);
    assert!(head.contains(">Page Not Found - Laravel</title>"), "{head}");
    assert!(!head.contains("Route title"), "{head}");
    assert!(!head.contains("Run time title"), "{head}");
    assert!(head.contains("content=\"noindex, follow\""), "{head}");

    let visit = client.get("/missing").inertia().send().await;
    visit.assert_status(404);
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.contains(
            &"<title data-inertia=\"title\">Page Not Found - Laravel</title>".to_string()
        ),
        "{tags:?}"
    );
}

// ---- Escaping ---------------------------------------------------------------

#[tokio::test]
async fn every_value_is_escaped() {
    let _container = TestContainer::fake();
    let client = client();
    let first = client
        .get("/hostile")
        .header("Accept", "text/html")
        .send()
        .await;
    let head = document_head(&first);
    assert!(!head.contains("<script>alert"), "{head}");
    assert!(
        head.contains(
            "<title data-inertia=\"title\">&lt;script&gt;alert(1)&lt;/script&gt;</title>"
        ),
        "{head}"
    );
    assert!(
        head.contains("content=\"&quot;quoted&quot; &amp; &lt;b&gt;bold&lt;/b&gt;\""),
        "{head}"
    );

    let visit = client.get("/hostile").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(tags.iter().all(|tag| !tag.contains("<script>")), "{tags:?}");
}

#[test]
fn a_schema_cannot_close_its_script_element() {
    let _container = TestContainer::fake();
    Head::defaults(|head| {
        head.schema(Schema::of("Product").set("name", "</script><script>alert(1)</script>"))
    });
    let html = Head::render_html();
    assert_eq!(
        html.matches("</script>").count(),
        1,
        "the element closes once, at its own end tag: {html}"
    );
    assert!(
        html.contains("<script data-inertia=\"schema:0\" type=\"application/ld+json\">"),
        "{html}"
    );
    assert!(html.contains("\\u003c/script\\u003e"), "{html}");
    let data = Head::to_array();
    let schema = data
        .as_array()
        .expect("a list of tags")
        .iter()
        .find(|tag| tag["key"] == "schema:0")
        .expect("the schema tag");
    assert_eq!(
        schema["content"]["name"],
        "</script><script>alert(1)</script>"
    );
    assert_eq!(schema["content"]["@type"], "Product");
    assert_eq!(schema["content"]["@context"], "https://schema.org");
}

// ---- The Inertia head prop ----------------------------------------------------

#[tokio::test]
async fn a_partial_reload_carries_no_head_prop() {
    let _container = TestContainer::fake();
    laravel_defaults();
    let visit = client()
        .get("/about")
        .inertia()
        .header("X-Inertia-Partial-Component", "About")
        .header("X-Inertia-Partial-Data", "other")
        .send()
        .await;
    assert!(head_prop(&visit, "head").is_none(), "{}", visit.body_text());
}

#[tokio::test]
async fn a_response_no_layer_sets_anything_for_carries_no_head_prop() {
    let _container = TestContainer::fake();
    let client = client();
    let visit = client.get("/plain").inertia().send().await;
    assert!(head_prop(&visit, "head").is_none(), "{}", visit.body_text());

    // The response's own title keeps working without a Head title.
    let first = client
        .get("/titled")
        .header("Accept", "text/html")
        .send()
        .await;
    let head = document_head(&first);
    assert!(head.contains("<title>From the response</title>"), "{head}");
    assert!(!head.contains("data-inertia"), "{head}");
}

#[tokio::test]
async fn the_head_prop_takes_the_name_head_inertia_gives_it() {
    let _container = TestContainer::fake();
    laravel_defaults();
    Head::inertia("_head");
    let visit = client().get("/about").inertia().send().await;
    assert!(head_prop(&visit, "head").is_none());
    let tags = head_prop(&visit, "_head").expect("the renamed prop");
    assert!(
        tags.iter().any(|tag| tag.contains("About - Laravel")),
        "{tags:?}"
    );
}

#[tokio::test]
async fn an_inertia_global_is_written_into_the_first_visit_only() {
    let _container = TestContainer::fake();
    laravel_defaults();
    Head::inertia_globals(|head| {
        head.color_scheme("light dark")
            .manifest("/site.webmanifest")
    });
    let client = client();

    let first = client
        .get("/about")
        .header("Accept", "text/html")
        .send()
        .await;
    let head = document_head(&first);
    assert!(
        head.contains("<meta name=\"color-scheme\" content=\"light dark\">"),
        "{head}"
    );
    assert!(
        head.contains("<link rel=\"manifest\" href=\"/site.webmanifest\">"),
        "{head}"
    );

    let visit = client.get("/about").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.iter()
            .all(|tag| !tag.contains("color-scheme") && !tag.contains("manifest")),
        "{tags:?}"
    );
}

/// An SSR gateway that answers with a head already holding the page's
/// keyed title, as Inertia's SSR does with `serverHead: true`.
struct KeyedSsr;

#[async_trait]
impl SsrGateway for KeyedSsr {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        _page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        Ok(Some(SsrResponse {
            head: vec![
                "<title data-inertia=\"title\">About - Laravel</title>".to_string(),
                "<link rel=\"stylesheet\" href=\"/ssr.css\">".to_string(),
            ],
            body: "<div data-server-rendered=\"true\" id=\"app\"></div>".to_string(),
        }))
    }
}

#[tokio::test]
async fn the_first_visit_deduplicates_against_the_ssr_head_by_key() {
    let _container = TestContainer::fake();
    laravel_defaults();
    Head::defaults(|head| head.description("Build something great."));
    TestContainer::bind::<dyn SsrGateway>(Arc::new(KeyedSsr));
    let router: Router = Router::new()
        .get("/ssr", |request: Request| async move {
            Head::title("About");
            InertiaResponse::new("About")
                .with_config(
                    config()
                        .ssr("http://127.0.0.1:9")
                        .ssr_ensure_bundle_exists(false),
                )
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let first = TestClient::new(router, MiddlewareRegistry::new())
        .get("/ssr")
        .header("Accept", "text/html")
        .send()
        .await;
    let head = document_head(&first);
    assert_eq!(head.matches("<title").count(), 1, "{head}");
    assert_eq!(head.matches("data-inertia=\"title\"").count(), 1, "{head}");
    assert_eq!(head.matches("name=\"description\"").count(), 1, "{head}");
    assert!(
        head.contains("<link rel=\"stylesheet\" href=\"/ssr.css\">"),
        "{head}"
    );
}

// ---- Canonical URLs -----------------------------------------------------------

#[tokio::test]
async fn canonical_names_the_request_url_over_https() {
    if crate::own_process_async::delegate(
        module_path!(),
        "canonical_names_the_request_url_over_https",
    )
    .await
    {
        return;
    }
    suprnova::config::Config::register(
        suprnova::config::AppConfig::builder()
            .url("http://example.test")
            .build(),
    );
    let _container = TestContainer::fake();
    let visit = client().get("/canonical").inertia().send().await;
    let tags = head_prop(&visit, "head").expect("a head prop");
    assert!(
        tags.contains(
            &"<link data-inertia=\"canonical\" rel=\"canonical\" href=\"https://example.test/canonical\">"
                .to_string()
        ),
        "{tags:?}"
    );

    let kept = HeadBuilder::new()
        .canonical_url("http://example.test/kept")
        .canonical_keep_scheme()
        .render_html();
    assert!(kept.contains("href=\"http://example.test/kept\""), "{kept}");
    let forced = HeadBuilder::new().canonical_url("/about").render_html();
    assert!(
        forced.contains("href=\"https://example.test/about\""),
        "{forced}"
    );
}

// ---- PAR-161: the metadata beyond the title -------------------------------------

#[test]
fn og_images_are_keyed_by_url() {
    let html = HeadBuilder::new()
        .og_image("/a.jpg")
        .og_image(OgMedia::new("/b.jpg").alt("Gallery"))
        .og_image(
            OgMedia::new("/a.jpg")
                .alt("Final")
                .width(1200)
                .height(630)
                .kind(ImageType::Jpeg),
        )
        .render_html();
    assert_eq!(html.matches("property=\"og:image\" ").count(), 2, "{html}");
    assert_eq!(html.matches("content=\"/a.jpg\"").count(), 1, "{html}");
    let first = html.find("content=\"/a.jpg\"").expect("the first image");
    let second = html.find("content=\"/b.jpg\"").expect("the second image");
    assert!(first < second, "the updated entry keeps its place: {html}");
    for expected in [
        "property=\"og:image:width\" content=\"1200\"",
        "property=\"og:image:height\" content=\"630\"",
        "property=\"og:image:alt\" content=\"Final\"",
        "property=\"og:image:type\" content=\"image/jpeg\"",
        "property=\"og:image:alt\" content=\"Gallery\"",
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
}

#[test]
fn page_media_replaces_default_media() {
    let html = HeadBuilder::new()
        .og_image("/default.jpg")
        .merge(HeadBuilder::new().og_image("/page.jpg"))
        .render_html();
    assert!(html.contains("/page.jpg"), "{html}");
    assert!(!html.contains("/default.jpg"), "{html}");
}

#[test]
fn the_document_title_and_description_fill_open_graph() {
    let html = HeadBuilder::new()
        .title("Introducing Head")
        .description("A fluent API.")
        .og(OpenGraph::new().kind(OgType::Article).site_name("Laravel"))
        .render_html();
    for expected in [
        "property=\"og:type\" content=\"article\"",
        "property=\"og:site_name\" content=\"Laravel\"",
        "property=\"og:title\" content=\"Introducing Head\"",
        "property=\"og:description\" content=\"A fluent API.\"",
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
    let explicit = HeadBuilder::new()
        .title("Document")
        .og(OpenGraph::new().title("Social"))
        .render_html();
    assert!(
        explicit.contains("property=\"og:title\" content=\"Social\""),
        "{explicit}"
    );
}

#[test]
fn twitter_cards_fill_from_the_page() {
    let defaults =
        HeadBuilder::new().twitter(TwitterCard::new().card(TwitterCardType::SummaryLargeImage));
    let html = defaults
        .merge(
            HeadBuilder::new()
                .title("Introducing Head")
                .description("A fluent API.")
                .og_image(OgMedia::new("https://example.com/social.jpg").alt("Social card")),
        )
        .render_html();
    for expected in [
        "name=\"twitter:card\" content=\"summary_large_image\"",
        "name=\"twitter:title\" content=\"Introducing Head\"",
        "name=\"twitter:description\" content=\"A fluent API.\"",
        "name=\"twitter:image\" content=\"https://example.com/social.jpg\"",
        "name=\"twitter:image:alt\" content=\"Social card\"",
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
}

#[test]
fn theme_colors_carry_their_media() {
    let html = HeadBuilder::new()
        .theme_color("#fff", Media::Light)
        .theme_color("#111827", Media::Dark)
        .theme_color("#0f172a", None)
        .render_html();
    assert!(
        html.contains(
            "name=\"theme-color\" content=\"#fff\" media=\"(prefers-color-scheme: light)\""
        ),
        "{html}"
    );
    assert!(
        html.contains(
            "name=\"theme-color\" content=\"#111827\" media=\"(prefers-color-scheme: dark)\""
        ),
        "{html}"
    );
    assert!(
        html.contains("name=\"theme-color\" content=\"#0f172a\">"),
        "{html}"
    );
}

#[test]
fn meta_chooses_property_for_open_graph_and_article_names() {
    let html = HeadBuilder::new()
        .meta("og:title", "x")
        .meta("article:author", "Taylor")
        .meta("format-detection", "telephone=no")
        .meta_for("theme-color", "#000", Media::query("(min-width: 600px)"))
        .render_html();
    assert!(
        html.contains("property=\"og:title\" content=\"x\""),
        "{html}"
    );
    assert!(!html.contains("name=\"og:title\""), "{html}");
    assert!(
        html.contains("property=\"article:author\" content=\"Taylor\""),
        "{html}"
    );
    assert!(
        html.contains("name=\"format-detection\" content=\"telephone=no\""),
        "{html}"
    );
    assert!(html.contains("media=\"(min-width: 600px)\""), "{html}");
}

#[test]
fn application_metadata_icons_and_pwa_render() {
    let html = HeadBuilder::new()
        .application_name("Laravel")
        .color_scheme("light dark")
        .referrer("strict-origin-when-cross-origin")
        .viewport("width=device-width, initial-scale=1")
        .apple_web_app_title("Laravel")
        .web_app_capable()
        .apple_web_app_status_bar_style("black")
        .favicon(suprnova::head::Icon::new("/favicon.svg").kind(ImageType::Svg))
        .icon(
            suprnova::head::Icon::new("/favicon-32x32.png")
                .kind(ImageType::Png)
                .sizes("32x32"),
        )
        .apple_touch_icon(suprnova::head::Icon::new("/apple-touch-icon.png").sizes("180x180"))
        .apple_touch_startup_image(suprnova::head::Icon::new("/launch.png").media(Media::Portrait))
        .mask_icon("/safari-pinned-tab.svg", "#111827")
        .manifest("/site.webmanifest")
        .render_html();
    for expected in [
        "name=\"application-name\" content=\"Laravel\"",
        "name=\"color-scheme\" content=\"light dark\"",
        "name=\"referrer\" content=\"strict-origin-when-cross-origin\"",
        "name=\"viewport\" content=\"width=device-width, initial-scale=1\"",
        "name=\"apple-mobile-web-app-title\" content=\"Laravel\"",
        "name=\"mobile-web-app-capable\" content=\"yes\"",
        "name=\"apple-mobile-web-app-status-bar-style\" content=\"black\"",
        "rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\"",
        "rel=\"icon\" href=\"/favicon-32x32.png\" type=\"image/png\" sizes=\"32x32\"",
        "rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\" sizes=\"180x180\"",
        "rel=\"apple-touch-startup-image\" href=\"/launch.png\" media=\"(orientation: portrait)\"",
        "rel=\"mask-icon\" href=\"/safari-pinned-tab.svg\" color=\"#111827\"",
        "rel=\"manifest\" href=\"/site.webmanifest\"",
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }

    let pwa = HeadBuilder::new()
        .pwa(
            suprnova::head::Pwa::new("Laravel")
                .manifest("/site.webmanifest")
                .theme_color("#0f172a")
                .apple_touch_icon("/apple-touch-icon.png")
                .apple_web_app_status_bar_style("black"),
        )
        .render_html();
    for expected in [
        "name=\"application-name\" content=\"Laravel\"",
        "rel=\"manifest\" href=\"/site.webmanifest\"",
        "name=\"theme-color\" content=\"#0f172a\"",
        "rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\"",
        "name=\"apple-mobile-web-app-title\" content=\"Laravel\"",
        "name=\"mobile-web-app-capable\" content=\"yes\"",
        "name=\"apple-mobile-web-app-status-bar-style\" content=\"black\"",
    ] {
        assert!(pwa.contains(expected), "{expected} in {pwa}");
    }
}

#[test]
fn performance_and_discovery_links_render() {
    let html = HeadBuilder::new()
        .preload(
            suprnova::head::Hint::new("/fonts/inter.woff2")
                .as_kind("font")
                .crossorigin(),
        )
        .prefetch("/images/next.webp")
        .preconnect("https://cdn.example.com")
        .dns_prefetch("https://analytics.example.com")
        .prev_page("/posts?page=1")
        .next_page("/posts?page=3")
        .alternates([
            ("en", "https://example.com/en/about"),
            ("x-default", "https://example.com/about"),
        ])
        .feed(suprnova::head::Feed::rss("/feed").title("Laravel RSS"))
        .feed(suprnova::head::Feed::atom("/feed.atom").title("Laravel Atom"))
        .link("search", "/opensearch.xml")
        .render_html();
    for expected in [
        "rel=\"preload\" href=\"/fonts/inter.woff2\" as=\"font\" crossorigin>",
        "rel=\"prefetch\" href=\"/images/next.webp\"",
        "rel=\"preconnect\" href=\"https://cdn.example.com\"",
        "rel=\"dns-prefetch\" href=\"https://analytics.example.com\"",
        "rel=\"prev\" href=\"/posts?page=1\"",
        "rel=\"next\" href=\"/posts?page=3\"",
        "rel=\"alternate\" hreflang=\"en\" href=\"https://example.com/en/about\"",
        "rel=\"alternate\" hreflang=\"x-default\" href=\"https://example.com/about\"",
        "rel=\"alternate\" type=\"application/rss+xml\" title=\"Laravel RSS\" href=\"/feed\"",
        "rel=\"alternate\" type=\"application/atom+xml\" title=\"Laravel Atom\" href=\"/feed.atom\"",
        "rel=\"search\" href=\"/opensearch.xml\"",
    ] {
        assert!(html.contains(expected), "{expected} in {html}");
    }
}

#[test]
fn breadcrumb_and_faq_schemas_follow_schema_org() {
    let _container = TestContainer::fake();
    Head::defaults(|head| {
        head.schema(
            Schema::breadcrumbs()
                .item("Home", "https://example.com/")
                .item("Shop", "https://example.com/shop"),
        )
        .schema(Schema::faq().question("Is it free?", "Yes."))
    });
    let data = Head::to_array();
    let tags = data.as_array().expect("a list of tags");
    let breadcrumbs = &tags
        .iter()
        .find(|tag| tag["key"] == "schema:0")
        .expect("the breadcrumbs")["content"];
    assert_eq!(breadcrumbs["@type"], "BreadcrumbList");
    assert_eq!(breadcrumbs["itemListElement"][1]["position"], 2);
    assert_eq!(breadcrumbs["itemListElement"][1]["name"], "Shop");
    let faq = &tags
        .iter()
        .find(|tag| tag["key"] == "schema:1")
        .expect("the FAQ")["content"];
    assert_eq!(faq["@type"], "FAQPage");
    assert_eq!(faq["mainEntity"][0]["acceptedAnswer"]["text"], "Yes.");
}

#[test]
fn when_and_robots_shortcuts_apply() {
    let html = HeadBuilder::new()
        .when(true, |head| head.hidden_from_robots())
        .when(false, |head| head.description("never"))
        .render_html();
    assert!(html.contains("name=\"robots\" content=\"none\""), "{html}");
    assert!(!html.contains("never"), "{html}");
    let rules = HeadBuilder::new()
        .robots_rules([
            suprnova::head::RobotsRule::NoIndex,
            suprnova::head::RobotsRule::NoFollow,
        ])
        .render_html();
    assert!(rules.contains("content=\"noindex, nofollow\""), "{rules}");
}

/// The keys of every `data-inertia` attribute in `html`.
fn keys_in(html: &str) -> BTreeSet<String> {
    html.split("data-inertia=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn every_kind_renders_through_html_data_and_the_head_prop_alike() {
    let _container = TestContainer::fake();
    Head::defaults(|head| {
        head.title("All")
            .description("Every kind")
            .robots("index, follow")
            .canonical_url("https://example.com/all")
            .og(OpenGraph::new().kind(OgType::Website))
            .og_image("/a.jpg")
            .og_video("/a.mp4")
            .og_audio("/a.mp3")
            .twitter(TwitterCard::new().site("@laravel"))
            .theme_color("#fff", Media::Light)
            .application_name("All")
            .color_scheme("light dark")
            .referrer("no-referrer")
            .viewport("width=device-width")
            .apple_web_app_title("All")
            .web_app_capable()
            .apple_web_app_status_bar_style("black")
            .icon("/icon.png")
            .apple_touch_icon("/touch.png")
            .apple_touch_startup_image("/launch.png")
            .mask_icon("/mask.svg", "#000")
            .manifest("/site.webmanifest")
            .preload(suprnova::head::Hint::new("/a.woff2").as_kind("font"))
            .prefetch("/next")
            .preconnect("https://cdn.example.com")
            .dns_prefetch("https://dns.example.com")
            .prev_page("/1")
            .next_page("/3")
            .alternates([("fr", "https://example.com/fr")])
            .feed(suprnova::head::Feed::rss("/feed"))
            .meta("format-detection", "telephone=no")
            .link("me", "https://social.example.com/@laravel")
            .schema(Schema::of("WebSite").set("name", "All"))
    });

    let html_keys = keys_in(&Head::render_html());
    let data_keys: BTreeSet<String> = Head::to_array()
        .as_array()
        .expect("a list of tags")
        .iter()
        .map(|tag| tag["key"].as_str().expect("a key").to_string())
        .collect();
    let visit = client().get("/plain").inertia().send().await;
    let prop_keys = keys_in(&head_prop(&visit, "head").expect("a head prop").join("\n"));
    assert_eq!(html_keys, data_keys);
    assert_eq!(html_keys, prop_keys);
    for key in [
        "title",
        "description",
        "robots",
        "canonical",
        "og:type",
        "og:image:0",
        "og:video:0",
        "og:audio:0",
        "twitter:site",
        "theme-color:(prefers-color-scheme: light)",
        "application-name",
        "manifest",
        "mask-icon",
        "preload:/a.woff2",
        "prev",
        "next",
        "alternate:fr",
        "feed:/feed",
        "meta:format-detection",
        "link:me:https://social.example.com/@laravel",
        "schema:0",
    ] {
        assert!(html_keys.contains(key), "{key} missing from {html_keys:?}");
    }
}
