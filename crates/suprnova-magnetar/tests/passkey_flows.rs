//! Passkey ceremonies driven end to end by a software authenticator, plus
//! every binding-invariant rejection the deployed suite pins.

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

use chrono::{Duration, Utc};
use magnetar::auth::SignInDecision;
use magnetar::passkey::RegistrationIntent;
use magnetar::plugins::password::PasswordAuthProvider;
use magnetar::sessions::SessionMetadata;
use magnetar::storage::{AuthMethod, MethodStore, PasskeyStore, UserStore};
use secrecy::SecretString;
use serde_json::json;

use factor::{
    FactorWorld, factor_world, passkey_test_origin, send, soft_authenticator, totp_code_now,
};
use harness::post_json;

const EMAIL: &str = "quinn@example.test";

/// Register a passkey for a brand-new email through the wire routes and
/// return (user_id, authenticator).
async fn signup(
    world: &FactorWorld,
    email: &str,
) -> (
    String,
    webauthn_authenticator_rs::WebauthnAuthenticator<
        webauthn_authenticator_rs::softpasskey::SoftPasskey,
    >,
) {
    let mut authenticator = soft_authenticator();
    let begun = send(
        world,
        post_json("/passkeys/register/options", json!({"email": email})),
    )
    .await;
    assert_eq!(
        begun.status, 200,
        "new-email signup needs no authentication"
    );
    let body = begun.body.expect("options body");
    let selector = body["selector"].as_str().unwrap().to_owned();
    let options = serde_json::from_value(body["options"].clone()).expect("valid options");
    let credential = authenticator
        .do_registration(passkey_test_origin(), options)
        .expect("software authenticator completes registration");
    let finished = send(
        world,
        post_json(
            "/passkeys/register",
            json!({
                "selector": selector,
                "email": email,
                "credential": serde_json::to_value(&credential).unwrap(),
            }),
        ),
    )
    .await;
    assert_eq!(finished.status, 200, "valid attestation is accepted");
    let user = world.storage.find_by_email(email).await.unwrap().unwrap();
    (user.user_id, authenticator)
}

async fn login(
    world: &FactorWorld,
    email: &str,
    authenticator: &mut webauthn_authenticator_rs::WebauthnAuthenticator<
        webauthn_authenticator_rs::softpasskey::SoftPasskey,
    >,
) -> harness::Reply {
    let begun = send(
        world,
        post_json("/passkeys/login/options", json!({"email": email})),
    )
    .await;
    assert_eq!(begun.status, 200);
    let body = begun.body.expect("options body");
    let selector = body["selector"].as_str().unwrap().to_owned();
    let options = serde_json::from_value(body["options"].clone()).expect("valid options");
    let credential = authenticator
        .do_authentication(passkey_test_origin(), options)
        .expect("software authenticator completes authentication");
    send(
        world,
        post_json(
            "/passkeys/login",
            json!({
                "selector": selector,
                "email": email,
                "credential": serde_json::to_value(&credential).unwrap(),
            }),
        ),
    )
    .await
}

#[tokio::test]
async fn a_registered_passkey_signs_in_and_advances_its_counterpart_state() {
    let world = factor_world().await;
    let (user_id, mut authenticator) = signup(&world, EMAIL).await;

    let user = world.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert!(
        user.password_hash.is_none(),
        "passkey signup is passwordless"
    );

    let stored_before = world.storage.passkeys_for_user(&user_id).await.unwrap();
    assert_eq!(stored_before.len(), 1);
    let envelope_before =
        magnetar::passkey::envelope::PasskeyEnvelope::parse(&stored_before[0].envelope_json)
            .unwrap();
    assert!(envelope_before.last_used_at().is_none());

    let reply = login(&world, EMAIL, &mut authenticator).await;
    assert_eq!(reply.status, 200);
    let grant = reply.grant.expect("session established through the gate");
    assert_eq!(grant.user_id(), user_id);

    // The stored envelope reflects webauthn state after authentication:
    // last_used_at stamps and the other fields survive untouched.
    let stored_after = world.storage.passkeys_for_user(&user_id).await.unwrap();
    let envelope_after =
        magnetar::passkey::envelope::PasskeyEnvelope::parse(&stored_after[0].envelope_json)
            .unwrap();
    assert!(envelope_after.last_used_at().is_some());
    assert_eq!(
        envelope_after.credential_id_b64().unwrap(),
        envelope_before.credential_id_b64().unwrap()
    );

    // A second sign-in still verifies against the rewritten envelope.
    let again = login(&world, EMAIL, &mut authenticator).await;
    assert_eq!(again.status, 200);
}

#[tokio::test]
async fn existing_account_enrollment_requires_owner_and_fresh_reauth() {
    let world = factor_world().await;
    send(&world, harness::register_request(EMAIL, "orange tabby cat")).await;
    let user = world.storage.find_by_email(EMAIL).await.unwrap().unwrap();

    // Anonymous caller against an existing email: refused (SEC-01).
    let anonymous = send(
        &world,
        post_json("/passkeys/register/options", json!({"email": EMAIL})),
    )
    .await;
    assert_eq!(anonymous.status, 401);

    // Authenticated as a different account: refused.
    let intruder = world
        .passkeys
        .begin_registration(RegistrationIntent {
            email: EMAIL.into(),
            actor_user_id: Some("999999".into()),
            reauthenticated_at: Some(Utc::now()),
        })
        .await
        .unwrap_err();
    assert!(matches!(&intruder,
        magnetar::Error::InvalidInput { field, .. } if field == "actor"
    ));

    // The exact owner without a fresh password confirmation: refused.
    let stale = world
        .passkeys
        .begin_registration(RegistrationIntent {
            email: EMAIL.into(),
            actor_user_id: Some(user.user_id.clone()),
            reauthenticated_at: Some(Utc::now() - Duration::hours(4)),
        })
        .await
        .unwrap_err();
    assert!(matches!(&stale,
        magnetar::Error::InvalidInput { field, .. } if field == "reauth"
    ));

    // Owner with a fresh stamp: the ceremony begins and excludes nothing.
    world
        .passkeys
        .begin_registration(RegistrationIntent {
            email: EMAIL.into(),
            actor_user_id: Some(user.user_id.clone()),
            reauthenticated_at: Some(Utc::now()),
        })
        .await
        .expect("owner with fresh reauth may enroll");
}

