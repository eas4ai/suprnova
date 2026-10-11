//! Laravel infrastructure gaps in broadcasting: a client for Pusher's HTTP
//! API, built on its own or taken from the hub (PAR-152).
//!
//! Laravel evidence: `Broadcasting/BroadcastManager.php:363`
//! (`Broadcast::pusher($config)` returns a configured client) and
//! `Broadcasting/Broadcasters/PusherBroadcaster.php` (`getPusher`). The
//! endpoints come from Pusher's HTTP API documentation, which is not in the
//! reference tree.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use md5::Md5;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova::broadcasting::{
    BroadcastEnvelope, BroadcastHub, Channel, ChannelParams, ChannelRegistry, ChannelVisibility,
    PresenceChannel,
};
use suprnova::{
    FrameworkError, PusherBroadcastHub, PusherClient, PusherConfig, PusherScheme, Request,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const APP_ID: &str = "3";
const KEY: &str = "278d425bdf160c739803";
const SECRET: &str = "7ad3773142a6692b25b8";

// ---- channels ----

/// Private by default.
struct Orders;
#[async_trait]
impl Channel for Orders {
    fn name(&self) -> &'static str {
        "orders.{id}"
    }
}

struct News;
#[async_trait]
impl Channel for News {
    fn name(&self) -> &'static str {
        "news"
    }
    fn visibility(&self) -> ChannelVisibility {
        ChannelVisibility::Public
    }
}

/// The presence channel `room`.
struct Room;
#[async_trait]
impl Channel for Room {
    fn name(&self) -> &'static str {
        "room"
    }
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}
#[async_trait]
impl PresenceChannel for Room {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        Ok(json!({ "name": "Ada" }))
    }
}

fn registry() -> Arc<ChannelRegistry> {
    let mut registry = ChannelRegistry::new();
    registry.register(Orders);
    registry.register(News);
    registry.register(Room);
    Arc::new(registry)
}

// ---- helpers ----

fn config_for(server: &MockServer) -> PusherConfig {
    let address = server.address();
    PusherConfig::new(APP_ID, KEY, SECRET)
        .host(address.ip().to_string())
        .port(address.port())
        .scheme(PusherScheme::Http)
}

