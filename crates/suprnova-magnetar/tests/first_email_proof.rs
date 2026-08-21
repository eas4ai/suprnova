use std::sync::Arc;

use async_trait::async_trait;
use magnetar::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofOutcome,
    FirstEmailProofStore, NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use magnetar::storage::PresentedToken;
use secrecy::SecretString;

#[cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]
#[path = "fixtures/fakes.rs"]
mod fakes;

struct FakeFirstEmailProofStore;

#[async_trait]
impl FirstEmailProofStore for FakeFirstEmailProofStore {
    async fn apply(
        &self,
        mutation: FirstEmailProofMutation,
    ) -> magnetar::Result<FirstEmailProofOutcome> {
        let kind = match mutation {
            FirstEmailProofMutation::PasswordReset { .. } => FirstEmailProofKind::PasswordReset,
            FirstEmailProofMutation::MagicLink { .. } => FirstEmailProofKind::MagicLink,
            FirstEmailProofMutation::OAuthEmailCompletion { .. } => {
                FirstEmailProofKind::OAuthEmailCompletion
            }
        };
        Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
            user_id: "42".to_owned(),
            kind,
            first_proof: true,
            provider_account_id: None,
            auth_epoch: 1,
            revoked_sessions: 0,
            revoked_remember_rows: 0,
        }))
    }

    async fn create_verified_provider_account(
        &self,
        _input: NewVerifiedProviderAccount,
    ) -> magnetar::Result<VerifiedProviderAccountCommit> {
        Ok(VerifiedProviderAccountCommit {
            user_id: "42".to_owned(),
            auth_epoch: 0,
        })
    }
}

fn accepts_object_safe_store(_store: Arc<dyn FirstEmailProofStore>) {}

#[tokio::test]
async fn first_email_proof_contract_is_object_safe_and_secret_safe() {
    accepts_object_safe_store(Arc::new(FakeFirstEmailProofStore));

    let mutations = [
        FirstEmailProofMutation::PasswordReset {
            token: PresentedToken::new("reset-secret"),
            expected_user_id: Some("42".to_owned()),
            new_password_hash: SecretString::from("password-hash-secret".to_owned()),
        },
        FirstEmailProofMutation::MagicLink {
            token: PresentedToken::new("magic-secret"),
        },
        FirstEmailProofMutation::OAuthEmailCompletion {
            token: PresentedToken::new("oauth-secret"),
        },
    ];

    for mutation in mutations {
        let debug = format!("{mutation:?}");
        assert!(!debug.contains("reset-secret"));
        assert!(!debug.contains("password-hash-secret"));
        assert!(!debug.contains("magic-secret"));
        assert!(!debug.contains("oauth-secret"));
    }
}

