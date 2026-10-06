//! PFX-006: the localization catalog URL, pagination links built from a
//! path, and a `Storage::url` with a root-relative base carry the root.
//! PFX-010: an absolute URL or a network-path reference given to a URL
//! builder is left as it is.

use std::sync::Arc;

use suprnova::{
    App, FluentTranslator, HttpResponse, InertiaResponse, LengthAwarePaginator, Locale,
    LocaleShare, LocalizationConfig, MiddlewareRegistry, Paginator, Request, Router, Storage,
    Translator, url,
};

use crate::support::{self, PREFIX};

fn bind_translator() {
    let config = LocalizationConfig {
        default_locale: Locale::parse("en").expect("a locale"),
        fallback_locale: Locale::parse("en").expect("a locale"),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
        parents: Default::default(),
    };
    let dir = std::env::temp_dir().join(format!("pfx-006-lang-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("en")).expect("the catalog directory");
    std::fs::write(dir.join("en/messages.ftl"), "welcome = Welcome\n").expect("a catalog");
    let translator = FluentTranslator::from_dir(&dir, &config).expect("a translator");
    App::bind::<dyn Translator>(Arc::new(translator));
}

#[tokio::test]
async fn pfx_006_the_lang_prop_catalog_url_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_006_the_lang_prop_catalog_url_carries_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    bind_translator();
    App::register_inertia_shared(Arc::new(LocaleShare));
    let router: Router = Router::new()
        .get("/page", |request: Request| async move {
            InertiaResponse::new("Page")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;

    let page = support::get(
        address,
        "/page",
        &[("x-forwarded-prefix", PREFIX), ("x-inertia", "true")],
    )
    .await
    .json();
    let url = page["props"]["lang"]["catalog"]["url"]
        .as_str()
        .unwrap_or_else(|| panic!("a catalog URL: {page}"))
        .to_owned();
    assert!(
        url.starts_with("/billing/_suprnova/lang/en.ftl?v="),
        "{url}"
    );

    // The proxy strips the root; what is left is the catalog the
    // framework serves.
    let forwarded = url.strip_prefix(PREFIX).expect("the root");
    let catalog = support::get_prefixed(address, forwarded).await;
    assert_eq!(catalog.status, 200, "{}", catalog.body);
    assert!(
        catalog.body.contains("welcome = Welcome"),
        "{}",
        catalog.body
    );

    let at_host_root = support::get(address, "/page", &[("x-inertia", "true")])
        .await
        .json();
    let url = at_host_root["props"]["lang"]["catalog"]["url"]
        .as_str()
        .expect("a catalog URL");
    assert!(url.starts_with("/_suprnova/lang/en.ftl?v="), "{url}");
}

#[tokio::test]
async fn pfx_006_pagination_links_built_from_a_path_carry_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_006_pagination_links_built_from_a_path_carry_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let router: Router = Router::new()
        .get("/users", |_request: Request| async {
            let from_path = LengthAwarePaginator::new(vec![1, 2], 10, 2, 1).with_path("/users");
            let rooted =
                LengthAwarePaginator::new(vec![1, 2], 10, 2, 1).with_path("/billing/users");
            let query_only = LengthAwarePaginator::new(vec![1, 2], 10, 2, 1);
            let simple = Paginator::new(vec![1, 2], 1, 2, true).with_path("/users?role=admin");
            let links = [
                from_path.next_page_url().unwrap_or_default(),
                rooted.next_page_url().unwrap_or_default(),
                query_only.next_page_url().unwrap_or_default(),
                simple.next_page_url().unwrap_or_default(),
            ];
            Ok(HttpResponse::text(links.join("\n")))
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;

    let behind = support::get_prefixed(address, "/users").await;
    assert_eq!(
        behind.body.lines().collect::<Vec<_>>(),
        [
            "/billing/users?page=2",
            "/billing/users?page=2",
            "?page=2",
            "/billing/users?role=admin&page=2",
        ]
    );
    let at_host_root = support::get(address, "/users", &[]).await;
    assert_eq!(at_host_root.body.lines().next(), Some("/users?page=2"));
}

#[tokio::test]
async fn pfx_006_a_storage_url_with_a_root_relative_base_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_006_a_storage_url_with_a_root_relative_base_carries_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    Storage::register_memory("public");
    Storage::set_public_url("public", "/storage/").expect("a base URL");
    Storage::register_memory("cdn");
    Storage::set_public_url("cdn", "https://cdn.example/files").expect("a base URL");
    let router: Router = Router::new()
        .get("/files", |_request: Request| async {
            let urls = [
                Storage::url("public", "a b.png"),
                Storage::url("cdn", "a b.png"),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            Ok(HttpResponse::text(urls.join("\n")))
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;

    let behind = support::get_prefixed(address, "/files").await;
    assert_eq!(
        behind.body.lines().collect::<Vec<_>>(),
        [
            "/billing/storage/a%20b.png",
            "https://cdn.example/files/a%20b.png"
        ]
    );
    let at_host_root = support::get(address, "/files", &[]).await;
    assert_eq!(at_host_root.body.lines().next(), Some("/storage/a%20b.png"));
}

#[tokio::test]
async fn pfx_010_an_absolute_url_or_a_network_path_given_to_a_url_builder_is_left_alone() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_010_an_absolute_url_or_a_network_path_given_to_a_url_builder_is_left_alone",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let router: Router = Router::new()
        .get("/builders", |_request: Request| async {
            Ok(HttpResponse::text(
                [
                    url::to("https://other.example/x"),
                    url::to("http://other.example/x?q=1"),
                    url::to("//cdn.example/x"),
                    url::secure("//cdn.example/x"),
                ]
                .join("\n"),
            ))
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;
    let behind = support::get_prefixed(address, "/builders").await;
    assert_eq!(
        behind.body.lines().collect::<Vec<_>>(),
        [
            "https://other.example/x",
            "http://other.example/x?q=1",
            "//cdn.example/x",
            "//cdn.example/x",
        ]
    );

    // A storage disk cannot be given a network-path base at all, so no
    // storage URL starts from one.
    Storage::register_memory("network");
    assert!(
        Storage::set_public_url("network", "//cdn.example/files").is_err(),
        "a network-path public base was accepted"
    );
}
