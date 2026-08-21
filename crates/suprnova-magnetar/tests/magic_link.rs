//! Magic-link flows: both registration policies, anti-enumeration sends,
//! atomic single-use consumption, verification stamping, and the shared
//! factor gate.

#![cfg(all(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    feature = "seaorm-sqlite"
))]

#[path = "fixtures/factor_harness.rs"]
mod factor;
#[path = "fixtures/password_harness.rs"]
mod harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use magnetar::password::LockoutConfig;
use magnetar::plugin::{Method, WireRequest};
use magnetar::plugins::magic_link::{MagicLinkIssued, MagicLinkService, RegistrationPolicy};
use magnetar::plugins::password::PasswordAuthProvider;
use magnetar::storage::{NewUser, UserRecord, UserStore};
use serde_json::json;

use factor::{FactorWorld, factor_world, factor_world_with, send, totp_code_now};
use harness::post_json;

const EMAIL: &str = "morgan@example.test";

fn link_from_last_mail(world: &FactorWorld) -> String {
    world.mail.last_payload().unwrap()["magic_link"]
        .as_str()
        .expect("magic-link mail carries the link")
        .to_owned()
}

fn token_from(link: &str) -> String {
    link.split("token=")
        .nth(1)
        .expect("link has token")
        .to_owned()
}

fn verify_request(token: &str) -> WireRequest {
    let mut request = WireRequest::new(Method::Get, "/magic-link/verify");
    request.query.insert("token".into(), token.into());
    request
        .headers
        .insert("user-agent".into(), "harness-agent".into());
    request
}

#[tokio::test]
async fn open_policy_first_use_is_a_passwordless_signup() {
    let world = factor_world().await;
    let reply = send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, Some(json!({"status": "ok"})));
    assert_eq!(world.mail.names(), vec!["magic_link"]);

    let user = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .expect("open policy creates the account on first use");
    assert!(
        user.password_hash.is_none(),
        "the created account is passwordless"
    );
    assert!(
        !world.provider.has_password(&user.user_id).await.unwrap(),
        "census sees no password credential"
    );

    // Clicking the link consumes the token, stamps verification, and
    // establishes the session through the gate.
    let token = token_from(&link_from_last_mail(&world));
    let verified = send(&world, verify_request(&token)).await;
    assert_eq!(verified.status, 200);
    let grant = verified
        .grant
        .expect("session established through the gate");
    assert_eq!(grant.user_id(), user.user_id);
    let stamped = world
        .storage
        .find_by_id(&user.user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stamped.email_verified_at.is_some(),
        "consume stamps email_verified_at (FLAGGED hardening)"
    );
}

#[tokio::test]
async fn send_responses_are_generic_in_both_policies() {
    // Open: absent and present addresses both mint and mail.
    let world = factor_world().await;
    let absent = send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let present = send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    assert_eq!(absent.status, present.status);
    assert_eq!(absent.body, present.body);
    assert_eq!(world.mail.count(), 2);

    // Existing-only: an absent address mints nothing and mails nothing
    // behind the byte-identical response.
    let world = factor_world_with(RegistrationPolicy::ExistingOnly, LockoutConfig::default()).await;
    send(
        &world,
        harness::register_request("onfile@example.test", "orange tabby cat"),
    )
    .await;
    let baseline = world.mail.count();
    let present = send(
        &world,
        post_json("/magic-link", json!({"email": "onfile@example.test"})),
    )
    .await;
    let absent = send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    assert_eq!(present.status, absent.status);
    assert_eq!(present.body, absent.body);
    assert_eq!(
        world.mail.count(),
        baseline + 1,
        "present mails exactly once; absent mails nothing"
    );
    assert!(
        world.storage.find_by_email(EMAIL).await.unwrap().is_none(),
        "existing-only never creates accounts"
    );
}

