#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::auth::VerifiedPrincipal;
use magnetar::oauth::{
    AutoLinkPolicy, EmailCompletionConfig, EmailCompletionService, IdentityOutcome,
    IdentityResolver, OAUTH_EMAIL_COMPLETION_PURPOSE, OAuthIntent, VerifiedProviderIdentity,
};
use magnetar::sessions::SessionMetadata;
use magnetar::storage::{LinkedAccountStore, PresentedToken, TokenStore, UserStore};

fn resolver(h: &oauth_harness::OAuthHarness, policy: AutoLinkPolicy) -> IdentityResolver {
    IdentityResolver::new(
        h.storage.clone(),
        h.storage.clone(),
        h.storage.clone(),
        h.encryptor.clone(),
        policy,
    )
}

fn completion(h: &oauth_harness::OAuthHarness) -> EmailCompletionService {
    EmailCompletionService::new(
        h.storage.clone(),
        h.storage.clone(),
        h.storage.clone(),
        h.storage.clone(),
        h.encryptor.clone(),
        h.mail.clone(),
        h.links.clone(),
        h.limiter.clone(),
        EmailCompletionConfig::default(),
    )
}

fn identity(
    provider: &str,
    subject: &str,
    email: Option<&str>,
    verified: bool,
) -> VerifiedProviderIdentity {
    VerifiedProviderIdentity {
        provider: provider.to_owned(),
        subject: subject.to_owned(),
        email: email.map(str::to_owned),
        email_verified: verified,
        display_name: None,
    }
}

#[tokio::test]
async fn known_provider_subject_signs_in() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let identity = identity("github", "sub-1", Some("known@example.test"), true);
    let outcome = resolver
        .resolve(
            identity.clone(),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let user_id = match outcome {
        IdentityOutcome::Create { user_id, .. } => user_id,
        _ => panic!("expected Create on first sign-in"),
    };

    let outcome2 = resolver
        .resolve(identity, OAuthIntent::SignIn, SessionMetadata::default())
        .await
        .unwrap();
    match outcome2 {
        IdentityOutcome::SignIn(principal) => {
            let principal: VerifiedPrincipal = principal;
            assert_eq!(principal.user_id(), user_id);
        }
        _ => panic!("expected SignIn on repeat login"),
    }
}

