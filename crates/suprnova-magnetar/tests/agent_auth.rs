//! AGENT-007 to AGENT-011: the agent auth plugin on SQLite.
//!
//! Every test drives the mounted routes the way an agent, a provider and
//! the user's browser would, against the bodies and codes of auth.md
//! v0.6.0, and checks tokens through the verification API a resource
//! server calls. The live PostgreSQL and MySQL runs are in
//! `agent_auth_live.rs`.

#![cfg(all(feature = "agent-auth", feature = "seaorm-sqlite"))]

#[path = "fixtures/agent_auth_harness.rs"]
mod harness;

use std::sync::Arc;

use harness::*;
use magnetar::agent_auth::sql::registrations;
use magnetar::agent_auth::{IdentityTypes, RefuseProvisioning, RegistrationLimit};
use magnetar::authorization_server::{AccessVerification, InvalidAccessReason};
use magnetar::plugin::LifecycleEventKind;
use serde_json::{Value, json};

const REVOKED_EVENT: &str =
    "https://schemas.workos.com/events/agent/auth/identity/assertion/revoked";

fn agent_kind(verification: &AccessVerification) -> (String, String, Option<String>, Vec<String>) {
    match verification {
        AccessVerification::Valid(access) => (
            access.grant_kind.clone(),
            access.token_kind.clone(),
            access.user_id.clone(),
            access.scopes.clone(),
        ),
        other => panic!("the token must verify, got {other:?}"),
    }
}

fn revoked() -> AccessVerification {
    AccessVerification::Invalid(InvalidAccessReason::Revoked)
}

fn access_token(response: &magnetar::plugin::WireResponse) -> String {
    assert_eq!(status(response), 200, "{:?}", body(response));
    body(response)["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned()
}

/// Present a fresh ID-JAG for `subject` with `email` verified.
async fn present_subject(
    h: &Harness,
    subject: &str,
    email: &str,
) -> magnetar::plugin::WireResponse {
    let claims = h.provider.id_jag_claims(subject, email);
    h.present(&h.provider.id_jag(&claims)).await
}

// AGENT-007 -----------------------------------------------------------------

#[tokio::test]
async fn agent_007_anonymous_registration_answers_the_v060_body_and_a_working_agent_token() {
    let h = sqlite_harness().await;
    let response = h
        .post_json("/agent/identity", json!({"type": "anonymous"}))
        .await;
    assert_eq!(status(&response), 200, "{:?}", body(&response));
    assert_eq!(
        header(&response, "cache-control").as_deref(),
        Some("no-store")
    );
    let registration = body(&response);
    let id = registration["registration_id"].as_str().expect("an id");
    assert!(id.starts_with("reg_"), "{id}");
    assert_eq!(registration["registration_type"], "anonymous");
    assert_eq!(registration["pre_claim_scopes"], json!(["tools:read"]));
    assert_eq!(
        registration["post_claim_scopes"],
        json!(["tools:read", "tools:write"])
    );
    assert_eq!(
        registration["claim_url"],
        "https://auth.example.test/agent/identity/claim"
    );
    assert!(
        registration["claim_token"]
            .as_str()
            .is_some_and(|token| token.starts_with("clm_"))
    );
    for field in ["assertion_expires", "claim_token_expires"] {
        let value = registration[field].as_str().expect(field);
        assert!(
            chrono::DateTime::parse_from_rfc3339(value).is_ok() && value.ends_with('Z'),
            "{field}: {value}"
        );
    }
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("an identity assertion");
    let header_json = jwt_header(assertion);
    assert_eq!(header_json["typ"], "oauth-id-jag+jwt");
    assert_eq!(header_json["alg"], "ES256");
    assert_eq!(header_json["kid"], "service-key-1");
    let claims = jwt_claims(assertion);
    assert_eq!(claims["sub"], id, "sub is the registration");
    assert_eq!(claims["iss"], ISSUER);
    assert!(
        claims.get("email").is_none(),
        "a pre-claim assertion names no user"
    );

    let tokens = h.exchange(assertion).await;
    assert_eq!(
        header(&tokens, "cache-control").as_deref(),
        Some("no-store")
    );
    let tokens_body = body(&tokens);
    assert!(
        tokens_body.get("refresh_token").is_none(),
        "{tokens_body:?}"
    );
    assert_eq!(tokens_body["token_type"], "Bearer");
    assert_eq!(tokens_body["scope"], "tools:read");
    let token = access_token(&tokens);
    let verification = h.verify(&token).await;
    let (grant_kind, token_kind, user, scopes) = agent_kind(&verification);
    assert_eq!(
        (grant_kind.as_str(), token_kind.as_str()),
        ("agent", "agent")
    );
    assert_eq!(user, None);
    assert_eq!(scopes, vec!["tools:read".to_owned()]);
    match verification {
        AccessVerification::Valid(access) => assert_eq!(access.client_id, id),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn agent_007_service_auth_answers_the_claim_block_and_no_assertion() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("Person@Example.test").await;
    assert_eq!(registration["registration_type"], "service_auth");
    assert!(registration.get("identity_assertion").is_none());
    assert!(registration["claim_token"].as_str().is_some());
    assert_eq!(
        registration["post_claim_scopes"],
        json!(["tools:read", "tools:write"])
    );
    let claim = &registration["claim"];
    let code = claim["user_code"].as_str().expect("a user code");
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
    assert_eq!(claim["expires_in"], 600);
    assert_eq!(claim["interval"], 5);
    let uri = claim["verification_uri"].as_str().expect("a uri");
    assert!(
        uri.starts_with("https://auth.example.test/login?return_to="),
        "{uri}"
    );
    assert!(!uri.contains(code), "the URL never carries the user code");
    let return_to = query_param(uri, "return_to").expect("return_to");
    assert!(
        return_to.starts_with("/agent/claim?claim_attempt_token="),
        "{return_to}"
    );
}

#[tokio::test]
async fn agent_009_verification_uri_is_the_claim_page_when_the_host_asks() {
    let mut options = Options::new();
    options.config.direct_claim_page = true;
    let h = sqlite_harness_with(options).await;
    let registration = h.service_auth("Person@Example.test").await;
    let uri = registration["claim"]["verification_uri"]
        .as_str()
        .expect("a uri");
    assert!(
        uri.starts_with("https://auth.example.test/agent/claim?claim_attempt_token=cat_"),
        "the user opens the claim page itself: {uri}"
    );
    assert!(query_param(uri, "return_to").is_none(), "{uri}");
    let attempt = query_param(uri, "claim_attempt_token").expect("the attempt token");
    let user = h.create_user("person@example.test", true).await;
    let session = h.sign_in(&user).await;
    let pending = h.pending(&attempt, Some(&session)).await;
    assert_eq!(status(&pending), 200, "{:?}", body(&pending));

    // A restarted attempt points at the claim page as well.
    let claim_token = registration["claim_token"].as_str().expect("a claim token");
    let started = h.initiate(claim_token, "person@example.test").await;
    let restarted = body(&started)["claim_attempt"]["verification_uri"]
        .as_str()
        .expect("a uri")
        .to_owned();
    assert!(
        restarted.starts_with("https://auth.example.test/agent/claim?claim_attempt_token=cat_"),
        "{restarted}"
    );
}

#[test]
fn agent_007_generated_signing_key_reads_back_from_its_pem() {
    use secrecy::ExposeSecret;
    let pem = magnetar::agent_auth::SigningKey::generate_pem().expect("a new key");
    assert!(
        pem.expose_secret()
            .starts_with("-----BEGIN PRIVATE KEY-----"),
        "PKCS#8 PEM"
    );
    let key = magnetar::agent_auth::SigningKey::from_pem("generated-1", pem.expose_secret())
        .expect("the PEM reads back as a P-256 signing key");
    assert_eq!(key.kid(), "generated-1");
    let other = magnetar::agent_auth::SigningKey::generate_pem().expect("a second key");
    assert_ne!(
        pem.expose_secret(),
        other.expose_secret(),
        "every call draws a new key"
    );
    assert!(
        !format!("{pem:?}").contains("PRIVATE"),
        "the PEM never prints"
    );
}

#[tokio::test]
async fn agent_007_identity_assertion_registers_and_its_token_carries_the_agent_kind() {
    let h = sqlite_harness().await;
    let response = present_subject(&h, "provider-user-1", "new.person@example.test").await;
    assert_eq!(status(&response), 200, "{:?}", body(&response));
    let registration = body(&response);
    assert_eq!(registration["registration_type"], "identity_assertion");
    assert_eq!(registration["scopes"], json!(["tools:read", "tools:write"]));
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("an assertion");
    assert_eq!(jwt_claims(assertion)["email"], "new.person@example.test");
    let token = access_token(&h.exchange(assertion).await);
    let (grant_kind, token_kind, user, scopes) = agent_kind(&h.verify(&token).await);
    assert_eq!(
        (grant_kind.as_str(), token_kind.as_str()),
        ("agent", "agent")
    );
    assert!(user.is_some(), "the provisioned user is bound");
    assert_eq!(
        scopes,
        vec!["tools:read".to_owned(), "tools:write".to_owned()]
    );
}

#[tokio::test]
async fn agent_007_disabled_types_answer_not_enabled_and_are_not_advertised() {
    let mut options = Options::new();
    options.config.identity_types = IdentityTypes {
        anonymous: false,
        service_auth: false,
        identity_assertion: true,
    };
    let h = sqlite_harness_with(options).await;
    let anonymous = h
        .post_json("/agent/identity", json!({"type": "anonymous"}))
        .await;
    assert_eq!(status(&anonymous), 400);
    assert_eq!(body(&anonymous)["error"], "anonymous_not_enabled");
    let service = h
        .post_json(
            "/agent/identity",
            json!({"type": "service_auth", "login_hint": "a@example.test"}),
        )
        .await;
    assert_eq!(body(&service)["error"], "service_auth_not_enabled");
    let block = &h.server.metadata_document()["agent_auth"];
    assert_eq!(
        block["identity_types_supported"],
        json!(["identity_assertion"])
    );

    let mut options = Options::new();
    options.config.identity_types = IdentityTypes {
        anonymous: true,
        service_auth: false,
        identity_assertion: false,
    };
    let h = sqlite_harness_with(options).await;
    let claims = h.provider.id_jag_claims("someone", "someone@example.test");
    let assertion = h.present(&h.provider.id_jag(&claims)).await;
    assert_eq!(status(&assertion), 400);
    assert_eq!(body(&assertion)["error"], "issuer_not_enabled");
    let block = &h.server.metadata_document()["agent_auth"];
    assert_eq!(block["identity_types_supported"], json!(["anonymous"]));
    assert!(block.get("events_endpoint").is_none());
    assert!(block.get("identity_assertion").is_none());
}

#[tokio::test]
async fn agent_007_metadata_carries_the_agent_auth_block_and_both_grants() {
    let h = sqlite_harness().await;
    let metadata = h.server.metadata_document();
    let grants = metadata["grant_types_supported"]
        .as_array()
        .expect("grant types");
    for grant in [
        "urn:ietf:params:oauth:grant-type:jwt-bearer",
        "urn:workos:agent-auth:grant-type:claim",
    ] {
        assert!(grants.iter().any(|value| value == grant), "{grant}");
    }
    let block = &metadata["agent_auth"];
    assert_eq!(block["skill"], "https://auth.example.test/auth.md");
    assert_eq!(
        block["identity_endpoint"],
        "https://auth.example.test/agent/identity"
    );
    assert_eq!(
        block["claim_endpoint"],
        "https://auth.example.test/agent/identity/claim"
    );
    assert_eq!(
        block["events_endpoint"],
        "https://auth.example.test/agent/event/notify"
    );
    assert_eq!(
        block["identity_types_supported"],
        json!(["anonymous", "identity_assertion", "service_auth"])
    );
    assert_eq!(
        block["identity_assertion"]["assertion_types_supported"],
        json!(["urn:ietf:params:oauth:token-type:id-jag"])
    );
    assert_eq!(block["events_supported"], json!([REVOKED_EVENT]));
}

#[tokio::test]
async fn agent_007_malformed_requests_answer_the_protocol_codes() {
    let h = sqlite_harness().await;
    for (request, code) in [
        (json!("anonymous"), "invalid_request"),
        (json!({}), "invalid_request"),
        (json!({"type": "magic"}), "invalid_request"),
        (json!({"type": "service_auth"}), "invalid_request"),
        (
            json!({"type": "service_auth", "login_hint": "not an email"}),
            "invalid_request",
        ),
        (
            json!({"type": "identity_assertion", "assertion_type": "verified_email", "assertion": "a@example.test"}),
            "invalid_request",
        ),
        (
            json!({"type": "identity_assertion", "assertion_type": "urn:ietf:params:oauth:token-type:id-jag", "assertion": "not.a.jwt"}),
            "invalid_request",
        ),
    ] {
        let response = h.post_json("/agent/identity", request.clone()).await;
        assert_eq!(status(&response), 400, "{request}");
        let answer = body(&response);
        assert_eq!(answer["error"], code, "{request}: {answer:?}");
        assert!(answer["error_description"].as_str().is_some(), "{answer:?}");
    }
    let unsupported = h
        .post_form("/oauth/token", &[("grant_type", "urn:example:other")], None)
        .await;
    assert_eq!(body(&unsupported)["error"], "unsupported_grant_type");
    let missing = h
        .post_form(
            "/oauth/token",
            &[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")],
            None,
        )
        .await;
    assert_eq!(body(&missing)["error"], "invalid_request");
    let forged = h
        .exchange(
            &h.provider
                .id_jag(&h.provider.id_jag_claims("x", "x@example.test")),
        )
        .await;
    assert_eq!(status(&forged), 400);
    assert_eq!(
        body(&forged)["error"],
        "invalid_grant",
        "a provider token is no service assertion"
    );
}

#[tokio::test]
async fn agent_007_agent_grants_never_issue_a_refresh_token() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    for _ in 0..2 {
        let tokens = body(&h.exchange(assertion).await);
        assert!(tokens.get("refresh_token").is_none(), "{tokens:?}");
        assert!(tokens["access_token"].as_str().is_some());
    }
    let (registration, _) = h.claimed_anonymous("refresh@example.test").await;
    h.forget_polls().await;
    let claimed = h
        .poll(registration["claim_token"].as_str().expect("claim token"))
        .await;
    assert_eq!(status(&claimed), 200, "{:?}", body(&claimed));
    assert!(body(&claimed).get("refresh_token").is_none());
}

#[tokio::test]
async fn agent_007_an_unclaimed_anonymous_registration_gets_only_the_pre_claim_scopes() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let wider = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion),
                ("scope", "tools:read tools:write"),
            ],
            None,
        )
        .await;
    assert_eq!(status(&wider), 400);
    assert_eq!(body(&wider)["error"], "invalid_scope");
    let token = access_token(&h.exchange(assertion).await);
    let (_, _, _, scopes) = agent_kind(&h.verify(&token).await);
    assert_eq!(scopes, vec!["tools:read".to_owned()]);
}

