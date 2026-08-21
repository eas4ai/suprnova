//! M2M (client-credentials) token cache suite (Task 5:
//! `docs/specs/suprnova-magnetar/11-token-broker.md`'s "M2M token cache"
//! section): key derivation (provider/client/sorted-scope), jittered
//! pre-expiry freshness at its configured edges, "never serves an expired
//! token," and bounded concurrent refreshes with single-flight both
//! enabled and disabled.

#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/broker_harness.rs"]
mod broker_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration as ChronoDuration, Utc};
use magnetar::broker::cache::needs_refresh;
use magnetar::broker::{M2MCacheConfig, M2MCacheKey, TokenBroker};
use secrecy::ExposeSecret;

use broker_harness::{
    BrokerMockProvider, DelayedScriptedHttpTransport, RecordingReuseHook, fast_config, harness,
};

fn registry_with(provider: BrokerMockProvider) -> magnetar::oauth::OAuthProviderRegistry {
    let mut registry = magnetar::oauth::OAuthProviderRegistry::new();
    registry
        .register(Arc::new(provider))
        .expect("mock provider registers");
    registry
}

// --- cache key: provider/client/sorted-scope ----------------------------

#[test]
fn cache_key_normalizes_scope_order_and_duplicates() {
    let a = M2MCacheKey::new("m2m", "client-4", vec!["b".to_owned(), "a".to_owned()]);
    let b = M2MCacheKey::new(
        "m2m",
        "client-4",
        vec!["a".to_owned(), "b".to_owned(), "b".to_owned()],
    );
    assert_eq!(
        a.record_id(),
        b.record_id(),
        "scope order/duplication must not change the cache key"
    );
}

#[test]
fn cache_key_differs_by_provider_client_or_scope_set() {
    let base = M2MCacheKey::new("m2m", "client", vec!["read".to_owned()]);
    let other_provider = M2MCacheKey::new("other", "client", vec!["read".to_owned()]);
    let other_client = M2MCacheKey::new("m2m", "other-client", vec!["read".to_owned()]);
    let other_scope = M2MCacheKey::new("m2m", "client", vec!["write".to_owned()]);
    assert_ne!(base.record_id(), other_provider.record_id());
    assert_ne!(base.record_id(), other_client.record_id());
    assert_ne!(base.record_id(), other_scope.record_id());
}

// --- jittered pre-expiry freshness, tested at its edges -----------------

#[test]
fn needs_refresh_always_true_for_expired_or_unknown_expiry() {
    let config = M2MCacheConfig::default();
    let now = Utc::now();
    assert!(needs_refresh(
        Some(now - ChronoDuration::seconds(1)),
        now,
        &config,
        0.0
    ));
    assert!(needs_refresh(
        Some(now - ChronoDuration::seconds(1)),
        now,
        &config,
        1.0
    ));
    assert!(
        needs_refresh(None, now, &config, 0.5),
        "an unknown expiry must always need refresh"
    );
}

#[test]
fn needs_refresh_is_bounded_by_the_jitter_window_edges() {
    let config = M2MCacheConfig {
        refresh_before: StdDuration::from_secs(60),
        jitter: StdDuration::from_secs(20),
    };
    let now = Utc::now();
    // 65s out sits inside the jitter band [60s, 80s): due at jitter=1.0
    // (lead 80s > 65s remaining) but not yet due at jitter=0.0 (lead 60s
    // < 65s remaining).
    let expires_in_band = now + ChronoDuration::seconds(65);
    assert!(
        !needs_refresh(Some(expires_in_band), now, &config, 0.0),
        "at jitter_fraction 0.0 the base 60s lead must not yet be due 65s out"
    );
    assert!(
        needs_refresh(Some(expires_in_band), now, &config, 1.0),
        "at jitter_fraction 1.0 the full 80s lead must be due 65s out"
    );

    // Comfortably beyond the whole jitter band: never due regardless of
    // jitter.
    let comfortably_fresh = now + ChronoDuration::seconds(200);
    assert!(!needs_refresh(Some(comfortably_fresh), now, &config, 0.0));
    assert!(!needs_refresh(Some(comfortably_fresh), now, &config, 1.0));

    // Already inside the base lead time even at jitter=0.0: always due.
    let deep_in_window = now + ChronoDuration::seconds(10);
    assert!(needs_refresh(Some(deep_in_window), now, &config, 0.0));
    assert!(needs_refresh(Some(deep_in_window), now, &config, 1.0));
}

// --- broker-level cache behavior -----------------------------------------

