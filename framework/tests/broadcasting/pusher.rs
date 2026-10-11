//! Integration tests for the Pusher-protocol driver: the REST publish
//! against a mock Pusher service, and the two authorization endpoints
//! driven through `handle_request` over a loopback socket.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crypto_secretbox::aead::Aead;
use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305};
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use md5::Md5;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova::broadcasting::{
    BroadcastEnvelope, BroadcastHub, Channel, ChannelParams, ChannelRegistry, ChannelVisibility,
    PresenceChannel,
};
use suprnova::session::{new_session_slot_for_test, session_scope_for_test};
use suprnova::testing::TestContainer;
use suprnova::{
    FrameworkError, Middleware, MiddlewareRegistry, Next, PusherBroadcastHub, PusherConfig,
    PusherScheme, Request, Response, Router, handle_request, pusher_channel_auth, pusher_user_auth,
};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const APP_ID: &str = "3";
const KEY: &str = "278d425bdf160c739803";
const SECRET: &str = "7ad3773142a6692b25b8";
const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SOCKET_ID: &str = "1234.1234";

// ---- channels ----

/// Private by default; admits order 42 only.
struct Orders;
#[async_trait]
impl Channel for Orders {
    fn name(&self) -> &'static str {
        "orders.{id}"
    }
    async fn authorize(&self, _req: &Request, params: &ChannelParams, _data: &Value) -> bool {
        params.get("id") == Some("42")
    }
}

/// Private; admits only a request whose body carried `token=valid`, so
/// the test can prove the remaining body fields reach `authorize`.
struct Tokened;
#[async_trait]
impl Channel for Tokened {
    fn name(&self) -> &'static str {
        "tokened"
    }
    async fn authorize(&self, _req: &Request, _params: &ChannelParams, data: &Value) -> bool {
        data["token"] == "valid"
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

/// Encrypted; refuses vault 13.
struct Vault;
#[async_trait]
impl Channel for Vault {
    fn name(&self) -> &'static str {
        "vault.{id}"
    }
    fn visibility(&self) -> ChannelVisibility {
        ChannelVisibility::Encrypted
    }
    async fn authorize(&self, _req: &Request, params: &ChannelParams, _data: &Value) -> bool {
        params.get("id") != Some("13")
    }
}

/// Private, and named so its wire name would be `vault.{id}`'s
/// encrypted name. The driver refuses it.
struct VaultLookalike;
#[async_trait]
impl Channel for VaultLookalike {
    fn name(&self) -> &'static str {
        "encrypted-vault.{id}"
    }
}

/// Public, and named like a private wire name. The driver refuses it.
struct PrivateLookingFeed;
#[async_trait]
impl Channel for PrivateLookingFeed {
    fn name(&self) -> &'static str {
        "private-feed"
    }
    fn visibility(&self) -> ChannelVisibility {
        ChannelVisibility::Public
    }
}

/// Private, with a first segment the caller chooses.
struct ShopOrders;
#[async_trait]
impl Channel for ShopOrders {
    fn name(&self) -> &'static str {
        "{slug}.orders"
    }
}

/// Private, with a name that itself starts with `private-`. Its wire
/// name `private-private-x` still reads back as this channel.
struct PrivateX;
#[async_trait]
impl Channel for PrivateX {
    fn name(&self) -> &'static str {
        "private-x"
    }
}

/// Presence with Encrypted visibility. Pusher has no encrypted presence
/// channels, so the driver refuses it.
struct SecretRoom;
#[async_trait]
impl Channel for SecretRoom {
    fn name(&self) -> &'static str {
        "secret-room"
    }
    fn visibility(&self) -> ChannelVisibility {
        ChannelVisibility::Encrypted
    }
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}
#[async_trait]
impl PresenceChannel for SecretRoom {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        Ok(json!({}))
    }
}

/// Presence; refuses room 3, and its `member_info` fails for room 2.
struct Room;
#[async_trait]
impl Channel for Room {
    fn name(&self) -> &'static str {
        "room.{id}"
    }
    async fn authorize(&self, _req: &Request, params: &ChannelParams, _data: &Value) -> bool {
        params.get("id") != Some("3")
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
        params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        if params.get("id") == Some("2") {
            return Err(FrameworkError::internal("profile lookup failed"));
        }
        Ok(json!({ "name": "Ada" }))
    }
}