#[tokio::test]
async fn agent_007_assertions_signed_before_a_key_rotation_still_verify() {
    let db = magnetar::default_schema::database().await;
    let (old_key, old_pem) = signing_key("key-2026-01");
    let mut options = Options::new();
    options.config = agent_config(old_key.clone());
    let before = build(db.clone(), options).await;
    let registration = before.anonymous().await;
    let old_assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion")
        .to_owned();

    let (new_key, _) = signing_key("key-2026-07");
    let mut options = Options::new();
    let mut config = agent_config(new_key.clone());
    config.previous_signing_keys = vec![magnetar::agent_auth::PreviousSigningKey::new(
        magnetar::agent_auth::SigningKey::from_pem("key-2026-01", &old_pem).expect("old key"),
        chrono::Utc::now(),
    )];
    options.config = config;
    let after = build(db.clone(), options).await;
    let exchanged = after.exchange(&old_assertion).await;
    assert_eq!(status(&exchanged), 200, "{:?}", body(&exchanged));
    let fresh = after.anonymous().await;
    assert_eq!(
        jwt_header(fresh["identity_assertion"].as_str().expect("assertion"))["kid"],
        "key-2026-07",
        "new assertions are signed with the new key"
    );

    let mut options = Options::new();
    options.config = agent_config(new_key);
    let retired = build(db, options).await;
    let refused = retired.exchange(&old_assertion).await;
    assert_eq!(status(&refused), 400);
    assert_eq!(body(&refused)["error"], "invalid_grant");
}

#[tokio::test]
async fn agent_007_narrowed_post_claim_scopes_apply_to_a_bound_registration() {
    let db = magnetar::default_schema::database().await;
    let (key, _) = signing_key("narrow-key");
    let mut options = Options::new();
    options.config = agent_config(key.clone());
    let wide = build(db.clone(), options).await;
    let (registration, _) = wide.claimed_anonymous("narrow@example.test").await;
    let claim_token = registration["claim_token"].as_str().expect("a claim token");

    // The operator narrows the post-claim scopes and restarts.
    let mut options = Options::new();
    let mut config = agent_config(key);
    config.post_claim_scopes = vec!["tools:read".to_owned()];
    options.config = config;
    let narrow = build(db, options).await;
    let polled = narrow.poll(claim_token).await;
    assert_eq!(status(&polled), 200, "{:?}", body(&polled));
    let polled = body(&polled);
    assert_eq!(polled["scope"], "tools:read", "the claim poll is cut");
    let assertion = polled["identity_assertion"].as_str().expect("an assertion");
    let wider = narrow
        .exchange_with(
            assertion,
            &[("scope", "tools:write"), ("resource", RESOURCE)],
        )
        .await;
    assert_eq!(status(&wider), 400, "{:?}", body(&wider));
    assert_eq!(body(&wider)["error"], "invalid_scope");
    let token = access_token(&narrow.exchange(assertion).await);
    let (_, _, _, scopes) = agent_kind(&narrow.verify(&token).await);
    assert_eq!(scopes, vec!["tools:read".to_owned()]);
}

/// A directory in which every verified address matches more than one
/// account.
struct AmbiguousDirectory;

#[async_trait::async_trait]
impl magnetar::agent_auth::AgentUserDirectory for AmbiguousDirectory {
    async fn find_by_id(
        &self,
        _user_id: &str,
    ) -> magnetar::Result<Option<magnetar::agent_auth::AgentUser>> {
        Ok(None)
    }

    async fn find_by_verified_email(
        &self,
        _email: &str,
    ) -> magnetar::Result<Option<magnetar::agent_auth::AgentUser>> {
        Err(magnetar::Error::Conflict {
            resource: "user".to_owned(),
            message: "the address matches more than one account".to_owned(),
        })
    }

    async fn find_by_verified_phone(
        &self,
        _phone: &str,
    ) -> magnetar::Result<Option<magnetar::agent_auth::AgentUser>> {
        Ok(None)
    }
}

/// A provisioning policy that counts what it is asked and refuses.
#[derive(Default)]
struct CountingPolicy(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl magnetar::agent_auth::ProvisioningPolicy for CountingPolicy {
    async fn provision(
        &self,
        _identity: &magnetar::agent_auth::ProvidedIdentity,
    ) -> magnetar::Result<magnetar::agent_auth::ProvisioningDecision> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(magnetar::agent_auth::ProvisioningDecision::Refused)
    }
}

#[tokio::test]
async fn agent_008_address_matching_several_accounts_is_refused() {
    let policy = Arc::new(CountingPolicy::default());
    let mut options = Options::new();
    options.users = Some(Arc::new(AmbiguousDirectory));
    options.provisioning = Some(policy.clone());
    let h = sqlite_harness_with(options).await;
    let response = present_subject(&h, "ambiguous-subject", "shared@example.test").await;
    assert_eq!(status(&response), 400, "{:?}", body(&response));
    let refusal = body(&response);
    assert_eq!(refusal["error"], "invalid_request");
    assert!(
        refusal["error_description"]
            .as_str()
            .is_some_and(|text| text.contains("more than one account")),
        "{refusal}"
    );
    assert_eq!(
        policy.0.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "never provisioned"
    );
}

// The claim event's outbox (AGENT-009) ---------------------------------------

#[tokio::test]
async fn agent_009_confirmed_claim_writes_one_outbox_event_and_delivery_marks_it() {
    let h = sqlite_harness().await;
    let (registration, user_id) = h.claimed_anonymous("outbox@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let outbox = h.outbox_of(id).await;
    assert_eq!(outbox.len(), 1, "exactly one event for the claim");
    let row = &outbox[0];
    assert_eq!(row.kind, "claim.confirmed");
    assert_eq!(row.user_id, user_id);
    let attributes: Value = serde_json::from_str(&row.attributes).expect("JSON attributes");
    assert_eq!(attributes["registration_type"], "anonymous");
    assert_eq!(attributes["claimed_by_user_id"], user_id.as_str());
    assert!(
        row.delivered_at.is_some(),
        "delivered right after the commit"
    );
    let delivered = h.events.of(LifecycleEventKind::AgentClaimConfirmed);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].mutation_id, row.id, "the stable id");
}

#[tokio::test]
async fn agent_009_claim_whose_transaction_fails_leaves_no_outbox_event() {
    use sea_orm::ConnectionTrait;
    let h = sqlite_harness().await;
    let email = "rolled-back@example.test";
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let user_id = h.create_user(email, true).await;
    let bearer = h.sign_in(&user_id).await;
    let started = body(
        &h.initiate(
            registration["claim_token"].as_str().expect("claim token"),
            email,
        )
        .await,
    );
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code")
        .to_owned();
    // The outbox write fails inside the claim's transaction.
    h.db.execute_unprepared("DROP TABLE auth_agent_events")
        .await
        .expect("drop the outbox");
    let failed = h.confirm(&attempt, &code, Some(&bearer)).await;
    assert_eq!(status(&failed), 500, "{:?}", body(&failed));
    let unclaimed = h.registration(id).await;
    assert_eq!(
        unclaimed.status, "unclaimed",
        "the claim rolled back with it"
    );
    assert!(unclaimed.user_id.is_none());
    magnetar::agent_auth::migrate(&h.db)
        .await
        .expect("recreate the outbox");
    assert!(
        h.outbox_of(id).await.is_empty(),
        "no event without its claim"
    );
    let confirmed = h.confirm(&attempt, &code, Some(&bearer)).await;
    assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
    assert_eq!(h.outbox_of(id).await.len(), 1);
}

#[tokio::test]
async fn agent_009_undelivered_claim_event_is_delivered_by_the_pass() {
    let h = sqlite_harness().await;
    h.events.refuse(LifecycleEventKind::AgentClaimConfirmed, 1);
    let (registration, _) = h.claimed_anonymous("retry@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let outbox = h.outbox_of(id).await;
    assert_eq!(outbox.len(), 1);
    assert!(outbox[0].delivered_at.is_none(), "the refused event waits");
    assert!(
        h.events
            .of(LifecycleEventKind::AgentClaimConfirmed)
            .is_empty()
    );
    assert_eq!(h.engine.deliver_pending_events().await.expect("a pass"), 1);
    let delivered = h.events.of(LifecycleEventKind::AgentClaimConfirmed);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].mutation_id, outbox[0].id, "the same event id");
    assert!(h.outbox_of(id).await[0].delivered_at.is_some());
    assert_eq!(
        h.engine.deliver_pending_events().await.expect("a pass"),
        0,
        "nothing is delivered twice by the passes"
    );
}

#[tokio::test]
async fn agent_009_two_concurrent_passes_deliver_an_event_once() {
    let db = magnetar::default_schema::database().await;
    let first = build(db.clone(), Options::new()).await;
    let second = build(db, Options::new()).await;
    first
        .events
        .refuse(LifecycleEventKind::AgentClaimConfirmed, 1);
    let (registration, _) = first.claimed_anonymous("twice@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let event_id = first.outbox_of(id).await[0].id.clone();
    let (left, right) = tokio::join!(
        first.engine.deliver_pending_events(),
        second.engine.deliver_pending_events()
    );
    assert_eq!(left.expect("a pass") + right.expect("a pass"), 1);
    let deliveries = [&first, &second]
        .iter()
        .map(|node| {
            node.events
                .of(LifecycleEventKind::AgentClaimConfirmed)
                .iter()
                .filter(|event| event.mutation_id == event_id)
                .count()
        })
        .sum::<usize>();
    assert_eq!(deliveries, 1, "one delivery across both nodes");
}

#[tokio::test]
async fn agent_009_delivered_events_are_pruned_after_the_retention() {
    use magnetar::agent_auth::sql::agent_events;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let h = sqlite_harness().await;
    let (old, _) = h.claimed_anonymous("old@example.test").await;
    h.events.refuse(LifecycleEventKind::AgentClaimConfirmed, 1);
    let (waiting, _) = h.claimed_anonymous("waiting@example.test").await;
    let old_id = old["registration_id"].as_str().expect("id");
    let waiting_id = waiting["registration_id"].as_str().expect("id");
    let eight_days_ago = chrono::Utc::now() - chrono::Duration::days(8);
    agent_events::Entity::update_many()
        .col_expr(
            agent_events::Column::DeliveredAt,
            Expr::value(eight_days_ago),
        )
        .col_expr(agent_events::Column::CreatedAt, Expr::value(eight_days_ago))
        .filter(agent_events::Column::RegistrationId.eq(old_id.to_owned()))
        .exec(&h.db)
        .await
        .expect("age the delivered event");
    agent_events::Entity::update_many()
        .col_expr(agent_events::Column::CreatedAt, Expr::value(eight_days_ago))
        .filter(agent_events::Column::RegistrationId.eq(waiting_id.to_owned()))
        .exec(&h.db)
        .await
        .expect("age the waiting event");
    assert_eq!(h.engine.prune_delivered_events().await.expect("prune"), 1);
    assert!(h.outbox_of(old_id).await.is_empty());
    assert_eq!(
        h.outbox_of(waiting_id).await.len(),
        1,
        "an undelivered event is never pruned"
    );
}

/// Every authorization server grant issued to a registration, as
/// `(grant id, revoked)`.
async fn grants_of(h: &Harness, registration_id: &str) -> Vec<(String, bool)> {
    use magnetar::authorization_server::sql::grants;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    grants::Entity::find()
        .filter(grants::Column::ClientId.eq(registration_id.to_owned()))
        .all(&h.db)
        .await
        .expect("read grants")
        .into_iter()
        .map(|grant| (grant.id, grant.revoked_at.is_some()))
        .collect()
}

#[tokio::test]
async fn agent_007_many_exchanges_leave_one_live_grant() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let mut tokens = Vec::new();
    for _ in 0..5 {
        tokens.push(access_token(&h.exchange(assertion).await));
    }
    assert_eq!(
        grants_of(&h, id).await.len(),
        1,
        "one grant, however many exchanges"
    );
    for token in &tokens {
        assert!(matches!(
            h.verify(token).await,
            AccessVerification::Valid(_)
        ));
    }

    // The claim ends the pre-claim grant; the poll's grant replaces it.
    let user_id = h.create_user("one.grant@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let started = body(&h.initiate(claim_token, "one.grant@example.test").await);
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code");
    assert_eq!(status(&h.confirm(&attempt, code, Some(&bearer)).await), 200);
    let claimed = body(&h.poll(claim_token).await);
    let v2 = claimed["identity_assertion"].as_str().expect("v2");
    for _ in 0..3 {
        access_token(&h.exchange(v2).await);
    }
    let grants = grants_of(&h, id).await;
    assert_eq!(grants.len(), 2, "{grants:?}");
    assert_eq!(
        grants.iter().filter(|(_, revoked)| !revoked).count(),
        1,
        "{grants:?}"
    );
}

fn two_resources() -> Options {
    let mut options = Options::new();
    let mut server = server_config();
    server.resources.push(SECOND_RESOURCE.to_owned());
    options.server_config = Some(server);
    options
}

#[tokio::test]
async fn agent_007_tokens_for_two_resources_are_both_valid() {
    let h = sqlite_harness_with(two_resources()).await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let mut first = Vec::new();
    let mut second = Vec::new();
    for _ in 0..3 {
        first.push(access_token(
            &h.exchange_with(assertion, &[("resource", RESOURCE)]).await,
        ));
        second.push(access_token(
            &h.exchange_with(assertion, &[("resource", SECOND_RESOURCE)])
                .await,
        ));
    }
    for token in &first {
        assert!(
            matches!(
                h.verify_for(token, RESOURCE).await,
                AccessVerification::Valid(_)
            ),
            "a token for the first resource survives exchanges for the second"
        );
    }
    for token in &second {
        assert!(matches!(
            h.verify_for(token, SECOND_RESOURCE).await,
            AccessVerification::Valid(_)
        ));
    }
    let grants = grants_of(&h, id).await;
    assert_eq!(grants.len(), 2, "one grant per resource: {grants:?}");
    assert!(grants.iter().all(|(_, revoked)| !revoked));
}

#[tokio::test]
async fn agent_007_a_bad_resource_leaves_the_live_token_valid() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let token = access_token(&h.exchange(assertion).await);
    let refused = h
        .exchange_with(
            assertion,
            &[("resource", "https://elsewhere.example.test/mcp")],
        )
        .await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["error"], "invalid_target");
    assert!(
        matches!(h.verify(&token).await, AccessVerification::Valid(_)),
        "a failed request revokes nothing"
    );
    assert_eq!(
        grants_of(&h, id).await,
        vec![(grants_of(&h, id).await[0].0.clone(), false)]
    );
}

