//! Ordinary SSR and Live islands through the real application: the public
//! page and the authenticated dashboard, an action through the production
//! middleware stack, CSRF and principal enforcement, polling, recovery, and
//! production artifact delivery.

mod live_support;

use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, action_request, attribute, config_json, decoded_snapshot, empty, fresh_render, get,
    idempotency, invoke, island_tag, request, seed_session, send, setup_app, snapshot_revision,
};
use serde_json::Value;
use suprnova::live::{LiveComponent, LiveRegistry, RegistryErrorKind, live};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_public_page_renders_for_anonymous_visitors_and_the_dashboard_requires_sign_in() {
    let app = setup_app(4).await;

    let reply = get(&app, "/live/public", None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(
        html.contains("<h1>Public counter</h1>"),
        "ordinary SSR content: {html}"
    );
    let counter = island_tag(&html, "public-counter");
    assert_eq!(
        attribute(counter, "data-suprnova-live-snapshot-kind"),
        "seed"
    );
    assert!(html.contains("id=\"suprnova-live-config\""), "{html}");
    assert!(html.contains("suprnova-live.esm.js"), "{html}");
    assert!(
        !html.contains("suprnova-live.uploads.esm.js"),
        "no upload role without an upload field"
    );
    assert!(
        !html.contains("suprnova-live.async.esm.js"),
        "no async role without a stream"
    );
    assert_eq!(config_json(&html)["endpoint"], "/__live/action");

    let reply = get(&app, "/live", None).await;
    assert!(
        reply.status.is_redirection(),
        "anonymous dashboard: {}",
        reply.status
    );
    assert!(
        reply
            .header("location")
            .is_some_and(|location| location.contains("/login"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_in_user_renders_the_dashboard_and_increments_the_counter() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(html.contains("<h1>Live dashboard</h1>"), "{html}");
    for key in ["dashboard-counter", "dashboard-uploader", "dashboard-feed"] {
        let tag = island_tag(&html, key);
        assert_eq!(
            attribute(tag, "data-suprnova-live-snapshot-kind"),
            "instance",
            "{key}"
        );
    }
    assert!(
        html.contains("suprnova-live.uploads.esm.js"),
        "upload role for the uploader: {html}"
    );
    assert!(
        html.contains("suprnova-live.async.esm.js"),
        "async role for the feed: {html}"
    );
    let feed = island_tag(&html, "dashboard-feed");
    assert_eq!(
        attribute(feed, "live:stream"),
        "activity",
        "the island root carries the declared stream: {feed}"
    );
    assert!(html.contains("Count: 0"), "{html}");

    let counter = island_tag(&html, "dashboard-counter");
    let snapshot = decoded_snapshot(counter);
    let revision = snapshot_revision(&snapshot);
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.counter",
                document_key: "dashboard-counter",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: invoke("increment"),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(1),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    let accepted = reply.json();
    assert_eq!(accepted["outcome"], "accepted", "{accepted}");
    assert!(
        accepted["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains("Count: 1")),
        "{accepted}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn csrf_origin_and_principal_gates_hold_on_the_real_stack() {
    let app = setup_app(8).await;
    let session = seed_session(&app).await;
    let reply = get(&app, "/live/public", Some(&session)).await;
    let html = reply.text();
    let snapshot = decoded_snapshot(island_tag(&html, "public-counter"));
    let spec = |key: u64| ActionSpec {
        component: "app.counter",
        document_key: "public-counter",
        snapshot: snapshot.clone(),
        seed: true,
        base_revision: "0",
        operations: invoke("increment"),
        model_proposals: Value::Object(Default::default()),
        idempotency_key: Box::leak(idempotency(key).into_boxed_str()),
    };

    // Signed in, same-origin: the public seed promotes and the action runs.
    let reply = send(
        app.addr,
        action_request(&app, spec(11), Some(&session), true),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["outcome"], "accepted");

    // No origin proof: token validation runs and refuses the runtime's request.
    let reply = send(
        app.addr,
        action_request(&app, spec(12), Some(&session), false),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::from_u16(419).expect("419"),
        "{}",
        reply.text()
    );

    // Cross-site proof: refused the same way.
    let cross = request(&app, Method::POST, "/__live/action", Some(&session), false)
        .header("sec-fetch-site", "cross-site")
        .header("content-type", live_support::LIVE_MEDIA)
        .body(empty())
        .expect("build");
    let reply = send(app.addr, cross).await;
    assert_eq!(reply.status, StatusCode::from_u16(419).expect("419"));

    // Anonymous, same-origin: a public seed promotes for the visitor's own
    // session and the action runs, because the guard's authentication is
    // optional and the public mount permits an anonymous principal.
    let reply = send(app.addr, action_request(&app, spec(13), None, true)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let accepted = reply.json();
    assert_eq!(accepted["outcome"], "accepted", "{accepted}");
    assert!(
        accepted["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains("Count: 1")),
        "{accepted}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn polling_recovery_and_assets_work_through_the_real_stack() {
    let app = setup_app(8).await;
    let session = seed_session(&app).await;
    let reply = get(&app, "/live", Some(&session)).await;
    let html = reply.text();
    let counter = island_tag(&html, "dashboard-counter");
    let snapshot = decoded_snapshot(counter);
    let revision = snapshot_revision(&snapshot);

    // Polling is the ordinary fresh-render request.
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.counter",
                document_key: "dashboard-counter",
                snapshot: snapshot.clone(),
                seed: false,
                base_revision: &revision,
                operations: fresh_render(),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(21),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        reply.json()["render"]["html"]
            .as_str()
            .is_some_and(|h| h.contains("Count: 0"))
    );

    // A tampered snapshot is a closed 409: no body, no state change.
    let mut tampered = snapshot.clone();
    tampered["body"]["extensions"]["x_suprnova_framework_document_path_v1"] =
        Value::String("/tampered".to_owned());
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.counter",
                document_key: "dashboard-counter",
                snapshot: tampered,
                seed: false,
                base_revision: &revision,
                operations: invoke("increment"),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(22),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(
        reply.body.is_empty(),
        "closed rejection body: {}",
        reply.text()
    );
    assert_eq!(reply.header("cache-control"), Some("no-store"));

    // Production artifacts come from the framework's immutable asset route.
    let identity = config_json(&html)["asset_identity"]
        .as_str()
        .expect("asset identity")
        .to_owned();
    let reply = get(
        &app,
        &format!("/__live/assets/{identity}/suprnova-live.esm.js"),
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        reply.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    let reply = get(&app, "/__live/assets/stale/suprnova-live.esm.js", None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
}

/// FORM-001, UI-019: the form gallery mounts every presentational library
/// component through the vendored macros, opts the suprnova-ui base in, and
/// renders the controls the checker proves.
#[tokio::test]
async fn the_form_gallery_renders_every_presentational_component_with_the_library_base() {
    let app = setup_app(7).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live/forms", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(html.contains("<h1>Form gallery</h1>"), "{html}");
    assert_eq!(
        html.matches("suprnova-ui.css").count(),
        1,
        "the opted-in document loads the base once: {html}"
    );
    let gallery = island_tag(&html, "forms-gallery");
    assert_eq!(
        attribute(gallery, "data-suprnova-live-snapshot-kind"),
        "instance"
    );
    for needle in [
        "class=\"sn-field\" data-sn-field=\"email\"",
        "<label class=\"sn-label\" for=\"email\">",
        "type=\"email\"",
        "live:model=\"email\"",
        "<textarea class=\"sn-textarea\" id=\"bio\"",
        "class=\"sn-input sn-number-input\" id=\"quantity\"",
        "type=\"range\"",
        "live:model.debounce.300ms=\"query\"",
        "<sn-password-reveal class=\"sn-password\">",
        "type=\"password\"",
        "class=\"sn-checkbox-input\" id=\"agree\"",
        "<fieldset class=\"sn-radio-group\" id=\"plan\"",
        "value=\"starter\"",
        "role=\"switch\"",
        "<select class=\"sn-select\" id=\"country\"",
        "<option value=\"ca\">Canada</option>",
        "<fieldset class=\"sn-checkbox-group\" id=\"topics\"",
        "type=\"file\"",
        "<fieldset class=\"sn-fieldset\">",
        "class=\"sn-form-actions\"",
        "role=\"group\" aria-label=\"Secondary actions\"",
        "live:click=\"reset\"",
        "<a class=\"sn-button\" role=\"button\" href=\"/live\"",
        "type=\"submit\" data-sn-variant=\"primary\" live:loading.disabled=\"save\"",
        "class=\"sn-validation-summary\"",
        "live:error.live.polite=\"email\"",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );
}

/// UI-015: the `suprnova.` namespace belongs to the shipped library; a
/// component from the application that claims it is refused at
/// registration, before any route can reach it.
#[derive(LiveComponent)]
#[live(name = "suprnova.probe", view = "live/counter.html")]
pub struct ReservedProbe {
    #[public]
    count: u64,
}

#[live]
impl ReservedProbe {
    #[action]
    pub fn touch(&mut self) {
        self.count += 1;
    }
}

#[test]
fn the_registry_refuses_the_reserved_namespace_from_the_application() {
    let refused = LiveRegistry::builder()
        .register::<ReservedProbe>()
        .expect_err("an application component under suprnova. is refused");
    assert_eq!(refused.kind(), RegistryErrorKind::ReservedName);
}

/// UI-006: every gallery control keeps its accessible name and its state
/// attributes in the markup itself, so nothing depends on the base layer.
#[tokio::test]
async fn the_gallery_keeps_names_and_state_without_the_base_layer() {
    let app = setup_app(8).await;
    let session = seed_session(&app).await;
    let html = get(&app, "/live/forms", Some(&session)).await.text();
    let gallery = island_tag(&html, "forms-gallery");
    let island_start = html.find(gallery).expect("gallery island");
    let island = &html[island_start..];
    for id in [
        "email", "secret", "bio", "quantity", "volume", "query", "country", "nickname", "avatar",
    ] {
        assert!(
            island.contains(&format!("for=\"{id}\"")),
            "control {id} has an explicit label"
        );
        assert!(
            island.contains(&format!("id=\"{id}\"")),
            "control {id} carries its id"
        );
    }
    for wrapped in [
        "<label class=\"sn-checkbox\"><input class=\"sn-checkbox-input\" id=\"agree\"",
        "<label class=\"sn-switch\"><input class=\"sn-switch-input\" id=\"newsletter\"",
    ] {
        assert!(
            island.contains(wrapped),
            "control wrapped by its label: {wrapped}"
        );
    }
    for legend in [
        "<legend>Plan</legend>",
        "<legend>Topics</legend>",
        "<legend>Account</legend>",
    ] {
        assert!(island.contains(legend), "group named by {legend}");
    }
    for state in [
        "role=\"switch\"",
        "aria-pressed=\"false\"",
        "aria-controls=\"secret\"",
        "aria-describedby=\"email-hint email-error\"",
        "aria-labelledby=\"save-summary-title\"",
        "live:error.live.polite=\"email\"",
        "live:loading.disabled=\"save\"",
    ] {
        assert!(
            island.contains(state),
            "state and naming attribute {state} present"
        );
    }
    assert!(
        !island.contains("class=\"is-") && !island.contains(" is-"),
        "no visual-only state class"
    );
}

/// FORM-004: the password binds through a transient field, so the gallery's
/// snapshot carries no password value and the control renders without one.
#[tokio::test]
async fn the_gallery_snapshot_never_carries_the_password() {
    let app = setup_app(9).await;
    let session = seed_session(&app).await;
    let html = get(&app, "/live/forms", Some(&session)).await.text();
    let gallery = island_tag(&html, "forms-gallery");
    let snapshot = decoded_snapshot(gallery);
    let serialized = serde_json::to_string(&snapshot).expect("snapshot serializes");
    assert!(
        !serialized.contains("\"secret\""),
        "the transient password field is not dehydrated: {serialized}"
    );
    let password_start = html.find("type=\"password\"").expect("password control");
    let password_tag_end = html[password_start..].find('>').expect("tag end") + password_start;
    let tag_start = html[..password_start].rfind('<').expect("tag start");
    let password_tag = &html[tag_start..password_tag_end];
    assert!(
        !password_tag.contains("value="),
        "no password value in markup: {password_tag}"
    );
}