struct Chat;
#[async_trait]
impl Channel for Chat {
    fn name(&self) -> &'static str {
        "chat"
    }
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}
#[async_trait]
impl PresenceChannel for Chat {
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
    registry.register(Tokened);
    registry.register(News);
    registry.register(Vault);
    registry.register(Chat);
    registry.register(VaultLookalike);
    registry.register(PrivateLookingFeed);
    registry.register(ShopOrders);
    registry.register(PrivateX);
    registry.register(SecretRoom);
    registry.register(Room);
    Arc::new(registry)
}

// ---- helpers ----

fn hmac_hex(message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).expect("any key length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn master_key() -> Vec<u8> {
    STANDARD.decode(MASTER_KEY_B64).expect("valid base64")
}

fn shared_secret(channel_name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(channel_name.as_bytes());
    hasher.update(master_key());
    hasher.finalize().into()
}

fn config_for(server: &MockServer) -> PusherConfig {
    let address = server.address();
    PusherConfig::new(APP_ID, KEY, SECRET)
        .host(address.ip().to_string())
        .port(address.port())
        .scheme(PusherScheme::Http)
}

async fn mock_pusher(status: u16, body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/apps/{APP_ID}/events")))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    server
}

/// Every query parameter of a recorded request.
fn query_of(request: &wiremock::Request) -> HashMap<String, String> {
    request
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

// ---- publishing ----

#[tokio::test]
async fn pusher_publish_posts_a_signed_event() {
    let server = mock_pusher(200, "{}").await;
    let hub = PusherBroadcastHub::new(config_for(&server), registry()).unwrap();

    hub.publish(BroadcastEnvelope::new(
        "orders.42",
        "OrderShipped",
        json!({ "id": 42 }),
    ))
    .await
    .expect("a 200 answer is a successful publish");
    hub.publish(
        BroadcastEnvelope::new("orders.42", "OrderShipped", json!({ "id": 42 }))
            .with_except("5678.5678"),
    )
    .await
    .expect("a 200 answer is a successful publish");

    let requests = server.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 2, "one envelope is one request");

    for request in &requests {
        assert_eq!(request.url.path(), "/apps/3/events");
        assert_eq!(
            request
                .headers
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );

        let query = query_of(request);
        assert_eq!(query["auth_key"], KEY);
        assert_eq!(query["auth_version"], "1.0");
        assert_eq!(
            query["body_md5"],
            hex::encode(Md5::digest(&request.body)),
            "body_md5 covers the exact bytes received"
        );
        let timestamp: u64 = query["auth_timestamp"].parse().expect("unix seconds");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(now.abs_diff(timestamp) < 300, "a current timestamp");

        let unsigned = format!(
            "auth_key={}&auth_timestamp={}&auth_version={}&body_md5={}",
            query["auth_key"], query["auth_timestamp"], query["auth_version"], query["body_md5"]
        );
        let expected = hmac_hex(&format!("POST\n/apps/3/events\n{unsigned}"));
        assert_eq!(query["auth_signature"], expected, "the signature verifies");
        assert_eq!(
            request.url.query(),
            Some(format!("{unsigned}&auth_signature={expected}").as_str()),
            "parameters sorted by key, auth_signature last"
        );
    }

    let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        first,
        json!({
            "name": "OrderShipped",
            "channels": ["private-orders.42"],
            "data": "{\"id\":42}",
        }),
        "no socket_id without an exclusion"
    );
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(second["socket_id"], "5678.5678");
    assert_eq!(second["channels"], json!(["private-orders.42"]));
}

#[tokio::test]
async fn pusher_publish_names_public_and_presence_channels() {
    let server = mock_pusher(200, "{}").await;
    let hub = PusherBroadcastHub::new(config_for(&server), registry()).unwrap();
    hub.publish(BroadcastEnvelope::new("news", "Posted", json!({})))
        .await
        .unwrap();
    hub.publish(BroadcastEnvelope::new("chat", "Said", json!({})))
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let channels: Vec<Value> = requests
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["channels"].clone())
        .collect();
    assert_eq!(channels, vec![json!(["news"]), json!(["presence-chat"])]);
}

