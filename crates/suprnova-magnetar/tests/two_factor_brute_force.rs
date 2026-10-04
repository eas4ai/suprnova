//! Failed second-factor attempts feed 05's lockout accounting under a
//! second-factor identity of their own, separate from password sign-in.

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
async fn failed_challenges_lock_the_second_factor_only() {
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
    let second_factor = magnetar::two_factor::lockout_identity(&user_id);
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

    // Wrong codes count toward the second factor's own lockout budget.
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
            world
                .second_factor_lockout
                .status(&second_factor)
                .await
                .unwrap()
                .failed_attempts,
            attempt
        );
    }
    assert!(
        world
            .second_factor_lockout
            .status(&second_factor)
            .await
            .unwrap()
            .is_locked
    );
    assert!(
        !world.lockout.status(EMAIL).await.unwrap().is_locked,
        "second-factor failures never lock password sign-in"
    );

    // The factor challenge is already bound to a verified primary actor, so
    // it may expose retry timing.
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

    // A password reset proves the mailbox, not the second factor, so it
    // leaves the second-factor lock in place.
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
    assert!(
        world
            .second_factor_lockout
            .status(&second_factor)
            .await
            .unwrap()
            .is_locked
    );

    // An admin unlock of the second factor ends the lock, and the next full
    // sign-in (password + factor) succeeds.
    assert!(world.two_factor.unlock(&user_id).await.unwrap());
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
async fn a_successful_challenge_resets_the_second_factor_counter() {
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
    let second_factor = magnetar::two_factor::lockout_identity(&user_id);
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
        world
            .second_factor_lockout
            .status(&second_factor)
            .await
            .unwrap()
            .failed_attempts,
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
        world
            .second_factor_lockout
            .status(&second_factor)
            .await
            .unwrap()
            .failed_attempts,
        0,
        "success clears the earlier typos"
    );
}

#[tokio::test]
async fn a_password_success_does_not_clear_second_factor_failures() {
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

    let mut evaluated = 0;
    let mut refused = 0;
    for _round in 0..2 {
        let login = send(&world, login_request(EMAIL, PASSWORD)).await;
        assert_eq!(login.status, 200, "the password is right");
        let selector = login.body.unwrap()["challenge_selector"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..2 {
            let reply = send(
                &world,
                post_json(
                    "/two-factor-challenge",
                    json!({"challenge_selector": selector, "code": "000000"}),
                ),
            )
            .await;
            match reply.status {
                401 => evaluated += 1,
                429 => refused += 1,
                status => panic!("unexpected challenge status {status}"),
            }
        }
    }
    assert_eq!(
        evaluated, 3,
        "the second-factor counter locks at its threshold"
    );
    assert_eq!(refused, 1, "the next wrong code is refused");

    // The wrong codes did not touch the password lockout.
    assert_eq!(
        world.lockout.status(EMAIL).await.unwrap().failed_attempts,
        0
    );
    let login = send(&world, login_request(EMAIL, PASSWORD)).await;
    assert_eq!(login.status, 200, "password sign-in is not locked");
}

/// Register the victim with a confirmed second factor under a threshold of
/// three; returns the world, the second-factor key, and the enrollment.
async fn victim_with_second_factor() -> (
    factor::FactorWorld,
    String,
    magnetar::two_factor::EnrollmentResponse,
) {
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
    let key = magnetar::two_factor::lockout_identity(&user_id);
    (world, key, enrollment)
}

/// A password sign-in counts against any string it is given as an address,
/// and anyone can register any string as one. A visitor who knows the
/// victim's password registers the second-factor key itself as a decoy
/// address and signs in to it between wrong codes: the second factor's
/// failures must still lock at the threshold.
#[tokio::test]
async fn a_decoy_account_named_after_the_second_factor_key_cannot_clear_its_failures() {
    let (world, key, _enrollment) = victim_with_second_factor().await;
    let decoy = send(&world, register_request(&key, "decoy password 123")).await;
    assert_eq!(decoy.status, 200);

    let mut evaluated = 0;
    let mut refused = 0;
    for _round in 0..4 {
        let login = send(&world, login_request(EMAIL, PASSWORD)).await;
        let selector = login.body.unwrap()["challenge_selector"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..2 {
            let reply = send(
                &world,
                post_json(
                    "/two-factor-challenge",
                    json!({"challenge_selector": selector, "code": "000000"}),
                ),
            )
            .await;
            match reply.status {
                401 => evaluated += 1,
                429 => refused += 1,
                status => panic!("unexpected challenge status {status}"),
            }
        }
        let decoy_sign_in = send(&world, login_request(&key, "decoy password 123")).await;
        assert_eq!(
            decoy_sign_in.status, 200,
            "the decoy's own password is right"
        );
    }

    assert_eq!(
        (evaluated, refused),
        (3, 5),
        "the decoy's sign-ins clear nothing on the second factor"
    );
}

/// Anyone can fail a password sign-in for the second-factor key as an
/// address, without an account or a password. Those failures must not lock
/// the victim's second factor.
#[tokio::test]
async fn failed_sign_ins_as_the_second_factor_key_do_not_lock_it() {
    let (world, key, enrollment) = victim_with_second_factor().await;
    for _ in 0..3 {
        let reply = send(&world, login_request(&key, "whatever password")).await;
        assert_eq!(reply.status, 401);
    }

    let login = send(&world, login_request(EMAIL, PASSWORD)).await;
    let selector = login.body.unwrap()["challenge_selector"]
        .as_str()
        .unwrap()
        .to_owned();
    let code = totp_code_at(
        &enrollment.otpauth_url,
        Utc::now().timestamp() + STEP_SECONDS,
    );
    let reply = send(
        &world,
        post_json(
            "/two-factor-challenge",
            json!({"challenge_selector": selector, "code": code}),
        ),
    )
    .await;

    assert_eq!(reply.status, 200, "strangers cannot lock the second factor");
}