#[cfg(feature = "seaorm-sqlite")]
mod sqlite {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::{Duration as ChronoDuration, Utc};
    use magnetar::crypto::AeadEncryptor;
    use magnetar::default_first_email_proof::SqlFirstEmailProofStore;
    use magnetar::default_schema::{
        DefaultAuthSchema, accounts, methods, provider_tokens, remembers, sessions, two_factor,
        users,
    };
    use magnetar::first_email_proof::{FirstEmailProofMutation, FirstEmailProofStore};
    use magnetar::storage::{
        IssueToken, LinkedAccountStore, PASSWORD_RESET_PURPOSE, PresentedToken, SeaOrmStorage,
        TokenStore,
    };
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, Database, DbBackend,
        EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, Statement,
    };
    use secrecy::SecretString;

    async fn seeded_squatted_account()
    -> (sea_orm::DatabaseConnection, magnetar::storage::IssuedToken) {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        magnetar::default_schema::migrate(&database).await.unwrap();
        let now = Utc::now();

        users::ActiveModel {
            id: Set(1),
            email: Set("victim@example.test".to_owned()),
            password_hash: Set(Some("squatter-password-hash".to_owned())),
            auth_epoch: Set(4),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        methods::ActiveModel {
            id: Set(10),
            user_id: Set(1),
            credential_id: Set(Some("squatter-passkey".to_owned())),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        accounts::ActiveModel {
            id: Set(20),
            user_id: Set(1),
            provider: Set("google".to_owned()),
            provider_account_id: Set("squatter-provider".to_owned()),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        provider_tokens::ActiveModel {
            id: Set("20".to_owned()),
            provider: Set("google".to_owned()),
            access_ciphertext: Set(vec![1]),
            refresh_ciphertext: Set(Some(vec![2])),
            raw_payload_ciphertext: Set(vec![3]),
            token_type: Set("Bearer".to_owned()),
            scopes: Set("openid".to_owned()),
            access_expires_at: Set(None),
            generation: Set(0),
            claim_id: Set(None),
            claim_deadline: Set(None),
            revoked_at: Set(None),
            revoked_reused: Set(None),
            created_at: Set(now),
        }
        .insert(&database)
        .await
        .unwrap();
        two_factor::ActiveModel {
            user_id: Set("1".to_owned()),
            secret: Set(vec![4]),
            recovery_codes: Set(Some(vec![5])),
            confirmed_at: Set(Some(now)),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        sessions::ActiveModel {
            id: Set("session-1".to_owned()),
            user_id: Set(1),
            token_digest: Set("session-digest".to_owned()),
            expires_at: Set(now + ChronoDuration::hours(1)),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        remembers::ActiveModel {
            id: Set("remember-1".to_owned()),
            selector: Set("selector-1".to_owned()),
            user_id: Set("1".to_owned()),
            verifier_hash: Set("remember-hash".to_owned()),
            expires_at: Set(now + ChronoDuration::hours(1)),
        }
        .insert(&database)
        .await
        .unwrap();

        let token = SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())
            .issue(IssueToken {
                user_id: "1".to_owned(),
                purpose: PASSWORD_RESET_PURPOSE.to_owned(),
                ttl: Duration::from_secs(900),
            })
            .await
            .unwrap();
        (database, token)
    }

    #[tokio::test]
    async fn password_reset_first_proof_removes_every_provisional_credential() {
        let (database, token) = seeded_squatted_account().await;
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([19; 32])));

        let commit = store
            .apply(FirstEmailProofMutation::PasswordReset {
                token: PresentedToken(token.plaintext),
                expected_user_id: Some("1".to_owned()),
                new_password_hash: SecretString::from("replacement-hash".to_owned()),
            })
            .await
            .unwrap()
            .into_commit()
            .unwrap();

        assert!(commit.first_proof);
        assert_eq!(commit.auth_epoch, 5);
        assert_eq!(commit.revoked_sessions, 1);
        assert_eq!(commit.revoked_remember_rows, 1);

        let user = users::Entity::find_by_id(1)
            .one(&database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.password_hash.as_deref(), Some("replacement-hash"));
        assert!(user.email_verified_at.is_some());
        assert!(
            methods::Entity::find_by_id(10)
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            accounts::Entity::find_by_id(20)
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            provider_tokens::Entity::find_by_id("20")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            two_factor::Entity::find_by_id("1")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            sessions::Entity::find_by_id("session-1")
                .one(&database)
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        assert!(
            remembers::Entity::find_by_id("remember-1")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn verified_password_reset_preserves_passkey_link_and_two_factor() {
        let (database, token) = seeded_squatted_account().await;
        let mut verified = users::Entity::find_by_id(1)
            .one(&database)
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        verified.email_verified_at = Set(Some(Utc::now()));
        verified.update(&database).await.unwrap();
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([20; 32])));

        let commit = store
            .apply(FirstEmailProofMutation::PasswordReset {
                token: PresentedToken(token.plaintext),
                expected_user_id: Some("1".to_owned()),
                new_password_hash: SecretString::from("verified-replacement".to_owned()),
            })
            .await
            .unwrap()
            .into_commit()
            .unwrap();

        assert!(!commit.first_proof);
        assert!(
            methods::Entity::find_by_id(10)
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            accounts::Entity::find_by_id(20)
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            provider_tokens::Entity::find_by_id("20")
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            two_factor::Entity::find_by_id("1")
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn first_proof_failure_rolls_back_remember_revocation() {
        let (database, token) = seeded_squatted_account().await;
        database
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "CREATE TRIGGER fail_first_proof BEFORE UPDATE ON app_users
                 BEGIN SELECT RAISE(ABORT, 'forced proof failure'); END"
                    .to_owned(),
            ))
            .await
            .unwrap();
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([21; 32])));

        assert!(
            store
                .apply(FirstEmailProofMutation::PasswordReset {
                    token: PresentedToken(token.plaintext),
                    expected_user_id: Some("1".to_owned()),
                    new_password_hash: SecretString::from("replacement-hash".to_owned()),
                })
                .await
                .is_err()
        );

        assert!(
            remembers::Entity::find_by_id("remember-1")
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            methods::Entity::find_by_id(10)
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            accounts::Entity::find_by_id(20)
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            two_factor::Entity::find_by_id("1")
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            sessions::Entity::find_by_id("session-1")
                .one(&database)
                .await
                .unwrap()
                .unwrap()
                .revoked_at
                .is_none()
        );
    }

    #[tokio::test]
    async fn verified_provider_account_is_initialized_as_email_verified() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        magnetar::default_schema::migrate(&database).await.unwrap();
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([22; 32])));

        let commit = store
            .create_verified_provider_account(
                magnetar::first_email_proof::NewVerifiedProviderAccount {
                    provider: "google".to_owned(),
                    provider_account_id: "provider-user".to_owned(),
                    email: "verified@example.test".to_owned(),
                },
            )
            .await
            .unwrap();
        let numeric_user_id = commit.user_id.parse::<i64>().unwrap();
        let user = users::Entity::find_by_id(numeric_user_id)
            .one(&database)
            .await
            .unwrap()
            .unwrap();
        assert!(user.email_verified_at.is_some());
        assert_eq!(user.auth_epoch, 0);
        assert!(
            accounts::Entity::find()
                .filter(accounts::Column::UserId.eq(numeric_user_id))
                .one(&database)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn provider_identity_race_returns_winner_without_orphan_user() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        magnetar::default_schema::migrate(&database).await.unwrap();
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([23; 32])));
        let first = store
            .create_verified_provider_account(
                magnetar::first_email_proof::NewVerifiedProviderAccount {
                    provider: "google".to_owned(),
                    provider_account_id: "stable-provider-user".to_owned(),
                    email: "winner@example.test".to_owned(),
                },
            )
            .await
            .unwrap();

        let second = store
            .create_verified_provider_account(
                magnetar::first_email_proof::NewVerifiedProviderAccount {
                    provider: "google".to_owned(),
                    provider_account_id: "stable-provider-user".to_owned(),
                    email: "loser@example.test".to_owned(),
                },
            )
            .await
            .unwrap();

        assert_eq!(second.user_id, first.user_id);
        assert_eq!(users::Entity::find().count(&database).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn magic_link_first_proof_clears_password_and_every_other_credential() {
        let (database, _reset_token) = seeded_squatted_account().await;
        let magic_token = SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())
            .issue(IssueToken {
                user_id: "1".to_owned(),
                purpose: "magic-link".to_owned(),
                ttl: Duration::from_secs(900),
            })
            .await
            .unwrap();
        let store =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([24; 32])));

        let commit = store
            .apply(FirstEmailProofMutation::MagicLink {
                token: PresentedToken(magic_token.plaintext),
            })
            .await
            .unwrap()
            .into_commit()
            .unwrap();

        assert!(commit.first_proof);
        assert_eq!(commit.auth_epoch, 5);
        let user = users::Entity::find_by_id(1)
            .one(&database)
            .await
            .unwrap()
            .unwrap();
        assert!(user.password_hash.is_none());
        assert!(user.email_verified_at.is_some());
        assert!(
            methods::Entity::find_by_id(10)
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            accounts::Entity::find_by_id(20)
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            provider_tokens::Entity::find_by_id("20")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            two_factor::Entity::find_by_id("1")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            remembers::Entity::find_by_id("remember-1")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[cfg(feature = "oauth")]
    #[tokio::test]
    async fn oauth_email_completion_reclaims_unverified_squatted_account() {
        use magnetar::oauth::{
            AutoLinkPolicy, EmailCompletionConfig, EmailCompletionService, IdentityOutcome,
            IdentityResolver, OAuthIntent, VerifiedProviderIdentity,
        };
        use magnetar::sessions::SessionMetadata;

        let (database, _reset_token) = seeded_squatted_account().await;
        let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(database.clone()));
        let encryptor = Arc::new(AeadEncryptor::new([25; 32]));
        let first_proof = Arc::new(SqlFirstEmailProofStore::new(
            database.clone(),
            encryptor.clone(),
        ));
        let resolver = IdentityResolver::new(
            storage.clone(),
            storage.clone(),
            storage.clone(),
            first_proof.clone(),
            encryptor.clone(),
            AutoLinkPolicy::ExplicitLinkRequired,
        );
        let pending = resolver
            .resolve(
                VerifiedProviderIdentity {
                    provider: "tiktok".to_owned(),
                    subject: "victim-provider".to_owned(),
                    email: None,
                    email_verified: false,
                    display_name: None,
                },
                OAuthIntent::SignIn,
                SessionMetadata::default(),
            )
            .await
            .unwrap();
        let IdentityOutcome::EmailCompletionRequired { pending_id } = pending else {
            panic!("expected email completion");
        };
        let mail = Arc::new(super::fakes::RecordingMail::default());
        let completion = EmailCompletionService::new(
            storage.clone(),
            storage.clone(),
            first_proof,
            encryptor,
            mail.clone(),
            Arc::new(super::fakes::TestLinks),
            Arc::new(super::fakes::CountingLimiter::default()),
            EmailCompletionConfig::default(),
        );
        completion
            .request(&pending_id, "victim@example.test")
            .await
            .unwrap();
        let link = mail.last_payload().unwrap()["completion_link"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = link.split("token=").nth(1).unwrap();

        let outcome = completion.consume(token).await.unwrap();
        assert!(matches!(
            outcome,
            IdentityOutcome::Link {
                actor_user_id,
                provider_account_id,
            } if actor_user_id == "1" && provider_account_id == "victim-provider"
        ));
        let user = users::Entity::find_by_id(1)
            .one(&database)
            .await
            .unwrap()
            .unwrap();
        assert!(user.password_hash.is_none());
        assert!(user.email_verified_at.is_some());
        assert!(
            methods::Entity::find_by_id(10)
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            provider_tokens::Entity::find_by_id("20")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            two_factor::Entity::find_by_id("1")
                .one(&database)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            storage
                .find_by_provider_subject("tiktok", "victim-provider")
                .await
                .unwrap()
                .is_some()
        );
    }
}
