//! AGENT-006: user tokens from an outside authorization server, verified
//! against the keys it publishes. The issuer here is a test issuer with a
//! generated P-256 key, whose key set comes through a fake resolver and
//! pinned client: no test reaches the network.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};
use suprnova::MiddlewareRegistry;
use suprnova::agents::{AgentsConfig, OutsideIssuer};
use suprnova::testing::TestClient;

use crate::oauth::{FakeDocuments, FakeResolver, container, new_account};

const ISSUER: &str = "https://login.issuer.test";
const JWKS: &str = "https://login.issuer.test/.well-known/jwks.json";
const RESOURCE: &str = "http://localhost/mcp";

/// One signing key of the test issuer.
struct IssuerKey {
    kid: String,
    pkcs8: Vec<u8>,
    x: String,
    y: String,
}

impl IssuerKey {
    fn generate(kid: &str) -> Self {
        let random = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random)
            .expect("generate a P-256 key");
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &random)
                .expect("read the key back");
        // An uncompressed point: 0x04, then X and Y, 32 bytes each.
        let point = pair.public_key().as_ref();
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        Self {
            kid: kid.to_owned(),
            pkcs8: pkcs8.as_ref().to_vec(),
            x: encode(&point[1..33]),
            y: encode(&point[33..65]),
        }
    }

    fn jwk(&self) -> Value {
        json!({
            "kty": "EC",
            "crv": "P-256",
            "x": self.x,
            "y": self.y,
            "kid": self.kid,
            "alg": "ES256",
            "use": "sig",
        })
    }

    /// `claims` signed with this key, under `kid` and `typ`.
    fn sign_as(&self, kid: &str, typ: Option<&str>, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid.to_owned());
        header.typ = typ.map(str::to_owned);
        jsonwebtoken::encode(&header, claims, &EncodingKey::from_ec_der(&self.pkcs8))
            .expect("sign a token")
    }

    fn sign(&self, claims: &Value) -> String {
        self.sign_as(&self.kid, Some("at+jwt"), claims)
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The claims of a valid access token for `user` with `scope`.
fn claims(user: &str, scope: &str) -> Value {
    json!({
        "iss": ISSUER,
        "sub": user,
        "aud": RESOURCE,
        "iat": now(),
        "exp": now() + 600,
        "scope": scope,
        "client_id": "outside-client",
    })
}

/// `claims` with `field` set to `value`.
fn with(mut claims: Value, field: &str, value: Value) -> Value {
    claims[field] = value;
    claims
}

/// The issuer's key set as the fake serves it.
fn key_set(keys: &[&IssuerKey]) -> Value {
    json!({ "keys": keys.iter().map(|key| key.jwk()).collect::<Vec<_>>() })
}

struct World {
    client: TestClient,
    documents: Arc<FakeDocuments>,
    _container: Box<dyn std::any::Any>,
}

/// An application that accepts the test issuer's tokens, its key set
/// published with `keys`, the driver adjusted by `configure`.
fn world_with(
    keys: &[&IssuerKey],
    configure: impl FnOnce(OutsideIssuer) -> OutsideIssuer,
) -> World {
    let container = container();
    let resolver = Arc::new(FakeResolver::default());
    resolver.answer("login.issuer.test", "93.184.215.14");
    let documents = Arc::new(FakeDocuments::default());
    documents.serve(JWKS, &key_set(keys));
    let issuer = configure(
        OutsideIssuer::new(ISSUER, JWKS)
            .expect("a valid issuer")
            .key_fetch(resolver, documents.clone()),
    );
    let client = TestClient::new(
        crate::oauth::router(AgentsConfig::new().resolver(Arc::new(issuer))),
        MiddlewareRegistry::new(),
    );
    World {
        client,
        documents,
        _container: container,
    }
}

fn world(keys: &[&IssuerKey]) -> World {
    world_with(keys, |issuer| issuer)
}

/// The refusal a call with `token` gets: its status and its reason.
async fn refusal(client: &TestClient, token: &str) -> (u16, String) {
    let refused = client
        .mcp()
        .with_token(token)
        .call("users.profile", json!({}))
        .await;
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    let reason = www
        .split("error_description=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_owned();
    (refused.status(), reason)
}

#[tokio::test]
async fn agent_006_valid_outside_token_calls_a_user_tool() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("outside");
    let token = key.sign(&claims(&user, "posts:read posts:write"));
    let profile = world
        .client
        .mcp()
        .with_token(&token)
        .call("users.profile", json!({}))
        .await;
    assert_eq!(profile.status(), 200, "{:?}", profile.response());
    assert_eq!(
        profile.structured(),
        Some(&json!({ "user": user, "browser": false }))
    );
    // A tool that accepts users and agents sees a user, through the
    // client the token names, and never an agent registration.
    let created = world
        .client
        .mcp()
        .with_token(&token)
        .call("posts.create", json!({ "title": "From outside" }))
        .await;
    assert_eq!(created.status(), 200, "{:?}", created.response());
    let created = created.structured().cloned().unwrap_or_default();
    assert_eq!(created["kind"], "user");
    assert_eq!(created["user"], json!(user));
    assert_eq!(created["client"], "outside-client");
    assert_eq!(created["registration"], Value::Null);
    // A time within the leeway is still good.
    let late = key.sign(&with(claims(&user, "posts:read"), "exp", json!(now() - 30)));
    assert_eq!(
        world
            .client
            .mcp()
            .with_token(&late)
            .call("users.profile", json!({}))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn agent_006_wrong_issuer_is_refused() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("issuer");
    let token = key.sign(&with(
        claims(&user, "posts:read"),
        "iss",
        json!("https://other.issuer.test"),
    ));
    assert_eq!(
        refusal(&world.client, &token).await,
        (401, "The token is not recognized.".to_owned()),
        "another issuer's token is no resolver's"
    );
}

#[tokio::test]
async fn agent_006_wrong_audience_is_refused() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("audience");
    for audience in [
        json!("https://other.example/mcp"),
        json!(["https://other.example/mcp"]),
        Value::Null,
    ] {
        let token = key.sign(&with(claims(&user, "posts:read"), "aud", audience.clone()));
        assert_eq!(
            refusal(&world.client, &token).await,
            (401, "The token was issued for another resource.".to_owned()),
            "{audience}"
        );
    }
    let listed = key.sign(&with(
        claims(&user, "posts:read"),
        "aud",
        json!(["https://other.example/mcp", RESOURCE]),
    ));
    assert_eq!(refusal(&world.client, &listed).await.0, 200);
}

#[tokio::test]
async fn agent_006_expired_token_is_refused() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("expired");
    let expired = key.sign(&with(
        claims(&user, "posts:read"),
        "exp",
        json!(now() - 120),
    ));
    assert_eq!(
        refusal(&world.client, &expired).await,
        (401, "The token expired.".to_owned())
    );
    let mut no_expiry = claims(&user, "posts:read");
    no_expiry.as_object_mut().map(|claims| claims.remove("exp"));
    assert_eq!(
        refusal(&world.client, &key.sign(&no_expiry)).await,
        (401, "The token has no expiry.".to_owned())
    );
    let future = key.sign(&with(
        claims(&user, "posts:read"),
        "nbf",
        json!(now() + 600),
    ));
    assert_eq!(
        refusal(&world.client, &future).await,
        (401, "The token is not valid yet.".to_owned())
    );
}