#[tokio::test]
async fn client_credentials_cold_start_provisions_and_serves_from_cache() {
    let harness = harness().await;
    let registry = registry_with(BrokerMockProvider::new("m2m", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"m2m-access-1","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let key = M2MCacheKey::new(
        "m2m",
        "client-1",
        vec!["read".to_owned(), "write".to_owned()],
    );
    let token = service
        .client_credentials(key.clone())
        .await
        .expect("cold start must provision and fetch");
    assert_eq!(token.value.expose_secret(), "m2m-access-1");
    assert_eq!(transport.request_count(), 1);

    let cached = service
        .client_credentials(key)
        .await
        .expect("a fresh cache entry must be served without error");
    assert_eq!(cached.value.expose_secret(), "m2m-access-1");
    assert_eq!(
        transport.request_count(),
        1,
        "a fresh cache entry must not re-hit the provider"
    );
}

#[tokio::test]
async fn client_credentials_never_serves_an_expired_token() {
    let harness = harness().await;
    let registry = registry_with(BrokerMockProvider::new("m2m", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    let key = M2MCacheKey::new("m2m", "client-2", vec!["scope".to_owned()]);
    let record_id = key.record_id();
    // Seed an already-expired cache entry directly.
    harness
        .seed(
            &record_id,
            "m2m",
            Some("stale-m2m-access"),
            None,
            Some(Utc::now() - ChronoDuration::hours(1)),
        )
        .await;
    transport.push_json(
        200,
        r#"{"access_token":"fresh-m2m-access","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let token = service
        .client_credentials(key)
        .await
        .expect("an expired cache entry must be refreshed transparently");
    assert_eq!(token.value.expose_secret(), "fresh-m2m-access");
    assert_eq!(transport.request_count(), 1);
}

#[tokio::test]
async fn client_credentials_scopes_never_share_a_cache_entry() {
    let harness = harness().await;
    let registry = registry_with(BrokerMockProvider::new("m2m", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json(
        200,
        r#"{"access_token":"read-token","token_type":"Bearer","expires_in":3600}"#,
    );
    transport.push_json(
        200,
        r#"{"access_token":"write-token","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    let service = harness.service(transport.clone(), registry, fast_config(false), hook);

    let read_key = M2MCacheKey::new("m2m", "client-3", vec!["read".to_owned()]);
    let write_key = M2MCacheKey::new("m2m", "client-3", vec!["write".to_owned()]);
    assert_ne!(read_key.record_id(), write_key.record_id());

    let read_token = service.client_credentials(read_key).await.unwrap();
    let write_token = service.client_credentials(write_key).await.unwrap();
    assert_eq!(read_token.value.expose_secret(), "read-token");
    assert_eq!(write_token.value.expose_secret(), "write-token");
    assert_eq!(
        transport.request_count(),
        2,
        "distinct scope sets must never share a cache entry"
    );
}

// --- bounded concurrent refreshes: single-flight is optimization-only --

#[tokio::test]
async fn concurrent_cold_start_callers_produce_one_provider_call_with_single_flight_disabled() {
    let harness = harness().await;
    let registry = registry_with(BrokerMockProvider::new("m2m", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json_after(
        StdDuration::from_millis(20),
        200,
        r#"{"access_token":"race-token","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    // Correctness must hold with single-flight disabled: the storage CAS
    // loop alone is the arbiter (spec 11's "Single-flight" acceptance
    // criterion: "the CAS suite passes with single-flight disabled").
    let config = fast_config(false);
    let service = Arc::new(harness.service(transport.clone(), registry, config, hook));

    let key = M2MCacheKey::new("m2m", "client-race-disabled", vec!["scope".to_owned()]);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let service = service.clone();
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            service.client_credentials(key).await
        }));
    }
    let mut results = Vec::new();
    for handle in handles {
        results.push(
            handle
                .await
                .unwrap()
                .expect("every concurrent caller must receive a valid token"),
        );
    }
    for token in &results {
        assert_eq!(token.value.expose_secret(), "race-token");
    }
    assert_eq!(
        transport.request_count(),
        1,
        "N concurrent callers must produce exactly one provider call via storage CAS alone"
    );
}

#[tokio::test]
async fn concurrent_cold_start_callers_produce_one_provider_call_with_single_flight_enabled() {
    let harness = harness().await;
    let registry = registry_with(BrokerMockProvider::new("m2m", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json_after(
        StdDuration::from_millis(20),
        200,
        r#"{"access_token":"race-token-coalesced","token_type":"Bearer","expires_in":3600}"#,
    );
    let hook = Arc::new(RecordingReuseHook::default());
    // In-process: N concurrent get-fresh-token calls produce one provider
    // call, this time via `SingleFlight` coalescing rather than storage
    // CAS alone.
    let config = fast_config(true);
    let service = Arc::new(harness.service(transport.clone(), registry, config, hook));

    let key = M2MCacheKey::new("m2m", "client-race-enabled", vec!["scope".to_owned()]);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let service = service.clone();
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            service.client_credentials(key).await
        }));
    }
    let mut results = Vec::new();
    for handle in handles {
        results.push(
            handle
                .await
                .unwrap()
                .expect("every concurrent caller must receive a valid token"),
        );
    }
    for token in &results {
        assert_eq!(token.value.expose_secret(), "race-token-coalesced");
    }
    assert_eq!(transport.request_count(), 1);
}