#[tokio::test]
async fn agent_007_an_out_of_entitlement_scope_is_refused_without_revoking() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let token = access_token(&h.exchange(assertion).await);
    let refused = h
        .exchange_with(assertion, &[("scope", "tools:write")])
        .await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["error"], "invalid_scope");
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
}

#[tokio::test]
async fn agent_007_a_narrower_request_gets_a_narrower_token_on_the_same_grant() {
    let h = sqlite_harness().await;
    let (registration, _) = h.claimed_anonymous("narrow@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let claimed = body(
        &h.poll(registration["claim_token"].as_str().expect("claim token"))
            .await,
    );
    let v2 = claimed["identity_assertion"].as_str().expect("v2");
    let narrow = h.exchange_with(v2, &[("scope", "tools:read")]).await;
    assert_eq!(body(&narrow)["scope"], "tools:read");
    let (_, _, _, scopes) = agent_kind(&h.verify(&access_token(&narrow)).await);
    assert_eq!(scopes, vec!["tools:read".to_owned()]);
    let full = h.exchange(v2).await;
    assert_eq!(body(&full)["scope"], "tools:read tools:write");
    let live: Vec<_> = grants_of(&h, id)
        .await
        .into_iter()
        .filter(|(_, revoked)| !revoked)
        .collect();
    assert_eq!(live.len(), 1, "the narrower request used the same grant");
}

#[tokio::test]
async fn agent_007_exchanges_are_rate_limited_per_registration() {
    let mut options = Options::new();
    options.config.limits.exchange = magnetar::abuse::AbusePolicy {
        max_requests: 3,
        window: std::time::Duration::from_secs(3600),
    };
    let h = sqlite_harness_with(options).await;
    let first = h.anonymous().await;
    let assertion = first["identity_assertion"].as_str().expect("assertion");
    for _ in 0..3 {
        access_token(&h.exchange(assertion).await);
    }
    let limited = h.exchange(assertion).await;
    assert_eq!(status(&limited), 429, "{:?}", body(&limited));
    assert_eq!(body(&limited)["error"], "rate_limited");
    let second = h.anonymous().await;
    access_token(
        &h.exchange(second["identity_assertion"].as_str().expect("assertion"))
            .await,
    );

    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    access_token(
        &h.exchange(
            registration["identity_assertion"]
                .as_str()
                .expect("assertion"),
        )
        .await,
    );
    let acquired = h.limiter.acquired.lock().clone();
    let (_, policy) = acquired
        .iter()
        .find(|(key, _)| key.starts_with("agent-auth.exchange:"))
        .expect("an exchange budget");
    assert_eq!((policy.max_requests, policy.window.as_secs()), (60, 3600));
}

#[tokio::test]
async fn agent_007_a_pruned_grant_is_replaced_on_the_next_exchange() {
    use magnetar::authorization_server::sql::{grants, tokens};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    access_token(&h.exchange(assertion).await);
    let (grant_id, _) = grants_of(&h, id).await.remove(0);
    tokens::Entity::delete_many()
        .filter(tokens::Column::GrantId.eq(grant_id.clone()))
        .exec(&h.db)
        .await
        .expect("drop the tokens");
    grants::Entity::delete_many()
        .filter(grants::Column::Id.eq(grant_id))
        .exec(&h.db)
        .await
        .expect("drop the grant, as pruning does");
    let token = access_token(&h.exchange(assertion).await);
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
    assert_eq!(grants_of(&h, id).await.len(), 1);
}

#[tokio::test]
async fn agent_007_a_previous_key_retires() {
    use magnetar::agent_auth::{PreviousSigningKey, SigningKey};
    let db = magnetar::default_schema::database().await;
    let (old_key, old_pem) = signing_key("retiring-key");
    let mut options = Options::new();
    options.config = agent_config(old_key);
    let before = build(db.clone(), options).await;
    let old_assertion = before.anonymous().await["identity_assertion"]
        .as_str()
        .expect("assertion")
        .to_owned();
    let rotated = |rotated_out_at: chrono::DateTime<chrono::Utc>,
                   until: Option<chrono::DateTime<chrono::Utc>>| {
        let mut previous = PreviousSigningKey::new(
            SigningKey::from_pem("retiring-key", &old_pem).expect("old key"),
            rotated_out_at,
        );
        if let Some(until) = until {
            previous = previous.verifying_until(until);
        }
        let mut options = Options::new();
        options.config.previous_signing_keys = vec![previous];
        options
    };
    let now = chrono::Utc::now();
    let fresh = build(db.clone(), rotated(now, None)).await;
    assert_eq!(status(&fresh.exchange(&old_assertion).await), 200);
    // Rotated out more than the longest assertion lifetime (30 days) ago:
    // retired, even for an assertion that has not expired.
    let retired = build(db.clone(), rotated(now - chrono::Duration::days(31), None)).await;
    let refused = retired.exchange(&old_assertion).await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["error"], "invalid_grant");
    let explicit = build(db, rotated(now, Some(now - chrono::Duration::seconds(1)))).await;
    assert_eq!(
        body(&explicit.exchange(&old_assertion).await)["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn agent_007_a_failed_grant_issue_leaves_no_unrecorded_grant() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    faults.fail("create_grant", 1);
    let failed = h.exchange(assertion).await;
    assert_eq!(status(&failed), 500, "{:?}", body(&failed));
    assert!(
        grants_of(&h, id).await.is_empty(),
        "no grant exists that the registration does not name"
    );
    assert_eq!(
        h.agent_grant_ids(id).await.len(),
        1,
        "the grant was recorded before it was issued"
    );
    let token = access_token(&h.exchange(assertion).await);
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
    assert_eq!(grants_of(&h, id).await.len(), 1);
}

/// An authorization server over `h`'s database with its own scopes and
/// resources, standing in for the same application after a configuration
/// change.
fn reconfigured_server(
    h: &Harness,
    scopes: Vec<magnetar::authorization_server::ScopeDefinition>,
    resources: Vec<String>,
) -> magnetar::authorization_server::AuthorizationServer {
    use magnetar::authorization_server::{
        AuthorizationServer, AuthorizationServerConfig, ClientMetadataFetcher,
        ClientMetadataPolicy, SqlAuthorizationStore,
    };
    let mut config = AuthorizationServerConfig::new(ISSUER, resources);
    config.scopes = scopes;
    AuthorizationServer::new(
        config,
        Arc::new(SqlAuthorizationStore::new(h.db.clone())),
        h.storage.clone(),
        h.limiter.clone(),
        Arc::new(ClientMetadataFetcher::new(
            h.resolver.clone(),
            h.key_sets.clone(),
            ClientMetadataPolicy::default(),
        )),
    )
    .expect("a server")
}

fn new_grant(
    kind: &str,
    client_id: &str,
    user_id: Option<&str>,
    scopes: &[&str],
) -> magnetar::authorization_server::NewGrant {
    magnetar::authorization_server::NewGrant {
        kind: kind.to_owned(),
        user_id: user_id.map(str::to_owned),
        client_id: client_id.to_owned(),
        client_name: "Seam probe".to_owned(),
        client_host: "probe.example.test".to_owned(),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        resource: RESOURCE.to_owned(),
        auth_epoch: user_id.map(|_| 0),
    }
}

fn agent_tokens() -> magnetar::authorization_server::IssueTokens {
    magnetar::authorization_server::IssueTokens {
        access_kind: "agent".to_owned(),
        refresh: false,
        access_lifetime: None,
    }
}

#[tokio::test]
async fn agent_007_issue_on_grant_extends_only_the_grant_it_is_told_about() {
    use magnetar::authorization_server::{GrantOwner, ScopeDefinition};
    let h = sqlite_harness().await;
    let issuer = h.server.token_issuer();
    let read = vec!["tools:read".to_owned()];
    let owner = GrantOwner {
        kind: "agent",
        client_id: "reg_probe",
    };
    let user_id = h.create_user("seam.owner@example.test", true).await;
    let user_grant = issuer
        .issue_grant(
            new_grant(
                "authorization_code",
                "mcl_someone",
                Some(&user_id),
                &["tools:read"],
            ),
            magnetar::authorization_server::IssueTokens {
                access_kind: "access".to_owned(),
                refresh: false,
                access_lifetime: None,
            },
        )
        .await
        .expect("an ordinary user grant")
        .grant_id;
    assert!(
        issuer
            .issue_on_grant(&user_grant, &owner, Some(&user_id), &read, RESOURCE)
            .await
            .expect("issue")
            .is_none(),
        "another client's ordinary grant is never extended"
    );
    let agent_grant = issuer
        .issue_grant(
            new_grant("agent", "reg_probe", None, &["tools:read", "tools:write"]),
            agent_tokens(),
        )
        .await
        .expect("an agent grant")
        .grant_id;
    let other_client = GrantOwner {
        kind: "agent",
        client_id: "reg_other",
    };
    assert!(
        issuer
            .issue_on_grant(&agent_grant, &other_client, None, &read, RESOURCE)
            .await
            .expect("issue")
            .is_none(),
        "another registration's grant is never extended"
    );
    assert!(
        issuer
            .issue_on_grant(&agent_grant, &owner, None, &read, RESOURCE)
            .await
            .expect("issue")
            .is_some(),
        "the owner's grant is extended"
    );

    // The application dropped tools:write and the resource since.
    let changed = reconfigured_server(
        &h,
        vec![ScopeDefinition::new("tools:read", "Read your tools")],
        vec!["https://app.example.test/other".to_owned()],
    );
    let changed_issuer = changed.token_issuer();
    assert!(
        changed_issuer
            .issue_on_grant(
                &agent_grant,
                &owner,
                None,
                &["tools:write".to_owned()],
                RESOURCE
            )
            .await
            .expect("issue")
            .is_none(),
        "a scope the server no longer offers"
    );
    assert!(
        changed_issuer
            .issue_on_grant(&agent_grant, &owner, None, &read, RESOURCE)
            .await
            .expect("issue")
            .is_none(),
        "a resource the server no longer serves"
    );

    // An epoch-ended grant and a revoked one are never extended.
    let user_owner = GrantOwner {
        kind: "agent",
        client_id: "reg_user",
    };
    let user_agent_grant = issuer
        .issue_grant(
            new_grant("agent", "reg_user", Some(&user_id), &["tools:read"]),
            agent_tokens(),
        )
        .await
        .expect("a user-bound agent grant")
        .grant_id;
    magnetar::sessions::JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &user_id)
        .await
        .expect("the epoch moves");
    assert!(
        issuer
            .issue_on_grant(
                &user_agent_grant,
                &user_owner,
                Some(&user_id),
                &read,
                RESOURCE
            )
            .await
            .expect("issue")
            .is_none(),
        "an epoch-ended grant"
    );
    issuer.revoke_grant(&agent_grant).await.expect("revoke");
    assert!(
        issuer
            .issue_on_grant(&agent_grant, &owner, None, &read, RESOURCE)
            .await
            .expect("issue")
            .is_none(),
        "a revoked grant"
    );
}

#[tokio::test]
async fn agent_007_revoke_token_touches_only_the_named_grant() {
    use secrecy::ExposeSecret;
    let h = sqlite_harness().await;
    let issuer = h.server.token_issuer();
    let first = issuer
        .issue_grant(
            new_grant("agent", "reg_one", None, &["tools:read"]),
            agent_tokens(),
        )
        .await
        .expect("a grant");
    let second = issuer
        .issue_grant(
            new_grant("agent", "reg_two", None, &["tools:read"]),
            agent_tokens(),
        )
        .await
        .expect("a grant");
    let token = first.response.access_token.expose_secret().to_owned();
    assert!(
        !issuer
            .revoke_token(&second.grant_id, &token)
            .await
            .expect("revoke"),
        "a token of another grant is not revoked"
    );
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
    assert!(
        issuer
            .revoke_token(&first.grant_id, &token)
            .await
            .expect("revoke")
    );
    assert_eq!(h.verify(&token).await, revoked());
}

#[tokio::test]
async fn agent_007_an_epoch_change_ends_a_bound_agent() {
    let h = sqlite_harness().await;
    let (registration, user_id) = h.claimed_anonymous("epoch@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    assert!(
        jwt_claims(registration["identity_assertion"].as_str().expect("v1"))
            .get("auth_epoch")
            .is_none(),
        "a pre-claim assertion has no user and no epoch"
    );
    let claimed = body(
        &h.poll(registration["claim_token"].as_str().expect("claim token"))
            .await,
    );
    let token = claimed["access_token"].as_str().expect("access");
    let assertion = claimed["identity_assertion"].as_str().expect("v2");
    assert_eq!(
        jwt_claims(assertion)["auth_epoch"],
        0,
        "a user-bound assertion records the user's epoch"
    );
    assert!(matches!(
        h.verify(token).await,
        AccessVerification::Valid(_)
    ));

    magnetar::sessions::JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &user_id)
        .await
        .expect("a password change moves the epoch");
    assert_eq!(
        h.verify(token).await,
        AccessVerification::Invalid(InvalidAccessReason::EpochChanged)
    );
    let again = h.exchange(assertion).await;
    assert_eq!(status(&again), 400, "{:?}", body(&again));
    assert_eq!(body(&again)["error"], "invalid_grant");
    assert_eq!(h.registration(id).await.status, "revoked");
    let revoked_events = h.events.of(LifecycleEventKind::AgentRegistrationRevoked);
    assert_eq!(revoked_events.len(), 1);
    assert_eq!(revoked_events[0].attributes["reason"], "epoch_changed");
    assert_eq!(
        body(&h.exchange(assertion).await)["error"],
        "invalid_grant",
        "the agent must claim again"
    );
}

#[tokio::test]
async fn agent_007_a_refused_exchange_finishes_a_pending_revocation() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let (registration, user_id) = h.claimed_anonymous("pending.cleanup@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let claimed = body(
        &h.poll(registration["claim_token"].as_str().expect("claim token"))
            .await,
    );
    let v2 = claimed["identity_assertion"].as_str().expect("v2");
    magnetar::sessions::JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &user_id)
        .await
        .expect("the epoch moves");
    faults.fail("revoke_grant", 2);
    assert_eq!(body(&h.exchange(v2).await)["error"], "invalid_grant");
    let row = h.registration(id).await;
    assert_eq!(row.status, "revoked");
    assert!(row.cleanup_pending, "the grant revocation failed");
    assert!(
        grants_of(&h, id).await.iter().any(|(_, revoked)| !revoked),
        "the grant is still live"
    );
    assert_eq!(body(&h.exchange(v2).await)["error"], "invalid_grant");
    assert!(
        grants_of(&h, id).await.iter().all(|(_, revoked)| *revoked),
        "the refused exchange finished the revocation"
    );
    assert!(!h.registration(id).await.cleanup_pending);
}

