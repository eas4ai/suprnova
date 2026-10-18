//! AGENT-007 to AGENT-010: agent auth on live PostgreSQL and MySQL/MariaDB.
//!
//! The SQLite suite serializes every statement on one connection, so it
//! cannot show two requests racing in the database. These run the
//! registration flows with values longer than 255 characters, a jti
//! presented to two nodes at once, concurrent claim confirmations and
//! polls, and both revocation layers against real engines, each request on
//! a connection of its own.
//!
//! The tests run on the default single-threaded runtime: the race happens
//! in the database, across pool connections, not in this process, and the
//! gate's inventory of live tests reads the plain `#[tokio::test]` form.
//!
//! PostgreSQL and MySQL are manual, ignored qualification tests against the
//! live backends named by `MAGNETAR_POSTGRES_TEST_URL` and
//! `MAGNETAR_MYSQL_TEST_URL`. Every test creates users and registrations
//! of its own, so repeated runs against one database never collide.

#![cfg(all(
    feature = "agent-auth",
    any(feature = "seaorm-postgres", feature = "seaorm-mysql")
))]

#[path = "fixtures/agent_auth_harness.rs"]
mod harness;

use std::sync::Arc;

use harness::*;
use magnetar::authorization_server::{AccessVerification, InvalidAccessReason};

fn postgres_url() -> String {
    std::env::var("MAGNETAR_POSTGRES_TEST_URL").expect("MAGNETAR_POSTGRES_TEST_URL is required")
}

fn mysql_url() -> String {
    std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required")
}

fn revoked() -> AccessVerification {
    AccessVerification::Invalid(InvalidAccessReason::Revoked)
}

