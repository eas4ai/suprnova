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
use magnetar::broker::{BrokerConfig, BrokerError, RefreshRequest, TokenBroker};
use magnetar::crypto::{CryptoPurpose, Encryptor};
use magnetar::oauth::{InvalidGrantMeaning, OAuthProviderRegistry};
use magnetar::storage::CommitProviderToken;
use secrecy::ExposeSecret;

use broker_harness::{
    BrokerMockProvider, DelayedScriptedHttpTransport, RecordingReuseHook, fast_config, harness,
};

fn registry_with(provider: BrokerMockProvider) -> OAuthProviderRegistry {
    let mut registry = OAuthProviderRegistry::new();
    registry
        .register(Arc::new(provider))
        .expect("mock provider registers");
    registry
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
async fn stale_completion_cannot_overwrite_a_reclaimed_lease() {
    let harness = harness().await;
    let record_id = "linked:reclaim-race";
    harness
        .seed(
            record_id,
            "mock",
            Some("old-access"),
            Some("original-refresh"),
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;

    let registry = registry_with(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    // The first caller through the door becomes leader and is scripted
    // slow; the second reclaims the lease once it expires and is fast.
    transport.push_json_after(
        Duration::from_millis(150),
        200,
        r#"{"access_token":"slow-leader-token","token_type":"Bearer","expires_in":3600,"refresh_token":"slow-leader-refresh"}"#,
    );
    transport.push_json(
        200,
        r#"{"access_token":"fast-reclaimer-token","token_type":"Bearer","expires_in":3600,"refresh_token":"fast-reclaimer-refresh"}"#,
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
    // Give the leader a deterministic head start so it claims first.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let reclaimer_service = service.clone();
    let reclaimer_record_id = record_id.to_owned();
    let reclaimer = tokio::spawn(async move {
        reclaimer_service
            .refresh(RefreshRequest {
                record_id: reclaimer_record_id,
                presented_generation: 0,
            })
            .await
    });

    let leader_result = leader.await.unwrap();
    let reclaimer_result = reclaimer.await.unwrap();

    let reclaimer_token = reclaimer_result.expect("the reclaimer must win and commit");
    assert_eq!(
        reclaimer_token.value.expose_secret(),
        "fast-reclaimer-token"
    );

    match leader_result {
        Err(BrokerError::Terminal { message, .. }) => {
            assert!(
                message.contains("reclaimed"),
                "unexpected terminal message: {message}"
            );
        }
        other => {
            panic!("expected the stale leader's late completion to be discarded, got {other:?}")
        }
    }

    let row = harness.store.read(record_id).await.unwrap().unwrap();
    assert_eq!(row.generation, 1);
    let final_access = harness
        .encryptor
        .decrypt(CryptoPurpose::ProviderToken, &row.access_ciphertext)
        .unwrap();
    assert_eq!(
        String::from_utf8(final_access).unwrap(),
        "fast-reclaimer-token",
        "the reclaimer's committed token must be the one actually stored"
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
        row.claim_id.is_some(),
        "a retriable failure must not clear the claim early"
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