#[tokio::test]
async fn agent_006_missing_scope_is_refused() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("scope");
    let token = key.sign(&claims(&user, "posts:read"));
    let refused = world
        .client
        .mcp()
        .with_token(&token)
        .call("posts.create", json!({ "title": "Not allowed" }))
        .await;
    assert_eq!(refused.status(), 403, "{:?}", refused.response());
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(www.contains("error=\"insufficient_scope\""), "{www}");
    // `scp`, a list, carries scopes too.
    let listed = key.sign(&with(
        with(claims(&user, ""), "scope", Value::Null),
        "scp",
        json!(["posts:write"]),
    ));
    let created = world
        .client
        .mcp()
        .with_token(&listed)
        .call("posts.create", json!({ "title": "Allowed" }))
        .await;
    assert_eq!(created.status(), 200, "{:?}", created.response());
}

#[tokio::test]
async fn agent_006_outside_token_is_never_an_agent() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("never-agent");
    // An identity assertion from the same issuer is not an access token.
    let assertion = key.sign_as("k1", Some("oauth-id-jag+jwt"), &claims(&user, "posts:read"));
    assert_eq!(
        refusal(&world.client, &assertion).await,
        (401, "The token is not an access token.".to_owned())
    );
    // A token the issuer signs is a user's: a user nobody here knows is
    // refused, never turned into an agent account.
    let stranger = key.sign(&claims("nobody-here", "posts:write"));
    assert_eq!(
        refusal(&world.client, &stranger).await,
        (401, "The token's user is not known here.".to_owned())
    );
    let token = key.sign(&claims(&user, "posts:write"));
    let created = world
        .client
        .mcp()
        .with_token(&token)
        .call("posts.create", json!({ "title": "x" }))
        .await;
    assert_eq!(
        created.structured().map(|value| value["kind"].clone()),
        Some(json!("user"))
    );
}