async fn mock(server: &MockServer, verb: &str, at: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn hmac_hex(message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).expect("any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Every query parameter of a recorded request, decoded.
fn query_of(request: &wiremock::Request) -> HashMap<String, String> {
    request
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Check the request's `auth_signature` the way the service does: every
/// other parameter, decoded, sorted by key and joined unescaped, under the
/// method and the path. Returns the signature.
fn assert_signed(request: &wiremock::Request) -> String {
    let query = query_of(request);
    assert_eq!(query["auth_key"], KEY);
    assert_eq!(query["auth_version"], "1.0");
    let mut signed: Vec<(&String, &String)> = query
        .iter()
        .filter(|(name, _)| name.as_str() != "auth_signature")
        .collect();
    signed.sort();
    let unsigned = signed
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let expected = hmac_hex(&format!(
        "{}\n{}\n{unsigned}",
        request.method.as_str(),
        request.url.path()
    ));
    assert_eq!(
        query["auth_signature"], expected,
        "the signature verifies under the app secret"
    );
    expected
}

// ---- PAR-152 ----

#[tokio::test]
async fn presence_users_lists_the_services_users_through_a_signed_get() {
    let server = MockServer::start().await;
    mock(
        &server,
        "GET",
        "/apps/3/channels/presence-room/users",
        200,
        json!({ "users": [{ "id": "1" }, { "id": "2" }] }),
    )
    .await;
    let hub = PusherBroadcastHub::new(config_for(&server), registry()).unwrap();

    let users = hub.client().presence_users("room").await.unwrap();
    assert_eq!(users, vec!["1".to_string(), "2".to_string()]);

    let requests = server.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method.as_str(), "GET");
    assert_eq!(request.url.path(), "/apps/3/channels/presence-room/users");
    assert_signed(request);
    assert!(
        !query_of(request).contains_key("body_md5"),
        "a request without a body carries no body_md5"
    );
}

#[tokio::test]
async fn presence_users_of_a_channel_that_is_not_presence_is_refused_before_a_request() {
    let server = MockServer::start().await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();
    let err = client
        .presence_users("orders.42")
        .await
        .expect_err("a private channel has no presence users");
    assert!(err.to_string().contains("presence"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn trigger_batch_sends_every_envelope_in_one_signed_request() {
    let server = MockServer::start().await;
    mock(&server, "POST", "/apps/3/batch_events", 200, json!({})).await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();

    client
        .trigger_batch(&[
            BroadcastEnvelope::new("orders.42", "OrderShipped", json!({ "id": 42 })),
            BroadcastEnvelope::new("news", "Posted", json!({ "title": "Hi" }))
                .with_except("5678.5678"),
        ])
        .await
        .expect("a 200 answer is a published batch");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "two envelopes are one request");
    let request = &requests[0];
    assert_eq!(request.url.path(), "/apps/3/batch_events");
    assert_eq!(
        query_of(request)["body_md5"],
        hex::encode(Md5::digest(&request.body)),
        "a request with a body signs its bytes"
    );
    assert_signed(request);
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({ "batch": [
            { "name": "OrderShipped", "channel": "private-orders.42", "data": "{\"id\":42}" },
            { "name": "Posted", "channel": "news", "data": "{\"title\":\"Hi\"}",
              "socket_id": "5678.5678" },
        ] }),
        "channel names go through the registry's wire mapping"
    );
}

#[tokio::test]
async fn trigger_batch_of_nothing_sends_nothing() {
    let server = MockServer::start().await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();
    client.trigger_batch(&[]).await.unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn channels_and_channel_query_the_service_with_their_parameters() {
    let server = MockServer::start().await;
    mock(
        &server,
        "GET",
        "/apps/3/channels",
        200,
        json!({ "channels": { "presence-room": { "user_count": 2 } } }),
    )
    .await;
    mock(
        &server,
        "GET",
        "/apps/3/channels/private-orders.42",
        200,
        json!({ "occupied": true, "subscription_count": 3 }),
    )
    .await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();

    let channels = client
        .channels(Some("presence-"), &["user_count"])
        .await
        .unwrap();
    assert_eq!(channels["channels"]["presence-room"]["user_count"], 2);
    let channel = client
        .channel("orders.42", &["user_count", "subscription_count"])
        .await
        .unwrap();
    assert_eq!(channel["subscription_count"], 3);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let listing = query_of(&requests[0]);
    assert_eq!(listing["filter_by_prefix"], "presence-");
    assert_eq!(listing["info"], "user_count");
    assert_signed(&requests[0]);
    assert_eq!(
        query_of(&requests[1])["info"],
        "user_count,subscription_count"
    );
    assert_signed(&requests[1]);
}

#[tokio::test]
async fn terminate_user_connections_posts_a_signed_body() {
    let server = MockServer::start().await;
    mock(
        &server,
        "POST",
        "/apps/3/users/42/terminate_connections",
        200,
        json!({}),
    )
    .await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();
    client.terminate_user_connections("42").await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        query_of(&requests[0])["body_md5"],
        hex::encode(Md5::digest(&requests[0].body))
    );
    assert_signed(&requests[0]);

    let err = client
        .terminate_user_connections("../events")
        .await
        .expect_err("a user id that would change the path is refused");
    assert!(err.to_string().contains("user id"), "{err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn get_sends_any_other_signed_get_and_refuses_the_signatures_own_parameters() {
    let server = MockServer::start().await;
    mock(
        &server,
        "GET",
        "/apps/3/channels",
        200,
        json!({ "channels": {} }),
    )
    .await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();

    let answer = client
        .get("/channels", &[("filter_by_prefix", "private-")])
        .await
        .unwrap();
    assert_eq!(answer, json!({ "channels": {} }));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(query_of(&requests[0])["filter_by_prefix"], "private-");
    assert_signed(&requests[0]);

    let reserved = client
        .get("/channels", &[("auth_timestamp", "1")])
        .await
        .expect_err("the signature's own parameters are refused");
    assert!(reserved.to_string().contains("reserved"), "{reserved}");
    let bad_path = client
        .get("/channels?x=1", &[])
        .await
        .expect_err("a path with its own query is refused");
    assert!(bad_path.to_string().contains("path"), "{bad_path}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_refused_request_never_quotes_the_secret_or_the_signature() {
    let server = MockServer::start().await;
    // A service that echoes what it was sent, secret included.
    Mock::given(method("GET"))
        .and(path("/apps/3/channels"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string(format!("invalid signature for {SECRET}")),
        )
        .mount(&server)
        .await;
    let client = PusherClient::new(config_for(&server), registry()).unwrap();

    let err = client
        .channels(None, &[])
        .await
        .expect_err("a 403 answer is an error");
    let message = err.to_string();
    assert!(message.contains("403"), "the status is named: {message}");
    assert!(!message.contains(SECRET), "never the secret: {message}");
    let requests = server.received_requests().await.unwrap();
    let signature = query_of(&requests[0])["auth_signature"].clone();
    assert!(
        !message.contains(&signature),
        "never the signature: {message}"
    );
}

#[tokio::test]
async fn the_hubs_list_members_keeps_answering_from_the_local_hub() {
    let server = MockServer::start().await;
    let hub = PusherBroadcastHub::new(config_for(&server), registry()).unwrap();
    hub.track_member("room", "7", json!({ "name": "Ada" }))
        .await
        .unwrap();

    let members = hub.list_members("room").await;
    assert_eq!(members.len(), 1, "{members:?}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no request reaches the service"
    );
}

#[test]
fn a_client_refuses_an_invalid_config_without_quoting_the_secret() {
    let config = PusherConfig::new("3", KEY, SECRET).host("evil.example/x?");
    let err = PusherClient::new(config, registry())
        .err()
        .map(|e| e.to_string())
        .expect("a bad host fails");
    assert!(err.contains("host"), "{err}");
    assert!(!err.contains(SECRET), "{err}");
}
