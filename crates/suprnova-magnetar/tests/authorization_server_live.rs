//! AGENT-005: the authorization server on live PostgreSQL and MySQL/MariaDB.
//!
//! The SQLite suite serializes every statement on one connection, so it
//! cannot show two refreshes racing in the database. These run the code
//! flow, the store guards, eight concurrent uses of one refresh token and
//! a rotation racing a revocation against real engines, each request on a
//! connection of its own.
//!
//! PostgreSQL and MySQL are manual, ignored qualification tests against the
//! live backends named by `MAGNETAR_POSTGRES_TEST_URL` and
//! `MAGNETAR_MYSQL_TEST_URL`. Every test creates users and clients of its
//! own, so repeated runs against one database never collide.

#![cfg(all(
    feature = "authorization-server",
    any(feature = "seaorm-postgres", feature = "seaorm-mysql")
))]

#[path = "fixtures/authorization_server_harness.rs"]
mod harness;

use std::sync::Arc;

use harness::*;
use magnetar::authorization_server::{AccessVerification, InvalidAccessReason};

async fn code_flow_rotation_and_revocation(url: &str) {
    let h = live_harness(url).await;
    let issued = h.issue("tools:read tools:write").await;
    assert!(matches!(
        h.verify(&issued.access_token).await,
        AccessVerification::Valid(_)
    ));
    let rotated = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&rotated), 200, "{:?}", body(&rotated));
    let rotated = body(&rotated);
    let reused = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(body(&reused)["error"], "invalid_grant");
    assert_eq!(
        h.verify(rotated["access_token"].as_str().expect("access"))
            .await,
        AccessVerification::Invalid(InvalidAccessReason::Revoked)
    );
    assert!(
        h.server
            .list_grants(&issued.user_id)
            .await
            .expect("list grants")
            .is_empty()
    );

    let second = h.issue("tools:read").await;
    let revoked = h
        .post_form("/oauth/revoke", &[("token", &second.refresh_token)], None)
        .await;
    assert_eq!(status(&revoked), 200);
    assert_eq!(
        h.verify(&second.access_token).await,
        AccessVerification::Invalid(InvalidAccessReason::Revoked)
    );
}

fn postgres_url() -> String {
    std::env::var("MAGNETAR_POSTGRES_TEST_URL").expect("MAGNETAR_POSTGRES_TEST_URL is required")
}

fn mysql_url() -> String {
    std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required")
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_postgres_code_flow_rotation_and_revocation() {
    code_flow_rotation_and_revocation(&postgres_url()).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_postgres_refresh_rotation_is_guarded_in_the_database() {
    guarded_rotation(&live_harness(&postgres_url()).await).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_postgres_concurrent_refresh_token_reuse_revokes_the_grant() {
    concurrent_refresh_reuse(Arc::new(live_harness(&postgres_url()).await)).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_mysql_code_flow_rotation_and_revocation() {
    code_flow_rotation_and_revocation(&mysql_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_mysql_refresh_rotation_is_guarded_in_the_database() {
    guarded_rotation(&live_harness(&mysql_url()).await).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_mysql_concurrent_refresh_token_reuse_revokes_the_grant() {
    concurrent_refresh_reuse(Arc::new(live_harness(&mysql_url()).await)).await;
}

async fn store_guards(url: &str) {
    let h = live_harness(url).await;
    guarded_redemption(&h).await;
    guarded_decision(&h).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_postgres_code_redemption_and_decisions_are_guarded_in_the_database() {
    store_guards(&postgres_url()).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_postgres_revocation_and_rotation_take_locks_in_one_order() {
    revocation_and_rotation_take_locks_in_one_order(&live_harness(&postgres_url()).await).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_mysql_code_redemption_and_decisions_are_guarded_in_the_database() {
    store_guards(&mysql_url()).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn agent_005_mysql_revocation_and_rotation_take_locks_in_one_order() {
    revocation_and_rotation_take_locks_in_one_order(&live_harness(&mysql_url()).await).await;
}