#[tokio::test]
async fn agent_008_an_epoch_change_makes_a_provider_agent_link_again() {
    let h = sqlite_harness().await;
    let email = "linked.provider@example.test";
    let first = body(&present_subject(&h, "epoch-subject", email).await);
    let old_assertion = first["identity_assertion"].as_str().expect("assertion");
    let token = access_token(&h.exchange(old_assertion).await);
    let (_, _, user, _) = agent_kind(&h.verify(&token).await);
    let user_id = user.expect("a bound user");
    magnetar::sessions::JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &user_id)
        .await
        .expect("sign out everywhere moves the epoch");
    let relinked = present_subject(&h, "epoch-subject", email).await;
    assert_eq!(status(&relinked), 401, "{:?}", body(&relinked));
    assert_eq!(body(&relinked)["error"], "interaction_required");
    assert_ne!(body(&relinked)["registration_id"], first["registration_id"]);
    assert_eq!(
        h.registration(first["registration_id"].as_str().expect("id"))
            .await
            .status,
        "revoked"
    );
    assert_eq!(
        body(&h.exchange(old_assertion).await)["error"],
        "invalid_grant"
    );
}

#[tokio::test]
async fn agent_007_an_expired_or_tampered_service_assertion_is_invalid_grant() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let mut parts: Vec<String> = assertion.split('.').map(str::to_owned).collect();
    let mut claims = jwt_claims(assertion);
    claims["sub"] = json!("reg_someone_else");
    parts[1] = base64_url(&serde_json::to_vec(&claims).expect("claims"));
    let tampered = parts.join(".");
    assert_eq!(body(&h.exchange(&tampered).await)["error"], "invalid_grant");
    let id = registration["registration_id"].as_str().expect("id");
    h.set_registration_time(
        id,
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    assert_eq!(
        body(&h.exchange(assertion).await)["error"],
        "invalid_grant",
        "an anonymous registration past its window no longer exchanges"
    );
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[tokio::test]
async fn agent_007_install_refuses_a_configuration_the_server_cannot_serve() {
    use magnetar::agent_auth::{
        AgentAuth, AgentAuthServices, KeySetFetcher, KeySetPolicy, SqlAgentAuthStore,
    };
    use magnetar::authorization_server::{
        AuthorizationServer, ClientMetadataFetcher, ClientMetadataPolicy, SqlAuthorizationStore,
    };
    let db = magnetar::default_schema::database().await;
    let install = |config: magnetar::agent_auth::AgentAuthConfig| {
        let db = db.clone();
        async move {
            let storage = Arc::new(magnetar::storage::SeaOrmStorage::<
                magnetar::default_schema::DefaultAuthSchema,
            >::new(db.clone()));
            let resolver = Arc::new(FakeResolver::default());
            let key_sets = Arc::new(FakeKeySets::default());
            let limiter = Arc::new(WindowLimiter::default());
            let server = AuthorizationServer::new(
                server_config(),
                Arc::new(SqlAuthorizationStore::new(db.clone())),
                storage.clone(),
                limiter.clone(),
                Arc::new(ClientMetadataFetcher::new(
                    resolver.clone(),
                    key_sets.clone(),
                    ClientMetadataPolicy::default(),
                )),
            )
            .expect("a server");
            AgentAuth::install(
                server,
                config,
                AgentAuthServices {
                    store: Arc::new(SqlAgentAuthStore::new(db)),
                    users: storage,
                    provisioning: Arc::new(RefuseProvisioning),
                    limiter,
                    key_sets: Arc::new(KeySetFetcher::new(
                        resolver,
                        key_sets,
                        KeySetPolicy::default(),
                    )),
                    events: Arc::new(RecordingEvents::default()),
                },
            )
            .map(|_| ())
        }
    };
    let field = |result: magnetar::Result<()>| match result {
        Err(magnetar::Error::InvalidInput { field, .. }) => field,
        other => panic!("the configuration must be refused, got {other:?}"),
    };
    let base = || agent_config(signing_key("service-key-1").0);
    assert!(install(base()).await.is_ok());

    let mut config = base();
    config.trusted_issuers.clear();
    assert_eq!(field(install(config).await), "trusted_issuers");
    let mut config = base();
    config.post_claim_scopes.push("admin:everything".to_owned());
    assert_eq!(field(install(config).await), "post_claim_scopes");
    let mut config = base();
    config.pre_claim_scopes = vec!["tools:write".to_owned()];
    config.post_claim_scopes = vec!["tools:read".to_owned()];
    assert_eq!(field(install(config).await), "pre_claim_scopes");
    let mut config = base();
    config.identity_types = IdentityTypes {
        anonymous: false,
        service_auth: false,
        identity_assertion: false,
    };
    assert_eq!(field(install(config).await), "identity_types");
    let mut config = base();
    config.trusted_issuers[0].jwks_uri = "http://provider.example.test/jwks.json".to_owned();
    assert_eq!(field(install(config).await), "trusted_issuers");
    let mut config = base();
    config.claim_page_url = "//evil.example.test/claim".to_owned();
    assert_eq!(field(install(config).await), "claim_page_url");
    let mut config = base();
    config.default_resource = Some("https://elsewhere.example.test/mcp".to_owned());
    assert_eq!(field(install(config).await), "default_resource");
    let mut config = base();
    config.previous_signing_keys = vec![magnetar::agent_auth::PreviousSigningKey::new(
        signing_key("service-key-1").0,
        chrono::Utc::now(),
    )];
    assert_eq!(field(install(config).await), "previous_signing_keys");
    assert!(
        magnetar::agent_auth::SigningKey::from_pem("kid", "not a key").is_err(),
        "a PEM that is not a P-256 key is refused"
    );
}

// AGENT-008 -----------------------------------------------------------------

#[tokio::test]
async fn agent_008_each_id_jag_refusal_answers_its_code() {
    let h = sqlite_harness().await;
    let now = chrono::Utc::now().timestamp();
    let base = || {
        h.provider
            .id_jag_claims("refused-user", "refused@example.test")
    };
    let cases: Vec<(&str, serde_json::Map<String, Value>, u16, &str)> = vec![
        (
            "wrong audience",
            {
                let mut claims = base();
                claims.insert("aud".into(), json!("https://elsewhere.example.test"));
                claims
            },
            400,
            "invalid_audience",
        ),
        (
            "expired",
            {
                let mut claims = base();
                claims.insert("exp".into(), json!(now - 1));
                claims
            },
            400,
            "expired",
        ),
        (
            "issued in the future",
            {
                let mut claims = base();
                claims.insert("iat".into(), json!(now + 600));
                claims
            },
            400,
            "invalid_request",
        ),
        (
            "no verified email or phone",
            {
                let mut claims = base();
                claims.insert("email_verified".into(), json!(false));
                claims
            },
            400,
            "missing_verified_email",
        ),
        (
            "another client",
            {
                let mut claims = base();
                claims.insert("client_id".into(), json!("https://other.example.test"));
                claims
            },
            400,
            "invalid_client_id",
        ),
        (
            "no auth_time",
            {
                let mut claims = base();
                claims.remove("auth_time");
                claims
            },
            401,
            "login_required",
        ),
        (
            "stale auth_time",
            {
                let mut claims = base();
                claims.insert("auth_time".into(), json!(now - 3601));
                claims
            },
            401,
            "login_required",
        ),
        (
            "future auth_time",
            {
                let mut claims = base();
                claims.insert("auth_time".into(), json!(now + 600));
                claims
            },
            401,
            "login_required",
        ),
    ];
    for (label, claims, code, error) in cases {
        let response = h.present(&h.provider.id_jag(&claims)).await;
        assert_eq!(status(&response), code, "{label}: {:?}", body(&response));
        assert_eq!(body(&response)["error"], error, "{label}");
        if code == 401 {
            let challenge = header(&response, "www-authenticate").expect("a challenge");
            assert!(
                challenge.starts_with("AgentAuth error=\"login_required\", max_age=\"3600\""),
                "{challenge}"
            );
            assert_eq!(body(&response)["max_age"], 3600);
        }
    }

    // An issuer off the trust list, even with a valid signature.
    let rogue = Provider::new(ROGUE, "rogue-key");
    let mut claims = rogue.id_jag_claims("victim", "victim@example.test");
    claims.insert("aud".into(), json!(ISSUER));
    let response = h.present(&rogue.id_jag(&claims)).await;
    assert_eq!(body(&response)["error"], "invalid_issuer");

    // A token naming the trusted issuer but signed with another key.
    let impostor = Provider::new(PROVIDER, "provider-key-1");
    let response = h
        .present(&impostor.id_jag(&impostor.id_jag_claims("victim", "victim@example.test")))
        .await;
    assert_eq!(body(&response)["error"], "invalid_signature");

    // HMAC is never accepted, whatever the key set holds.
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.typ = Some("oauth-id-jag+jwt".to_owned());
    header.kid = Some("provider-key-1".to_owned());
    let hmac = jsonwebtoken::encode(
        &header,
        &Value::Object(base()),
        &jsonwebtoken::EncodingKey::from_secret(b"guessable"),
    )
    .expect("an HMAC token");
    assert_eq!(body(&h.present(&hmac).await)["error"], "invalid_signature");

    // A token of another type is refused before anything is fetched.
    let wrong_type = h.provider.sign("JWT", &Value::Object(base()));
    assert_eq!(
        body(&h.present(&wrong_type).await)["error"],
        "invalid_request"
    );
    assert!(
        h.events
            .of(LifecycleEventKind::AgentRegistrationCreated)
            .is_empty(),
        "no refused ID-JAG registers anything"
    );
}

#[tokio::test]
async fn agent_008_iat_is_accepted_up_to_the_clock_skew() {
    let h = sqlite_harness().await;
    // Start at the top of a second so the test's clock and the server's
    // name the same second.
    let sub_second = chrono::Utc::now().timestamp_subsec_millis();
    if sub_second > 500 {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(
            1010 - sub_second,
        )))
        .await;
    }
    for (ahead, accepted) in [(119_i64, true), (121, false)] {
        let mut claims = h
            .provider
            .id_jag_claims(&format!("iat-{ahead}"), &format!("iat{ahead}@example.test"));
        let now = chrono::Utc::now().timestamp();
        claims.insert("iat".into(), json!(now + ahead));
        let response = h.present(&h.provider.id_jag(&claims)).await;
        if accepted {
            assert_eq!(status(&response), 200, "+{ahead}s: {:?}", body(&response));
        } else {
            assert_eq!(status(&response), 400, "+{ahead}s: {:?}", body(&response));
            assert_eq!(body(&response)["error"], "invalid_request");
        }
    }
}

#[tokio::test]
async fn agent_008_the_algorithm_allowlist_refuses_hmac_before_any_key_is_read() {
    use base64::Engine;
    let h = sqlite_harness().await;
    let secret = b"a provider published a symmetric key by mistake";
    h.key_sets.serve(
        PROVIDER_JWKS,
        json!({"keys": [{
            "kty": "oct",
            "kid": "shared-secret",
            "k": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret),
        }]}),
        None,
    );
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    header.typ = Some("oauth-id-jag+jwt".to_owned());
    header.kid = Some("shared-secret".to_owned());
    let token = jsonwebtoken::encode(
        &header,
        &Value::Object(h.provider.id_jag_claims("hmac", "hmac@example.test")),
        &jsonwebtoken::EncodingKey::from_secret(secret),
    )
    .expect("an HMAC token");
    let refused = h.present(&token).await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["error"], "invalid_signature");
    assert_eq!(
        h.key_sets.request_count(PROVIDER_JWKS),
        0,
        "the algorithm was refused before the key set was read"
    );
}

#[tokio::test]
async fn agent_008_a_replayed_jti_is_refused_on_a_second_node() {
    let db = magnetar::default_schema::database().await;
    let provider = Provider::new(PROVIDER, "provider-key-1");
    let mut options = Options::new();
    options.provider = provider.clone();
    let first = build(db.clone(), options).await;
    let mut options = Options::new();
    options.provider = provider.clone();
    let second = build(db.clone(), options).await;
    let claims = provider.id_jag_claims("replayed-user", "replayed@example.test");
    let token = provider.id_jag(&claims);
    let accepted = first.present(&token).await;
    assert_eq!(status(&accepted), 200, "{:?}", body(&accepted));
    let replayed = second.present(&token).await;
    assert_eq!(status(&replayed), 400);
    assert_eq!(body(&replayed)["error"], "replay_detected");
    let again = first.present(&token).await;
    assert_eq!(body(&again)["error"], "replay_detected");
    use sea_orm::EntityTrait;
    let records = magnetar::agent_auth::sql::replays::Entity::find()
        .all(&db)
        .await
        .expect("read replay records");
    assert_eq!(
        records.len(),
        1,
        "one database record holds the jti for both nodes"
    );
}

#[tokio::test]
async fn agent_008_an_existing_delegation_matches_first() {
    let h = sqlite_harness().await;
    let first = present_subject(&h, "stable-subject", "first@example.test").await;
    assert_eq!(status(&first), 200, "{:?}", body(&first));
    let registration_id = body(&first)["registration_id"].clone();
    // The provider later reports another email; the delegation still wins.
    let second = present_subject(&h, "stable-subject", "changed@example.test").await;
    assert_eq!(status(&second), 200, "{:?}", body(&second));
    assert_eq!(body(&second)["registration_id"], registration_id);
    assert_eq!(
        jwt_claims(
            body(&second)["identity_assertion"]
                .as_str()
                .expect("assertion")
        )["email"],
        "first@example.test",
        "the assertion carries the bound user's own email"
    );
}

#[tokio::test]
async fn agent_008_an_email_only_match_steps_up_and_never_binds_silently() {
    let h = sqlite_harness().await;
    let user_id = h.create_user("existing@example.test", true).await;
    let response = present_subject(&h, "provider-sub-9", "existing@example.test").await;
    assert_eq!(status(&response), 401, "{:?}", body(&response));
    let challenge = header(&response, "www-authenticate").expect("a challenge");
    assert!(
        challenge.starts_with("AgentAuth error=\"interaction_required\""),
        "{challenge}"
    );
    let step_up = body(&response);
    assert_eq!(step_up["error"], "interaction_required");
    assert_eq!(step_up["registration_type"], "identity_assertion");
    assert!(
        step_up.get("identity_assertion").is_none(),
        "nothing binds yet"
    );
    let claim_token = step_up["claim_token"].as_str().expect("a claim token");
    let attempt = attempt_token(step_up["claim"]["verification_uri"].as_str().expect("uri"));
    let code = step_up["claim"]["user_code"].as_str().expect("code");
    let row = h
        .registration(step_up["registration_id"].as_str().expect("id"))
        .await;
    assert_eq!(row.user_id, None, "the registration is not bound");
    assert_eq!(row.status, "pending");

    // The confirmation page names the provider by the trust list's name,
    // never by the name the provider put in its token.
    let bearer = h.sign_in(&user_id).await;
    let pending = h.pending(&attempt, Some(&bearer)).await;
    assert_eq!(status(&pending), 200, "{:?}", body(&pending));
    let view = body(&pending);
    assert_eq!(view["provider"]["display_name"], PROVIDER_NAME);
    assert!(!view.to_string().contains("Provider-Chosen Name"));
    assert_eq!(
        view["advisories"],
        json!(["first_time_provider", "first_time_account"])
    );
    let confirmed = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
    let tokens = h.poll(claim_token).await;
    assert_eq!(status(&tokens), 200, "{:?}", body(&tokens));
    let (_, _, user, _) = agent_kind(&h.verify(&access_token(&tokens)).await);
    assert_eq!(user.as_deref(), Some(user_id.as_str()));

    // The next presentation for the same delegation is accepted directly.
    let again = present_subject(&h, "provider-sub-9", "existing@example.test").await;
    assert_eq!(status(&again), 200, "{:?}", body(&again));
    assert_eq!(body(&again)["registration_id"], step_up["registration_id"]);
}

