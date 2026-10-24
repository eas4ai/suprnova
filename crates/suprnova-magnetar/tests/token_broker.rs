//! Single-process token-broker suite (Task 5:
//! `docs/specs/suprnova-magnetar/11-token-broker.md`): the pre-call
//! lease/CAS protocol, dossier-driven `invalid_grant` handling,
//! `GenerationProvenance` semantics, Retry-After propagation, raw-payload
//! byte fidelity, and encryption at rest.

#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/broker_harness.rs"]
mod broker_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use magnetar::broker::{
    BrokerConfig, BrokerError, RefreshRequest, TokenBroker, TokenBrokerService,
};
use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};
use magnetar::oauth::{InvalidGrantMeaning, OAuthProviderRegistry};
use magnetar::storage::CommitProviderToken;
use magnetar::storage::provider_tokens::{exchange_claim_id, is_exchange_claim_id};
use magnetar::{Error, Result};
use secrecy::ExposeSecret;

use broker_harness::{
    BrokerMockProvider, DelayedScriptedHttpTransport, LegacyDelegatingProviderTokenStore,
    RecordingReuseHook, fast_config, harness,
};

fn registry_with(provider: BrokerMockProvider) -> OAuthProviderRegistry {
    let mut registry = OAuthProviderRegistry::new();
    registry
        .register(Arc::new(provider))
        .expect("mock provider registers");
    registry
}

fn exchange_commit(now: chrono::DateTime<Utc>) -> CommitProviderToken {
    CommitProviderToken {
        access_ciphertext: vec![1],
        refresh_ciphertext: Some(vec![2]),
        raw_payload_ciphertext: vec![3],
        token_type: "Bearer".to_owned(),
        scopes: "read".to_owned(),
        access_expires_at: Some(now + ChronoDuration::hours(1)),
        new_generation: 1,
    }
}

struct RejectRefreshEncryption {
    inner: Arc<AeadEncryptor>,
}

impl Encryptor for RejectRefreshEncryption {
    fn encrypt(&self, purpose: CryptoPurpose, plaintext: &[u8]) -> Result<Vec<u8>> {
        if purpose == CryptoPurpose::RefreshToken {
            return Err(Error::Internal {
                message: "scripted refresh-token encryption failure".to_owned(),
            });
        }
        self.inner.encrypt(purpose, plaintext)
    }

    fn decrypt(&self, purpose: CryptoPurpose, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.inner.decrypt(purpose, ciphertext)
    }
}

// --- lease happy path / freshness-driven refresh -----------------------

