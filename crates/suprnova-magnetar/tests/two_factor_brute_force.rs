//! Failed second-factor attempts feed 05's lockout accounting - the ported
//! cross-provider brute-force integration contract.

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

use chrono::Utc;
use magnetar::password::LockoutConfig;
use magnetar::plugins::magic_link::RegistrationPolicy;
use magnetar::storage::UserStore;
use magnetar::two_factor::totp::STEP_SECONDS;
use serde_json::json;

use factor::{credential_actor, factor_world_with, send, totp_code_at, totp_code_now};
use harness::{login_request, post_json, register_request};

const EMAIL: &str = "sasha@example.test";
const PASSWORD: &str = "orange tabby cat";

#[tokio::test]
async fn failed_challenges_lock_the_account_across_providers() {
    let config = LockoutConfig {
        max_failed_attempts: 3,
        ..LockoutConfig::default()
    };
    let world = factor_world_with(RegistrationPolicy::Open, config).await;
    send(&world, register_request(EMAIL, PASSWORD)).await;
    let user_id = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .unwrap()
        .user_id;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();

    // A password login opens a challenge.
    let login = send(&world, login_request(EMAIL, PASSWORD)).await;
    let selector = login.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();

    // Wrong codes count toward the shared lockout budget.
    for attempt in 1..=3 {
        let reply = send(
            &world,
            post_json(
                "/two-factor-challenge",
                json!({"challenge_selector": selector, "code": "000000"}),
            ),
        )
        .await;
        assert_eq!(reply.status, 401, "attempt {attempt} fails generically");
        assert_eq!(
            world.lockout.status(EMAIL).await.unwrap().failed_attempts,
            attempt
        );
    }
    assert!(world.lockout.status(EMAIL).await.unwrap().is_locked);

    // The factor challenge is already bound to a verified primary actor, so
    // it may expose retry timing. A fresh password login remains generic.
    let correct = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let refused = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": correct}),
        ),
    )
    .await;
    assert_eq!(refused.status, 429);
    assert!(refused.grant.is_none());
    let login_refused = send(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(login_refused.status, 401);

    // Password reset remains the recovery path: it unlocks, and the next
    // full sign-in (password + factor) succeeds.
    send(
        &world,
        post_json("/forgot-password", json!({"email": EMAIL})),
    )
    .await;
    let reset_link = world.mail.last_payload().unwrap()["reset_link"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = reset_link.split("token=").nth(1).unwrap().to_owned();
    let reset = send(
        &world,
        post_json(
            "/reset-password",
            json!({"token": token, "password": "fresh honest password"}),
        ),
    )
    .await;
    assert_eq!(reset.status, 200);
    assert!(!world.lockout.status(EMAIL).await.unwrap().is_locked);

    let login = send(&world, login_request(EMAIL, "fresh honest password")).await;
    assert_eq!(login.status, 200);
    let selector = login.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();
    // The locked-out attempt above never reached the verifier, so the
    // forward edge is still unclaimed.
    let code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let completed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": code}),
        ),
    )
    .await;
    assert_eq!(completed.status, 200);
    assert!(completed.grant.is_some());
}

#[tokio::test]
async fn a_successful_challenge_resets_the_shared_counter() {
    let config = LockoutConfig {
        max_failed_attempts: 5,
        ..LockoutConfig::default()
    };
    let world = factor_world_with(RegistrationPolicy::Open, config).await;
    send(&world, register_request(EMAIL, PASSWORD)).await;
    let user_id = world
        .storage
        .find_by_email(EMAIL)
        .await
        .unwrap()
        .unwrap()
        .user_id;
    let actor = credential_actor(&world, &user_id).await;
    let enrollment = world.two_factor.enroll(&actor).await.unwrap();
    world
        .two_factor
        .confirm(&actor, &totp_code_now(&enrollment.otpauth_url))
        .await
        .unwrap();

    let login = send(&world, login_request(EMAIL, PASSWORD)).await;
    let selector = login.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();
    for _ in 0..2 {
        send(
            &world,
            post_json(
                "/two-factor-challenge",
                json!({"challenge_selector": selector, "code": "000000"}),
            ),
        )
        .await;
    }
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        2
    );

    let code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let completed = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": code}),
        ),
    )
    .await;
    assert_eq!(completed.status, 200);
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0,
        "success clears the earlier typos"
    );
}