#[tokio::test]
async fn agent_008_a_repeated_presentation_during_step_up_reuses_the_row() {
    let h = sqlite_harness().await;
    h.create_user("twice@example.test", true).await;
    let first = body(&present_subject(&h, "twice-sub", "twice@example.test").await);
    let second = body(&present_subject(&h, "twice-sub", "twice@example.test").await);
    assert_eq!(second["error"], "interaction_required");
    assert_eq!(second["registration_id"], first["registration_id"]);
    assert_ne!(second["claim_token"], first["claim_token"]);
    let stale = h
        .pending(
            &attempt_token(first["claim"]["verification_uri"].as_str().expect("uri")),
            None,
        )
        .await;
    assert_eq!(status(&stale), 401, "nothing is shown before sign-in");
}

#[tokio::test]
async fn agent_008_a_case_variant_of_a_verified_email_steps_up() {
    let h = sqlite_harness().await;
    let stored = magnetar::password::normalize_email("Émile.Zola@Example.test");
    let user_id = h.create_user(&stored, true).await;
    let response = present_subject(&h, "case-variant", "ÉMILE.ZOLA@EXAMPLE.TEST").await;
    assert_eq!(status(&response), 401, "{:?}", body(&response));
    assert_eq!(body(&response)["error"], "interaction_required");
    let row = h
        .registration(body(&response)["registration_id"].as_str().expect("id"))
        .await;
    assert_eq!(row.claim_user_id.as_deref(), Some(user_id.as_str()));
    use magnetar::storage::UserStore;
    assert!(
        h.storage
            .find_by_email("ÉMILE.ZOLA@EXAMPLE.TEST")
            .await
            .expect("read")
            .is_none(),
        "no second account was provisioned"
    );
}

#[tokio::test]
async fn agent_008_the_provisioning_policy_decides_when_nothing_matches() {
    let mut options = Options::new();
    options.provisioning = Some(Arc::new(RefuseProvisioning));
    let h = sqlite_harness_with(options).await;
    let refused = present_subject(&h, "unknown-sub", "nobody@example.test").await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["error"], "invalid_request");
    use magnetar::storage::UserStore;
    assert!(
        h.storage
            .find_by_email("nobody@example.test")
            .await
            .expect("read")
            .is_none(),
        "a refusing policy creates no user"
    );

    let h = sqlite_harness().await;
    let created = present_subject(&h, "unknown-sub", "nobody@example.test").await;
    assert_eq!(status(&created), 200, "{:?}", body(&created));
    let user = h
        .storage
        .find_by_email("nobody@example.test")
        .await
        .expect("read")
        .expect("the provisioning policy created the user");
    assert!(user.email_verified_at.is_some());
}

#[tokio::test]
async fn agent_008_an_unverified_account_is_not_matched_by_email() {
    let mut options = Options::new();
    options.provisioning = Some(Arc::new(RefuseProvisioning));
    let h = sqlite_harness_with(options).await;
    h.create_user("unverified@example.test", false).await;
    let response = present_subject(&h, "some-sub", "unverified@example.test").await;
    assert_eq!(status(&response), 400, "{:?}", body(&response));
    assert_ne!(body(&response)["error"], "interaction_required");
}

#[tokio::test]
async fn agent_008_key_sets_are_cached_with_one_refetch_on_an_unknown_kid() {
    let h = sqlite_harness().await;
    assert_eq!(
        status(&present_subject(&h, "cache-1", "cache1@example.test").await),
        200
    );
    assert_eq!(
        status(&present_subject(&h, "cache-2", "cache2@example.test").await),
        200
    );
    assert_eq!(
        h.key_sets.request_count(PROVIDER_JWKS),
        1,
        "the second read is cached"
    );

    // The provider rotates: a token with the new key refetches once.
    let rotated = Provider::new(PROVIDER, "provider-key-2");
    h.key_sets.serve(
        PROVIDER_JWKS,
        json!({"keys": [h.provider.jwk.clone(), rotated.jwk.clone()]}),
        Some("max-age=86400"),
    );
    let claims = rotated.id_jag_claims("cache-3", "cache3@example.test");
    let response = h.present(&rotated.id_jag(&claims)).await;
    assert_eq!(status(&response), 200, "{:?}", body(&response));
    assert_eq!(h.key_sets.request_count(PROVIDER_JWKS), 2);

    // An unknown key right after does not refetch again.
    let unknown = Provider::new(PROVIDER, "provider-key-unknown");
    let response = h
        .present(&unknown.id_jag(&unknown.id_jag_claims("cache-4", "cache4@example.test")))
        .await;
    assert_eq!(body(&response)["error"], "invalid_signature");
    assert_eq!(h.key_sets.request_count(PROVIDER_JWKS), 2);
}

#[tokio::test]
async fn agent_008_a_failed_key_set_fetch_is_remembered_briefly() {
    let h = sqlite_harness().await;
    h.key_sets.withdraw(PROVIDER_JWKS);
    for subject in ["missing-keys-1", "missing-keys-2", "missing-keys-3"] {
        let response = present_subject(&h, subject, "missing.keys@example.test").await;
        assert_eq!(body(&response)["error"], "invalid_signature");
    }
    assert_eq!(
        h.key_sets.request_count(PROVIDER_JWKS),
        1,
        "the failure is cached for a minute"
    );
}

#[tokio::test]
async fn agent_008_an_exp_far_ahead_is_refused() {
    let h = sqlite_harness().await;
    for exp in [json!(9e15), json!(chrono::Utc::now().timestamp() + 7200)] {
        let mut claims = h.provider.id_jag_claims("far-exp", "far.exp@example.test");
        claims.insert("exp".into(), exp.clone());
        let response = h.present(&h.provider.id_jag(&claims)).await;
        assert_eq!(status(&response), 400, "exp {exp}: {:?}", body(&response));
        assert_eq!(body(&response)["error"], "invalid_request", "exp {exp}");
    }
    let mut event = h.provider.revocation_event("far-exp");
    event.insert("exp".into(), json!(9e15));
    let refused = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            h.provider.event(&event).into_bytes(),
        )
        .await;
    assert_eq!(status(&refused), 400, "{:?}", body(&refused));
    assert_eq!(body(&refused)["err"], "invalid_request");
}

#[tokio::test]
async fn agent_008_a_key_set_on_a_private_address_is_never_fetched() {
    let h = sqlite_harness().await;
    h.resolver.answer(PROVIDER_HOST, &["10.0.0.8"]);
    let response = present_subject(&h, "private", "private@example.test").await;
    assert_eq!(body(&response)["error"], "invalid_signature");
    assert_eq!(h.key_sets.request_count(PROVIDER_JWKS), 0);
}

// AGENT-009 -----------------------------------------------------------------

#[tokio::test]
async fn agent_009_tokens_and_codes_are_stored_only_as_digests() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("digest@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let row = h.registration(id).await;
    assert_eq!(
        row.claim_token_hash.as_deref(),
        Some(sha256_hex(claim_token).as_str())
    );
    assert_eq!(
        row.attempt_token_hash.as_deref(),
        Some(sha256_hex(&attempt).as_str())
    );
    assert_eq!(
        row.user_code_hash.as_deref(),
        Some(sha256_hex(code).as_str())
    );
    let stored = format!("{row:?}");
    for secret in [claim_token, attempt.as_str()] {
        assert!(!stored.contains(secret), "a plaintext token is stored");
    }
    assert!(
        !stored.contains(&format!("\"{code}\"")),
        "the user code is stored in plain text"
    );
}

#[tokio::test]
async fn agent_009_each_token_is_returned_once() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let first = body(&h.initiate(claim_token, "once@example.test").await);
    let second = body(&h.initiate(claim_token, "once@example.test").await);
    assert!(first.get("claim_token").is_none() && second.get("claim_token").is_none());
    assert_ne!(first["claim_attempt_id"], second["claim_attempt_id"]);
    assert_ne!(
        first["claim_attempt"]["verification_uri"],
        second["claim_attempt"]["verification_uri"]
    );
    // The first attempt's URL stopped working with the second attempt.
    let user_id = h.create_user("once@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    let old = attempt_token(
        first["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    assert_eq!(status(&h.pending(&old, Some(&bearer)).await), 404);
    // The claim token yields its tokens once: a second poll after success
    // knows it no more.
    let attempt = attempt_token(
        second["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = second["claim_attempt"]["user_code"].as_str().expect("code");
    assert_eq!(status(&h.confirm(&attempt, code, Some(&bearer)).await), 200);
    let claimed = h.poll(claim_token).await;
    assert_eq!(status(&claimed), 200, "{:?}", body(&claimed));
    h.forget_polls().await;
    let again = h.poll(claim_token).await;
    assert_eq!(body(&again)["error"], "expired_token");
}

#[tokio::test]
async fn agent_009_a_user_code_stops_working_after_ten_minutes() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("late@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let row = h.registration(id).await;
    let lifetime = row.user_code_expires_at.expect("an expiry") - row.created_at;
    assert!(
        (599..=601).contains(&lifetime.num_seconds()),
        "the code lasts ten minutes, not {lifetime}"
    );
    let user_id = h.create_user("late@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    h.set_registration_time(
        id,
        registrations::Column::UserCodeExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let late = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&late), 410, "{:?}", body(&late));
    assert_eq!(h.registration(id).await.status, "pending");
    let poll = h
        .poll(registration["claim_token"].as_str().expect("token"))
        .await;
    assert_eq!(body(&poll)["error"], "expired_token");
}

#[tokio::test]
async fn agent_009_a_user_code_stops_working_after_the_wrong_entry_limit() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("guess@example.test").await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let user_id = h.create_user("guess@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    for remaining in (0..5).rev() {
        let response = h.confirm(&attempt, wrong, Some(&bearer)).await;
        assert_eq!(status(&response), 400, "{:?}", body(&response));
        assert_eq!(body(&response)["error"], "invalid_user_code");
        assert_eq!(body(&response)["attempts_remaining"], remaining);
    }
    let right_too_late = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&right_too_late), 410, "{:?}", body(&right_too_late));
    let id = registration["registration_id"].as_str().expect("id");
    assert_eq!(h.registration(id).await.status, "pending");
    // The agent starts a new attempt, which works.
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let fresh = body(&h.initiate(claim_token, "guess@example.test").await);
    let attempt = attempt_token(
        fresh["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = fresh["claim_attempt"]["user_code"].as_str().expect("code");
    assert_eq!(status(&h.confirm(&attempt, code, Some(&bearer)).await), 200);
}

#[tokio::test]
async fn agent_009_a_user_signed_in_with_another_email_is_refused() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("owner@example.test").await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let intruder = h.create_user("intruder@example.test", true).await;
    let bearer = h.sign_in(&intruder).await;
    let view = h.pending(&attempt, Some(&bearer)).await;
    assert_eq!(status(&view), 403, "{:?}", body(&view));
    let confirm = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&confirm), 403, "{:?}", body(&confirm));
    assert_no_framing(&confirm);
    let id = registration["registration_id"].as_str().expect("id");
    assert_eq!(h.registration(id).await.status, "pending");
}

#[tokio::test]
async fn agent_009_the_claim_routes_refuse_a_request_without_a_signed_in_user() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("nosession@example.test").await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let view = h.pending(&attempt, None).await;
    assert_eq!(status(&view), 401);
    assert_no_framing(&view);
    let confirm = h.confirm(&attempt, code, None).await;
    assert_eq!(status(&confirm), 401);
    assert_eq!(body(&confirm)["error"], "login_required");
    assert_no_framing(&confirm);
    let id = registration["registration_id"].as_str().expect("id");
    assert_eq!(h.registration(id).await.status, "pending");
    // A session from before the user's password change speaks for no one:
    // the engine compares the session's epoch with the user's.
    let user_id = h.create_user("nosession@example.test", true).await;
    magnetar::sessions::JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &user_id)
        .await
        .expect("a password change moves the epoch");
    let stale_session = magnetar::agent_auth::SignedInUser {
        user_id: user_id.clone(),
        auth_epoch: Some(0),
    };
    let view = h.engine.pending_claim(&attempt, Some(&stale_session)).await;
    assert_eq!(status(&view), 401, "{:?}", body(&view));
    assert_eq!(body(&view)["error"], "login_required");
    let stale = h
        .engine
        .confirm_claim(&attempt, code, Some(&stale_session), Some(CLIENT_ADDRESS))
        .await;
    assert_eq!(status(&stale), 401, "{:?}", body(&stale));
    assert_eq!(body(&stale)["error"], "login_required");
    let current = magnetar::agent_auth::SignedInUser {
        user_id,
        auth_epoch: Some(1),
    };
    assert_eq!(
        status(&h.engine.pending_claim(&attempt, Some(&current)).await),
        200,
        "the same user at the current epoch sees the claim"
    );
    assert_eq!(h.registration(id).await.status, "pending");
}