#[tokio::test]
async fn access_token_serves_fresh_token_without_a_provider_call() {
    let harness = harness().await;
    let record_id = "linked:fresh";
    harness
        .seed(
            record_id,
            "mock",
            Some("still-fresh"),
            Some("refresh-v0"),
            Some(Utc::now() + ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .access_token(record_id)
        .await
        .expect("fresh token path must not error");
    assert_eq!(token.value.expose_secret(), "still-fresh");
    assert_eq!(
        transport.request_count(),
        0,
        "a fresh token must never reach the provider"
    );
}

#[tokio::test]
async fn access_token_refreshes_expired_token_and_commits_rotated_generation() {
    let harness = harness().await;
    let record_id = "linked:rotating";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"access-v1","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-v1"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .access_token(record_id)
        .await
        .expect("refresh must succeed");
    assert_eq!(token.value.expose_secret(), "access-v1");
    assert!(token.expires_at.is_some());

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert_eq!(
        row.generation, 1,
        "a rotating response must advance the generation"
    );
    let refresh_plain = harness
        .encryptor
        .decrypt(
            CryptoPurpose::RefreshToken,
            row.refresh_ciphertext.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(String::from_utf8(refresh_plain).unwrap(), "refresh-v1");
    assert_eq!(transport.request_count(), 1);
}

#[tokio::test]
async fn non_rotating_response_retains_refresh_token_and_generation() {
    let harness = harness().await;
    let record_id = "linked:non-rotating";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    // No `refresh_token` field: this exchange did not rotate.
    transport.push_json(
        200,
        r#"{"access_token":"access-v1","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .access_token(record_id)
        .await
        .expect("refresh must succeed");
    assert_eq!(token.value.expose_secret(), "access-v1");

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert_eq!(
        row.generation, 0,
        "a non-rotating response must not advance the generation"
    );
    let refresh_plain = harness
        .encryptor
        .decrypt(
            CryptoPurpose::RefreshToken,
            row.refresh_ciphertext.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(refresh_plain).unwrap(),
        "refresh-v0",
        "the previously stored refresh token must be retained untouched"
    );
}

// --- generation CAS: stale winner discards ------------------------------

#[tokio::test]
async fn rotated_success_then_local_failure_never_replays_predecessor_after_claim_expiry() {
    let harness = harness().await;
    let record_id = "linked:rotated-local-failure";
    harness.seed_expired(record_id, "mock").await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"access-v1","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-v1"}"#,
    );
    transport.push_json(
        200,
        r#"{"access_token":"replayed-access","token_type":"Bearer","expires_in":3600,"refresh_token":"replayed-refresh"}"#,
    );
    let config = BrokerConfig {
        single_flight: false,
        provider_call_timeout: Duration::from_millis(20),
        lease_grace: Duration::from_millis(10),
        poll_interval: Duration::from_millis(2),
        ..BrokerConfig::default()
    };
    let service = TokenBrokerService::new(
        harness.store.clone(),
        Arc::new(RejectRefreshEncryption {
            inner: harness.encryptor.clone(),
        }),
        transport.clone(),
        Arc::new(registry_with(BrokerMockProvider::new(
            "mock",
            "https://mock.test/token",
        ))),
        config,
    );

    let first = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(
        matches!(first, Err(BrokerError::Storage(_))),
        "the scripted local encryption failure must surface after the provider succeeds, got {first:?}"
    );
    assert_eq!(transport.request_count(), 1);

    tokio::time::sleep(Duration::from_millis(45)).await;
    let second = tokio::time::timeout(
        Duration::from_millis(100),
        service.refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        }),
    )
    .await
    .expect("an uncertain completed exchange must resolve without waiting on a new provider call");
    assert_eq!(
        transport.request_count(),
        1,
        "the predecessor refresh token must never be replayed after an uncertain rotated exchange"
    );
    assert!(matches!(
        second,
        Err(BrokerError::Revoked {
            reused: false,
            ref record_id,
        }) if record_id == "linked:rotated-local-failure"
    ));
}

#[tokio::test]
async fn abandoned_exchange_started_claim_requires_reauthorization_without_second_request() {
    let harness = harness().await;
    let record_id = "linked:abandoned-exchange";
    harness.seed_expired(record_id, "mock").await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json_after(
        Duration::from_secs(5),
        200,
        r#"{"access_token":"abandoned-access","token_type":"Bearer","expires_in":3600,"refresh_token":"abandoned-refresh"}"#,
    );
    transport.push_json(
        200,
        r#"{"access_token":"replayed-access","token_type":"Bearer","expires_in":3600,"refresh_token":"replayed-refresh"}"#,
    );
    let config = BrokerConfig {
        single_flight: false,
        provider_call_timeout: Duration::from_millis(20),
        lease_grace: Duration::from_millis(10),
        poll_interval: Duration::from_millis(2),
        ..BrokerConfig::default()
    };
    let service = Arc::new(harness.service(
        transport.clone(),
        registry_with(BrokerMockProvider::new("mock", "https://mock.test/token")),
        config,
        Arc::new(RecordingReuseHook::default()),
    ));

    let leader_service = service.clone();
    let leader_record_id = record_id.to_owned();
    let leader = tokio::spawn(async move {
        leader_service
            .refresh(RefreshRequest {
                record_id: leader_record_id,
                presented_generation: 0,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), transport.wait_for_request_count(1))
        .await
        .expect("the leader must begin its provider exchange");
    let in_flight = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(
        in_flight
            .claim_id
            .as_deref()
            .is_some_and(is_exchange_claim_id),
        "the durable exchange marker must be committed before the transport observes the request"
    );
    leader.abort();
    let _ = leader.await;

    tokio::time::sleep(Duration::from_millis(45)).await;
    let second = tokio::time::timeout(
        Duration::from_millis(100),
        service.refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        }),
    )
    .await
    .expect("an abandoned exchange must fail closed without waiting on another provider call");
    assert_eq!(
        transport.request_count(),
        1,
        "an abandoned exchange-started claim must never send a second provider request"
    );
    assert!(matches!(
        second,
        Err(BrokerError::Revoked {
            reused: false,
            ref record_id,
        }) if record_id == "linked:abandoned-exchange"
    ));
}

