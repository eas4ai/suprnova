//! Every document of the kit renders, as its visitor sees it: the guest
//! pages for a visitor, the account pages for a signed-in user with a note
//! of their own, the profile endpoint over JSON, and the error page for a
//! path no route matches or a method no route takes.
//!
//! The test boots what `cmd/main.rs` boots, without a port: a fresh SQLite
//! database with every migration, the auth provider, the Live registry and
//! its gate, the HTTP stack, the Live routes with the render cache, and the
//! Live runtime. `TestClient` sends each request through the pipeline a
//! browser's request takes, so a document that fails to mount, to read its
//! user or to query its notes fails here.

use std::sync::Arc;

use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::serde_json::json;
use suprnova::testing::{TestClient, TestDatabase};
use suprnova::{
    App, Auth, AuthConfig, AuthManager, CacheStore, EloquentUserProvider, InMemoryCache,
    MiddlewareRegistry, Model, attrs,
};

use {package_name}::migrations::Migrator;
use {package_name}::models::note::Note;
use {package_name}::models::user::User;
use {package_name}::{bootstrap, live, routes};

/// The attribute the document marks a mounted island with.
fn island(component: &str) -> String {
    format!(r#"data-suprnova-live-component="{package_name}.{component}""#)
}

/// The CSRF token of a signed-in page, from the sign-out form its layout
/// carries.
fn csrf_token(html: &str) -> String {
    let marker = r#"name="_token" value=""#;
    let start = html
        .find(marker)
        .expect("a signed-in page carries the sign-out form's CSRF token")
        + marker.len();
    let end = html[start..].find('"').expect("the token ends") + start;
    html[start..end].to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_document_renders_for_its_visitor() {
    // The key `.env` holds for the server; a test process loads no `.env`,
    // and the Live runtime, the session and the CSRF cookie all need one.
    suprnova::testing::install_test_encryption_key();
    let _db = TestDatabase::fresh::<Migrator>()
        .await
        .expect("a fresh database with every migration");
    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");
    // The cache store the server boots from `CACHE_DRIVER`; session
    // blocking takes its lock through it.
    App::bind_if_absent::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    App::singleton(live::registry().expect("Live component registry"));
    live::authorize();
    bootstrap::register_http_stack();
    let router = live::routes_with_render_cache(routes::register())
        .await
        .expect("the routes with the render cache");
    let runtime = prepare_live_router_for_test(&router).expect("the Live runtime");
    App::singleton(runtime);
    let guest = TestClient::new(router, MiddlewareRegistry::from_global());

    // The guest pages, each in the guest layout with its island.
    for (path, component) in [
        ("/login", "login-form"),
        ("/register", "register-form"),
        ("/forgot-password", "forgot-password-form"),
        (
            "/reset-password?token=a-mailed-token",
            "reset-password-form",
        ),
    ] {
        guest
            .get(path)
            .send()
            .await
            .assert_ok()
            .assert_see(r#"data-layout="guest""#)
            .assert_see(&island(component));
    }
    // A visitor's Name box starts empty, not with the field's own name.
    guest
        .get("/register")
        .send()
        .await
        .assert_see(r#"aria-describedby="name-hint name-error" required value="""#);
    guest
        .get("/")
        .send()
        .await
        .assert_ok()
        .assert_see(r#"data-layout="app""#);
    // A page that needs a user sends a guest to sign in.
    guest.get("/dashboard").send().await.assert_status(302);

    let user = User::create("Ada Lovelace", "ada@example.com", "correct horse battery")
        .await
        .expect("a user");
    let note = <Note as Model>::create(attrs! {
        user_id: user.id,
        title: "First note",
        body: Some("Written in the test.".to_string()),
    })
    .await
    .expect("a note");
    let client = guest.clone().acting_as(&user);

    // The account pages, each in its layout with its islands and the
    // user's own data.
    client
        .get("/verify-email")
        .send()
        .await
        .assert_ok()
        .assert_see(r#"data-layout="guest""#)
        .assert_see(&island("verify-email"))
        .assert_see("ada@example.com");
    let dashboard = client.get("/dashboard").send().await;
    dashboard
        .assert_ok()
        .assert_see(r#"data-layout="app""#)
        .assert_see(&island("profile-name"))
        .assert_see(&island("note-stats"))
        .assert_see("Ada Lovelace")
        // The profile form shows the user's own name in its Name box.
        .assert_see(r#"aria-describedby="name-hint name-error" required value="Ada Lovelace""#);
    client
        .get("/notes")
        .send()
        .await
        .assert_ok()
        .assert_see(&island("note-form"))
        .assert_see(&island("note-list"))
        .assert_see("First note");
    client
        .get(&format!("/notes/{}", note.id))
        .send()
        .await
        .assert_ok()
        .assert_see("First note")
        .assert_see("Written in the test.");

    // An id the user does not own is the same 404 as one that does not
    // exist; a path no route matches is the error page; a method no route
    // takes keeps its `Allow` header on the page.
    client
        .get("/notes/999999")
        .header("Accept", "text/html")
        .send()
        .await
        .assert_status(404)
        .assert_see(r#"id="error-status">404<"#);
    guest
        .get("/no-such-page")
        .header("Accept", "text/html")
        .send()
        .await
        .assert_status(404)
        .assert_see(r#"<h1 class="kit-error-status" id="error-status">404</h1>"#);
    guest
        .get("/logout")
        .header("Accept", "text/html")
        .send()
        .await
        .assert_status(405)
        .assert_header("Allow", "POST")
        .assert_see(r#"id="error-status">405<"#);

    // The display name over JSON: `422` with the field's message, then
    // `200` with the user as the server now has them. A client that
    // accepts JSON keeps the JSON error body; the error page is for pages.
    let token = csrf_token(&dashboard.body_text());
    let rejected = client
        .post("/profile/name")
        .header("Accept", "application/json")
        .header("X-CSRF-TOKEN", token.clone())
        .json(&json!({ "name": "" }))
        .send()
        .await;
    rejected.assert_status(422);
    assert!(
        !rejected.json()["errors"]["name"].is_null(),
        "{}",
        rejected.body_text()
    );
    let saved = client
        .post("/profile/name")
        .header("Accept", "application/json")
        .header("X-CSRF-TOKEN", token)
        .json(&json!({ "name": "Ada L." }))
        .send()
        .await;
    saved.assert_ok();
    assert_eq!(
        saved.json()["user"]["name"],
        "Ada L.",
        "{}",
        saved.body_text()
    );
}