#[tokio::test]
async fn agent_006_signature_and_algorithm_are_checked() {
    let key = IssuerKey::generate("k1");
    let impostor = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let user = new_account("signature");
    let forged = impostor.sign(&claims(&user, "posts:read"));
    assert_eq!(
        refusal(&world.client, &forged).await,
        (401, "The token's signature does not verify.".to_owned())
    );
    let hmac = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims(&user, "posts:read"),
        &EncodingKey::from_secret(b"a secret nobody should share"),
    )
    .expect("sign with HMAC");
    assert_eq!(
        refusal(&world.client, &hmac).await,
        (
            401,
            "The token's signing algorithm is not accepted.".to_owned()
        )
    );
}

#[tokio::test]
async fn agent_006_unknown_key_is_fetched_for_once() {
    let first = IssuerKey::generate("k1");
    let second = IssuerKey::generate("k2");
    let world = world_with(&[&first], |issuer| {
        issuer.key_cache(Duration::from_secs(600), Duration::ZERO)
    });
    let user = new_account("rotation");
    let requests = || world.documents.requests.lock().unwrap().len();
    assert_eq!(
        refusal(&world.client, &first.sign(&claims(&user, "posts:read")))
            .await
            .0,
        200
    );
    assert_eq!(requests(), 1);
    // The issuer rotates: a token under the new key fetches the set again.
    world.documents.serve(JWKS, &key_set(&[&first, &second]));
    assert_eq!(
        refusal(&world.client, &second.sign(&claims(&user, "posts:read")))
            .await
            .0,
        200
    );
    assert_eq!(requests(), 2);
    assert_eq!(
        refusal(&world.client, &second.sign(&claims(&user, "posts:read")))
            .await
            .0,
        200
    );
    assert_eq!(requests(), 2, "a known key is served from the cache");
    // A key the issuer does not publish costs one fetch and is refused.
    let unknown = second.sign_as("k9", Some("at+jwt"), &claims(&user, "posts:read"));
    assert_eq!(
        refusal(&world.client, &unknown).await,
        (
            401,
            "The token's signing key is not one its issuer publishes.".to_owned()
        )
    );
    assert_eq!(requests(), 3);
}

#[tokio::test]
async fn agent_006_made_up_keys_cannot_force_a_fetch_each_time() {
    let first = IssuerKey::generate("k1");
    let world = world(&[&first]);
    let user = new_account("made-up");
    assert_eq!(
        refusal(&world.client, &first.sign(&claims(&user, "posts:read")))
            .await
            .0,
        200
    );
    for kid in ["k7", "k8", "k9"] {
        let made_up = first.sign_as(kid, Some("at+jwt"), &claims(&user, "posts:read"));
        assert_eq!(refusal(&world.client, &made_up).await.0, 401);
    }
    assert_eq!(
        world.documents.requests.lock().unwrap().len(),
        1,
        "within the refetch interval the cached set answers"
    );
}

#[tokio::test]
async fn agent_006_key_set_on_a_private_address_is_not_fetched() {
    let key = IssuerKey::generate("k1");
    let container = container();
    let resolver = Arc::new(FakeResolver::default());
    resolver.answer("login.issuer.test", "10.0.0.9");
    let documents = Arc::new(FakeDocuments::default());
    documents.serve(JWKS, &key_set(&[&key]));
    let issuer = OutsideIssuer::new(ISSUER, JWKS)
        .expect("a valid issuer")
        .key_fetch(resolver, documents.clone());
    let client = TestClient::new(
        crate::oauth::router(AgentsConfig::new().resolver(Arc::new(issuer))),
        MiddlewareRegistry::new(),
    );
    let user = new_account("private-keys");
    let (status, _) = refusal(&client, &key.sign(&claims(&user, "posts:read"))).await;
    assert_eq!(
        status, 503,
        "the token cannot be checked, so it is not served"
    );
    assert!(documents.requests.lock().unwrap().is_empty());
    drop(container);
}

#[tokio::test]
async fn agent_006_issuer_is_listed_in_protected_resource_metadata() {
    let key = IssuerKey::generate("k1");
    let world = world(&[&key]);
    let document = world
        .client
        .get("/.well-known/oauth-protected-resource/mcp")
        .send()
        .await
        .json();
    assert_eq!(document["authorization_servers"], json!([ISSUER]));
}