#[tokio::test]
async fn exchange_claim_transition_is_strict_and_never_reclaimable() {
    let harness = harness().await;
    let record_id = "linked:exchange-transition";
    harness.seed_expired(record_id, "mock").await;
    let store = harness.store.as_ref();
    let now = Utc::now();
    let deadline = now + ChronoDuration::seconds(30);
    assert!(
        store
            .claim(record_id, 0, "ordinary-owner", deadline, now)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_exchange_started(record_id, "wrong-owner", 0, now)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_exchange_started(record_id, "ordinary-owner", 1, now)
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_exchange_started(record_id, "ordinary-owner", 0, now)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_exchange_started(record_id, "ordinary-owner", 0, now)
            .await
            .unwrap(),
        "the ordinary-to-exchange transition must be one-shot"
    );

    let marked = store.read(record_id).await.unwrap().unwrap();
    let expected_exchange_id = exchange_claim_id("ordinary-owner");
    assert_eq!(
        marked.claim_id.as_deref(),
        Some(expected_exchange_id.as_str())
    );
    assert_eq!(marked.claim_deadline, Some(deadline));
    assert!(
        !store
            .heartbeat(
                record_id,
                &expected_exchange_id,
                deadline + ChronoDuration::seconds(30),
            )
            .await
            .unwrap(),
        "heartbeat must not extend an exchange fence"
    );
    assert!(
        !store
            .mark_revoked_by_claim(record_id, &expected_exchange_id, true)
            .await
            .unwrap(),
        "legacy revocation must not bypass the exchange generation check"
    );
    assert!(
        !store
            .revoke_family_if_unrevoked(record_id, deadline)
            .await
            .unwrap(),
        "stale-presenter revocation must not consume an exchange fence"
    );
    assert!(
        !store
            .claim(record_id, 0, "late-reclaimer", deadline, deadline)
            .await
            .unwrap(),
        "an expired exchange-started claim must remain quarantined"
    );

    let expired_id = "linked:expired-pre-exchange";
    harness.seed_expired(expired_id, "mock").await;
    assert!(
        store
            .claim(
                expired_id,
                0,
                "expired-owner",
                now - ChronoDuration::milliseconds(1),
                now,
            )
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_exchange_started(expired_id, "expired-owner", 0, now)
            .await
            .unwrap(),
        "an expired ordinary owner must not begin an exchange"
    );
    assert!(
        store
            .claim(
                expired_id,
                0,
                "replacement-owner",
                now + ChronoDuration::seconds(30),
                now,
            )
            .await
            .unwrap(),
        "an expired pre-exchange claim must remain reclaimable"
    );

    let revoked_id = "linked:revoked-pre-exchange";
    harness.seed_expired(revoked_id, "mock").await;
    assert!(
        store
            .claim(
                revoked_id,
                0,
                "revoked-owner",
                now + ChronoDuration::seconds(30),
                now,
            )
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_revoked_by_claim(revoked_id, "revoked-owner", false)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_exchange_started(revoked_id, "revoked-owner", 0, now)
            .await
            .unwrap(),
        "a revoked family must not begin an exchange"
    );
}