#[tokio::test]
async fn agent_009_reinitiation_and_polling_answer_the_protocol_codes() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let id = registration["registration_id"].as_str().expect("id");
    let pending = h.poll(claim_token).await;
    assert_eq!(status(&pending), 400);
    assert_eq!(body(&pending)["error"], "authorization_pending");
    let fast = h.poll(claim_token).await;
    assert_eq!(body(&fast)["error"], "slow_down");
    let started = h.initiate(claim_token, "poller@example.test").await;
    assert_eq!(status(&started), 200, "{:?}", body(&started));
    let started = body(&started);
    assert_eq!(started["status"], "initiated");
    assert_eq!(started["registration_id"], id);
    assert!(
        started["claim_attempt_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("cla_"))
    );
    assert_eq!(started["claim_attempt"]["expires_in"], 600);
    h.forget_polls().await;
    assert_eq!(
        body(&h.poll(claim_token).await)["error"],
        "authorization_pending"
    );
    // The user code's ten minutes pass: polling says expired_token, and
    // re-initiating with the same claim token mints a new code.
    h.set_registration_time(
        id,
        registrations::Column::UserCodeExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    h.forget_polls().await;
    assert_eq!(body(&h.poll(claim_token).await)["error"], "expired_token");
    let again = h.initiate(claim_token, "poller@example.test").await;
    assert_eq!(status(&again), 200, "{:?}", body(&again));
    h.forget_polls().await;
    assert_eq!(
        body(&h.poll(claim_token).await)["error"],
        "authorization_pending"
    );
    // The registration window closes: re-initiation answers claim_expired.
    h.set_registration_time(
        id,
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    let closed = h.initiate(claim_token, "poller@example.test").await;
    assert_eq!(status(&closed), 410);
    assert_eq!(body(&closed)["error"], "claim_expired");
    h.forget_polls().await;
    assert_eq!(body(&h.poll(claim_token).await)["error"], "expired_token");
    let unknown = h.initiate("clm_unknown", "poller@example.test").await;
    assert_eq!(body(&unknown)["error"], "invalid_claim_token");
}

#[tokio::test]
async fn agent_009_a_failed_poll_keeps_the_claim_token() {
    let h = sqlite_harness().await;
    let (registration, _) = h.claimed_anonymous("failed.poll@example.test").await;
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let elsewhere = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
                ("claim_token", claim_token),
                ("resource", "https://elsewhere.example.test/mcp"),
            ],
            None,
        )
        .await;
    assert_eq!(status(&elsewhere), 400, "{:?}", body(&elsewhere));
    assert_eq!(body(&elsewhere)["error"], "invalid_target");
    h.forget_polls().await;
    let claimed = h.poll(claim_token).await;
    assert_eq!(
        status(&claimed),
        200,
        "the claim token survives a poll that issued nothing: {:?}",
        body(&claimed)
    );
}

#[tokio::test]
async fn agent_009_a_claimed_token_that_is_never_polled_expires() {
    let h = sqlite_harness().await;
    let (registration, _) = h.claimed_anonymous("never.polled@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let long_ago = chrono::Utc::now() - chrono::Duration::hours(25);
    h.set_registration_time(id, registrations::Column::ClaimedAt, long_ago)
        .await;
    h.set_registration_time(id, registrations::Column::ClaimExpiresAt, long_ago)
        .await;
    let late = h
        .poll(registration["claim_token"].as_str().expect("claim token"))
        .await;
    assert_eq!(status(&late), 400, "{:?}", body(&late));
    assert_eq!(body(&late)["error"], "expired_token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_009_a_claim_racing_an_exchange_leaves_no_pre_claim_token() {
    let mut options = Options::new();
    options.config.limits.anonymous = RegistrationLimit::per_hour(100, 1000);
    let h = Arc::new(sqlite_harness_with(options).await);
    for round in 0..12 {
        // Odd rounds race an exchange that reuses an existing pre-claim
        // grant; even rounds race the registration's first exchange.
        let reuse = round % 2 == 1;
        let email = format!("race{round}@example.test");
        let registration = h.anonymous().await;
        let v1 = registration["identity_assertion"]
            .as_str()
            .expect("v1")
            .to_owned();
        let earlier = if reuse {
            Some(access_token(&h.exchange(&v1).await))
        } else {
            None
        };
        let (bearer, attempt, code) = start_claim(&h, &registration, &email).await;
        let exchange = tokio::spawn({
            let (h, v1) = (h.clone(), v1.clone());
            async move { h.exchange(&v1).await }
        });
        let confirm = tokio::spawn({
            let h = h.clone();
            async move { h.confirm(&attempt, &code, Some(&bearer)).await }
        });
        let exchanged = exchange.await.expect("the exchange");
        let confirmed = confirm.await.expect("the confirmation");
        assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
        let mut pre_claim: Vec<String> = earlier.into_iter().collect();
        if status(&exchanged) == 200 {
            pre_claim.push(
                body(&exchanged)["access_token"]
                    .as_str()
                    .expect("access")
                    .to_owned(),
            );
        }
        for token in pre_claim {
            assert_eq!(
                h.verify(&token).await,
                revoked(),
                "round {round} (reuse {reuse}): a pre-claim token outlived the claim"
            );
        }
        assert_eq!(body(&h.exchange(&v1).await)["error"], "invalid_grant");
    }
}

/// Start a claim of `registration` for a new verified user; return the
/// user's bearer, the attempt token and the code.
async fn start_claim(h: &Harness, registration: &Value, email: &str) -> (String, String, String) {
    let user_id = h.create_user(email, true).await;
    let bearer = h.sign_in(&user_id).await;
    let started = body(
        &h.initiate(
            registration["claim_token"].as_str().expect("claim token"),
            email,
        )
        .await,
    );
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code")
        .to_owned();
    (bearer, attempt, code)
}

/// Drop a grant and its tokens from the authorization server's tables, as
/// pruning does, so the next exchange replaces it.
async fn prune_grant(h: &Harness, grant_id: &str) {
    use magnetar::authorization_server::sql::{grants, tokens};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    tokens::Entity::delete_many()
        .filter(tokens::Column::GrantId.eq(grant_id.to_owned()))
        .exec(&h.db)
        .await
        .expect("drop the tokens");
    grants::Entity::delete_many()
        .filter(grants::Column::Id.eq(grant_id.to_owned()))
        .exec(&h.db)
        .await
        .expect("drop the grant");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_009_a_claim_revokes_a_pre_claim_grant_an_exchange_made_while_it_ran() {
    for replace in [false, true] {
        let (options, faults) = faulty();
        let h = Arc::new(sqlite_harness_with(options).await);
        let registration = h.anonymous().await;
        let id = registration["registration_id"].as_str().expect("id");
        let v1 = registration["identity_assertion"]
            .as_str()
            .expect("v1")
            .to_owned();
        if replace {
            // An earlier grant the next exchange has to replace.
            access_token(&h.exchange(&v1).await);
            prune_grant(&h, &grants_of(&h, id).await[0].0).await;
        }
        let (bearer, attempt, code) =
            start_claim(&h, &registration, &format!("ran{replace}@example.test")).await;
        // The claim has read what it reads and waits to commit.
        let (reached, release) = faults.gate_before("confirm_claim");
        let confirm = tokio::spawn({
            let h = h.clone();
            async move { h.confirm(&attempt, &code, Some(&bearer)).await }
        });
        reached.notified().await;
        // An exchange makes a pre-claim grant and mints on it now.
        let token = access_token(&h.exchange(&v1).await);
        release.notify_one();
        assert_eq!(status(&confirm.await.expect("the confirmation")), 200);
        assert_eq!(
            h.verify(&token).await,
            revoked(),
            "replace {replace}: a pre-claim token outlived the claim"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_007_an_exchange_never_creates_a_pre_claim_grant_for_a_claimed_registration() {
    let (options, faults) = faulty();
    let h = Arc::new(sqlite_harness_with(options).await);
    let registration = h.anonymous().await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let v1 = registration["identity_assertion"]
        .as_str()
        .expect("v1")
        .to_owned();
    let (bearer, attempt, code) = start_claim(&h, &registration, "guard.create@example.test").await;
    // The exchange reads the unclaimed row, then waits.
    let (reached, release) = faults.gate("find_registration");
    let exchange = tokio::spawn({
        let (h, v1) = (h.clone(), v1.clone());
        async move { h.exchange(&v1).await }
    });
    reached.notified().await;
    assert_eq!(
        status(&h.confirm(&attempt, &code, Some(&bearer)).await),
        200
    );
    release.notify_one();
    let exchanged = exchange.await.expect("the exchange");
    assert_eq!(
        body(&exchanged)["error"],
        "invalid_grant",
        "{:?}",
        body(&exchanged)
    );
    assert!(
        grants_of(&h, &id).await.is_empty(),
        "the database refused the pre-claim grant before it existed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_007_an_exchange_never_extends_a_pre_claim_grant_for_a_claimed_registration() {
    use magnetar::authorization_server::sql::tokens;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let (options, faults) = faulty();
    let h = Arc::new(sqlite_harness_with(options).await);
    let registration = h.anonymous().await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let v1 = registration["identity_assertion"]
        .as_str()
        .expect("v1")
        .to_owned();
    let earlier = access_token(&h.exchange(&v1).await);
    let grant = grants_of(&h, &id).await[0].0.clone();
    let (bearer, attempt, code) = start_claim(&h, &registration, "guard.extend@example.test").await;
    let (reached, release) = faults.gate("find_registration");
    let exchange = tokio::spawn({
        let (h, v1) = (h.clone(), v1.clone());
        async move { h.exchange(&v1).await }
    });
    reached.notified().await;
    // The claim commits, and its revocation of the pre-claim grant fails.
    faults.fail("revoke_grant", 2);
    assert_eq!(
        status(&h.confirm(&attempt, &code, Some(&bearer)).await),
        200
    );
    release.notify_one();
    let exchanged = exchange.await.expect("the exchange");
    assert_eq!(
        body(&exchanged)["error"],
        "invalid_grant",
        "{:?}",
        body(&exchanged)
    );
    let minted = tokens::Entity::find()
        .filter(tokens::Column::GrantId.eq(grant))
        .all(&h.db)
        .await
        .expect("read tokens");
    assert_eq!(minted.len(), 1, "nothing was minted on the pre-claim grant");
    // The failed revocation is finished by the agent's next request.
    h.forget_polls().await;
    assert_eq!(
        status(
            &h.poll(registration["claim_token"].as_str().expect("claim token"))
                .await
        ),
        200
    );
    assert_eq!(h.verify(&earlier).await, revoked());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_009_a_poll_between_the_claim_and_its_tail_keeps_its_token() {
    let (options, faults) = faulty();
    let h = Arc::new(sqlite_harness_with(options).await);
    let registration = h.anonymous().await;
    access_token(
        &h.exchange(registration["identity_assertion"].as_str().expect("v1"))
            .await,
    );
    let (bearer, attempt, code) = start_claim(&h, &registration, "gap@example.test").await;
    let (reached, release) = faults.gate("confirm_claim");
    let confirm = tokio::spawn({
        let h = h.clone();
        async move { h.confirm(&attempt, &code, Some(&bearer)).await }
    });
    reached.notified().await;
    // The claim is committed; its tail has not run. The agent polls now.
    let polled = h
        .poll(registration["claim_token"].as_str().expect("claim token"))
        .await;
    let token = access_token(&polled);
    release.notify_one();
    assert_eq!(status(&confirm.await.expect("the confirmation")), 200);
    assert!(
        matches!(h.verify(&token).await, AccessVerification::Valid(_)),
        "the claim's tail revoked only the grant it read before the claim"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_007_an_exchange_that_read_the_row_before_a_claim_hands_out_no_pre_claim_token() {
    let (options, faults) = faulty();
    let h = Arc::new(sqlite_harness_with(options).await);
    let registration = h.anonymous().await;
    let v1 = registration["identity_assertion"]
        .as_str()
        .expect("v1")
        .to_owned();
    access_token(&h.exchange(&v1).await);
    let (bearer, attempt, code) = start_claim(&h, &registration, "reread@example.test").await;
    let (reached, release) = faults.gate("add_tokens");
    let exchange = tokio::spawn({
        let (h, v1) = (h.clone(), v1.clone());
        async move { h.exchange(&v1).await }
    });
    reached.notified().await;
    // The exchange read the row and minted on the pre-claim grant; the
    // claim lands now, and its best-effort revocation of that grant fails.
    faults.fail("revoke_grant", 2);
    assert_eq!(
        status(&h.confirm(&attempt, &code, Some(&bearer)).await),
        200
    );
    release.notify_one();
    let exchanged = exchange.await.expect("the exchange");
    assert_eq!(status(&exchanged), 400, "{:?}", body(&exchanged));
    assert_eq!(body(&exchanged)["error"], "invalid_grant");
}

#[tokio::test]
async fn agent_009_service_auth_reinitiation_needs_the_bound_email() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("bound@example.test").await;
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let other = h.initiate(claim_token, "other@example.test").await;
    assert_eq!(status(&other), 400);
    assert_eq!(body(&other)["error"], "invalid_request");
    let same = h.initiate(claim_token, "BOUND@example.test").await;
    assert_eq!(status(&same), 200, "{:?}", body(&same));
}

#[tokio::test]
async fn agent_009_claiming_revokes_pre_claim_tokens_and_answers_a_second_assertion() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let v1 = registration["identity_assertion"]
        .as_str()
        .expect("v1")
        .to_owned();
    let pre_claim = access_token(&h.exchange(&v1).await);
    assert!(matches!(
        h.verify(&pre_claim).await,
        AccessVerification::Valid(_)
    ));

    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let user_id = h.create_user("claimer@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    let started = body(&h.initiate(claim_token, "claimer@example.test").await);
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let view = body(&h.pending(&attempt, Some(&bearer)).await);
    assert_eq!(view["signed_in_as"], "claimer@example.test");
    assert_eq!(
        view["scopes"][1]["description"],
        "Change data through tools"
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code");
    let confirmed = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
    assert_no_framing(&confirmed);
    assert_eq!(
        h.verify(&pre_claim).await,
        revoked(),
        "the pre-claim token is dead"
    );

    let claimed = h.poll(claim_token).await;
    assert_eq!(status(&claimed), 200, "{:?}", body(&claimed));
    let claimed = body(&claimed);
    assert_eq!(claimed["scope"], "tools:read tools:write");
    let v2 = claimed["identity_assertion"].as_str().expect("v2");
    assert!(claimed["assertion_expires"].as_str().is_some());
    assert_eq!(jwt_claims(v2)["email"], "claimer@example.test");
    assert_eq!(jwt_claims(v2)["email_verified"], true);
    let post_claim = claimed["access_token"].as_str().expect("access");
    let (_, _, user, scopes) = agent_kind(&h.verify(post_claim).await);
    assert_eq!(user.as_deref(), Some(user_id.as_str()));
    assert_eq!(scopes.len(), 2);
    assert_eq!(
        body(&h.exchange(&v1).await)["error"],
        "invalid_grant",
        "the pre-claim assertion is superseded"
    );
    assert_eq!(status(&h.exchange(v2).await), 200);

    let events = h.events.of(LifecycleEventKind::AgentClaimConfirmed);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].user_id, user_id);
    assert_eq!(events[0].attributes["registration_id"], id);
    assert_eq!(events[0].attributes["claimed_by_user_id"], user_id);
}

#[tokio::test]
async fn agent_009_a_user_whose_email_is_not_verified_cannot_claim() {
    let h = sqlite_harness().await;
    let registration = h.service_auth("unverified.claimer@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"].as_str().expect("code");
    let user_id = h
        .create_user("unverified.claimer@example.test", false)
        .await;
    let bearer = h.sign_in(&user_id).await;
    let view = h.pending(&attempt, Some(&bearer)).await;
    assert_eq!(status(&view), 403, "{:?}", body(&view));
    assert_eq!(body(&view)["error"], "access_denied");
    let confirm = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&confirm), 403, "{:?}", body(&confirm));
    assert_eq!(body(&confirm)["error"], "access_denied");
    assert_eq!(h.registration(id).await.status, "pending");
}

#[tokio::test]
async fn agent_009_emails_compare_the_way_magnetar_normalizes_them() {
    let h = sqlite_harness().await;
    let stored = magnetar::password::normalize_email("Ana.Muñoz@Example.test");
    let registration = h.service_auth("ANA.MUÑOZ@EXAMPLE.TEST").await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let user_id = h.create_user(&stored, true).await;
    let bearer = h.sign_in(&user_id).await;
    let view = h.pending(&attempt, Some(&bearer)).await;
    assert_eq!(status(&view), 200, "{:?}", body(&view));
}

// AGENT-010 -----------------------------------------------------------------

#[tokio::test]
async fn agent_010_rfc7009_revokes_agent_tokens_and_answers_200_for_an_unknown_one() {
    let h = sqlite_harness().await;
    let registration = h.anonymous().await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("assertion");
    let token = access_token(&h.exchange(assertion).await);
    let revoked_response = h
        .post_form(
            "/oauth/revoke",
            &[("token", &token), ("token_type_hint", "access_token")],
            None,
        )
        .await;
    assert_eq!(status(&revoked_response), 200);
    assert_eq!(h.verify(&token).await, revoked());
    let unknown = h
        .post_form("/oauth/revoke", &[("token", "mat_unknown")], None)
        .await;
    assert_eq!(status(&unknown), 200);
    let events = h.events.of(LifecycleEventKind::AgentTokenRevoked);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].attributes["registration_id"],
        registration["registration_id"].as_str().expect("id")
    );
    assert_eq!(events[0].attributes["client_address"], CLIENT_ADDRESS);
    // The assertion still works: re-exchanging mints a fresh token.
    assert_eq!(status(&h.exchange(assertion).await), 200);
}

struct SlowListener;

#[async_trait::async_trait]
impl magnetar::authorization_server::RevocationListener for SlowListener {
    async fn token_revoked(&self, _revoked: magnetar::authorization_server::RevokedToken) {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

#[tokio::test]
#[tracing_test::traced_test]
async fn agent_010_a_slow_revocation_listener_does_not_hold_the_revocation() {
    let mut options = Options::new();
    let mut server = server_config();
    server.revocation_listener_timeout = std::time::Duration::from_millis(100);
    options.server_config = Some(server);
    options.revocation_listener = Some(Arc::new(SlowListener));
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    let token = access_token(
        &h.exchange(
            registration["identity_assertion"]
                .as_str()
                .expect("assertion"),
        )
        .await,
    );
    let started = std::time::Instant::now();
    let revoked_response = h
        .post_form("/oauth/revoke", &[("token", &token)], None)
        .await;
    assert_eq!(status(&revoked_response), 200);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the endpoint waited {:?} for a listener",
        started.elapsed()
    );
    assert_eq!(
        h.verify(&token).await,
        revoked(),
        "the revocation is stored"
    );
    assert!(logs_contain("revocation listener ran past its timeout"));
}

#[tokio::test]
async fn agent_010_a_security_event_revokes_the_registration_and_its_tokens() {
    let h = sqlite_harness().await;
    let registered = body(&present_subject(&h, "revoked-sub", "revoked@example.test").await);
    let assertion = registered["identity_assertion"]
        .as_str()
        .expect("assertion");
    let first = access_token(&h.exchange(assertion).await);
    let second = access_token(&h.exchange(assertion).await);
    let event = h
        .provider
        .event(&h.provider.revocation_event("revoked-sub"));
    let delivered = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(status(&delivered), 202, "{:?}", body(&delivered));
    assert_eq!(body(&delivered), Value::Null, "202 has no body");
    assert_eq!(h.verify(&first).await, revoked());
    assert_eq!(h.verify(&second).await, revoked());
    assert_eq!(body(&h.exchange(assertion).await)["error"], "invalid_grant");
    let events = h.events.of(LifecycleEventKind::AgentRegistrationRevoked);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].attributes["iss"], PROVIDER);
    assert_eq!(events[0].attributes["sub"], "revoked-sub");
}

#[tokio::test]
async fn agent_010_forged_and_replayed_security_events_are_refused() {
    let h = sqlite_harness().await;
    let registered = body(&present_subject(&h, "kept-sub", "kept@example.test").await);
    let assertion = registered["identity_assertion"]
        .as_str()
        .expect("assertion");
    let token = access_token(&h.exchange(assertion).await);
    let deliver = |event: String| {
        h.post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
    };

    let impostor = Provider::new(PROVIDER, "provider-key-1");
    let forged = deliver(impostor.event(&impostor.revocation_event("kept-sub"))).await;
    assert_eq!(status(&forged), 400);
    assert_eq!(body(&forged)["err"], "authentication_failed");
    let rogue = Provider::new(ROGUE, "rogue-key");
    let mut rogue_claims = rogue.revocation_event("kept-sub");
    rogue_claims.insert("aud".into(), json!(ISSUER));
    let off_list = deliver(rogue.event(&rogue_claims)).await;
    assert_eq!(body(&off_list)["err"], "invalid_issuer");
    let mut elsewhere = h.provider.revocation_event("kept-sub");
    elsewhere.insert("aud".into(), json!("https://elsewhere.example.test"));
    assert_eq!(
        body(&deliver(h.provider.event(&elsewhere)).await)["err"],
        "invalid_audience"
    );
    let wrong_type = h
        .post_bytes(
            "/agent/event/notify",
            "application/json",
            h.provider
                .event(&h.provider.revocation_event("kept-sub"))
                .into_bytes(),
        )
        .await;
    assert_eq!(body(&wrong_type)["err"], "invalid_request");
    assert!(
        matches!(h.verify(&token).await, AccessVerification::Valid(_)),
        "no refused event revoked anything"
    );

    // A genuine event for another subject, delivered twice: the second
    // delivery is a replay.
    let genuine = h
        .provider
        .event(&h.provider.revocation_event("someone-else"));
    assert_eq!(status(&deliver(genuine.clone()).await), 202);
    let replayed = deliver(genuine).await;
    assert_eq!(status(&replayed), 400);
    assert_eq!(body(&replayed)["err"], "invalid_request");
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
}

#[tokio::test]
async fn agent_010_a_user_lists_and_revokes_their_agent_registrations() {
    let h = sqlite_harness().await;
    let (registration, user_id) = h.claimed_anonymous("lister@example.test").await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let tokens = body(
        &h.poll(registration["claim_token"].as_str().expect("token"))
            .await,
    );
    let token = tokens["access_token"].as_str().expect("access").to_owned();
    let connections = h.engine.list_connections(&user_id).await.expect("list");
    assert!(
        connections.client_grants.is_empty(),
        "agent grants are not client grants"
    );
    assert_eq!(connections.agent_registrations.len(), 1);
    let listed = &connections.agent_registrations[0];
    assert_eq!(listed.registration_id, id);
    assert_eq!(listed.registration_type, "anonymous");
    assert_eq!(listed.scopes.len(), 2);

    let stranger = h.create_user("stranger@example.test", true).await;
    assert!(
        !h.engine
            .revoke_registration(&stranger, &id, Some(CLIENT_ADDRESS))
            .await
            .expect("revoke")
    );
    assert!(matches!(
        h.verify(&token).await,
        AccessVerification::Valid(_)
    ));
    assert!(
        h.engine
            .revoke_registration(&user_id, &id, Some(CLIENT_ADDRESS))
            .await
            .expect("revoke")
    );
    assert_eq!(h.verify(&token).await, revoked());
    assert!(
        h.engine
            .list_connections(&user_id)
            .await
            .expect("list")
            .agent_registrations
            .is_empty()
    );
    assert!(
        !h.engine
            .revoke_registration(&user_id, &id, Some(CLIENT_ADDRESS))
            .await
            .expect("revoke")
    );
}

#[tokio::test]
async fn agent_010_revoking_everything_ends_every_agent_credential() {
    let h = sqlite_harness().await;
    let mut tokens = Vec::new();
    let mut assertions = Vec::new();
    for _ in 0..3 {
        let registration = h.anonymous().await;
        let assertion = registration["identity_assertion"]
            .as_str()
            .expect("assertion")
            .to_owned();
        tokens.push(access_token(&h.exchange(&assertion).await));
        assertions.push(assertion);
    }
    let registered = body(&present_subject(&h, "everything", "everything@example.test").await);
    let assertion = registered["identity_assertion"]
        .as_str()
        .expect("assertion")
        .to_owned();
    tokens.push(access_token(&h.exchange(&assertion).await));
    assertions.push(assertion);
    let pending = h.service_auth("pending@example.test").await;

    let revoked_count = h
        .engine
        .revoke_all_agent_credentials()
        .await
        .expect("revoke all");
    assert_eq!(revoked_count, 5);
    for token in &tokens {
        assert_eq!(h.verify(token).await, revoked());
    }
    for assertion in &assertions {
        assert_eq!(body(&h.exchange(assertion).await)["error"], "invalid_grant");
    }
    h.forget_polls().await;
    assert_eq!(
        body(
            &h.poll(pending["claim_token"].as_str().expect("token"))
                .await
        )["error"],
        "expired_token"
    );
    assert_eq!(
        h.engine
            .revoke_all_agent_credentials()
            .await
            .expect("again"),
        0
    );
}

fn faulty() -> (Options, Arc<Faults>) {
    let faults = Arc::new(Faults::default());
    let mut options = Options::new();
    options.faults = Some(faults.clone());
    (options, faults)
}

#[tokio::test]
async fn agent_010_a_revocation_event_survives_a_storage_failure_after_verification() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registered = body(&present_subject(&h, "flaky-event", "flaky.event@example.test").await);
    let token = access_token(
        &h.exchange(
            registered["identity_assertion"]
                .as_str()
                .expect("assertion"),
        )
        .await,
    );
    let event = h
        .provider
        .event(&h.provider.revocation_event("flaky-event"));
    faults.fail("find_by_delegation", 1);
    let failed = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.clone().into_bytes(),
        )
        .await;
    assert_eq!(status(&failed), 500, "{:?}", body(&failed));
    let retried = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.clone().into_bytes(),
        )
        .await;
    assert_eq!(
        status(&retried),
        202,
        "the provider's retry revokes: {:?}",
        body(&retried)
    );
    assert_eq!(h.verify(&token).await, revoked());
    let replayed = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(
        body(&replayed)["err"],
        "invalid_request",
        "after success it is a replay"
    );
}