#[tokio::test]
async fn pusher_publish_encrypts_for_encrypted_channels() {
    let server = mock_pusher(200, "{}").await;
    let config = config_for(&server)
        .encryption_master_key_base64(MASTER_KEY_B64)
        .unwrap();
    let hub = PusherBroadcastHub::new(config, registry()).unwrap();
    hub.publish(BroadcastEnvelope::new(
        "vault.7",
        "Opened",
        json!({ "pin": 1234 }),
    ))
    .await
    .unwrap();

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["channels"], json!(["private-encrypted-vault.7"]));
    let data = body["data"].as_str().expect("data is a JSON string");
    assert!(!data.contains("1234"), "never plaintext: {data}");
    assert_eq!(
        open(shared_secret("private-encrypted-vault.7"), data),
        json!({ "pin": 1234 })
    );
}

/// Decrypt an encrypted event's `data` the way pusher-js does, with
/// the channel's shared secret.
fn open(shared_secret: [u8; 32], data: &str) -> Value {
    let payload: Value = serde_json::from_str(data).unwrap();
    let nonce = STANDARD.decode(payload["nonce"].as_str().unwrap()).unwrap();
    let ciphertext = STANDARD
        .decode(payload["ciphertext"].as_str().unwrap())
        .unwrap();
    let key = crypto_secretbox::Key::from(shared_secret);
    let nonce: [u8; 24] = nonce.try_into().expect("24-byte nonce");
    let plaintext = XSalsa20Poly1305::new(&key)
        .decrypt(&crypto_secretbox::Nonce::from(nonce), ciphertext.as_slice())
        .expect("a Pusher client with the shared secret can decrypt it");
    serde_json::from_slice(&plaintext).unwrap()
}

/// Publish to `channel` and assert it is refused before anything is sent
/// to the service, while the in-process subscriber still receives it.
async fn assert_refused_remotely(channel: &str) {
    let server = mock_pusher(200, "{}").await;
    let config = config_for(&server)
        .encryption_master_key_base64(MASTER_KEY_B64)
        .unwrap();
    let hub = PusherBroadcastHub::new(config, registry()).unwrap();
    let mut local = hub.subscribe(channel);
    let err = hub
        .publish(BroadcastEnvelope::new(channel, "Leak", json!({ "pin": 1 })))
        .await
        .expect_err("a colliding name is refused");
    assert!(err.to_string().contains(channel), "{channel}: {err}");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "{channel} must never reach the service"
    );
    assert_eq!(
        local
            .try_recv()
            .expect("local delivery still happens")
            .event,
        "Leak"
    );
}

#[tokio::test]
async fn pusher_publish_refuses_names_that_collide_with_another_wire_name() {
    // Private `encrypted-vault.{id}` would share `private-encrypted-vault.9`
    // with the encrypted `vault.{id}`.
    assert_refused_remotely("encrypted-vault.9").await;
    // Public `private-feed` would share its bare name with a private `feed`.
    assert_refused_remotely("private-feed").await;
    // Unregistered names fail closed to `private-`, so this would read as
    // the encrypted `x`.
    assert_refused_remotely("encrypted-x").await;
    // A pattern parameter cannot forge the prefix either.
    assert_refused_remotely("encrypted-shop.orders").await;
    // Pusher has no encrypted presence channels: never plaintext.
    assert_refused_remotely("secret-room").await;
}

#[tokio::test]
async fn pusher_encrypted_secret_opens_only_its_own_channel() {
    let server = mock_pusher(200, "{}").await;
    let config = config_for(&server)
        .encryption_master_key_base64(MASTER_KEY_B64)
        .unwrap();
    let hub = PusherBroadcastHub::new(config, registry()).unwrap();

    hub.publish(BroadcastEnvelope::new(
        "encrypted-vault.9",
        "Leak",
        json!({ "pin": 1 }),
    ))
    .await
    .expect_err("the lookalike is refused");
    hub.publish(BroadcastEnvelope::new(
        "vault.9",
        "Opened",
        json!({ "pin": 9 }),
    ))
    .await
    .expect("the encrypted channel publishes");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "only vault.9 reached the service");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["channels"], json!(["private-encrypted-vault.9"]));
    assert_eq!(body["name"], "Opened");

    let (status, answer) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-encrypted-vault.9"),
    )
    .await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(
        answer["auth"],
        format!("{KEY}:{}", hmac_hex("1234.1234:private-encrypted-vault.9"))
    );
    let secret: [u8; 32] = STANDARD
        .decode(answer["shared_secret"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        open(secret, body["data"].as_str().unwrap()),
        json!({ "pin": 9 }),
        "the secret handed out decrypts vault.9's own events"
    );
}

