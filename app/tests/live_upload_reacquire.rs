//! Uploads through the real application: create, transfer, reacquire through
//! the application-owned authenticated route outside `/__live/`, complete, and
//! finalize through the component action and the application finalizer.

mod live_support;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, UPLOAD_PATH, action_request, dashboard_html, decoded_snapshot, idempotency,
    island_tag, request, seed_session, send, setup_app, sha256_hex, snapshot_revision, tiny_png,
};
use serde_json::{Value, json};

async fn control(
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
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&body).expect("encode"),
        )))
        .expect("build control request");
    send(app.addr, request).await
}

async fn reacquire(
    app: &live_support::TestApp,
    session: Option<&live_support::SeededSession>,
    handle: &str,
) -> live_support::Reply {
    let request = request(
        app,
        Method::POST,
        &format!("/account/uploads/{handle}/reacquire"),
        session,
        true,
    )
    .header("content-type", "application/json")
    .header("accept", "application/json")
    .body(Full::new(Bytes::from(
        serde_json::to_vec(
            &json!({"handle": handle, "operation": "reacquire", "protocol_version": 1}),
        )
        .expect("encode"),
    )))
    .expect("build reacquire request");
    send(app.addr, request).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_in_user_uploads_reacquires_completes_and_finalizes_an_avatar() {
    let app = setup_app(16).await;
    let owner = seed_session(&app).await;
    let stranger = seed_session(&app).await;
    let html = dashboard_html(&app, &owner).await;
    let uploader = island_tag(&html, "dashboard-uploader");
    let snapshot = decoded_snapshot(uploader);
    let revision = snapshot_revision(&snapshot);
    let bytes = tiny_png();
    let checksum = sha256_hex(&bytes);

    let created = control(
        &app,
        &owner,
        None,
        json!({
            "field": "avatar",
            "file": {"lastModified": 1, "name": "avatar.png", "size": bytes.len(), "type": "image/png"},
            "idempotency_key": "create-avatar-1",
            "island": {"component": "app.avatar-uploader", "documentKey": "dashboard-uploader", "slot": "uploader"},
            "operation": "create",
            "protocol_version": 1,
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    let handle = created["handle"].as_str().expect("handle").to_owned();
    let grant = created["grant"].as_str().expect("grant").to_owned();

    let chunk = request(&app, Method::POST, UPLOAD_PATH, Some(&owner), true)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-handle", &handle)
        .header("x-suprnova-upload-idempotency", "put-avatar-0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(Bytes::from(bytes.clone())))
        .expect("build chunk request");
    let stored = send(app.addr, chunk).await;
    assert_eq!(stored.status, StatusCode::OK, "{}", stored.text());
    let after_chunk = stored.json()["revision"]
        .as_str()
        .expect("revision")
        .to_owned();

    // Reacquisition: the owner gets a fresh grant, nobody else does.
    let reacquired = reacquire(&app, Some(&owner), &handle).await;
    assert_eq!(reacquired.status, StatusCode::OK, "{}", reacquired.text());
    assert_eq!(reacquired.header("cache-control"), Some("no-store"));
    let reacquired = reacquired.json();
    assert_eq!(reacquired["state"], "transferring");
    assert_eq!(reacquired["uploadedBytes"], bytes.len());
    let fresh_grant = reacquired["grant"]
        .as_str()
        .expect("fresh grant")
        .to_owned();
    assert_ne!(fresh_grant, grant, "reacquisition mints a fresh grant");

    let anonymous = reacquire(&app, None, &handle).await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text()
    );
    let foreign = reacquire(&app, Some(&stranger), &handle).await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN, "{}", foreign.text());
    assert!(
        !foreign.text().contains(&handle),
        "the handle never leaks to another user"
    );

    let completed = control(
        &app,
        &owner,
        Some(&fresh_grant),
        json!({
            "expected_revision": after_chunk,
            "handle": handle,
            "idempotency_key": "complete-avatar-1",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum,
        }),
    )
    .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text());
    assert_eq!(completed.json()["state"], "ready");

    // The component action syncs the model proposal and finalizes it through
    // the application's finalizer.
    let reply = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.avatar-uploader",
                document_key: "dashboard-uploader",
                snapshot,
                seed: false,
                base_revision: &revision,
                operations: json!([
                    {"field": "avatar", "kind": "sync_model"},
                    {"arguments": {}, "kind": "invoke_action", "name": "save_avatar"},
                ]),
                model_proposals: json!({"avatar": handle}),
                idempotency_key: &idempotency(31),
            },
            Some(&owner),
            true,
        ),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{} {:?}",
        reply.text(),
        reply.headers
    );
    assert_eq!(reply.json()["outcome"], "accepted", "{}", reply.text());
    assert_eq!(
        app.finalizer.committed().len(),
        1,
        "the application finalizer committed the upload"
    );
}