fn token_of(response: &magnetar::plugin::WireResponse) -> String {
    assert_eq!(status(response), 200, "{:?}", body(response));
    body(response)["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned()
}

/// Every registration type end to end, with a subject and an email longer
/// than a `varchar(255)` would take where the schema stores them.
async fn registration_flows(url: &str) {
    let h = build(live_database(url).await, Options::new()).await;

    let anonymous = h.anonymous().await;
    let pre_claim = token_of(
        &h.exchange(anonymous["identity_assertion"].as_str().expect("assertion"))
            .await,
    );
    let email = format!("{}@example.test", unique("live-claim"));
    let user_id = h.create_user(&email, true).await;
    let bearer = h.sign_in(&user_id).await;
    let claim_token = anonymous["claim_token"].as_str().expect("claim token");
    let started = body(&h.initiate(claim_token, &email).await);
    let attempt = attempt_token(
        started["claim_attempt"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = started["claim_attempt"]["user_code"]
        .as_str()
        .expect("code");
    assert_eq!(status(&h.confirm(&attempt, code, Some(&bearer)).await), 200);
    assert_eq!(h.verify(&pre_claim).await, revoked());
    let claimed = h.poll(claim_token).await;
    let post_claim = token_of(&claimed);
    assert!(matches!(
        h.verify(&post_claim).await,
        AccessVerification::Valid(_)
    ));
    assert!(body(&claimed).get("refresh_token").is_none());

    let long_local = "l".repeat(240);
    let long_email = format!("{long_local}@example.test");
    let service = h.service_auth(&long_email).await;
    let row = h
        .registration(service["registration_id"].as_str().expect("id"))
        .await;
    assert_eq!(row.claim_email.as_deref(), Some(long_email.as_str()));

    let long_subject = format!("{}-{}", unique("subject"), "s".repeat(600));
    let registered = h
        .present(&h.provider.id_jag(&h.provider.id_jag_claims(
            &long_subject,
            &format!("{}@example.test", unique("live-jit")),
        )))
        .await;
    assert_eq!(status(&registered), 200, "{:?}", body(&registered));
    let registered = body(&registered);
    let row = h
        .registration(registered["registration_id"].as_str().expect("id"))
        .await;
    assert_eq!(row.subject.as_deref(), Some(long_subject.as_str()));
    let token = token_of(
        &h.exchange(
            registered["identity_assertion"]
                .as_str()
                .expect("assertion"),
        )
        .await,
    );
    let revocation = h
        .post_form("/oauth/revoke", &[("token", &token)], None)
        .await;
    assert_eq!(status(&revocation), 200);
    assert_eq!(h.verify(&token).await, revoked());
}

/// One ID-JAG presented to two nodes at once: exactly one accepts it.
async fn replay_across_nodes(url: &str) {
    let provider = Provider::new(PROVIDER, "provider-key-1");
    let mut first = Options::new();
    first.provider = provider.clone();
    let mut second = Options::new();
    second.provider = provider.clone();
    let nodes = [
        Arc::new(build(live_database(url).await, first).await),
        Arc::new(build(live_database(url).await, second).await),
    ];
    let claims = provider.id_jag_claims(
        &unique("replay-subject"),
        &format!("{}@example.test", unique("replay")),
    );
    let token = provider.id_jag(&claims);
    let mut tasks = Vec::new();
    for round in 0..6 {
        let node = nodes[round % 2].clone();
        let token = token.clone();
        tasks.push(tokio::spawn(async move {
            let response = node.present(&token).await;
            (status(&response), body(&response))
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        let (code, answer) = task.await.expect("a presentation");
        if code == 200 {
            accepted += 1;
        } else {
            assert_eq!(answer["error"], "replay_detected", "{answer:?}");
        }
    }
    assert_eq!(accepted, 1, "exactly one node accepts the jti");
}

/// Concurrent confirmations with the right code claim once; concurrent
/// polls after the claim spend the claim token once; concurrent wrong codes
/// never count past the limit.
async fn one_time_steps(url: &str) {
    let h = Arc::new(build(live_database(url).await, Options::new()).await);
    let email = format!("{}@example.test", unique("race"));
    let registration = h.service_auth(&email).await;
    let user_id = h.create_user(&email, true).await;
    let bearer = h.sign_in(&user_id).await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let code = registration["claim"]["user_code"]
        .as_str()
        .expect("code")
        .to_owned();
    let mut confirmations = Vec::new();
    for _ in 0..6 {
        let (h, attempt, code, bearer) = (h.clone(), attempt.clone(), code.clone(), bearer.clone());
        confirmations.push(tokio::spawn(async move {
            status(&h.confirm(&attempt, &code, Some(&bearer)).await)
        }));
    }
    let mut claimed = 0;
    for task in confirmations {
        if task.await.expect("a confirmation") == 200 {
            claimed += 1;
        }
    }
    assert_eq!(claimed, 1, "exactly one confirmation claims");

    let claim_token = registration["claim_token"]
        .as_str()
        .expect("claim token")
        .to_owned();
    let mut polls = Vec::new();
    for _ in 0..6 {
        let (h, claim_token) = (h.clone(), claim_token.clone());
        polls.push(tokio::spawn(async move {
            let response = h.poll(&claim_token).await;
            (status(&response), body(&response))
        }));
    }
    let mut issued = 0;
    for task in polls {
        let (code, answer) = task.await.expect("a poll");
        if code == 200 {
            issued += 1;
        } else {
            assert!(
                ["slow_down", "expired_token"].contains(&answer["error"].as_str().unwrap_or("")),
                "{answer:?}"
            );
        }
    }
    assert_eq!(issued, 1, "the claim token yields its tokens once");

    // Under the route's poll guard, the claim token's own guard holds in the
    // database: of concurrent spends of one token, exactly one wins.
    use magnetar::agent_auth::{AgentAuthStore, SqlAgentAuthStore};
    let (registration, _) = h
        .claimed_anonymous(&format!("{}@example.test", unique("spend")))
        .await;
    let id = registration["registration_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let hash = sha256_hex(registration["claim_token"].as_str().expect("claim token"));
    let mut spends = Vec::new();
    for _ in 0..6 {
        let store = SqlAgentAuthStore::new(h.db.clone());
        let (id, hash) = (id.clone(), hash.clone());
        spends.push(tokio::spawn(async move {
            store.spend_claim_token(&id, &hash).await.expect("a spend")
        }));
    }
    let mut won = 0;
    for task in spends {
        if task.await.expect("a spend task") {
            won += 1;
        }
    }
    assert_eq!(won, 1, "exactly one spend of the claim token wins");

    let email = format!("{}@example.test", unique("guess"));
    let registration = h.service_auth(&email).await;
    let user_id = h.create_user(&email, true).await;
    let bearer = h.sign_in(&user_id).await;
    let attempt = attempt_token(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("uri"),
    );
    let right = registration["claim"]["user_code"].as_str().expect("code");
    let wrong = if right == "000000" {
        "111111"
    } else {
        "000000"
    };
    let mut guesses = Vec::new();
    for _ in 0..12 {
        let (h, attempt, bearer) = (h.clone(), attempt.clone(), bearer.clone());
        guesses.push(tokio::spawn(async move {
            status(&h.confirm(&attempt, wrong, Some(&bearer)).await)
        }));
    }
    for task in guesses {
        task.await.expect("a guess");
    }
    let row = h
        .registration(registration["registration_id"].as_str().expect("id"))
        .await;
    assert_eq!(row.user_code_failures, 5, "the count stops at the limit");
    assert_eq!(
        status(&h.confirm(&attempt, right, Some(&bearer)).await),
        410,
        "the right code after the limit is refused"
    );
}

/// A revocation event ends a delegation's tokens, and the operator's
/// revoke-everything ends the rest.
async fn revocation_layers(url: &str) {
    let h = build(live_database(url).await, Options::new()).await;
    let subject = unique("event-subject");
    let registered = body(
        &h.present(
            &h.provider.id_jag(
                &h.provider
                    .id_jag_claims(&subject, &format!("{}@example.test", unique("event"))),
            ),
        )
        .await,
    );
    let assertion = registered["identity_assertion"]
        .as_str()
        .expect("assertion");
    let token = token_of(&h.exchange(assertion).await);
    let event = h.provider.event(&h.provider.revocation_event(&subject));
    let delivered = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.clone().into_bytes(),
        )
        .await;
    assert_eq!(status(&delivered), 202, "{:?}", body(&delivered));
    assert_eq!(h.verify(&token).await, revoked());
    let replayed = h
        .post_bytes(
            "/agent/event/notify",
            "application/secevent+jwt",
            event.into_bytes(),
        )
        .await;
    assert_eq!(body(&replayed)["err"], "invalid_request");

    let anonymous = h.anonymous().await;
    let anonymous_token = token_of(
        &h.exchange(anonymous["identity_assertion"].as_str().expect("assertion"))
            .await,
    );
    assert!(
        h.engine
            .revoke_all_agent_credentials()
            .await
            .expect("revoke all")
            >= 1
    );
    assert_eq!(h.verify(&anonymous_token).await, revoked());
    assert_eq!(
        body(
            &h.exchange(anonymous["identity_assertion"].as_str().expect("assertion"))
                .await
        )["error"],
        "invalid_grant"
    );
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_007_postgres_registration_flows_and_tokens() {
    registration_flows(&postgres_url()).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_008_postgres_a_replayed_jti_is_refused_across_two_nodes() {
    replay_across_nodes(&postgres_url()).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_009_postgres_one_time_steps_are_guarded_in_the_database() {
    one_time_steps(&postgres_url()).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_010_postgres_security_events_and_revoke_all() {
    revocation_layers(&postgres_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_007_mysql_registration_flows_and_tokens() {
    registration_flows(&mysql_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_008_mysql_a_replayed_jti_is_refused_across_two_nodes() {
    replay_across_nodes(&mysql_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_009_mysql_one_time_steps_are_guarded_in_the_database() {
    one_time_steps(&mysql_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_010_mysql_security_events_and_revoke_all() {
    revocation_layers(&mysql_url()).await;
}