#[tokio::test]
async fn pusher_publish_failure_is_err_and_local_subscriber_still_receives() {
    let answer = format!("boom {SECRET} {}", "x".repeat(300));
    let server = mock_pusher(500, &answer).await;
    let hub = PusherBroadcastHub::new(config_for(&server), registry()).unwrap();
    let mut local = hub.subscribe("orders.42");

    let err = hub
        .publish(BroadcastEnvelope::new(
            "orders.42",
            "OrderShipped",
            json!({ "id": 42 }),
        ))
        .await
        .expect_err("a 500 answer is an error");

    let message = err.to_string();
    assert!(
        message.contains("500"),
        "the error names the status: {message}"
    );
    assert!(message.contains("boom"), "and a body excerpt: {message}");
    assert!(
        !message.contains(&"x".repeat(200)),
        "the excerpt is capped at 200 bytes: {message}"
    );
    assert!(!message.contains(SECRET), "never the secret: {message}");
    let requests = server.received_requests().await.unwrap();
    let signature = query_of(&requests[0])["auth_signature"].clone();
    assert!(
        !message.contains(&signature),
        "never the signature: {message}"
    );

    let got = local
        .try_recv()
        .expect("the in-process subscriber still received the envelope");
    assert_eq!(got.event, "OrderShipped");
}

#[tokio::test]
async fn pusher_publish_transport_error_never_quotes_the_signed_url() {
    // Bind and drop a listener to get a port nothing listens on.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let config = PusherConfig::new(APP_ID, KEY, SECRET)
        .host("127.0.0.1")
        .port(port)
        .scheme(PusherScheme::Http)
        .timeout(Duration::from_secs(2));
    let hub = PusherBroadcastHub::new(config, registry()).unwrap();
    let err = hub
        .publish(BroadcastEnvelope::new(
            "orders.42",
            "OrderShipped",
            json!({}),
        ))
        .await
        .expect_err("nothing listens on the port");
    let message = err.to_string();
    assert!(
        !message.contains("auth_signature"),
        "the signed URL is never quoted: {message}"
    );
    assert!(!message.contains(SECRET), "never the secret: {message}");
}

#[tokio::test]
#[ignore = "needs a running Soketi server over HTTP; set PUSHER_TEST_HOST, PUSHER_TEST_PORT, \
            PUSHER_TEST_APP_ID, PUSHER_TEST_KEY and PUSHER_TEST_SECRET"]
async fn pusher_publish_reaches_a_running_soketi_server() {
    let var = |name: &str| {
        std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set for this test"))
    };
    let config = PusherConfig::new(
        var("PUSHER_TEST_APP_ID"),
        var("PUSHER_TEST_KEY"),
        var("PUSHER_TEST_SECRET"),
    )
    .host(var("PUSHER_TEST_HOST"))
    .port(
        var("PUSHER_TEST_PORT")
            .parse()
            .expect("PUSHER_TEST_PORT is a port"),
    )
    .scheme(PusherScheme::Http);
    let hub = PusherBroadcastHub::new(config, registry()).unwrap();
    hub.publish(BroadcastEnvelope::new(
        "orders.42",
        "OrderShipped",
        json!({ "id": 42 }),
    ))
    .await
    .expect("Soketi answered 2xx");
}

// ---- authorization endpoints ----

/// Stand-in for `SessionMiddleware`: scopes a session whose user is the
/// given id, or a guest session.
struct ActingAs(Option<&'static str>);

#[async_trait]
impl Middleware for ActingAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let slot = new_session_slot_for_test();
        if let Some(user) = self.0 {
            slot.lock().unwrap().as_mut().unwrap().user_id = Some(user.to_string());
        }
        session_scope_for_test(slot, next(request)).await
    }
}