#[tokio::test]
async fn exchange_completion_requires_exact_owner_generation_and_live_family() {
    let harness = harness().await;
    let record_id = "linked:exchange-completion";
    harness.seed_expired(record_id, "mock").await;
    let store = harness.store.as_ref();
    let now = Utc::now();
    let exchange_id = harness
        .start_exchange(
            record_id,
            0,
            "completion-owner",
            now + ChronoDuration::seconds(30),
            now,
        )
        .await;
    assert!(
        !store
            .commit_exchange(
                record_id,
                &exchange_claim_id("wrong-owner"),
                0,
                exchange_commit(now),
            )
            .await
            .unwrap()
    );
    assert!(
        !store
            .commit_exchange(record_id, &exchange_id, 1, exchange_commit(now))
            .await
            .unwrap()
    );
    assert!(
        store
            .commit_exchange(record_id, &exchange_id, 0, exchange_commit(now))
            .await
            .unwrap()
    );
    assert!(
        !store
            .commit_exchange(record_id, &exchange_id, 0, exchange_commit(now))
            .await
            .unwrap(),
        "an exchange completion must be one-shot"
    );

    let committed = store.read(record_id).await.unwrap().unwrap();
    assert_eq!(committed.generation, 1);
    assert!(committed.claim_id.is_none());
    assert!(committed.revoked_at.is_none());
}

