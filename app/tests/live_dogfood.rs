//! Ordinary SSR and Live islands through the real application: the public
//! page and the authenticated dashboard, an action through the production
//! middleware stack, CSRF and principal enforcement, polling, recovery, and
//! production artifact delivery.

mod live_support;

use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, UPLOAD_PATH, action_request, attribute, config_json, decoded_snapshot, empty,
    fresh_render, get, idempotency, invoke, island_tag, request, seed_session, send, setup_app,
    sha256_hex, snapshot_revision, tiny_png,
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
        "live:model.debounce.250ms=\"query\"",
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
    for asset in [
        "<link rel=\"stylesheet\" href=\"/suprnova-ui/field/field.css\">",
        "<script type=\"module\" src=\"/suprnova-ui/password-input/password-input.js\"></script>",
    ] {
        assert!(
            html.contains(asset),
            "the page links its vendored component assets: {asset}"
        );
    }

    // UI-017: the vendored stylesheet and script are served from the
    // component's own directory under the reserved template root, and
    // nothing else in it is reachable.
    let css = get(&app, "/suprnova-ui/field/field.css", None).await;
    assert_eq!(css.status, StatusCode::OK);
    assert_eq!(css.header("content-type"), Some("text/css; charset=utf-8"));
    assert!(css.text().contains(".sn-field"));
    let js = get(&app, "/suprnova-ui/password-input/password-input.js", None).await;
    assert_eq!(js.status, StatusCode::OK);
    assert_eq!(
        js.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(js.text().contains("sn-password-reveal"));
    for closed in [
        "/suprnova-ui/field/field.html",
        "/suprnova-ui/field/manifest.json",
        "/suprnova-ui/Field/field.css",
    ] {
        let reply = get(&app, closed, None).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{closed}");
    }
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

/// OVL-001 to OVL-004: the overlay gallery renders every overlay on its
/// native primitive, keyed and preserved, with no Live directive on an open
/// or close control, and the vendored assets beside them.
#[tokio::test]
async fn the_overlay_gallery_renders_every_overlay_on_its_native_primitive() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live/overlays", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(html.contains("<h1>Overlay gallery</h1>"), "{html}");
    assert_eq!(html.matches("suprnova-ui.css").count(), 1, "{html}");
    let gallery = island_tag(&html, "overlays-gallery");
    assert_eq!(
        attribute(gallery, "data-suprnova-live-snapshot-kind"),
        "instance"
    );
    for needle in [
        "<span class=\"sn-tooltip-bubble\" id=\"save-tip\" role=\"tooltip\">",
        "aria-describedby=\"save-tip\"",
        "<details class=\"sn-collapsible\" live:key=\"notes\" live:preserve.self>",
        "<details class=\"sn-accordion-item\" name=\"faq\" live:key=\"faq-open\" live:preserve.self open>",
        "<div class=\"sn-popover\" id=\"hint\" popover aria-label=\"Hint\"",
        "popovertarget=\"hint\"",
        "<div class=\"sn-menu\" id=\"actions\" popover aria-label=\"Actions\"",
        "<a class=\"sn-menu-link\" href=\"/live\">Dashboard</a>",
        "<button class=\"sn-menu-action\" type=\"button\" live:click=\"add_note\" popovertarget=\"actions\" popovertargetaction=\"hide\">",
        "<sn-dialog class=\"sn-dialog-host\" tabindex=\"-1\">",
        "<dialog class=\"sn-dialog\" id=\"confirm\" aria-labelledby=\"confirm-title\" closedby=\"any\" live:key=\"confirm\" live:preserve.self>",
        "data-sn-dialog-open=\"confirm\"",
        "data-sn-dialog-close=\"confirm\"",
        "live:click=\"confirm_delete\"",
        "<dialog class=\"sn-sheet\" id=\"details-sheet\"",
        "<dialog class=\"sn-drawer\" id=\"nav-drawer\"",
        "data-sn-side=\"start\"",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );
    // OVL-004: no open or close control carries a Live directive.
    for tag in html.split('<').filter(|tag| {
        tag.starts_with("summary")
            || tag.contains("popovertarget=")
            || tag.contains("data-sn-dialog-open=")
            || tag.contains("data-sn-sheet-open=")
            || tag.contains("data-sn-drawer-open=")
            || tag.contains("-close=")
    }) {
        let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
        assert!(
            !tag.contains(" live:") || tag.contains("live:click=\"add_note\""),
            "an open or close control carries a Live directive: <{tag}>"
        );
    }
    for asset in [
        "<link rel=\"stylesheet\" href=\"/suprnova-ui/dialog/dialog.css\">",
        "<script type=\"module\" src=\"/suprnova-ui/dialog/dialog.js\"></script>",
        "<script type=\"module\" src=\"/suprnova-ui/sheet/sheet.js\"></script>",
        "<script type=\"module\" src=\"/suprnova-ui/drawer/drawer.js\"></script>",
    ] {
        assert!(html.contains(asset), "the page links {asset}");
    }
    let js = get(&app, "/suprnova-ui/dialog/dialog.js", None).await;
    assert_eq!(js.status, StatusCode::OK);
    assert!(js.text().contains("sn-dialog"));
    assert!(
        !js.text().contains("setAttribute(\"open\""),
        "the script never owns the open attribute"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_feedback_gallery_renders_every_feedback_component_on_real_state() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live/feedback", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(html.contains("<h1>Feedback gallery</h1>"), "{html}");
    assert_eq!(html.matches("suprnova-ui.css").count(), 1, "{html}");
    let gallery = island_tag(&html, "feedback-gallery");
    assert_eq!(
        attribute(gallery, "data-suprnova-live-snapshot-kind"),
        "instance"
    );
    for needle in [
        // FDB-001: the role follows the variant and each variant carries its own cue.
        "<div class=\"sn-alert\" id=\"welcome\" data-sn-variant=\"info\" role=\"status\">",
        "<span class=\"sn-alert-label\">Information:</span>",
        "<div class=\"sn-alert\" id=\"saved-alert\" data-sn-variant=\"success\" role=\"status\">",
        "<span class=\"sn-alert-label\">Success:</span>",
        "<div class=\"sn-alert\" id=\"quota\" data-sn-variant=\"warning\" role=\"status\">",
        "<span class=\"sn-alert-label\">Warning:</span>",
        // FDB-002: loading presentation is bound and authored hidden.
        "<span class=\"sn-spinner\" role=\"status\" live:loading.show=\"save\" hidden>",
        "<div class=\"sn-skeleton\" role=\"status\" live:loading.show=\"refresh\" hidden>",
        "<span class=\"sn-skeleton-lines\" aria-hidden=\"true\" data-sn-lines=\"2\">",
        // FDB-006: native progress, a value only when determinate, a label and a readout.
        "<label class=\"sn-progress-label\" for=\"upload\">Upload</label>",
        "<progress class=\"sn-progress-bar\" id=\"upload\" max=\"100\" value=\"0\"></progress>",
        "<output class=\"sn-progress-readout\" for=\"upload\">0 of 100</output>",
        "<progress class=\"sn-progress-bar\" id=\"indexing\" max=\"100\" ></progress>",
        "<output class=\"sn-progress-readout\" for=\"indexing\">Working</output>",
        // FDB-003: the reason is server state.
        "<section class=\"sn-empty-state\" id=\"inbox\" data-sn-reason=\"empty\" aria-labelledby=\"inbox-title\">",
        // FDB-004: one polite status region, empty until an outcome arrives.
        "<div class=\"sn-toast-list\" id=\"toasts\" role=\"status\" aria-live=\"polite\" aria-label=\"Notifications\"></div>",
        "<link rel=\"stylesheet\" href=\"/suprnova-ui/toast/toast.css\">",
        "<script type=\"module\" src=\"/suprnova-ui/toast/toast.js\"></script>",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    assert!(
        !html.contains("id=\"failure\""),
        "no failure before one happens"
    );
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );

    // A failed save renders the persistent alert and the error toast together.
    let snapshot = decoded_snapshot(gallery);
    let revision = snapshot_revision(&snapshot);
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.feedback-gallery",
                document_key: "feedback-gallery",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: invoke("fail"),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(1),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let accepted = reply.json();
    assert_eq!(accepted["outcome"], "accepted", "{accepted}");
    let rendered = accepted["render"]["html"].as_str().expect("a render");
    assert!(
        rendered.contains(
            "<div class=\"sn-alert\" id=\"failure\" data-sn-variant=\"error\" role=\"alert\">"
        ),
        "{rendered}"
    );
    assert!(
        rendered.contains("<div class=\"sn-toast\" data-sn-variant=\"error\" data-sn-duration=\"6000\" live:key=\"toast-1\" live:preserve.self>"),
        "{rendered}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_empty_state_takes_its_reason_from_the_document_and_offers_no_action_without_permission()
 {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;
    for (reason, title, action) in [
        ("empty", "Nothing here yet", Some("Create the first item")),
        ("no-results", "No results", Some("Clear filters")),
        ("no-permission", "Nothing to show", None),
        ("disconnected", "Disconnected", Some("Retry")),
    ] {
        let reply = get(
            &app,
            &format!("/live/feedback?reason={reason}"),
            Some(&session),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{reason}: {}", reply.text());
        let html = reply.text();
        let start = html
            .find("<section class=\"sn-empty-state\"")
            .unwrap_or_else(|| panic!("{reason}: no empty state in {html}"));
        let end = html[start..].find("</section>").expect("closed") + start;
        let empty = &html[start..end];
        assert!(
            empty.contains(&format!("data-sn-reason=\"{reason}\"")),
            "{empty}"
        );
        assert!(empty.contains(title), "{reason}: {empty}");
        match action {
            Some(text) => assert!(empty.contains(text), "{reason}: {empty}"),
            None => assert!(
                !empty.contains("<button"),
                "{reason}: an action the principal cannot take: {empty}"
            ),
        }
    }
    // An unknown reason is not echoed: the gallery falls back to `empty`.
    let reply = get(&app, "/live/feedback?reason=%3Cscript%3E", Some(&session)).await;
    let html = reply.text();
    assert!(html.contains("data-sn-reason=\"empty\""), "{html}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_flash_region_shows_a_notice_once_after_the_redirect() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live/feedback/notice", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER, "{}", reply.text());
    assert_eq!(reply.header("location"), Some("/live/feedback"));

    let reply = get(&app, "/live/feedback", Some(&session)).await;
    let html = reply.text();
    assert!(
        html.contains(
            "<div class=\"sn-flash\" data-sn-variant=\"success\">Your changes were saved</div>"
        ),
        "{html}"
    );
    let reply = get(&app, "/live/feedback", Some(&session)).await;
    let html = reply.text();
    assert!(
        !html.contains("class=\"sn-flash\""),
        "the flash was consumed: {html}"
    );
    assert!(
        html.contains(
            "<div class=\"sn-flash-region\" id=\"flash\" role=\"status\" aria-label=\"Notices\">"
        ),
        "{html}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_navigation_gallery_keeps_route_semantics_and_takes_current_from_the_server() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;

    let reply = get(&app, "/live/navigation", Some(&session)).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let html = reply.text();
    assert!(html.contains("<h1>Navigation gallery</h1>"), "{html}");
    let gallery = island_tag(&html, "navigation-gallery");
    assert_eq!(
        attribute(gallery, "data-suprnova-live-snapshot-kind"),
        "instance"
    );
    for needle in [
        "<a class=\"sn-header-link\" href=\"/live/navigation\" aria-current=\"page\">Navigation</a>",
        "<a class=\"sn-header-link\" href=\"/live\">Dashboard</a>",
        "<details class=\"sn-sidebar-group\" live:key=\"sidebar-library\" live:preserve.self open>",
        "<details class=\"sn-sidebar-group\" live:key=\"sidebar-account\" live:preserve.self>",
        "<a class=\"sn-sidebar-link\" href=\"/live/navigation\" aria-current=\"page\">Navigation</a>",
        "<li class=\"sn-breadcrumb\" aria-current=\"page\">Navigation</li>",
        "<sn-tabs class=\"sn-tabs\" id=\"local-tabs\" data-sn-mode=\"local\" data-sn-label=\"Details\">",
        "<div class=\"sn-tablist\" role=\"tablist\" aria-label=\"Details\">",
        "<button class=\"sn-tab\" type=\"button\" role=\"tab\" id=\"tab-summary\" aria-controls=\"panel-summary\" aria-selected=\"true\" live:key=\"tab-summary\"",
        "<div class=\"sn-tab-panel\" role=\"tabpanel\" id=\"panel-history\" aria-labelledby=\"tab-history\" tabindex=\"0\" hidden live:key=\"panel-history\"",
        "<nav class=\"sn-tabs\" id=\"route-tabs\" data-sn-mode=\"route\" aria-label=\"Sections\">",
        "<a class=\"sn-tab\" href=\"/live/navigation\" aria-current=\"page\">Navigation</a>",
        "<a class=\"sn-page\" href=\"/live/navigation?page=2\">2</a>",
        "<a class=\"sn-page\" href=\"/live/navigation?page=1\" aria-current=\"page\">1</a>",
        "<span class=\"sn-page\" aria-disabled=\"true\">Previous</span>",
        "<button class=\"sn-page\" type=\"button\" live:click=\"previous_page\" live:loading.disabled=\"previous_page\" live:loading.busy=\"previous_page\" disabled>Previous</button>",
        "<button class=\"sn-page\" type=\"button\" live:click=\"next_page\" live:loading.disabled=\"next_page\" live:loading.busy=\"next_page\">Next</button>",
        "<span class=\"sn-page-position\">Page 1 of 3</span>",
        "<li class=\"sn-feed-item\" live:key=\"row-1\">Row 1</li>",
        "<button class=\"sn-load-more\" type=\"button\" live:click=\"load_more\"",
        "<footer class=\"sn-footer\" id=\"site-footer\">",
        "<script type=\"module\" src=\"/suprnova-ui/tabs/tabs.js\"></script>",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    // NAV-001: anchors navigate, buttons act.
    for tag in html
        .split('<')
        .filter(|tag| tag.starts_with("a ") || tag.starts_with("button "))
    {
        let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
        if tag.starts_with("a ") {
            assert!(
                tag.contains(" href=\""),
                "an anchor without a destination: <{tag}>"
            );
            assert!(
                !tag.contains(" live:"),
                "an anchor performs an action: <{tag}>"
            );
        } else {
            assert!(!tag.contains(" href="), "a button navigates: <{tag}>");
        }
    }
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );

    // The page query mounts onto that page.
    let reply = get(&app, "/live/navigation?page=3", Some(&session)).await;
    let html = reply.text();
    assert!(html.contains("<span data-page=\"3\">3</span>"), "{html}");
    assert!(
        html.contains("live:loading.busy=\"next_page\" disabled>Next</button>"),
        "{html}"
    );
    let reply = get(&app, "/live/navigation?page=99", Some(&session)).await;
    assert!(
        reply.text().contains("<span data-page=\"3\">3</span>"),
        "clamped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_pagination_reflects_the_page_and_load_more_appends_keyed_rows() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;
    let html = get(&app, "/live/navigation", Some(&session)).await.text();
    let gallery = island_tag(&html, "navigation-gallery");
    let mut snapshot = decoded_snapshot(gallery);
    let mut sequence = 0;
    let mut run = |action: &'static str, snapshot: Value| {
        sequence += 1;
        let idempotency_key = idempotency(sequence);
        let revision = snapshot_revision(&snapshot);
        let app = &app;
        let session = &session;
        async move {
            let reply = send(
                app.addr,
                action_request(
                    app,
                    ActionSpec {
                        component: "app.navigation-gallery",
                        document_key: "navigation-gallery",
                        snapshot,
                        seed: false,
                        base_revision: &revision,
                        operations: invoke(action),
                        model_proposals: Value::Object(Default::default()),
                        idempotency_key: &idempotency_key,
                    },
                    Some(session),
                    true,
                ),
            )
            .await;
            assert_eq!(reply.status, StatusCode::OK, "{action}: {}", reply.text());
            let accepted = reply.json();
            assert_eq!(accepted["outcome"], "accepted", "{action}: {accepted}");
            accepted
        }
    };

    // NAV-003: the accepted result reflects the page into the same route's query.
    let accepted = run("next_page", snapshot.clone()).await;
    let rendered = accepted["render"]["html"].as_str().expect("a render");
    assert!(
        rendered.contains("<span data-page=\"2\">2</span>"),
        "{rendered}"
    );
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({ "kind": "reflected", "target": "/live/navigation?page=2" }),
        "{accepted}"
    );
    snapshot = accepted["snapshot"].clone();
    let accepted = run("previous_page", snapshot).await;
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({ "kind": "reflected", "target": "/live/navigation?page=1" }),
        "{accepted}"
    );

    // NAV-006: every row stays, three more arrive, and the control leaves on the last page.
    let html = get(&app, "/live/navigation", Some(&session)).await.text();
    let mut snapshot = decoded_snapshot(island_tag(&html, "navigation-gallery"));
    for expected in [6, 9] {
        let accepted = run("load_more", snapshot.clone()).await;
        let rendered = accepted["render"]["html"].as_str().expect("a render");
        assert_eq!(
            rendered.matches("class=\"sn-feed-item\"").count(),
            expected,
            "{rendered}"
        );
        assert!(rendered.contains("live:key=\"row-1\""), "{rendered}");
        assert_eq!(
            rendered.contains("class=\"sn-load-more\""),
            expected < 9,
            "{rendered}"
        );
        snapshot = accepted["snapshot"].clone();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_data_display_gallery_renders_every_component_with_text_and_server_rendered_marks() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;
    let html = get(&app, "/live/data-display", Some(&session)).await.text();
    // DATA-001: labeled regions and groups, no anonymous wrapper, native semantics.
    for needle in [
        "<hr class=\"sn-separator\">",
        "id=\"activity-scroll\" role=\"region\" aria-label=\"Recent activity\" tabindex=\"0\"",
        "<img class=\"sn-aspect-image\"",
        "<article class=\"sn-card\" id=\"plan-card\" aria-labelledby=\"plan-card-title\"",
        "<section class=\"sn-card\" id=\"team-card\" aria-labelledby=\"team-card-title\"",
        "<h3 class=\"sn-card-title\" id=\"team-card-title\">Team</h3>",
        "<dl class=\"sn-description-list\" id=\"plan-details\" aria-label=\"Plan details\">",
        "<dt class=\"sn-description-term\">Owner</dt><dd class=\"sn-description-value\">Ada Lovelace</dd>",
        "role=\"group\" aria-label=\"Actions\"",
        // DATA-002: text for every status, a data element and a text direction.
        "<span class=\"sn-badge\" data-sn-variant=\"success\">Active</span>",
        "<span class=\"sn-avatar\" role=\"img\" aria-label=\"Ada Lovelace\" data-sn-size=\"lg\">AL</span>",
        "<ul class=\"sn-avatar-group\" role=\"list\" aria-label=\"Team members\">",
        "<data value=\"64\">64</data>",
        "data-sn-trend=\"up\"><span class=\"sn-stat-direction\">Up</span> 12%",
        "data-sn-trend=\"down\"><span class=\"sn-stat-direction\">Down</span> 0.4 pts",
        // DATA-003: keyed items.
        "<li class=\"sn-list-item\" live:key=\"act-1\">",
        // DATA-004: SVG marks in the plain GET, a summary and a data table.
        "<figure class=\"sn-chart\" id=\"revenue-chart\" aria-labelledby=\"revenue-chart-title\">",
        "<div class=\"sn-chart-marks\" aria-hidden=\"true\"><svg",
        "<p class=\"sn-chart-summary\">Revenue rose from 42k in Apr to 64k in Sep",
        "<details class=\"sn-chart-data\"><summary>Data table</summary>",
        "<table class=\"sn-chart-table\"><caption>Revenue by month</caption>",
        "<th scope=\"row\">Sep</th><td>64</td>",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );
    assert!(
        !html.contains("<script src=\"/suprnova-ui/"),
        "the data-display family ships no script"
    );
    assert_eq!(
        html.matches("data-suprnova-live-island").count(),
        2,
        "two islands: the gallery and one table"
    );

    // DATA-003 through the document: a reorder keeps every key and reverses the order.
    let gallery = island_tag(&html, "data-display-gallery");
    let snapshot = decoded_snapshot(gallery);
    let revision = snapshot_revision(&snapshot);
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.data-display-gallery",
                document_key: "data-display-gallery",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: invoke("reorder"),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &idempotency(1),
            },
            Some(&session),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let accepted = reply.json();
    let rendered = accepted["render"]["html"].as_str().unwrap_or_default();
    let first = rendered.find("live:key=\"act-4\"").expect("act-4 rendered");
    let last = rendered.find("live:key=\"act-1\"").expect("act-1 rendered");
    assert!(first < last, "the reorder reversed the keyed list");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_datatable_mounts_from_the_query_and_reflects_sort_filter_and_page() {
    let app = setup_app(8).await;
    let session = seed_session(&app).await;

    // DATA-005: native table semantics and one island per table.
    let html = get(&app, "/live/data-display", Some(&session)).await.text();
    for needle in [
        "<table class=\"sn-datatable\" id=\"invoices\">",
        "<caption class=\"sn-datatable-caption\">Invoices <span class=\"sn-datatable-count\">(14 rows)</span></caption>",
        "<th scope=\"col\" class=\"sn-datatable-column\" aria-sort=\"ascending\">",
        "<th scope=\"row\" class=\"sn-datatable-cell\">1031</th>",
        "role=\"search\" aria-label=\"Filter invoices\" live:submit.prevent=\"filter\"",
        "<tr class=\"sn-datatable-row\" live:key=\"inv-1\">",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    let table_islands = html
        .matches("data-suprnova-live-key=\"datatable-gallery\"")
        .count();
    assert!(table_islands <= 1, "one island per table");

    // The shared URL mounts the same view: sorted by amount descending, filtered, on page 2.
    let shared = get(
        &app,
        "/live/data-display?sort=customer&dir=desc&filter=open&page=2",
        Some(&session),
    )
    .await
    .text();
    assert!(shared.contains("aria-sort=\"descending\""), "{shared}");
    assert!(
        shared.contains("value=\"open\""),
        "the filter input carries the query's filter"
    );
    assert!(shared.contains("Page 2 of 2"), "{shared}");
    let clamped = get(&app, "/live/data-display?page=99", Some(&session))
        .await
        .text();
    assert!(clamped.contains("Page 4 of 4"), "{clamped}");

    // A sort submit reflects the new query through the URL intent, and the same column again flips it.
    let table = island_tag(&html, "datatable-gallery");
    let mut snapshot = decoded_snapshot(table);
    let mut sequence = 0;
    let mut run = |action: &'static str, proposals: Value, snapshot: Value| {
        sequence += 1;
        let idempotency_key = idempotency(sequence);
        let revision = snapshot_revision(&snapshot);
        let app = &app;
        let session = &session;
        // A submit synchronizes each proposed model field before the action runs.
        let mut operations: Vec<Value> = proposals
            .as_object()
            .map(|fields| {
                fields
                    .keys()
                    .map(|field| serde_json::json!({"field": field, "kind": "sync_model"}))
                    .collect()
            })
            .unwrap_or_default();
        operations.extend(invoke(action).as_array().cloned().unwrap_or_default());
        async move {
            let reply = send(
                app.addr,
                action_request(
                    app,
                    ActionSpec {
                        component: "app.datatable-gallery",
                        document_key: "datatable-gallery",
                        snapshot,
                        seed: false,
                        base_revision: &revision,
                        operations: Value::Array(operations),
                        model_proposals: proposals,
                        idempotency_key: &idempotency_key,
                    },
                    Some(session),
                    true,
                ),
            )
            .await;
            assert_eq!(reply.status, StatusCode::OK, "{action}: {}", reply.text());
            let accepted = reply.json();
            assert_eq!(accepted["outcome"], "accepted", "{action}: {accepted}");
            accepted
        }
    };
    let accepted = run("sort", serde_json::json!({"sort": "amount"}), snapshot).await;
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({"kind": "reflected", "target": "/live/data-display?sort=amount"})
    );
    snapshot = accepted["snapshot"].clone();
    let accepted = run("sort", serde_json::json!({"sort": "amount"}), snapshot).await;
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({"kind": "reflected", "target": "/live/data-display?dir=desc&sort=amount"})
    );
    snapshot = accepted["snapshot"].clone();
    let accepted = run("filter", serde_json::json!({"filter": "acme"}), snapshot).await;
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({"kind": "reflected", "target": "/live/data-display?dir=desc&filter=acme&sort=amount"})
    );
    let rendered = accepted["render"]["html"].as_str().unwrap_or_default();
    assert!(rendered.contains("(2 rows)"), "{rendered}");
    snapshot = accepted["snapshot"].clone();
    let accepted = run("next_page", Value::Object(Default::default()), snapshot).await;
    assert_eq!(
        accepted["url_intent"],
        serde_json::json!({"kind": "reflected", "target": "/live/data-display?dir=desc&filter=acme&sort=amount"}),
        "one page of two rows has no next page to reflect"
    );
}

/// One control request on the reserved upload route, as the widget's runtime sends it.
async fn upload_control(
    app: &live_support::TestApp,
    session: &live_support::SeededSession,
    grant: Option<&str>,
    body: Value,
) -> live_support::Reply {
    let mut builder = request(app, Method::POST, UPLOAD_PATH, Some(session), true)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("x-suprnova-live", "upload-v1");
    if let Some(grant) = grant {
        builder = builder.header("authorization", format!("SuprnovaUpload {grant}"));
    }
    let request = builder
        .body(http_body_util::Full::new(bytes::Bytes::from(
            serde_json::to_vec(&body).expect("encode"),
        )))
        .expect("build control request");
    send(app.addr, request).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_native_gallery_renders_every_component_on_native_controls() {
    let app = setup_app(6).await;
    let session = seed_session(&app).await;
    let html = get(&app, "/live/live-native", Some(&session)).await.text();
    for needle in [
        // FORM-005: the upload widget over the shipped protocol, every state as text.
        "<input class=\"sn-upload-input\" id=\"attachment\" type=\"file\" live:upload=\"attachment\" live:key=\"attachment-input\" accept=\"image/png\">",
        "<progress class=\"sn-upload-progress\" live:progress=\"attachment\" live:key=\"attachment-progress\" max=\"100\" aria-label=\"Attachment upload progress\"></progress>",
        "data-sn-state=\"ready\">Verified. Not saved until you submit.</span>",
        "live:upload.cancel=\"attachment\"",
        "live:upload.retry=\"attachment\"",
        "live:upload.remove=\"attachment\"",
        // FORM-006: one native input for the code, cells hidden from assistive technology.
        "<sn-input-otp class=\"sn-otp\" data-sn-length=\"6\" live:key=\"code-otp\" live:preserve.self>",
        "id=\"code\" name=\"code\" type=\"text\" inputmode=\"numeric\" autocomplete=\"one-time-code\" pattern=\"[0-9]{6}\" maxlength=\"6\"",
        "<span class=\"sn-otp-cells\" aria-hidden=\"true\">",
        "<span class=\"sn-otp-cell\" data-sn-index=\"5\"></span>",
        // FORM-007: a date input and native radio strips with legends.
        "<sn-date-picker class=\"sn-date\" live:key=\"when-date\" live:preserve.self>",
        "id=\"when\" name=\"when\" type=\"date\" min=\"2026-01-01\" max=\"2028-12-31\" live:model=\"when\"",
        "<fieldset class=\"sn-date-strip\" data-sn-part=\"year\"><legend class=\"sn-date-legend\">Year</legend>",
        "<input class=\"sn-date-radio\" type=\"radio\" name=\"when-month\" value=\"12\">December",
        "<input class=\"sn-date-radio\" type=\"radio\" name=\"when-day\" value=\"31\">31",
        // FORM-008: the combobox pattern over a native input, with a datalist before upgrade.
        "<sn-combobox class=\"sn-combobox\" live:key=\"country-combobox\" live:preserve.self>",
        "role=\"combobox\" aria-autocomplete=\"list\" aria-expanded=\"false\" aria-controls=\"country-listbox\"",
        "<datalist id=\"country-datalist\"><option value=\"Canada\"></option>",
        "<ul class=\"sn-combobox-listbox\" id=\"country-listbox\" role=\"listbox\" aria-label=\"Country suggestions\" data-sn-query=\"\"",
        "<li class=\"sn-combobox-option\" id=\"country-option-1\" role=\"option\" aria-selected=\"false\" data-sn-value=\"ca\" live:key=\"ca\">Canada</li>",
        // FDB-005: the feed and the bell render the disconnected default and a polite status.
        "<section class=\"sn-live-feed\" id=\"activity\" aria-labelledby=\"activity-heading\">",
        "<p class=\"sn-live-feed-status\" data-live-stream-status role=\"status\" aria-live=\"polite\">Updates disconnected</p>",
        "<li class=\"sn-live-feed-item\" live:key=\"post-0\">",
        "<span class=\"sn-bell-count\" data-sn-count=\"0\">0 unread</span>",
        "<span class=\"sn-bell-status\" id=\"bell-status\" data-live-stream-status role=\"status\" aria-live=\"polite\">Updates disconnected</span>",
        // NAV-005: the account menu is its own island, a details disclosure with anchors and a form.
        "<details class=\"sn-account-menu\" id=\"account\">",
        "<summary class=\"sn-account-menu-summary\" aria-label=\"Account: Ada Lovelace\">",
        "<form class=\"sn-account-menu-form\" method=\"post\" action=\"/live/sign-out\"><input type=\"hidden\" name=\"_token\" value=\"",
    ] {
        assert!(html.contains(needle), "missing {needle} in {html}");
    }
    assert!(
        html.contains("live:stream"),
        "the gallery island declares its stream: {html}"
    );
    assert!(
        !html.contains(" style="),
        "no shipped view carries a style attribute"
    );
    assert_eq!(
        html.matches("data-suprnova-live-island").count(),
        2,
        "two islands: the account menu and the gallery"
    );
    for script in [
        "/suprnova-ui/input-otp/input-otp.js",
        "/suprnova-ui/date-picker/date-picker.js",
        "/suprnova-ui/combobox/combobox.js",
    ] {
        assert!(html.contains(script), "the document loads {script}");
    }
    // The transient code never enters the snapshot.
    let snapshot = decoded_snapshot(island_tag(&html, "live-native-gallery"));
    assert!(
        snapshot.get("code").is_none(),
        "the one-time code is transient: {snapshot}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_upload_widget_drives_the_shipped_protocol_and_finalizes_through_the_action() {
    let app = setup_app(10).await;
    let owner = seed_session(&app).await;
    let html = get(&app, "/live/live-native", Some(&owner)).await.text();
    let island = island_tag(&html, "live-native-gallery");
    let snapshot = decoded_snapshot(island);
    let revision = snapshot_revision(&snapshot);
    let bytes = tiny_png();
    let checksum = sha256_hex(&bytes);

    // Every request below goes to the reserved upload route: create, one chunk, complete.
    let created = upload_control(
        &app,
        &owner,
        None,
        serde_json::json!({
            "field": "attachment",
            "file": {"lastModified": 1, "name": "attachment.png", "size": bytes.len(), "type": "image/png"},
            "idempotency_key": "create-attachment-1",
            "island": {"component": "app.live-native-gallery", "documentKey": "live-native-gallery", "slot": "gallery"},
            "operation": "create",
            "protocol_version": 1,
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    assert_eq!(created["state"], "queued", "{created}");
    let handle = created["handle"].as_str().expect("handle").to_owned();
    let grant = created["grant"].as_str().expect("grant").to_owned();

    let chunk = request(&app, Method::POST, UPLOAD_PATH, Some(&owner), true)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-handle", &handle)
        .header("x-suprnova-upload-idempotency", "put-attachment-0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(http_body_util::Full::new(bytes::Bytes::from(bytes.clone())))
        .expect("build chunk request");
    let stored = send(app.addr, chunk).await;
    assert_eq!(stored.status, StatusCode::OK, "{}", stored.text());
    let stored = stored.json();
    assert_eq!(stored["state"], "transferring", "{stored}");
    let after_chunk = stored["revision"].as_str().expect("revision").to_owned();

    let completed = upload_control(
        &app,
        &owner,
        Some(&grant),
        serde_json::json!({
            "expected_revision": after_chunk,
            "handle": handle,
            "idempotency_key": "complete-attachment-1",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum,
        }),
    )
    .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());
    assert_eq!(
        completed.json()["state"],
        "ready",
        "ready is verified, not saved: {}",
        completed.text()
    );
    assert_eq!(
        app.finalizer.committed().len(),
        0,
        "nothing is durable before the finalizing action"
    );

    // The finalizing action: the widget's form submits save_attachment with the
    // handle as the model proposal.
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.live-native-gallery",
                document_key: "live-native-gallery",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: serde_json::json!([
                    {"field": "attachment", "kind": "sync_model"},
                    {"arguments": {}, "kind": "invoke_action", "name": "save_attachment"},
                ]),
                model_proposals: serde_json::json!({"attachment": handle}),
                idempotency_key: &idempotency(41),
            },
            Some(&owner),
            true,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let reply = reply.json();
    assert_eq!(reply["outcome"], "accepted", "{reply}");
    assert_eq!(
        app.finalizer.committed().len(),
        1,
        "the application finalizer committed the attachment"
    );
    let rendered = reply["render"]["html"].as_str().unwrap_or_default();
    assert!(
        rendered.contains("data-saved=\"1\">Saved 1"),
        "the view counts the finalized attachment: {reply}"
    );
}