#[tokio::test]
async fn agent_010_a_retried_event_spares_an_agent_linked_after_it() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let email = "relinked@example.test";
    let first = body(&present_subject(&h, "relink-sub", email).await);
    let first_id = first["registration_id"].as_str().expect("id");
    h.set_registration_time(
        first_id,
        registrations::Column::CreatedAt,
        chrono::Utc::now() - chrono::Duration::hours(1),
    )
    .await;
    let mut claims = h.provider.revocation_event("relink-sub");
    claims.insert("iat".into(), json!(chrono::Utc::now().timestamp() - 60));
    let event = h.provider.event(&claims);
    faults.fail("record_replay", 1);
    let failed = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.clone().into_bytes(),
        )
        .await;
    assert_eq!(status(&failed), 500, "{:?}", body(&failed));
    assert_eq!(h.registration(first_id).await.status, "revoked");

    // The user links the provider again before the provider retries.
    let relink = body(&present_subject(&h, "relink-sub", email).await);
    assert_eq!(relink["error"], "interaction_required");
    use magnetar::storage::UserStore;
    let user = h
        .storage
        .find_by_email(email)
        .await
        .expect("read")
        .expect("the provisioned user");
    let bearer = h.sign_in(&user.user_id).await;
    let attempt = attempt_token(relink["claim"]["verification_uri"].as_str().expect("uri"));
    let code = relink["claim"]["user_code"].as_str().expect("code");
    assert_eq!(status(&h.confirm(&attempt, code, Some(&bearer)).await), 200);
    let token = access_token(
        &h.poll(relink["claim_token"].as_str().expect("claim token"))
            .await,
    );

    let retried = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(status(&retried), 202, "{:?}", body(&retried));
    assert!(
        matches!(h.verify(&token).await, AccessVerification::Valid(_)),
        "the retried event spares the agent linked after it"
    );
}

#[tokio::test]
async fn agent_010_a_failed_grant_revocation_is_finished_by_a_retry() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registered = body(&present_subject(&h, "flaky-grant", "flaky.grant@example.test").await);
    let token = access_token(
        &h.exchange(
            registered["identity_assertion"]
                .as_str()
                .expect("assertion"),
        )
        .await,
    );
    let event = h
        .provider
        .event(&h.provider.revocation_event("flaky-grant"));
    faults.fail("revoke_grant", 2);
    let failed = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.clone().into_bytes(),
        )
        .await;
    assert_eq!(status(&failed), 500, "{:?}", body(&failed));
    let retried = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(status(&retried), 202, "{:?}", body(&retried));
    assert_eq!(
        h.verify(&token).await,
        revoked(),
        "the retry found the grant"
    );
    let id = registered["registration_id"].as_str().expect("id");
    assert_eq!(h.registration(id).await.delegation_key, None);

    // The user's own revocation finishes on a retry too.
    let (registration, user_id) = h.claimed_anonymous("flaky.user@example.test").await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let claimed = body(
        &h.poll(registration["claim_token"].as_str().expect("claim token"))
            .await,
    );
    let token = claimed["access_token"].as_str().expect("access").to_owned();
    faults.fail("revoke_grant", 2);
    assert!(
        h.engine
            .revoke_registration(&user_id, &id, Some(CLIENT_ADDRESS))
            .await
            .is_err()
    );
    h.engine
        .revoke_registration(&user_id, &id, Some(CLIENT_ADDRESS))
        .await
        .expect("the retry");
    assert_eq!(h.verify(&token).await, revoked());
}