enum Body {
    Form(Vec<(&'static str, &'static str)>),
    /// A form body sent under an explicit `Content-Type` spelling.
    TypedForm(&'static str, Vec<(&'static str, &'static str)>),
    Json(Value),
}

fn channel_form(channel_name: &'static str) -> Body {
    Body::Form(vec![
        ("socket_id", SOCKET_ID),
        ("channel_name", channel_name),
    ])
}

/// POST one request to `path` with `auth` bound (or nothing bound) and
/// `user` logged in, and return the status and the JSON body.
async fn call(
    auth: Option<suprnova::PusherAuth>,
    user: Option<&'static str>,
    path: &'static str,
    body: Body,
) -> (u16, Value) {
    let middleware = MiddlewareRegistry::new().append(ActingAs(user));
    call_through(auth, || {}, middleware, path, body).await
}

/// POST one request to `path` through `middleware`, with `auth` bound (or
/// nothing bound) and `install` run first in the request's container scope,
/// and return the status and the JSON body.
async fn call_through(
    auth: Option<suprnova::PusherAuth>,
    install: fn(),
    middleware: MiddlewareRegistry,
    path: &'static str,
    body: Body,
) -> (u16, Value) {
    TestContainer::scope(async move {
        if let Some(auth) = auth {
            TestContainer::singleton(auth);
        }
        install();
        let router: Router = Router::new()
            .post("/broadcasting/auth", pusher_channel_auth)
            .post("/broadcasting/user-auth", pusher_user_auth)
            .into();
        let router = Arc::new(router);
        let middleware = Arc::new(middleware);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = TestContainer::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |request| {
                let router = router.clone();
                let middleware = middleware.clone();
                async move {
                    Ok::<_, Infallible>(handle_request(router, middleware, request).await)
                }
            });
            Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let request = client
            .post(format!("http://{address}{path}"))
            .header("Connection", "close");
        let request = match body {
            Body::Form(fields) => request.form(&fields),
            Body::TypedForm(content_type, fields) => {
                let mut encoded = url::form_urlencoded::Serializer::new(String::new());
                for (name, value) in fields {
                    encoded.append_pair(name, value);
                }
                request
                    .header("Content-Type", content_type)
                    .body(encoded.finish())
            }
            Body::Json(value) => request.json(&value),
        };
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("server timeout")
            .unwrap();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, body)
    })
    .await
}

fn hub_with_master_key() -> PusherBroadcastHub {
    let config = PusherConfig::new(APP_ID, KEY, SECRET)
        .encryption_master_key_base64(MASTER_KEY_B64)
        .unwrap();
    PusherBroadcastHub::new(config, registry()).unwrap()
}

#[tokio::test]
async fn pusher_channel_auth_signs_an_authorized_private_channel() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-orders.42"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "auth": format!("{KEY}:{}", hmac_hex("1234.1234:private-orders.42")) })
    );
}

/// Media types are case-insensitive (RFC 9110 8.3.1): a form sent as
/// `Application/X-WWW-Form-Urlencoded` carries the same fields, so the
/// endpoint must read them rather than refuse the request for missing
/// `socket_id` and `channel_name`.
#[tokio::test]
async fn pusher_channel_auth_reads_a_form_whatever_the_media_type_case() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::TypedForm(
            "Application/X-WWW-Form-Urlencoded; charset=UTF-8",
            vec![
                ("socket_id", SOCKET_ID),
                ("channel_name", "private-tokened"),
                ("token", "valid"),
            ],
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "auth": format!("{KEY}:{}", hmac_hex("1234.1234:private-tokened")) })
    );
}

#[tokio::test]
async fn pusher_channel_auth_refuses_when_authorize_is_false() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-orders.7"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_refuses_an_unknown_channel() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-nope.42"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_refuses_a_prefix_that_mismatches_the_visibility() {
    for channel_name in [
        // `orders.{id}` is private, not encrypted and not presence.
        "private-encrypted-orders.42",
        "presence-orders.42",
        // `news` is public: its wire name has no prefix.
        "private-news",
        // `vault.{id}` is encrypted, so plain `private-` is not its name.
        "private-vault.9",
        // `chat` is presence.
        "private-chat",
    ] {
        let hub = hub_with_master_key();
        let (status, body) = call(
            Some(hub.auth()),
            Some("7"),
            "/broadcasting/auth",
            channel_form(channel_name),
        )
        .await;
        assert_eq!(status, 403, "{channel_name}");
        assert_eq!(body, json!({}), "{channel_name}");
    }
}

#[tokio::test]
async fn pusher_channel_auth_refuses_a_bare_public_name() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("news"),
    )
    .await;
    assert_eq!(status, 403, "public channels need no authorization");
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_rejects_a_socket_id_with_a_colon() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::Form(vec![
            ("socket_id", "1234.1234:private-orders.42"),
            ("channel_name", "private-orders.42"),
        ]),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["errors"]["socket_id"].is_array(), "{body}");
    assert!(body["errors"].get("channel_name").is_none(), "{body}");
}

#[tokio::test]
async fn pusher_channel_auth_rejects_a_channel_name_with_a_colon() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-orders.42:x"),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["errors"]["channel_name"].is_array(), "{body}");
}