#[tokio::test]
async fn exchange_revocation_requires_exact_owner_and_generation() {
    let harness = harness().await;
    let store = harness.store.as_ref();
    let now = Utc::now();

    let revoked_id = "linked:exchange-revocation";
    harness.seed_expired(revoked_id, "mock").await;
    let revoke_exchange_id = harness
        .start_exchange(
            revoked_id,
            0,
            "revoke-owner",
            now + ChronoDuration::seconds(30),
            now,
        )
        .await;
    assert!(
        !store
            .mark_revoked_by_exchange(revoked_id, &exchange_claim_id("wrong-owner"), 0, true,)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_revoked_by_exchange(revoked_id, &revoke_exchange_id, 1, true)
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_revoked_by_exchange(revoked_id, &revoke_exchange_id, 0, true)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn expired_exchange_is_not_reclaimed_while_original_completion_can_commit() {
    let harness = harness().await;
    let record_id = "linked:reclaim-race";
    harness.seed_expired(record_id, "mock").await;

    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    // The first caller through the door becomes leader and is scripted
    // past its lease deadline. Its durable exchange marker must prevent a
    // second predecessor request even while its completion remains valid.
    transport.push_json_after(
        Duration::from_millis(150),
        200,
        r#"{"access_token":"slow-leader-token","token_type":"Bearer","expires_in":3600,"refresh_token":"slow-leader-refresh"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let config = BrokerConfig {
        single_flight: false,
        provider_call_timeout: Duration::from_millis(50),
        lease_grace: Duration::from_millis(20),
        poll_interval: Duration::from_millis(5),
        ..BrokerConfig::default()
    };
    let service = Arc::new(harness.service(transport.clone(), registry, config, hook.clone()));

    let leader_service = service.clone();
    let leader_record_id = record_id.to_owned();
    let leader = tokio::spawn(async move {
        leader_service
            .refresh(RefreshRequest {
                record_id: leader_record_id,
                presented_generation: 0,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), transport.wait_for_request_count(1))
        .await
        .expect("the original leader must begin its provider exchange");
    let follower_service = service.clone();
    let follower_record_id = record_id.to_owned();
    let follower = tokio::spawn(async move {
        follower_service
            .refresh(RefreshRequest {
                record_id: follower_record_id,
                presented_generation: 0,
            })
            .await
    });

    let leader_result = leader.await.unwrap();
    let follower_result = follower.await.unwrap();

    let leader_token = leader_result.expect("the fenced leader may complete after its deadline");
    assert_eq!(leader_token.value.expose_secret(), "slow-leader-token");
    assert!(matches!(
        follower_result,
        Err(BrokerError::Revoked {
            reused: false,
            ref record_id,
        }) if record_id == "linked:reclaim-race"
    ));
    assert_eq!(
        transport.request_count(),
        1,
        "an expired exchange marker must prevent predecessor replay"
    );
    assert_eq!(hook.count(), 0);

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert_eq!(row.generation, 1);
    let final_access = harness
        .encryptor
        .decrypt(CryptoPurpose::ProviderToken, &row.access_ciphertext)
        .unwrap();
    assert_eq!(
        String::from_utf8(final_access).unwrap(),
        "slow-leader-token",
        "the original fenced leader's token must be the one actually stored"
    );
}

#[tokio::test]
async fn expired_external_claim_is_reclaimed_never_treated_as_reuse() {
    let harness = harness().await;
    let record_id = "linked:reclaim-external";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    // Simulate a crashed external leader: a live claim whose deadline has
    // already passed.
    let now = Utc::now();
    let claimed = harness
        .store
        .claim(
            record_id,
            0,
            "crashed-leader",
            now - chrono::Duration::milliseconds(1),
            now,
        )
        .await
        .unwrap();
    assert!(claimed, "seeding the crashed external claim must succeed");

    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"reclaimed-access","token_type":"Bearer","expires_in":3600,"refresh_token":"reclaimed-refresh"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(
        transport.clone(),
        registry,
        fast_config(false),
        hook.clone(),
    );

    let token = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await
        .expect("an expired external claim must be reclaimable, never an attack signal");
    assert_eq!(token.value.expose_secret(), "reclaimed-access");
    assert_eq!(
        hook.count(),
        0,
        "a failed/expired claim must never revoke a healthy family"
    );
}

#[tokio::test]
async fn live_external_claim_before_its_deadline_is_not_reclaimable() {
    let harness = harness().await;
    let record_id = "linked:not-yet-reclaimable";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    let now = Utc::now();
    let claimed = harness
        .store
        .claim(
            record_id,
            0,
            "live-leader",
            now + chrono::Duration::seconds(30),
            now,
        )
        .await
        .unwrap();
    assert!(claimed);

    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default()); // never consulted
    let hook = Arc::new(RecordingReuseHook::default());
    let config = BrokerConfig {
        single_flight: false,
        provider_call_timeout: Duration::from_millis(20),
        lease_grace: Duration::from_millis(10),
        poll_interval: Duration::from_millis(5),
        ..BrokerConfig::default()
    };
    let service = harness.service(transport.clone(), registry, config, hook);

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(
        matches!(result, Err(BrokerError::LeaseTimeout { .. })),
        "a live claim well before its own deadline must never be reclaimed early, got {result:?}"
    );
    assert_eq!(transport.request_count(), 0);
}

#[tokio::test]
async fn legacy_custom_store_defaults_fail_closed_before_provider_io() {
    let harness = harness().await;
    let record_id = "linked:legacy-custom-store";
    harness.seed_expired(record_id, "mock").await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"must-not-send","token_type":"Bearer","expires_in":3600,"refresh_token":"must-not-rotate"}"#,
    );
    let service = TokenBrokerService::new(
        Arc::new(LegacyDelegatingProviderTokenStore::new(
            harness.store.clone(),
        )),
        harness.encryptor.clone(),
        transport.clone(),
        Arc::new(registry_with(BrokerMockProvider::new(
            "mock",
            "https://mock.test/token",
        ))),
        fast_config(false),
    );

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(
        result,
        Err(BrokerError::Storage(Error::Internal { .. }))
    ));
    assert_eq!(
        transport.request_count(),
        0,
        "a custom store without exchange fencing support must fail before provider I/O"
    );
    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(
        row.claim_id
            .as_deref()
            .is_some_and(|claim_id| !is_exchange_claim_id(claim_id)),
        "a failed custom-store fence must leave the pre-exchange claim ordinary"
    );
}

// --- GenerationProvenance: Observed adopts, Asserted revokes as reuse --