#[tokio::test]
async fn unknown_verified_no_match_creates_user_and_link() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let outcome = resolver
        .resolve(
            identity("github", "sub-2", Some("Fresh@Example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let user_id = match outcome {
        IdentityOutcome::Create {
            user_id,
            provider_account_id,
        } => {
            assert_eq!(provider_account_id, "sub-2");
            user_id
        }
        _ => panic!("expected Create"),
    };
    let user = h.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert_eq!(user.email, "fresh@example.test");
    let account = h
        .storage
        .find_by_provider_subject("github", "sub-2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.user_id, user_id);
}

#[tokio::test]
async fn verified_email_collision_defaults_to_explicit_link_required() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    // Seed an existing account with that email via a Create resolution.
    resolver
        .resolve(
            identity("google", "sub-existing", Some("collide@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();

    let outcome = resolver
        .resolve(
            identity("github", "sub-3", Some("collide@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    match outcome {
        IdentityOutcome::ExplicitLinkRequired { normalized_email } => {
            assert_eq!(normalized_email, "collide@example.test");
        }
        _ => panic!("expected ExplicitLinkRequired"),
    }
    // No linked account was created for the colliding attempt.
    assert!(
        h.storage
            .find_by_provider_subject("github", "sub-3")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn auto_link_policy_links_matching_verified_email() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::AutoLink);

    resolver
        .resolve(
            identity(
                "google",
                "sub-existing2",
                Some("autolink@example.test"),
                true,
            ),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();

    let outcome = resolver
        .resolve(
            identity("github", "sub-4", Some("autolink@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    match outcome {
        IdentityOutcome::Link {
            provider_account_id,
            ..
        } => assert_eq!(provider_account_id, "sub-4"),
        _ => panic!("expected Link under AutoLink policy"),
    }
}

#[tokio::test]
async fn unverified_email_never_links_and_never_matches() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::AutoLink);

    resolver
        .resolve(
            identity("google", "sub-existing3", Some("victim@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();

    let outcome = resolver
        .resolve(
            identity("github", "sub-5", Some("victim@example.test"), false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    match outcome {
        IdentityOutcome::EmailCompletionRequired { .. } => {}
        _ => {
            panic!("expected EmailCompletionRequired for unverified email, got a different outcome")
        }
    }
    // Never attached to the existing victim account.
    assert!(
        h.storage
            .find_by_provider_subject("github", "sub-5")
            .await
            .unwrap()
            .is_none()
    );
    let _ = outcome;
}

#[tokio::test]
async fn no_email_provider_requires_completion() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let outcome = resolver
        .resolve(
            identity("x", "sub-6", None, false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let pending_id = match outcome {
        IdentityOutcome::EmailCompletionRequired { pending_id } => pending_id,
        _ => panic!("expected EmailCompletionRequired"),
    };
    assert!(!pending_id.is_empty());
}

#[tokio::test]
async fn email_completion_consumes_once_and_creates_user() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);
    let completion = completion(&h);

    let outcome = resolver
        .resolve(
            identity("x", "sub-7", None, false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
        panic!("expected EmailCompletionRequired");
    };

    completion
        .request(&pending_id, "new-owner@example.test")
        .await
        .unwrap();
    assert_eq!(h.mail.count(), 1);
    let link = h.mail.last_payload().unwrap();
    let link = link["completion_link"].as_str().unwrap().to_owned();
    let token = link.split("token=").nth(1).unwrap().to_owned();

    let outcome = completion.consume(&token).await.unwrap();
    let user_id = match outcome {
        IdentityOutcome::Create {
            user_id,
            provider_account_id,
        } => {
            assert_eq!(provider_account_id, "sub-7");
            user_id
        }
        _ => panic!("expected Create"),
    };
    let user = h.storage.find_by_id(&user_id).await.unwrap().unwrap();
    assert_eq!(user.email, "new-owner@example.test");

    // Replay: the token is gone, and so is the pending identity.
    let replay = completion.consume(&token).await;
    assert!(replay.is_err());
}

#[tokio::test]
async fn email_completion_collision_returns_explicit_link_required() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);
    let completion = completion(&h);

    // Seed an existing user with the address the completion will target.
    resolver
        .resolve(
            identity("google", "sub-existing4", Some("taken@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();

    let outcome = resolver
        .resolve(
            identity("x", "sub-8", None, false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
        panic!("expected EmailCompletionRequired");
    };

    completion
        .request(&pending_id, "taken@example.test")
        .await
        .unwrap();
    let link = h.mail.last_payload().unwrap();
    let link = link["completion_link"].as_str().unwrap().to_owned();
    let token = link.split("token=").nth(1).unwrap().to_owned();

    let outcome = completion.consume(&token).await.unwrap();
    match outcome {
        IdentityOutcome::ExplicitLinkRequired { normalized_email } => {
            assert_eq!(normalized_email, "taken@example.test");
        }
        _ => panic!("expected ExplicitLinkRequired on collision"),
    }
    // No account was created or attached for the provider identity.
    assert!(
        h.storage
            .find_by_provider_subject("x", "sub-8")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn email_completion_resend_is_generic_for_present_and_absent_pending() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);
    let completion = completion(&h);

    let outcome = resolver
        .resolve(
            identity("x", "sub-9", None, false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
        panic!("expected EmailCompletionRequired");
    };

    let ok_present = completion.resend(&pending_id, "present@example.test").await;
    let ok_absent = completion
        .resend("never-issued-pending-id", "absent@example.test")
        .await;
    assert!(ok_present.is_ok());
    assert!(ok_absent.is_ok());
    // Only the live pending identity actually triggered mail.
    assert_eq!(h.mail.count(), 1);
}

#[tokio::test]
async fn link_intent_attaches_identity_to_the_begin_time_actor() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let created = resolver
        .resolve(
            identity("github", "sub-link-1", Some("linker@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let actor_user_id = match created {
        IdentityOutcome::Create { user_id, .. } => user_id,
        _ => panic!("expected Create to seed the actor"),
    };

    let outcome = resolver
        .resolve(
            identity("gitlab", "sub-link-2", None, false),
            OAuthIntent::Link {
                actor_user_id: actor_user_id.clone(),
            },
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    match outcome {
        IdentityOutcome::Link {
            actor_user_id: linked_to,
            provider_account_id,
        } => {
            assert_eq!(linked_to, actor_user_id);
            assert_eq!(provider_account_id, "sub-link-2");
        }
        _ => panic!("expected Link"),
    }
    let account = h
        .storage
        .find_by_provider_subject("gitlab", "sub-link-2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.user_id, actor_user_id);
}

#[tokio::test]
async fn link_intent_is_idempotent_for_the_same_actor() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let created = resolver
        .resolve(
            identity("github", "sub-idem-1", Some("idem@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let actor_user_id = match created {
        IdentityOutcome::Create { user_id, .. } => user_id,
        _ => panic!("expected Create to seed the actor"),
    };

    let link_intent = OAuthIntent::Link {
        actor_user_id: actor_user_id.clone(),
    };
    let identity = identity("gitlab", "sub-idem-2", None, false);
    resolver
        .resolve(
            identity.clone(),
            link_intent.clone(),
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let second = resolver
        .resolve(identity, link_intent, SessionMetadata::default())
        .await
        .unwrap();
    match second {
        IdentityOutcome::Link {
            actor_user_id: linked_to,
            ..
        } => {
            assert_eq!(linked_to, actor_user_id);
        }
        _ => panic!("expected idempotent Link on repeat"),
    }
}

#[tokio::test]
async fn link_intent_conflicts_when_identity_belongs_to_a_different_user() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let first = resolver
        .resolve(
            identity("github", "sub-conflict-a", Some("first@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let first_user = match first {
        IdentityOutcome::Create { user_id, .. } => user_id,
        _ => panic!("expected Create"),
    };
    let second = resolver
        .resolve(
            identity(
                "github",
                "sub-conflict-b",
                Some("second@example.test"),
                true,
            ),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let second_user = match second {
        IdentityOutcome::Create { user_id, .. } => user_id,
        _ => panic!("expected Create"),
    };

    // Attach a shared provider identity to the first user via an explicit link.
    resolver
        .resolve(
            identity("gitlab", "sub-shared", None, false),
            OAuthIntent::Link {
                actor_user_id: first_user,
            },
            SessionMetadata::default(),
        )
        .await
        .unwrap();

    let conflict = resolver
        .resolve(
            identity("gitlab", "sub-shared", None, false),
            OAuthIntent::Link {
                actor_user_id: second_user,
            },
            SessionMetadata::default(),
        )
        .await;
    assert!(conflict.is_err());
}

#[tokio::test]
async fn link_intent_not_found_when_actor_is_gone() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);

    let outcome = resolver
        .resolve(
            identity("gitlab", "sub-ghost", None, false),
            OAuthIntent::Link {
                actor_user_id: "999999999".into(),
            },
            SessionMetadata::default(),
        )
        .await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn linked_account_store_rejects_empty_input_directly() {
    let h = oauth_harness::harness().await;
    let empty_provider = h
        .storage
        .create(magnetar::storage::NewLinkedAccount {
            user_id: "1".into(),
            provider: String::new(),
            provider_account_id: "sub".into(),
        })
        .await;
    assert!(empty_provider.is_err());

    let empty_subject = h
        .storage
        .create(magnetar::storage::NewLinkedAccount {
            user_id: "1".into(),
            provider: "github".into(),
            provider_account_id: String::new(),
        })
        .await;
    assert!(empty_subject.is_err());

    let empty_lookup = h.storage.find_by_provider_subject("", "sub").await;
    assert!(empty_lookup.is_err());
}

#[tokio::test]
async fn linked_account_store_rejects_duplicate_provider_subject() {
    let h = oauth_harness::harness().await;
    h.storage
        .create(magnetar::storage::NewLinkedAccount {
            user_id: "1".into(),
            provider: "github".into(),
            provider_account_id: "dup-sub".into(),
        })
        .await
        .unwrap();
    let dup = h
        .storage
        .create(magnetar::storage::NewLinkedAccount {
            user_id: "1".into(),
            provider: "github".into(),
            provider_account_id: "dup-sub".into(),
        })
        .await;
    assert!(matches!(dup, Err(magnetar::Error::Conflict { .. })));
}

#[tokio::test]
async fn concurrent_identical_identity_resolutions_have_one_linked_account() {
    let h = oauth_harness::harness().await;
    let resolver = std::sync::Arc::new(resolver(&h, AutoLinkPolicy::ExplicitLinkRequired));

    let (left, right) = tokio::join!(
        resolver.resolve(
            identity("github", "sub-race", Some("racer@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        ),
        resolver.resolve(
            identity("github", "sub-race", Some("racer@example.test"), true),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        ),
    );

    fn resolved_user_id(outcome: &IdentityOutcome) -> String {
        match outcome {
            IdentityOutcome::Create { user_id, .. } => user_id.clone(),
            IdentityOutcome::SignIn(principal) => principal.user_id().to_owned(),
            _ => panic!("expected Create or SignIn from a race on one identity"),
        }
    }
    let left_user = resolved_user_id(&left.unwrap());
    let right_user = resolved_user_id(&right.unwrap());
    // Spec 01: "linking one provider identity to two users is impossible
    // under concurrency." Both racing callers must agree on the single
    // winning user.
    assert_eq!(left_user, right_user);

    let account = h
        .storage
        .find_by_provider_subject("github", "sub-race")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.user_id, left_user);
}

#[tokio::test]
async fn resend_invalidates_the_earlier_completion_token() {
    let h = oauth_harness::harness().await;
    let resolver = resolver(&h, AutoLinkPolicy::ExplicitLinkRequired);
    let completion = completion(&h);

    let outcome = resolver
        .resolve(
            identity("x", "sub-sibling", None, false),
            OAuthIntent::SignIn,
            SessionMetadata::default(),
        )
        .await
        .unwrap();
    let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
        panic!("expected EmailCompletionRequired");
    };

    completion
        .request(&pending_id, "owner@example.test")
        .await
        .unwrap();
    let first = h.mail.last_payload().unwrap();
    let first_link = first["completion_link"].as_str().unwrap().to_owned();
    let token1 = first_link.split("token=").nth(1).unwrap().to_owned();

    let token1_live = || async {
        h.storage
            .check(
                PresentedToken::new(token1.clone()),
                OAUTH_EMAIL_COMPLETION_PURPOSE,
            )
            .await
            .unwrap()
    };
    assert!(token1_live().await, "token1 must start out live");

    completion
        .resend(&pending_id, "owner@example.test")
        .await
        .unwrap();
    let second = h.mail.last_payload().unwrap();
    let second_link = second["completion_link"].as_str().unwrap().to_owned();
    let token2 = second_link.split("token=").nth(1).unwrap().to_owned();
    assert_ne!(token1, token2);

    // Minting token2 alone does not retroactively invalidate token1 --
    // TokenStore's sibling invalidation fires on consume, not on issue.
    assert!(
        token1_live().await,
        "resend must not kill the prior token by itself"
    );

    // Consuming token2 completes the flow. This asserts the claim at the
    // TokenStore layer directly (`check`, which never touches the
    // CeremonyStore pending/binding records at all) -- so it isolates
    // sibling invalidation from the unrelated fact that a completed flow
    // also deletes the pending identity, which alone would make any later
    // `EmailCompletionService::consume(token1)` call fail regardless of
    // whether sibling invalidation actually ran.
    let outcome = completion.consume(&token2).await.unwrap();
    assert!(matches!(outcome, IdentityOutcome::Create { .. }));
    assert!(
        !token1_live().await,
        "consuming token2 must sibling-invalidate token1"
    );

    // And, consistently, the higher-level API also refuses it.
    let stale = completion.consume(&token1).await;
    assert!(stale.is_err());
}
