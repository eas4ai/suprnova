//! Every password-adjacent route acquires a purpose-scoped abuse permit for
//! present and absent identities alike, returns the same generic outcome,
//! and fails closed when the limiter backend errors.

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

use magnetar::plugins::abuse_key;
use magnetar::storage::UserStore;
use serde_json::{Value, json};

use factor::{FactorWorld, factor_world, send};
use harness::{LimiterMode, login_request, post_json, register_request};

const PRESENT: &str = "carol@example.test";
const ABSENT: &str = "nobody@example.test";
const PASSWORD: &str = "orange tabby cat";

async fn seeded() -> FactorWorld {
    let world = factor_world().await;
    let reply = send(&world, register_request(PRESENT, PASSWORD)).await;
    assert_eq!(reply.status, 200);
    world.limiter.acquired.lock().clear();
    world
}

fn request_for(route: &str, email: &str) -> magnetar::plugin::WireRequest {
    match route {
        "password.register" => register_request(email, PASSWORD),
        "password.login" => login_request(email, "wrong password"),
        "password.forgot" => post_json("/forgot-password", json!({"email": email})),
        "email.verification-resend" => {
            post_json("/email/verification-notification", json!({"email": email}))
        }
        "magic-link.send" => post_json("/magic-link", json!({"email": email})),
        "passkey.register" => post_json("/passkeys/register/options", json!({"email": email})),
        "passkey.login" => post_json("/passkeys/login/options", json!({"email": email})),
        other => panic!("unknown route {other}"),
    }
}

/// Routes whose responses are anti-enumeration generic for present and
/// absent identities alike.
const GENERIC_ROUTES: &[&str] = &[
    "password.register",
    "password.login",
    "password.forgot",
    "email.verification-resend",
    "magic-link.send",
];

/// Every abuse-limited auth start, including the WebAuthn begins whose
/// responses unavoidably differ by account state (deployed parity).
const ROUTES: &[&str] = &[
    "password.register",
    "password.login",
    "password.forgot",
    "email.verification-resend",
    "magic-link.send",
    "passkey.register",
    "passkey.login",
];

#[tokio::test]
async fn every_route_acquires_one_purpose_key_for_present_and_absent() {
    for route in ROUTES {
        let world = seeded().await;
        for email in [PRESENT, ABSENT] {
            let before = world.limiter.keys().len();
            let _ = send(&world, request_for(route, email)).await;
            let keys = world.limiter.keys();
            assert_eq!(
                keys.len(),
                before + 1,
                "{route} must acquire exactly once for {email}"
            );
            assert_eq!(
                keys.last().unwrap(),
                &abuse_key(route, email),
                "{route} must scope its key by purpose and hashed identity"
            );
        }
    }
}

#[tokio::test]
async fn generic_outcomes_do_not_distinguish_present_from_absent() {
    // For each route, the (status, body) pair for a present identity equals
    // the absent identity's, in every limiter mode.
    for route in GENERIC_ROUTES {
        for mode in [LimiterMode::Allow, LimiterMode::Reject, LimiterMode::Error] {
            let world = seeded().await;
            world.limiter.set_mode(mode);
            let present = send(&world, request_for(route, PRESENT)).await;
            let absent = send(&world, request_for(route, ABSENT)).await;
            assert_eq!(
                present.status, absent.status,
                "{route} status must not reveal existence under {mode:?}"
            );
            assert_eq!(
                present.body, absent.body,
                "{route} body must not reveal existence under {mode:?}"
            );
        }
    }
}

#[tokio::test]
async fn over_budget_requests_answer_429_before_any_domain_work() {
    let world = seeded().await;
    world.limiter.set_mode(LimiterMode::Reject);
    let baseline_mail = world.mail.count();

    for route in ROUTES {
        let reply = send(&world, request_for(route, PRESENT)).await;
        assert_eq!(reply.status, 429, "{route} over budget answers 429");
        assert!(
            reply
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("retry-after")),
            "{route} over budget carries retry timing"
        );
    }
    assert_eq!(
        world.mail.count(),
        baseline_mail,
        "rejected requests reach no mail dispatch"
    );
}

#[tokio::test]
async fn limiter_backend_failure_fails_closed() {
    let world = seeded().await;
    world.limiter.set_mode(LimiterMode::Error);
    let baseline_mail = world.mail.count();
    let expected: Option<Value> = Some(json!({"message": "service unavailable"}));

    for route in ROUTES {
        let reply = send(&world, request_for(route, PRESENT)).await;
        assert_eq!(reply.status, 503, "{route} fails closed on limiter outage");
        assert_eq!(reply.body, expected);
        assert!(reply.grant.is_none());
    }
    assert_eq!(
        world.mail.count(),
        baseline_mail,
        "failed-closed requests perform no domain work"
    );

    // No user was created by the register attempt while failing closed.
    world.limiter.set_mode(LimiterMode::Allow);
    assert!(world.storage.find_by_email(ABSENT).await.unwrap().is_none());
}