#[tokio::test]
async fn asserted_stale_generation_is_reuse_and_fires_hook_exactly_once() {
    let harness = harness().await;
    let record_id = "linked:reuse-asserted";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() + ChronoDuration::hours(1)),
        )
        .await;
    // Advance to generation 1 through the store's own claim/commit CAS,
    // simulating a rotation that already committed.
    let now = Utc::now();
    assert!(
        harness
            .store
            .claim(
                record_id,
                0,
                "advance",
                now + chrono::Duration::seconds(30),
                now
            )
            .await
            .unwrap()
    );
    assert!(
        harness
            .store
            .commit(
                record_id,
                "advance",
                0,
                CommitProviderToken {
                    access_ciphertext: harness
                        .encryptor
                        .encrypt(CryptoPurpose::ProviderToken, b"access-v1")
                        .unwrap(),
                    refresh_ciphertext: Some(
                        harness
                            .encryptor
                            .encrypt(CryptoPurpose::RefreshToken, b"refresh-v1")
                            .unwrap()
                    ),
                    raw_payload_ciphertext: harness
                        .encryptor
                        .encrypt(CryptoPurpose::ProviderToken, b"{}")
                        .unwrap(),
                    token_type: "Bearer".to_owned(),
                    scopes: String::new(),
                    access_expires_at: Some(Utc::now() + ChronoDuration::hours(1)),
                    new_generation: 1,
                },
            )
            .await
            .unwrap()
    );

    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default()); // never reached
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(
        transport.clone(),
        registry,
        fast_config(false),
        hook.clone(),
    );

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    match result {
        Err(BrokerError::Revoked {
            record_id: rid,
            reused,
        }) => {
            assert_eq!(rid, record_id);
            assert!(reused);
        }
        other => panic!("expected reuse revocation, got {other:?}"),
    }
    assert_eq!(hook.count(), 1);
    assert_eq!(hook.calls()[0].0, record_id);
    assert_eq!(
        transport.request_count(),
        0,
        "reuse detection must never reach the provider"
    );

    // A second stale presentation must not re-fire the hook.
    let second = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(
        second,
        Err(BrokerError::Revoked { reused: true, .. })
    ));
    assert_eq!(
        hook.count(),
        1,
        "a second stale presentation must not re-fire the hook"
    );
}

