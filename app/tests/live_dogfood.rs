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
    assert_eq!(config_json(&html)["endpoint"], "/__live/v1/action");

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
    let cross = request(
        &app,
        Method::POST,
        "/__live/v1/action",
        Some(&session),
        false,
    )
    .header("sec-fetch-site", "cross-site")
    .header("content-type", live_support::LIVE_MEDIA)
    .body(empty())
    .expect("build");
    let reply = send(app.addr, cross).await;
    assert_eq!(reply.status, StatusCode::from_u16(419).expect("419"));

    // Anonymous, same-origin: the route guard's AuthMiddleware answers first.
    let reply = send(app.addr, action_request(&app, spec(13), None, true)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
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
        &format!("/__live/v1/assets/{identity}/suprnova-live.esm.js"),
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
    let reply = get(&app, "/__live/v1/assets/stale/suprnova-live.esm.js", None).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.body.is_empty());
}
