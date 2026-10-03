#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use magnetar::oauth::{
    CeremonyBinding, OAuthAuthorizationConfig, OAuthAuthorizationService, OAuthBeginInput,
    OAuthCallbackInput, OAuthIntent, PkcePosture,
};
use magnetar::storage::CeremonyStore;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};

fn service(h: &oauth_harness::OAuthHarness) -> OAuthAuthorizationService {
    OAuthAuthorizationService::new(
        h.storage.clone(),
        h.encryptor.clone(),
        h.limiter.clone(),
        OAuthAuthorizationConfig::default(),
    )
}

#[tokio::test]
async fn state_is_issued_before_redirect_and_readable_by_peek() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();
    assert!(!begun.selector.is_empty());
    assert!(begun.code_challenge.is_some());
    let peeked = h
        .storage
        .peek(&begun.selector, magnetar::oauth::OAUTH_AUTHORIZATION_KIND)
        .await
        .unwrap();
    assert!(peeked.is_some());
}

#[tokio::test]
async fn concurrent_callbacks_on_one_state_have_a_single_winner() {
    let h = oauth_harness::harness().await;
    let svc = Arc::new(service(&h));
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let (left, right) = tokio::join!(
        svc.complete(OAuthCallbackInput {
            state: begun.selector.clone(),
            provider: "github".into(),
            host_session_digest: None,
        }),
        svc.complete(OAuthCallbackInput {
            state: begun.selector.clone(),
            provider: "github".into(),
            host_session_digest: None,
        }),
    );
    let successes = [left.is_ok(), right.is_ok()]
        .into_iter()
        .filter(|ok| *ok)
        .count();
    assert_eq!(successes, 1);
}

#[tokio::test]
async fn wrong_provider_callback_is_rejected_and_consumes_the_ceremony() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let result = svc
        .complete(OAuthCallbackInput {
            state: begun.selector.clone(),
            provider: "google".into(),
            host_session_digest: None,
        })
        .await;
    assert!(result.is_err());

    // The wrong-provider callback is checked only after the atomic
    // consume (deliberate, documented asymmetry with the digest check):
    // the ceremony is burned even though the callback failed. A retry with
    // the correct provider must also fail.
    let retry = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await;
    assert!(retry.is_err());
}

#[tokio::test]
async fn link_actor_binding_survives_with_no_callback_session() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    storage_schema::users::ActiveModel {
        id: Set(42),
        email: Set("oauth-link@example.test".to_owned()),
        auth_epoch: Set(17),
        ..Default::default()
    }
    .insert(&h.db)
    .await
    .unwrap();
    let actor = storage_schema::credential_actor(&h.db, "42", 17, "oauth-link-session").await;
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::Link {
                    actor_user_id: "42".into(),
                },
                actor: Some(actor.clone()),
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "42",
        )
        .await
        .unwrap();

    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    match ceremony.intent {
        OAuthIntent::Link { actor_user_id } => assert_eq!(actor_user_id, "42"),
        OAuthIntent::SignIn => panic!("expected link intent"),
    }
    assert_eq!(ceremony.actor.as_ref(), Some(&actor));
}

#[tokio::test]
async fn session_bound_callback_succeeds_with_matching_digest() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let digest = [7u8; 32];
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::HostSessionDigest(digest),
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: Some(digest),
        })
        .await
        .unwrap();
    assert_eq!(ceremony.provider, "github");
}

#[tokio::test]
async fn mismatched_callback_session_is_rejected_without_mutating_state() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let digest = [7u8; 32];
    let other = [8u8; 32];
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::HostSessionDigest(digest),
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let rejected = svc
        .complete(OAuthCallbackInput {
            state: begun.selector.clone(),
            provider: "github".into(),
            host_session_digest: Some(other),
        })
        .await;
    assert!(rejected.is_err());

    // The ceremony must remain live: the legitimate session can still
    // complete it afterward.
    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: Some(digest),
        })
        .await
        .unwrap();
    assert_eq!(ceremony.provider, "github");
}

#[tokio::test]
async fn explicit_state_only_mode_ignores_absent_digest() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    assert_eq!(ceremony.binding, CeremonyBinding::StateOnly);
}

#[tokio::test]
async fn state_mismatch_is_rejected_and_mutates_nothing() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    // Never issued -- pure caller error.
    let result = svc
        .complete(OAuthCallbackInput {
            state: "never-issued-state".into(),
            provider: "github".into(),
            host_session_digest: None,
        })
        .await;
    assert!(result.is_err());

    // A legitimately issued, still-live ceremony is untouched by the above.
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();
    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    assert_eq!(ceremony.provider, "github");
}

#[tokio::test]
async fn pkce_disabled_mints_no_verifier() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "apple".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Disabled,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();
    assert!(begun.code_challenge.is_none());
    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "apple".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    assert!(ceremony.verifier.is_none());
}

#[tokio::test]
async fn code_challenge_is_base64url_nopad_sha256_of_the_verifier() {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use secrecy::ExposeSecret;
    use sha2::{Digest, Sha256};

    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();
    let challenge = begun.code_challenge.clone().unwrap();

    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    let verifier = ceremony.verifier.unwrap();
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose_secret().as_bytes()));
    assert_eq!(challenge, expected);
}

#[tokio::test]
async fn bound_ceremony_completed_with_an_absent_digest_is_rejected() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let digest = [3u8; 32];
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::HostSessionDigest(digest),
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();

    let rejected = svc
        .complete(OAuthCallbackInput {
            state: begun.selector.clone(),
            provider: "github".into(),
            host_session_digest: None,
        })
        .await;
    assert!(rejected.is_err());

    // Not mutated: the legitimate session can still complete it.
    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: Some(digest),
        })
        .await
        .unwrap();
    assert_eq!(ceremony.provider, "github");
}

#[tokio::test]
async fn nonce_is_issued_when_required_and_readable_through_complete() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "apple".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Disabled,
            true,
            "203.0.113.1",
        )
        .await
        .unwrap();
    let minted_nonce = begun.nonce.clone().expect("a nonce was minted");
    assert!(!minted_nonce.is_empty());

    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "apple".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    // The ceremony hands back the *same* nonce that was minted at begin --
    // the caller passes this into `ProviderResponse::AppleIdToken::nonce`
    // so `resolve_identity` can check it against the ID token's claim.
    assert_eq!(ceremony.nonce, Some(minted_nonce));
}

#[tokio::test]
async fn nonce_is_not_minted_when_not_required() {
    let h = oauth_harness::harness().await;
    let svc = service(&h);
    let begun = svc
        .begin(
            OAuthBeginInput {
                provider: "github".into(),
                intent: OAuthIntent::SignIn,
                actor: None,
                binding: CeremonyBinding::StateOnly,
            },
            PkcePosture::Required,
            false,
            "203.0.113.1",
        )
        .await
        .unwrap();
    assert!(begun.nonce.is_none());
    let ceremony = svc
        .complete(OAuthCallbackInput {
            state: begun.selector,
            provider: "github".into(),
            host_session_digest: None,
        })
        .await
        .unwrap();
    assert!(ceremony.nonce.is_none());
}