#[tokio::test]
async fn consumption_is_single_use_with_sibling_invalidation() {
    let world = factor_world().await;
    send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let first = token_from(&link_from_last_mail(&world));
    send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let second = token_from(&link_from_last_mail(&world));

    let consumed = send(&world, verify_request(&second)).await;
    assert_eq!(consumed.status, 200);

    // The consumed token cannot replay, and consuming one invalidates the
    // outstanding sibling (02's discipline).
    let replay = send(&world, verify_request(&second)).await;
    let sibling = send(&world, verify_request(&first)).await;
    let garbage = send(&world, verify_request("not-a-token")).await;
    for reply in [&replay, &sibling, &garbage] {
        assert_eq!(reply.status, 401);
        assert_eq!(
            reply.body, garbage.body,
            "consumed, sibling-invalidated, expired, and unknown tokens are indistinguishable"
        );
        assert!(reply.grant.is_none());
    }
}

#[tokio::test]
async fn enrolled_user_gets_a_challenge_before_any_session() {
    let world = factor_world().await;
    send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let user = world.storage.find_by_email(EMAIL).await.unwrap().unwrap();

    // Enroll and confirm 2FA directly through the service.
    let enrollment = world.two_factor.enroll(&user.user_id).await.unwrap();
    world
        .two_factor
        .confirm(&user.user_id, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();

    send(&world, post_json("/magic-link", json!({"email": EMAIL}))).await;
    let token = token_from(&link_from_last_mail(&world));
    let interrupted = send(&world, verify_request(&token)).await;
    assert_eq!(interrupted.status, 200);
    assert!(interrupted.grant.is_none(), "no session before the factor");
    let selector = interrupted.body.unwrap()["challenge_selector"]
        .as_str()
        .expect("challenge selector returned")
        .to_owned();

    // The next timestep's code completes the challenge (the confirm call
    // above already claimed the current step; the matched-step contract
    // refuses a same-or-earlier step, so step forward once).
    let next_code = factor::totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + magnetar::two_factor::totp::STEP_SECONDS,
    );
    let completed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": next_code}),
        ),
    )
    .await;
    assert_eq!(completed.status, 200);
    assert_eq!(
        completed.grant.expect("exactly one session").user_id(),
        user.user_id
    );
}

/// A user binding that cannot represent passwordless accounts: it surfaces
/// a credential even when none was stored.
struct BrokenBinding(Arc<magnetar::storage::SeaOrmStorage<storage_schema::StorageSchema>>);

#[async_trait::async_trait]
impl UserStore for BrokenBinding {
    async fn find_by_email(&self, email: &str) -> magnetar::Result<Option<UserRecord>> {
        self.0.find_by_email(email).await
    }
    async fn find_by_id(&self, user_id: &str) -> magnetar::Result<Option<UserRecord>> {
        self.0.find_by_id(user_id).await
    }
    async fn create_user(&self, input: NewUser) -> magnetar::Result<UserRecord> {
        let mut record = self.0.create_user(input).await?;
        record.password_hash = Some("$2b$04$broken-binding-junk".into());
        Ok(record)
    }
    async fn set_password_hash(&self, user_id: &str, hash: &str) -> magnetar::Result<()> {
        self.0.set_password_hash(user_id, hash).await
    }
    async fn mark_email_verified(&self, user_id: &str, at: DateTime<Utc>) -> magnetar::Result<()> {
        self.0.mark_email_verified(user_id, at).await
    }
    async fn set_locked_at_by_email(
        &self,
        email: &str,
        at: Option<DateTime<Utc>>,
    ) -> magnetar::Result<()> {
        self.0.set_locked_at_by_email(email, at).await
    }
}

#[tokio::test]
async fn open_mode_refuses_a_binding_that_cannot_represent_passwordless() {
    let world = factor_world().await;
    let service = MagicLinkService::new(
        Arc::new(BrokenBinding(world.storage.clone())),
        world.storage.clone(),
        world.gate.clone(),
        RegistrationPolicy::Open,
    );
    let error = service.issue("victim@example.test").await.unwrap_err();
    assert!(matches!(error, magnetar::Error::Internal { .. }));

    // A compliant NOT-NULL binding maps its sentinel back to None, so the
    // real store issues normally.
    let issued = world.magic.issue(EMAIL).await.unwrap();
    assert!(matches!(issued, MagicLinkIssued::Minted(_)));
}