#[tokio::test]
async fn pusher_channel_auth_refuses_presence_without_a_user() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("presence-chat"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_signs_presence_channel_data() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-chat"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let channel_data = body["channel_data"]
        .as_str()
        .expect("channel_data is a string");
    assert_eq!(
        serde_json::from_str::<Value>(channel_data).unwrap(),
        json!({ "user_id": "7", "user_info": { "name": "Ada" } })
    );
    assert_eq!(
        body["auth"],
        format!(
            "{KEY}:{}",
            hmac_hex(&format!("1234.1234:presence-chat:{channel_data}"))
        ),
        "the signature covers the exact channel_data string returned"
    );
}

#[tokio::test]
async fn pusher_channel_auth_returns_the_shared_secret_for_an_encrypted_channel() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-encrypted-vault.9"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["auth"],
        format!("{KEY}:{}", hmac_hex("1234.1234:private-encrypted-vault.9"))
    );
    assert_eq!(
        body["shared_secret"],
        STANDARD.encode(shared_secret("private-encrypted-vault.9"))
    );
}

#[tokio::test]
async fn pusher_channel_auth_for_an_encrypted_channel_without_a_master_key_is_500() {
    let hub = PusherBroadcastHub::new(PusherConfig::new(APP_ID, KEY, SECRET), registry()).unwrap();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-encrypted-vault.9"),
    )
    .await;
    assert_eq!(status, 500, "{body}");
    assert!(body.get("shared_secret").is_none(), "{body}");
    assert!(body.get("auth").is_none(), "{body}");
}

#[tokio::test]
async fn pusher_channel_auth_reads_a_json_body_and_passes_the_rest_to_authorize() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::Json(json!({
            "socket_id": SOCKET_ID,
            "channel_name": "private-tokened",
            "token": "valid",
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["auth"],
        format!("{KEY}:{}", hmac_hex("1234.1234:private-tokened"))
    );

    let hub = hub_with_master_key();
    let (status, _) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::Json(json!({
            "socket_id": SOCKET_ID,
            "channel_name": "private-tokened",
            "token": "forged",
        })),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn pusher_channel_auth_passes_remaining_form_fields_to_authorize() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::Form(vec![
            ("socket_id", SOCKET_ID),
            ("channel_name", "private-tokened"),
            ("token", "valid"),
        ]),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "auth": format!("{KEY}:{}", hmac_hex("1234.1234:private-tokened")) })
    );

    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        Body::Form(vec![
            ("socket_id", SOCKET_ID),
            ("channel_name", "private-tokened"),
            ("token", "forged"),
        ]),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_refuses_an_encrypted_presence_channel() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-secret-room"),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_channel_auth_refuses_names_that_do_not_read_back() {
    for channel_name in [
        // A pattern parameter cannot forge the encrypted prefix.
        "private-encrypted-shop.orders",
        // The public `private-feed` has no private name.
        "private-private-feed",
        "private-feed",
    ] {
        let hub = hub_with_master_key();
        let (status, body) = call(
            Some(hub.auth()),
            Some("7"),
            "/broadcasting/auth",
            channel_form(channel_name),
        )
        .await;
        assert_eq!(status, 403, "{channel_name}: {body}");
        assert_eq!(body, json!({}), "{channel_name}");
    }
}

#[tokio::test]
async fn pusher_channel_auth_refuses_an_encrypted_channel_when_authorize_is_false() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-encrypted-vault.13"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}), "no shared_secret on a refusal");
}

#[tokio::test]
async fn pusher_channel_auth_refuses_presence_when_authorize_is_false() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-room.3"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));

    // The same channel admits another room.
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-room.1"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn pusher_channel_auth_member_info_error_is_500() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-room.2"),
    )
    .await;
    assert_eq!(status, 500, "{body}");
    assert!(body.get("auth").is_none(), "{body}");
    assert!(body.get("channel_data").is_none(), "{body}");
}