#[tokio::test]
async fn agent_009_a_claim_is_confirmed_even_when_the_pre_claim_grant_revocation_fails() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    let pre_claim = access_token(
        &h.exchange(registration["identity_assertion"].as_str().expect("v1"))
            .await,
    );
    let claim_token = registration["claim_token"].as_str().expect("claim token");
    let user_id = h.create_user("flaky.claim@example.test", true).await;
    let bearer = h.sign_in(&user_id).await;
    let started = body(&h.initiate(claim_token, "flaky.claim@example.test").await);
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code");
    faults.fail("revoke_grant", 2);
    let confirmed = h.confirm(&attempt, code, Some(&bearer)).await;
    assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
    assert_eq!(
        h.events.of(LifecycleEventKind::AgentClaimConfirmed).len(),
        1,
        "the committed claim is recorded"
    );
    // The next poll replaces the pre-claim grant, which ends it.
    assert_eq!(status(&h.poll(claim_token).await), 200);
    assert_eq!(h.verify(&pre_claim).await, revoked());
}

#[tokio::test]
async fn agent_011_an_expiry_is_recorded_even_when_its_grant_revocation_fails() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    let id = registration["registration_id"].as_str().expect("id");
    let token = access_token(
        &h.exchange(registration["identity_assertion"].as_str().expect("v1"))
            .await,
    );
    h.set_registration_time(
        id,
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    faults.fail("revoke_grant", 2);
    let _ = h.engine.expire_registrations().await;
    assert_eq!(
        h.events
            .of(LifecycleEventKind::AgentRegistrationExpired)
            .len(),
        1,
        "the committed expiry is recorded"
    );
    h.engine
        .expire_registrations()
        .await
        .expect("the next run finishes");
    assert_eq!(h.verify(&token).await, revoked());
    assert_eq!(
        h.events
            .of(LifecycleEventKind::AgentRegistrationExpired)
            .len(),
        1,
        "and records it once"
    );
}

#[tokio::test]
async fn agent_011_an_expiry_event_survives_a_failed_lookup() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    h.set_registration_time(
        registration["registration_id"].as_str().expect("id"),
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    faults.fail("find_registration", 1);
    let _ = h.engine.expire_registrations().await;
    h.engine.expire_registrations().await.expect("the rerun");
    assert_eq!(
        h.events
            .of(LifecycleEventKind::AgentRegistrationExpired)
            .len(),
        1,
        "the expiry is recorded once, failed lookup or not"
    );
}

#[tokio::test]
async fn agent_011_the_expiry_pass_revokes_only_what_it_expired() {
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let registration = h.anonymous().await;
    let token = access_token(
        &h.exchange(registration["identity_assertion"].as_str().expect("v1"))
            .await,
    );
    h.set_registration_time(
        registration["registration_id"].as_str().expect("id"),
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    // The expiry loses to a concurrent change (a claim, say).
    faults.lose("expire_registration", 1);
    h.engine.expire_registrations().await.expect("the pass");
    assert!(
        matches!(h.verify(&token).await, AccessVerification::Valid(_)),
        "a registration the pass did not expire keeps its grants"
    );
}

#[tokio::test]
async fn agent_009_a_poll_that_loses_the_spend_takes_back_its_token() {
    use magnetar::authorization_server::sql::tokens;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let (options, faults) = faulty();
    let h = sqlite_harness_with(options).await;
    let (registration, _) = h.claimed_anonymous("lost.spend@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    faults.lose("spend_claim_token", 1);
    let lost = h
        .poll(registration["claim_token"].as_str().expect("claim token"))
        .await;
    assert_eq!(body(&lost)["error"], "expired_token");
    let grant = h.agent_grant_ids(id).await.remove(0);
    let minted = tokens::Entity::find()
        .filter(tokens::Column::GrantId.eq(grant))
        .all(&h.db)
        .await
        .expect("read tokens");
    assert!(!minted.is_empty());
    assert!(
        minted.iter().all(|token| token.revoked_at.is_some()),
        "the losing poll's token is taken back"
    );
}

#[tokio::test]
async fn agent_010_registration_revoked_names_the_actor_and_address() {
    let h = sqlite_harness().await;
    let (registration, user_id) = h.claimed_anonymous("actor@example.test").await;
    let id = registration["registration_id"].as_str().expect("id");
    assert!(
        h.engine
            .revoke_registration(&user_id, id, Some("203.0.113.9"))
            .await
            .expect("revoke")
    );
    let events = h.events.of(LifecycleEventKind::AgentRegistrationRevoked);
    assert_eq!(events[0].attributes["actor"], "user");
    assert_eq!(events[0].attributes["client_address"], "203.0.113.9");
    h.anonymous().await;
    h.engine
        .revoke_all_agent_credentials()
        .await
        .expect("revoke all");
    let events = h.events.of(LifecycleEventKind::AgentRegistrationRevoked);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].attributes["actor"], "operator");
}

// AGENT-011 -----------------------------------------------------------------

#[tokio::test]
async fn agent_011_auth_md_matches_the_configuration() {
    let h = sqlite_harness().await;
    let response = h.get("/auth.md", &[], None).await;
    assert_eq!(status(&response), 200);
    assert_eq!(
        header(&response, "content-type").as_deref(),
        Some("text/markdown; charset=utf-8")
    );
    let document = body(&response)
        .as_str()
        .expect("the document text")
        .to_owned();
    for expected in [
        "https://auth.example.test/agent/identity",
        "https://auth.example.test/agent/identity/claim",
        "https://auth.example.test/oauth/token",
        "https://auth.example.test/oauth/revoke",
        "https://auth.example.test/agent/event/notify",
        "### anonymous",
        "### service_auth",
        "### identity_assertion",
        "Acme Agents",
        "`tools:write`: Change data through tools",
        "urn:workos:agent-auth:grant-type:claim",
    ] {
        assert!(document.contains(expected), "missing {expected:?}");
    }
    assert!(!document.contains('\u{2014}') && !document.contains('\u{2013}'));
    assert!(
        !document.contains("invalid_login_hint"),
        "the document names only v0.6.0 codes"
    );

    let mut options = Options::new();
    options.config.identity_types = IdentityTypes {
        anonymous: false,
        service_auth: true,
        identity_assertion: false,
    };
    options.config.trusted_issuers.clear();
    options.config.post_claim_scopes = vec!["tools:read".to_owned()];
    options.config.pre_claim_scopes.clear();
    let h = sqlite_harness_with(options).await;
    let document = h.engine.auth_md();
    for absent in [
        "### anonymous",
        "### identity_assertion",
        "event/notify",
        "Acme Agents",
        "tools:write",
        "`anonymous`, ",
    ] {
        assert!(!document.contains(absent), "names {absent:?}");
    }
    assert!(document.contains("### service_auth"));
}

#[tokio::test]
async fn agent_011_registration_is_limited_per_address_and_per_application() {
    let mut options = Options::new();
    options.config.limits.anonymous = RegistrationLimit::per_hour(5, 7);
    let h = sqlite_harness_with(options).await;
    for _ in 0..5 {
        h.anonymous().await;
    }
    let limited = h
        .post_json("/agent/identity", json!({"type": "anonymous"}))
        .await;
    assert_eq!(status(&limited), 429);
    assert_eq!(body(&limited)["error"], "rate_limited");
    assert!(header(&limited, "retry-after").is_some());
    for address in ["203.0.113.1", "203.0.113.2"] {
        let other = h
            .post_json_from(
                "/agent/identity",
                json!({"type": "anonymous"}),
                Some(address),
            )
            .await;
        assert_eq!(status(&other), 200, "{:?}", body(&other));
    }
    let application_full = h
        .post_json_from(
            "/agent/identity",
            json!({"type": "anonymous"}),
            Some("203.0.113.3"),
        )
        .await;
    assert_eq!(
        status(&application_full),
        429,
        "the application budget is spent"
    );

    // The defaults are the auth.md numbers.
    let h = sqlite_harness().await;
    h.anonymous().await;
    present_subject(&h, "limits", "limits@example.test").await;
    let acquired = h.limiter.acquired.lock().clone();
    let budget = |prefix: &str| {
        acquired
            .iter()
            .find(|(key, _)| key.starts_with(prefix))
            .map(|(_, policy)| (policy.max_requests, policy.window.as_secs()))
            .unwrap_or_else(|| panic!("no {prefix} budget"))
    };
    assert_eq!(budget("agent-auth.anonymous.address:"), (5, 3600));
    assert_eq!(budget("agent-auth.anonymous.application:"), (100, 3600));
    assert_eq!(budget("agent-auth.identity_assertion.address:"), (60, 3600));
    assert_eq!(
        budget("agent-auth.identity_assertion.application:"),
        (1000, 3600)
    );
    // A limiter outage fails closed.
    *h.limiter.failing.lock() = true;
    let outage = h
        .post_json("/agent/identity", json!({"type": "anonymous"}))
        .await;
    assert_eq!(status(&outage), 503);
}

#[tokio::test]
async fn agent_011_the_events_endpoint_is_limited_per_address() {
    let mut options = Options::new();
    options.config.limits.events_per_address = magnetar::abuse::AbusePolicy {
        max_requests: 2,
        window: std::time::Duration::from_secs(60),
    };
    let h = sqlite_harness_with(options).await;
    for subject in ["limited-1", "limited-2"] {
        let event = h.provider.event(&h.provider.revocation_event(subject));
        let accepted = h
            .post_bytes(
                "/agent/event/notify",
                "application/secevent+jwt",
                event.into_bytes(),
            )
            .await;
        assert_eq!(status(&accepted), 202);
    }
    let event = h.provider.event(&h.provider.revocation_event("limited-3"));
    let limited = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(status(&limited), 429, "{:?}", body(&limited));
    assert_eq!(body(&limited)["err"], "rate_limited");
}

#[tokio::test]
async fn agent_011_the_claim_endpoint_is_limited_per_claim_token_and_address() {
    let mut options = Options::new();
    options.config.limits.claim_per_token = magnetar::abuse::AbusePolicy {
        max_requests: 2,
        window: std::time::Duration::from_secs(3600),
    };
    options.config.limits.claim_per_address = magnetar::abuse::AbusePolicy {
        max_requests: 3,
        window: std::time::Duration::from_secs(3600),
    };
    let h = sqlite_harness_with(options).await;
    let first = h.anonymous().await;
    let claim_token = first["claim_token"].as_str().expect("claim token");
    for _ in 0..2 {
        assert_eq!(
            status(&h.initiate(claim_token, "limit@example.test").await),
            200
        );
    }
    let per_token = h.initiate(claim_token, "limit@example.test").await;
    assert_eq!(status(&per_token), 429, "{:?}", body(&per_token));
    assert_eq!(body(&per_token)["error"], "rate_limited");
    let second = h.anonymous().await;
    let other = second["claim_token"].as_str().expect("claim token");
    let per_address = h.initiate(other, "limit@example.test").await;
    assert_eq!(
        status(&per_address),
        429,
        "the address spent its budget: {:?}",
        body(&per_address)
    );
}

#[tokio::test]
async fn agent_011_every_audit_event_is_recorded_with_the_client_address() {
    let h = sqlite_harness().await;
    let (registration, _) = h.claimed_anonymous("audit@example.test").await;
    let claim_token = registration["claim_token"].as_str().expect("token");
    let tokens = body(&h.poll(claim_token).await);
    h.post_form(
        "/oauth/revoke",
        &[("token", tokens["access_token"].as_str().expect("access"))],
        None,
    )
    .await;
    present_subject(&h, "audited", "audited@example.test").await;
    h.post_bytes(
        "/agent/event/notify",
        "application/secevent+jwt",
        h.provider
            .event(&h.provider.revocation_event("audited"))
            .into_bytes(),
    )
    .await;
    let expiring = h.service_auth("expiring@example.test").await;
    h.set_registration_time(
        expiring["registration_id"].as_str().expect("id"),
        registrations::Column::ClaimExpiresAt,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    assert_eq!(h.engine.expire_registrations().await.expect("expire"), 1);

    for kind in [
        LifecycleEventKind::AgentRegistrationCreated,
        LifecycleEventKind::AgentAssertionIssued,
        LifecycleEventKind::AgentTokenIssued,
        LifecycleEventKind::AgentTokenRevoked,
        LifecycleEventKind::AgentClaimRequested,
        LifecycleEventKind::AgentUserCodeMinted,
        LifecycleEventKind::AgentClaimConfirmed,
        LifecycleEventKind::AgentRegistrationExpired,
        LifecycleEventKind::AgentRegistrationRevoked,
    ] {
        let events = h.events.of(kind);
        assert!(!events.is_empty(), "no {kind:?} event");
        for event in events {
            assert_eq!(
                event.attributes.get("client_address").map(String::as_str),
                Some(CLIENT_ADDRESS),
                "{kind:?} lacks the client address: {:?}",
                event.attributes
            );
            assert!(event.attributes.contains_key("registration_id"), "{kind:?}");
            assert!(kind.audit_name().is_some());
        }
    }
    let requested = h.events.of(LifecycleEventKind::AgentClaimRequested);
    assert!(
        requested
            .iter()
            .any(|event| event.attributes["email"] == "audit@example.test")
    );
    let issued = h.events.of(LifecycleEventKind::AgentTokenIssued);
    assert!(
        issued
            .iter()
            .all(|event| event.attributes.contains_key("scope"))
    );
}

#[tokio::test]
async fn agent_011_oversized_bodies_are_refused() {
    let h = sqlite_harness().await;
    let huge = "x".repeat(20 * 1024);
    let identity = h
        .post_json(
            "/agent/identity",
            json!({"type": "anonymous", "padding": huge}),
        )
        .await;
    assert_eq!(status(&identity), 413);
    let claim = h
        .post_json(
            "/agent/identity/claim",
            json!({"claim_token": huge, "email": "a@example.test"}),
        )
        .await;
    assert_eq!(status(&claim), 413);
    let complete = h
        .post_form(
            "/agent/identity/claim/complete",
            &[("claim_attempt_token", &huge), ("user_code", "123456")],
            None,
        )
        .await;
    assert_eq!(status(&complete), 413);
    assert_no_framing(&complete);
    let event = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            huge.clone().into_bytes(),
        )
        .await;
    assert_eq!(status(&event), 413);
    assert_eq!(body(&event)["err"], "invalid_request");
    let attempt = h
        .get(
            "/agent/identity/claim/attempt",
            &[("claim_attempt_token", &huge)],
            None,
        )
        .await;
    assert_eq!(status(&attempt), 413);
    assert!(
        h.events.events.lock().is_empty(),
        "nothing oversized was processed"
    );
}

#[tokio::test]
async fn agent_011_client_endpoints_work_without_a_csrf_token() {
    let h = sqlite_harness().await;
    // No cookie, no CSRF header or field: the client routes answer.
    let registration = h.anonymous().await;
    let started = h
        .initiate(
            registration["claim_token"].as_str().expect("token"),
            "csrf@example.test",
        )
        .await;
    assert_eq!(status(&started), 200);
    let event = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            h.provider
                .event(&h.provider.revocation_event("nobody"))
                .into_bytes(),
        )
        .await;
    assert_eq!(status(&event), 202);
    assert_eq!(status(&h.get("/auth.md", &[], None).await), 200);
}