#[tokio::test]
async fn finish_mismatch_and_replay_burn_the_ceremony() {
    let world = factor_world().await;
    let mut authenticator = soft_authenticator();
    let begun = world
        .passkeys
        .begin_registration(RegistrationIntent {
            email: EMAIL.into(),
            actor_user_id: None,
            reauthenticated_at: None,
        })
        .await
        .unwrap();
    let credential = authenticator
        .do_registration(passkey_test_origin(), begun.options)
        .unwrap();

    // Finishing for a different email fails AND consumes the ceremony.
    let mismatch = world
        .passkeys
        .finish_registration(&begun.selector, "other@example.test", &credential)
        .await
        .unwrap_err();
    assert!(matches!(&mismatch,
        magnetar::Error::InvalidInput { field, .. } if field == "email"
    ));

    // The correct email now fails too: no retry oracle survives a mismatch.
    let burned = world
        .passkeys
        .finish_registration(&begun.selector, EMAIL, &credential)
        .await
        .unwrap_err();
    assert!(matches!(&burned,
        magnetar::Error::InvalidInput { field, .. } if field == "selector"
    ));
}

#[tokio::test]
async fn a_completed_ceremony_cannot_be_replayed() {
    let world = factor_world().await;
    let (user_id, mut authenticator) = signup(&world, EMAIL).await;
    let begun = send(
        &world,
        post_json("/passkeys/login/options", json!({"email": EMAIL})),
    )
    .await;
    let body = begun.body.unwrap();
    let selector = body["selector"].as_str().unwrap().to_owned();
    let options = serde_json::from_value(body["options"].clone()).unwrap();
    let credential = authenticator
        .do_authentication(passkey_test_origin(), options)
        .unwrap();
    let decision = world
        .passkeys
        .finish_authentication(&selector, EMAIL, &credential, SessionMetadata::default())
        .await
        .unwrap();
    assert!(
        matches!(&decision, SignInDecision::SessionAllowed(grant) if grant.user_id() == user_id)
    );

    // The same selector and assertion replayed: the ceremony is gone.
    let replay = world
        .passkeys
        .finish_authentication(&selector, EMAIL, &credential, SessionMetadata::default())
        .await
        .unwrap_err();
    assert!(matches!(&replay,
        magnetar::Error::InvalidInput { field, .. } if field == "selector"
    ));
}

#[tokio::test]
async fn unknown_accounts_and_empty_accounts_fail_identically() {
    let world = factor_world().await;
    send(&world, harness::register_request(EMAIL, "orange tabby cat")).await;

    let unknown = send(
        &world,
        post_json(
            "/passkeys/login/options",
            json!({"email": "nobody@example.test"}),
        ),
    )
    .await;
    let no_credentials = send(
        &world,
        post_json("/passkeys/login/options", json!({"email": EMAIL})),
    )
    .await;
    assert_eq!(unknown.status, 401);
    assert_eq!(unknown.status, no_credentials.status);
    assert_eq!(unknown.body, no_credentials.body);
}

#[tokio::test]
async fn enrolled_user_is_challenged_after_a_valid_assertion() {
    let world = factor_world().await;
    let (user_id, mut authenticator) = signup(&world, EMAIL).await;
    let enrollment = world.two_factor.enroll(&user_id).await.unwrap();
    world
        .two_factor
        .confirm(&user_id, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();

    let reply = login(&world, EMAIL, &mut authenticator).await;
    assert_eq!(reply.status, 200);
    assert!(reply.grant.is_none(), "no session before the factor");
    assert!(reply.body.unwrap()["challenge_selector"].is_string());
}

#[tokio::test]
async fn last_passkey_removal_is_census_guarded() {
    let world = factor_world().await;
    let (user_id, _authenticator) = signup(&world, EMAIL).await;
    let stored = world.storage.passkeys_for_user(&user_id).await.unwrap();
    let passkey_id = stored[0].passkey_id.clone();

    // The only sign-in method: removal refused atomically.
    let census = world.storage.census(&user_id).await.unwrap();
    assert_eq!(census, 1);
    assert!(
        !world
            .storage
            .remove_method_if_not_last(&user_id, AuthMethod::Passkey(passkey_id.clone()), census)
            .await
            .unwrap()
    );
    assert_eq!(
        world
            .storage
            .passkeys_for_user(&user_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // With a password on file the census is two and the removal wins.
    world
        .provider
        .set_password(&user_id, SecretString::from("orange tabby cat"))
        .await
        .unwrap();
    let census = world.storage.census(&user_id).await.unwrap();
    assert_eq!(census, 2);
    assert!(
        world
            .storage
            .remove_method_if_not_last(&user_id, AuthMethod::Passkey(passkey_id), census)
            .await
            .unwrap()
    );
    assert!(
        world
            .storage
            .passkeys_for_user(&user_id)
            .await
            .unwrap()
            .is_empty()
    );
}