#[tokio::test]
async fn pusher_channel_auth_reads_doubled_prefixes_by_the_first_one() {
    // `private-x` is a private channel: its wire name is
    // `private-private-x`, and nothing else claims that name.
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/auth",
        channel_form("private-private-x"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "auth": format!("{KEY}:{}", hmac_hex("1234.1234:private-private-x")) })
    );

    // `presence-private-x` asks for a presence channel named `private-x`,
    // which is not what `private-x` is.
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/auth",
        channel_form("presence-private-x"),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_auth_without_a_bound_pusher_auth_is_500() {
    let (status, _) = call(
        None,
        None,
        "/broadcasting/auth",
        channel_form("private-orders.42"),
    )
    .await;
    assert_eq!(status, 500);
    let (status, _) = call(
        None,
        Some("7"),
        "/broadcasting/user-auth",
        Body::Form(vec![("socket_id", SOCKET_ID)]),
    )
    .await;
    assert_eq!(status, 500);
}

#[tokio::test]
async fn pusher_user_auth_refuses_a_guest() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        None,
        "/broadcasting/user-auth",
        Body::Form(vec![("socket_id", SOCKET_ID)]),
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body, json!({}));
}

#[tokio::test]
async fn pusher_user_auth_signs_the_user_data() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/user-auth",
        Body::Form(vec![("socket_id", SOCKET_ID)]),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let user_data = body["user_data"].as_str().expect("user_data is a string");
    assert_eq!(
        serde_json::from_str::<Value>(user_data).unwrap(),
        json!({ "id": "7" })
    );
    assert_eq!(
        body["auth"],
        format!(
            "{KEY}:{}",
            hmac_hex(&format!("1234.1234::user::{user_data}"))
        )
    );
}

#[tokio::test]
async fn pusher_user_auth_rejects_an_invalid_socket_id() {
    let hub = hub_with_master_key();
    let (status, body) = call(
        Some(hub.auth()),
        Some("7"),
        "/broadcasting/user-auth",
        Body::Form(vec![("socket_id", "1234.1234::user::x")]),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["errors"]["socket_id"].is_array(), "{body}");
}

// ---- the route's guard ----

/// A user known by its id alone.
struct GuardUser(&'static str);

impl suprnova::Authenticatable for GuardUser {
    fn get_auth_identifier(&self) -> String {
        self.0.to_owned()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// Resolves nobody: the users below are signed in through `set_user`.
struct NoLookups;

#[async_trait]
impl suprnova::UserProvider for NoLookups {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn suprnova::Authenticatable>>, suprnova::FrameworkError> {
        Ok(None)
    }
}

/// Registers the session guards `web` (the default) and `admin`.
fn install_web_and_admin_guards() {
    let config =
        suprnova::AuthConfig::new("web").guard("admin", suprnova::GuardConfig::session("admins"));
    TestContainer::singleton(suprnova::AuthManager::new(config));
    suprnova::Auth::register_provider("users", Arc::new(NoLookups)).unwrap();
    suprnova::Auth::register_provider("admins", Arc::new(NoLookups)).unwrap();
}

/// Signs web user 7 in on the default guard and admin 9 on `admin`.
struct WebAndAdmin;

#[async_trait]
impl Middleware for WebAndAdmin {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::Auth::guard("web")?
            .set_user(Arc::new(GuardUser("7")))
            .await;
        suprnova::Auth::guard("admin")?
            .set_user(Arc::new(GuardUser("9")))
            .await;
        next(request).await
    }
}

/// The Pusher endpoints behind `AuthMiddleware::new().for_guard("admin")`.
fn admin_route() -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(ActingAs(None))
        .append(WebAndAdmin)
        .append(suprnova::AuthMiddleware::new().for_guard("admin"))
}

/// Behind a second guard, the user authentication and presence endpoints
/// sign that guard's user as `admin:9`, never the default guard's user in
/// the same session.
#[tokio::test]
async fn pusher_endpoints_sign_the_route_guards_user() {
    let hub = hub_with_master_key();
    let (status, body) = call_through(
        Some(hub.auth()),
        install_web_and_admin_guards,
        admin_route(),
        "/broadcasting/user-auth",
        Body::Form(vec![("socket_id", SOCKET_ID)]),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let user_data = body["user_data"].as_str().expect("user_data is a string");
    assert_eq!(
        serde_json::from_str::<Value>(user_data).unwrap(),
        json!({ "id": "admin:9" })
    );

    let (status, body) = call_through(
        Some(hub.auth()),
        install_web_and_admin_guards,
        admin_route(),
        "/broadcasting/auth",
        channel_form("presence-chat"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let channel_data = body["channel_data"]
        .as_str()
        .expect("channel_data is a string");
    assert_eq!(
        serde_json::from_str::<Value>(channel_data).unwrap()["user_id"],
        json!("admin:9")
    );
}
