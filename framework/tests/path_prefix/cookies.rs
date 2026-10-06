//! PFX-007 and PFX-013: the framework's cookies take the root as their
//! `Path` when the application sets none, and of two cookies of one name
//! the first sent wins.

use std::sync::Arc;
use std::time::Duration;

use suprnova::http::{CookiePrefix, parse_cookies};
use suprnova::{
    CsrfMiddleware, FileMaintenanceMode, HttpResponse, MaintenanceMiddleware, MaintenanceMode,
    MaintenancePayload, MiddlewareRegistry, Request, Router, SessionConfig, SessionMiddleware,
};

use crate::support::{self, MemorySessionStore, PREFIX};

fn session_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// A session, its CSRF middleware, and two routes: one that starts a
/// session and one that reads and writes a marker in it.
fn stack(config: SessionConfig) -> (Router, MiddlewareRegistry) {
    let router: Router = Router::new()
        .get("/start", |_request: Request| async {
            suprnova::session::session_mut(|session| session.put("started", true));
            Ok(HttpResponse::text("started"))
        })
        .get("/mark/{value}", |request: Request| async move {
            let value = request.param("value").unwrap_or_default().to_owned();
            suprnova::session::session_mut(|session| session.put("marker", value));
            Ok(HttpResponse::text("marked"))
        })
        .get("/marker", |_request: Request| async {
            let marker = suprnova::session::session()
                .and_then(|session| session.get::<String>("marker"))
                .unwrap_or_else(|| "none".to_owned());
            Ok(HttpResponse::text(marker))
        })
        .get("/remember", |_request: Request| async {
            let mut config = SessionConfig::default();
            config.cookie_secure = false;
            let cookie = suprnova::session::middleware::create_remember_cookie(
                &config,
                "token",
                Duration::from_secs(60),
            )
            .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            let forget = suprnova::session::middleware::create_forget_remember_cookie(&config);
            Ok(HttpResponse::text(format!(
                "{}\n{}",
                cookie.to_header_value(),
                forget.to_header_value()
            )))
        })
        .get("/cookie/{name}", |request: Request| async move {
            let name = request.param("name").unwrap_or_default().to_owned();
            Ok(HttpResponse::text(
                request.cookie(&name).unwrap_or_else(|| "none".to_owned()),
            ))
        })
        .into();
    let middleware = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            config.clone(),
            Arc::new(MemorySessionStore::default()),
        ))
        .append(CsrfMiddleware::new().with_session_config(&config));
    (router, middleware)
}

async fn served(config: SessionConfig) -> std::net::SocketAddr {
    support::ensure_crypt();
    support::install("http://localhost");
    let (router, middleware) = stack(config);
    support::serve(router, middleware).await
}

#[tokio::test]
async fn pfx_007_the_session_and_xsrf_cookies_take_the_root_of_each_request() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_the_session_and_xsrf_cookies_take_the_root_of_each_request",
    )
    .await
    {
        return;
    }
    let address = served(session_config()).await;

    let behind = support::get_prefixed(address, "/start").await;
    assert_eq!(behind.cookie_path("suprnova_session"), "/billing");
    assert_eq!(behind.cookie_path("XSRF-TOKEN"), "/billing");

    let at_host_root = support::get(address, "/start", &[]).await;
    assert_eq!(at_host_root.cookie_path("suprnova_session"), "/");
    assert_eq!(at_host_root.cookie_path("XSRF-TOKEN"), "/");
}

#[tokio::test]
async fn pfx_007_two_applications_on_one_host_keep_their_own_session_cookie() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_two_applications_on_one_host_keep_their_own_session_cookie",
    )
    .await
    {
        return;
    }
    let address = served(session_config()).await;
    let a = support::get(address, "/start", &[("x-forwarded-prefix", "/a")]).await;
    let b = support::get(address, "/start", &[("x-forwarded-prefix", "/b")]).await;
    // A browser keys a cookie by name, host and path: these two never
    // replace each other.
    assert_eq!(a.cookie_path("suprnova_session"), "/a");
    assert_eq!(b.cookie_path("suprnova_session"), "/b");
}

#[tokio::test]
async fn pfx_007_a_host_prefixed_cookie_keeps_path_slash() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_a_host_prefixed_cookie_keeps_path_slash",
    )
    .await
    {
        return;
    }
    let mut config = session_config();
    config.cookie_prefix = CookiePrefix::Host;
    let address = served(config).await;
    let behind = support::get_prefixed(address, "/start").await;
    assert_eq!(behind.cookie_path("__Host-suprnova_session"), "/");
}

