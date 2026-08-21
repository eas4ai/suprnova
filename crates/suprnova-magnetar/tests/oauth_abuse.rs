#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::oauth::{
    AutoLinkPolicy, CeremonyBinding, EmailCompletionConfig, EmailCompletionService,
    IdentityOutcome, IdentityResolver, OAuthAuthorizationConfig, OAuthAuthorizationService,
    OAuthBeginInput, OAuthIntent, PkcePosture, VerifiedProviderIdentity,
};
use magnetar::sessions::SessionMetadata;
use oauth_harness::LimiterMode;
use sea_orm::EntityTrait;

fn service(h: &oauth_harness::OAuthHarness) -> OAuthAuthorizationService {
    OAuthAuthorizationService::new(
        h.storage.clone(),
        h.encryptor.clone(),
        h.limiter.clone(),
        OAuthAuthorizationConfig::default(),
    )
}

fn completion(h: &oauth_harness::OAuthHarness) -> EmailCompletionService {
    EmailCompletionService::new(
        h.storage.clone(),
        h.storage.clone(),
        h.first_proof.clone(),
        h.encryptor.clone(),
        h.mail.clone(),
        h.links.clone(),
        h.limiter.clone(),
        EmailCompletionConfig::default(),
    )
}

fn resolver(h: &oauth_harness::OAuthHarness) -> IdentityResolver {
    IdentityResolver::new(
        h.storage.clone(),
        h.storage.clone(),
        h.storage.clone(),
        h.first_proof.clone(),
        h.encryptor.clone(),
        AutoLinkPolicy::ExplicitLinkRequired,
    )
}

#[tokio::test]
async fn begin_consults_the_limiter_identically_for_any_identity() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);

    svc.begin(
        OAuthBeginInput {
            provider: "github".into(),
            intent: OAuthIntent::SignIn,
            binding: CeremonyBinding::StateOnly,
        },
        PkcePosture::Required,
        false,
        "known-actor",
    )
    .await
    .unwrap();
    svc.begin(
        OAuthBeginInput {
            provider: "github".into(),
            intent: OAuthIntent::SignIn,
            binding: CeremonyBinding::StateOnly,
        },
        PkcePosture::Required,
        false,
        "unknown-actor",
    )
    .await
    .unwrap();

    assert_eq!(h.limiter.count(), 2);
}

#[tokio::test]
async fn begin_fails_closed_on_limiter_backend_error_without_minting_a_ceremony() {
    let h = oauth_harness::harness().await;
    h.limiter.set_mode(LimiterMode::Error);
    let svc = service(&h);

    let result = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "any-actor",
        )
        .await;
    assert!(result.is_err());
    assert_eq!(h.limiter.count(), 1);

    // Directly count ceremony rows: the failed acquisition must return
    // before ever reaching ceremony creation, not merely before the
    // caller observes success.
    let ceremonies = storage_schema::ceremonies::Entity::find()
        .all(&h.db)
        .await
        .unwrap();
    assert!(
        ceremonies.is_empty(),
        "begin must not mint a ceremony when the abuse limiter fails closed"
    );
}

#[tokio::test]
async fn begin_rejects_over_budget_with_generic_error_regardless_of_identity() {
    let h = oauth_harness::harness().await;
    h.limiter.set_mode(LimiterMode::Reject);
    let svc = service(&h);

    let present = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "present-actor",
        )
        .await;
    let absent = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "absent-actor",
        )
        .await;
    assert!(present.is_err());
    assert!(absent.is_err());
    assert_eq!(h.limiter.count(), 2);
}

#[tokio::test]
async fn resend_consults_limiter_and_returns_generic_ok_for_present_and_absent_pending() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h);
    let completion = completion(&h);

    let outcome = resolver
        .resolve(
            VerifiedProviderIdentity {
                provider: "x".into(),
                subject: "sub-abuse-present".into(),
                email: None,
                email_verified: false,
                display_name: None,
            },
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
        panic!("expected EmailCompletionRequired");
    };

    let present = completion.resend(&pending_id, "someone@example.test").await;
    let absent = completion
        .resend("never-issued-pending-id", "someone-else@example.test")
        .await;
    assert!(present.is_ok());
    assert!(absent.is_ok());
    assert_eq!(h.limiter.count(), 2);
    // The live pending identity triggered exactly one mail; the absent one
    // triggered none -- but both outcomes and both limiter observations
    // are identical from the caller's side.
    assert_eq!(h.mail.count(), 1);

    // No raw pending id or email reaches the limiter backend as a key.
    for key in h.limiter.keys() {
        assert!(!key.contains(&pending_id));
        assert!(!key.contains("someone@example.test"));
        assert!(!key.contains("someone-else@example.test"));
    }
}

#[tokio::test]
async fn resend_fails_closed_on_limiter_backend_error() {
    let h = oauth_harness::harness().await;
    h.limiter.set_mode(LimiterMode::Error);
    let completion = completion(&h);

    let result = completion
        .resend("some-pending-id", "someone@example.test")
        .await;
    assert!(result.is_err());
    assert_eq!(h.mail.count(), 0);
}

#[tokio::test]
async fn resend_rejects_over_budget() {
    let h = oauth_harness::harness().await;
    h.limiter.set_mode(LimiterMode::Reject);
    let completion = completion(&h);

    let result = completion
        .resend("some-pending-id", "someone@example.test")
        .await;
    assert!(result.is_err());
    assert_eq!(h.mail.count(), 0);
}

#[tokio::test]
async fn resend_budget_is_shared_across_rotated_addresses_for_one_pending_id() {
    let h = oauth_harness::harness().await;
    let completion = completion(&h);

    completion
        .resend("some-pending-id", "victim-one@example.test")
        .await
        .unwrap();
    completion
        .resend("some-pending-id", "victim-two@example.test")
        .await
        .unwrap();
    completion
        .resend("some-pending-id", "victim-three@example.test")
        .await
        .unwrap();

    let keys = h.limiter.keys();
    assert_eq!(keys.len(), 3);
    // Rotating the address must not buy a fresh budget: all three
    // acquisitions land on the identical key, so a real limiter backend
    // would count them against one shared window.
    assert_eq!(keys[0], keys[1]);
    assert_eq!(keys[1], keys[2]);
}