#[tokio::test]
async fn observed_generation_advance_from_a_sibling_instance_is_adopted_never_reuse() {
    let harness = harness().await;
    let record_id = "linked:sibling-adopt";
    harness
        .seed(
            record_id,
            "mock",
            Some("stale-access"),
            Some("shared-refresh"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    // Only ever one provider call: pod A's.
    transport.push_json_after(
        Duration::from_millis(30),
        200,
        r#"{"access_token":"pod-a-token","token_type":"Bearer","expires_in":3600,"refresh_token":"pod-a-refresh"}"#,
    );

    let hook_a = Arc::new(RecordingReuseHook::default());
    let hook_b = Arc::new(RecordingReuseHook::default());
    let config = fast_config(false);
    let pod_a = Arc::new(harness.service(
        transport.clone(),
        registry_with(BrokerMockProvider::new("mock", "https://mock.test/token")),
        config.clone(),
        hook_a.clone(),
    ));
    let pod_b = Arc::new(harness.service(
        transport.clone(),
        registry_with(BrokerMockProvider::new("mock", "https://mock.test/token")),
        config,
        hook_b.clone(),
    ));

    let leader_pod = pod_a.clone();
    let leader_record_id = record_id.to_owned();
    let leader = tokio::spawn(async move { leader_pod.access_token(&leader_record_id).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    let follower_pod = pod_b.clone();
    let follower_record_id = record_id.to_owned();
    let follower =
        tokio::spawn(async move { follower_pod.access_token(&follower_record_id).await });

    let leader_token = leader.await.unwrap().expect("leader refresh must succeed");
    let follower_token = follower
        .await
        .unwrap()
        .expect("the follower must adopt the sibling's committed generation, never error");

    assert_eq!(leader_token.value.expose_secret(), "pod-a-token");
    assert_eq!(follower_token.value.expose_secret(), "pod-a-token");
    assert_eq!(
        transport.request_count(),
        1,
        "exactly one provider call across both sibling instances"
    );
    assert_eq!(hook_a.count(), 0);
    assert_eq!(
        hook_b.count(),
        0,
        "an observed generation advance must never be treated as reuse"
    );

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert_eq!(row.generation, 1);
}

// --- dossier-driven invalid_grant handling ------------------------------

#[tokio::test]
async fn invalid_grant_reuse_dossier_revokes_family_and_fires_hook_once() {
    let harness = harness().await;
    let record_id = "linked:invalid-grant-reuse";
    harness
        .seed(
            record_id,
            "rotator",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    let provider = BrokerMockProvider::new("rotator", "https://mock.test/token")
        .with_invalid_grant_meaning(InvalidGrantMeaning::ReuseOrExternalRevocation);
    let registry = registry_with(provider);
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_status(
        400,
        r#"{"error":"invalid_grant","error_description":"external spend"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(
        transport.clone(),
        registry,
        fast_config(false),
        hook.clone(),
    );

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    match result {
        Err(BrokerError::Revoked { reused, .. }) => assert!(reused),
        other => panic!("expected reuse revocation, got {other:?}"),
    }
    assert_eq!(hook.count(), 1);

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(row.revoked_at.is_some());
    assert_eq!(row.revoked_reused, Some(true));
}

#[tokio::test]
async fn invalid_grant_ordinary_dossier_revokes_without_firing_the_hook() {
    let harness = harness().await;
    let record_id = "linked:invalid-grant-ordinary";
    harness
        .seed(
            record_id,
            "longlived",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    let provider = BrokerMockProvider::new("longlived", "https://mock.test/token")
        .with_invalid_grant_meaning(InvalidGrantMeaning::OrdinaryRevocation);
    let registry = registry_with(provider);
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_status(
        400,
        r#"{"error":"invalid_grant","error_description":"token expired"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(
        transport.clone(),
        registry,
        fast_config(false),
        hook.clone(),
    );

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    match result {
        Err(BrokerError::Revoked { reused, .. }) => assert!(!reused),
        other => panic!("expected ordinary revocation, got {other:?}"),
    }
    assert_eq!(
        hook.count(),
        0,
        "ordinary revocation must never fire the reuse hook"
    );

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(row.revoked_at.is_some());
    assert_eq!(row.revoked_reused, Some(false));
}

#[tokio::test]
async fn provider_declared_non_invalid_grant_remains_quarantined() {
    let harness = harness().await;
    let record_id = "linked:provider-rejection";
    harness.seed_expired(record_id, "mock").await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_status(
        400,
        r#"{"error":"invalid_scope","error_description":"scope rejected"}"#,
    );
    transport.push_json(
        200,
        r#"{"access_token":"retry-access","token_type":"Bearer","expires_in":3600,"refresh_token":"retry-refresh"}"#,
    );
    let service = harness.service(
        transport.clone(),
        registry_with(BrokerMockProvider::new("mock", "https://mock.test/token")),
        fast_config(false),
        Arc::new(RecordingReuseHook::default()),
    );

    let first = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(first, Err(BrokerError::Terminal { .. })));
    let quarantined = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(
        quarantined
            .claim_id
            .as_deref()
            .is_some_and(is_exchange_claim_id)
    );

    tokio::time::sleep(Duration::from_millis(45)).await;
    let retry = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(
        retry,
        Err(BrokerError::Revoked { reused: false, .. })
    ));
    assert_eq!(
        transport.request_count(),
        1,
        "a post-send provider error cannot prove the predecessor remained usable"
    );
}

#[tokio::test]
async fn malformed_provider_response_remains_quarantined() {
    let harness = harness().await;
    let record_id = "linked:malformed-response";
    harness.seed_expired(record_id, "mock").await;

    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_status(200, r#"{"access_token":42,"token_type":"Bearer"}"#);
    transport.push_json(
        200,
        r#"{"access_token":"must-not-replay","token_type":"Bearer","expires_in":3600,"refresh_token":"must-not-rotate"}"#,
    );
    let service = harness.service(
        transport.clone(),
        registry_with(BrokerMockProvider::new("mock", "https://mock.test/token")),
        fast_config(false),
        Arc::new(RecordingReuseHook::default()),
    );

    let first = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(first, Err(BrokerError::Terminal { .. })));

    tokio::time::sleep(Duration::from_millis(45)).await;
    let retry = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    assert!(matches!(
        retry,
        Err(BrokerError::Revoked { reused: false, .. })
    ));
    assert_eq!(
        transport.request_count(),
        1,
        "a malformed post-send response must never make the predecessor reclaimable"
    );
}

// --- upstream error handling: Retry-After -------------------------------

#[tokio::test]
async fn retry_after_propagates_from_upstream_5xx() {
    let harness = harness().await;
    let record_id = "linked:retry-after";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_upstream_unavailable(503, 30);
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let result = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await;
    match result {
        Err(BrokerError::Retriable { retry_after, .. }) => {
            assert_eq!(retry_after, Some(Duration::from_secs(30)));
        }
        other => panic!("expected a retriable failure with retry-after, got {other:?}"),
    }
    // The claim is left to expire on its own bound rather than cleared
    // early, so a genuinely transient failure does not race a concurrent
    // reclaim.
    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert!(
        row.claim_id.as_deref().is_some_and(is_exchange_claim_id),
        "a retriable post-send failure must retain the exchange quarantine"
    );
}

// --- raw-payload byte fidelity ------------------------------------------

#[tokio::test]
async fn raw_payload_round_trips_byte_faithfully() {
    let harness = harness().await;
    let record_id = "linked:raw-payload";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    let raw_body = r#"{"access_token":"raw-access","token_type":"Bearer","expires_in":3600,"refresh_token":"raw-refresh","provider_specific_field":"xyz","nested":{"a":1}}"#;
    transport.push_json(200, raw_body);
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await
        .expect("refresh succeeds");
    assert_eq!(token.value.expose_secret(), "raw-access");

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    let stored_raw = harness
        .encryptor
        .decrypt(CryptoPurpose::ProviderToken, &row.raw_payload_ciphertext)
        .unwrap();
    assert_eq!(
        String::from_utf8(stored_raw).unwrap(),
        raw_body,
        "the raw payload must round-trip byte-faithfully, including fields TokenSuccessResponse drops"
    );
}

// --- encryption at rest ---------------------------------------------------

#[tokio::test]
async fn raw_row_bytes_never_contain_plaintext_tokens() {
    let harness = harness().await;
    let record_id = "linked:crypto-boundary";
    harness
        .seed(
            record_id,
            "mock",
            Some("access-v0"),
            Some("refresh-v0"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"super-secret-access-plaintext","token_type":"Bearer","expires_in":3600,"refresh_token":"super-secret-refresh-plaintext"}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .refresh(RefreshRequest {
            record_id: record_id.to_owned(),
            presented_generation: 0,
        })
        .await
        .expect("refresh succeeds");
    assert_eq!(token.value.expose_secret(), "super-secret-access-plaintext");

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    let access_lossy = String::from_utf8_lossy(&row.access_ciphertext);
    let empty = Vec::new();
    let refresh_lossy = String::from_utf8_lossy(row.refresh_ciphertext.as_ref().unwrap_or(&empty));
    let raw_lossy = String::from_utf8_lossy(&row.raw_payload_ciphertext);
    assert!(!access_lossy.contains("super-secret-access-plaintext"));
    assert!(!refresh_lossy.contains("super-secret-refresh-plaintext"));
    assert!(!raw_lossy.contains("super-secret-access-plaintext"));
    assert!(!raw_lossy.contains("super-secret-refresh-plaintext"));

    // Ciphertext is bound to its purpose as associated data: it is never
    // decryptable under a different purpose.
    assert!(
        harness
            .encryptor
            .decrypt(CryptoPurpose::RefreshToken, &row.access_ciphertext)
            .is_err()
    );
}