#[tokio::test]
async fn pfx_007_a_path_the_application_sets_is_kept() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_a_path_the_application_sets_is_kept",
    )
    .await
    {
        return;
    }
    let mut config = session_config();
    config.cookie_path = "/custom".to_owned();
    let address = served(config).await;
    let behind = support::get_prefixed(address, "/start").await;
    assert_eq!(behind.cookie_path("suprnova_session"), "/custom");
    assert_eq!(behind.cookie_path("XSRF-TOKEN"), "/custom");
}

/// The `Path` of a `Set-Cookie` value.
fn path_of(set_cookie: &str) -> &str {
    set_cookie
        .split(';')
        .map(str::trim)
        .find_map(|attribute| attribute.strip_prefix("Path="))
        .unwrap_or_else(|| panic!("no Path on {set_cookie}"))
}

#[tokio::test]
async fn pfx_007_the_remember_me_cookie_takes_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_the_remember_me_cookie_takes_the_root",
    )
    .await
    {
        return;
    }
    let address = served(session_config()).await;
    let behind = support::get_prefixed(address, "/remember").await;
    let lines: Vec<&str> = behind.body.lines().collect();
    assert_eq!(path_of(lines[0]), "/billing", "{}", lines[0]);
    assert_eq!(path_of(lines[1]), "/billing", "{}", lines[1]);

    let at_host_root = support::get(address, "/remember", &[]).await;
    assert_eq!(
        path_of(at_host_root.body.lines().next().expect("a line")),
        "/"
    );
}

#[tokio::test]
async fn pfx_007_the_maintenance_bypass_cookie_takes_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_007_the_maintenance_bypass_cookie_takes_the_root",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let down = std::env::temp_dir().join(format!("pfx-007-down-{}", std::process::id()));
    let mode = Arc::new(FileMaintenanceMode::with_path(&down));
    mode.activate(&MaintenancePayload {
        secret: Some("s3cret".to_owned()),
        ..MaintenancePayload::new()
    })
    .await
    .expect("go down");
    let router: Router = Router::new()
        .get("/", |_request: Request| async {
            Ok(HttpResponse::text("home"))
        })
        .into();
    let address = support::serve(
        router,
        MiddlewareRegistry::new().append(MaintenanceMiddleware::with_driver(mode.clone())),
    )
    .await;

    let behind = support::get_prefixed(address, "/s3cret").await;
    assert_eq!(behind.status, 302);
    assert_eq!(behind.header("location").as_deref(), Some("/billing/"));
    assert_eq!(behind.cookie_path("suprnova_maintenance"), PREFIX);

    let at_host_root = support::get(address, "/s3cret", &[]).await;
    assert_eq!(at_host_root.cookie_path("suprnova_maintenance"), "/");
    let _ = std::fs::remove_file(&down);
}

#[test]
fn pfx_013_parse_cookies_keeps_the_first_cookie_of_a_name() {
    let cookies = parse_cookies("suprnova_session=scoped; suprnova_session=stale");
    assert_eq!(
        cookies.get("suprnova_session").map(String::as_str),
        Some("scoped")
    );
    // A literal name still wins over an alias that only decodes to it,
    // wherever the alias stands.
    let cookies = parse_cookies("sessi%6Fn=alias; session=literal; session=later");
    assert_eq!(cookies.get("session").map(String::as_str), Some("literal"));
}

#[tokio::test]
async fn pfx_013_every_framework_cookie_reads_the_first_of_a_name() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_013_every_framework_cookie_reads_the_first_of_a_name",
    )
    .await
    {
        return;
    }
    let address = served(session_config()).await;
    for name in ["XSRF-TOKEN", "remember_me", "suprnova_maintenance"] {
        let header = format!("{name}=scoped; {name}=stale");
        let reply = support::get(address, &format!("/cookie/{name}"), &[("cookie", &header)]).await;
        assert_eq!(reply.body, "scoped", "{name}");
    }
}

#[tokio::test]
async fn pfx_013_the_session_middleware_loads_the_session_sent_first() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_013_the_session_middleware_loads_the_session_sent_first",
    )
    .await
    {
        return;
    }
    let address = served(session_config()).await;
    let scoped = support::get_prefixed(address, "/mark/scoped")
        .await
        .cookie_pair("suprnova_session");
    let stale = support::get(address, "/mark/stale", &[])
        .await
        .cookie_pair("suprnova_session");

    let both = format!("{scoped}; {stale}");
    let reply = support::get_prefixed_with_cookie(address, "/marker", &both).await;
    assert_eq!(reply.body, "scoped");

    let reversed = format!("{stale}; {scoped}");
    let reply = support::get_prefixed_with_cookie(address, "/marker", &reversed).await;
    assert_eq!(reply.body, "stale");
}
