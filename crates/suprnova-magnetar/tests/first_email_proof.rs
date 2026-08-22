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
    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    use magnetar::sessions::{
        OpaqueConfig, OpaqueSessionProvider, SessionQueries, VerifiedSession,
    };
    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    use magnetar::storage::CredentialActor;
    #[cfg(feature = "oauth")]
    use magnetar::storage::LinkedAccountStore;
    use magnetar::storage::{
        IssueToken, PASSWORD_RESET_PURPOSE, PresentedToken, SeaOrmStorage, TokenStore,
    };
    #[cfg(all(feature = "oauth", feature = "magic-link"))]
    use magnetar::storage::{LinkedAccountRecord, NewLinkedAccount};
    #[cfg(all(feature = "passkey", feature = "magic-link"))]
    use magnetar::storage::{PasskeyRow, PasskeyStore};
    use sha2::{Digest, Sha256};

    const SEEDED_SESSION_TOKEN: &str = "first-proof-session-token";

    fn seeded_session_digest() -> String {
        Sha256::digest(SEEDED_SESSION_TOKEN.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    async fn verified_seeded_session(database: &sea_orm::DatabaseConnection) -> VerifiedSession {
        OpaqueSessionProvider::new(
            Arc::new(magnetar::default_schema::sql_stores::SqlSessionStore(
                database.clone(),
            )),
            OpaqueConfig::default(),
        )
        .verify_bearer(SEEDED_SESSION_TOKEN)
        .await
        .expect("official opaque provider verifies the seeded live session")
    }
    #[cfg(all(feature = "password", feature = "magic-link"))]
    use magnetar::storage::{UserRecord, UserStore};
    #[cfg(all(feature = "two-factor", feature = "magic-link"))]
    use magnetar::two_factor::{TwoFactorProofClaim, TwoFactorRow, TwoFactorStore};
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
            enrollment_auth_epoch: Set(4),
            enrollment_session_id: Set(Some("session-1".to_owned())),
            enrollment_expires_at: Set(Some(now + ChronoDuration::hours(1))),
            rotation_pending: Set(false),
            confirmed_at: Set(Some(now)),
            ..Default::default()
        }
        .insert(&database)
        .await
        .unwrap();
        sessions::ActiveModel {
            id: Set("session-1".to_owned()),
            user_id: Set(1),
            auth_epoch: Set(4),
            token_digest: Set(seeded_session_digest()),
            token_hash: Set(Some(seeded_session_digest())),
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
            auth_epoch: Set(4),
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

    // The service has already authenticated the mutation when it reaches these
    // object-safe stores. Two barriers freeze that write at the SQL boundary
    // until a real first-proof transaction has committed.
    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    #[derive(Clone)]
    struct PersistenceGate {
        ready: Arc<tokio::sync::Barrier>,
        release: Arc<tokio::sync::Barrier>,
    }

    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    impl PersistenceGate {
        fn new() -> Self {
            Self {
                ready: Arc::new(tokio::sync::Barrier::new(2)),
                release: Arc::new(tokio::sync::Barrier::new(2)),
            }
        }

        async fn pause_immediately_before_persist(&self) {
            self.ready.wait().await;
            self.release.wait().await;
        }

        async fn wait_until_ready_to_persist(&self) {
            self.ready.wait().await;
        }

        async fn release_write(&self) {
            self.release.wait().await;
        }
    }

    #[cfg(all(feature = "password", feature = "magic-link"))]
    struct GatedPasswordWrite {
        inner: Arc<dyn UserStore>,
        gate: PersistenceGate,
    }

    #[cfg(all(feature = "password", feature = "magic-link"))]
    #[async_trait::async_trait]
    impl UserStore for GatedPasswordWrite {
        async fn find_by_email(&self, email: &str) -> magnetar::Result<Option<UserRecord>> {
            self.inner.find_by_email(email).await
        }

        async fn find_by_id(&self, user_id: &str) -> magnetar::Result<Option<UserRecord>> {
            self.inner.find_by_id(user_id).await
        }

        async fn create_user(
            &self,
            input: magnetar::storage::NewUser,
        ) -> magnetar::Result<UserRecord> {
            self.inner.create_user(input).await
        }

        async fn set_password_hash(
            &self,
            actor: &CredentialActor,
            password_hash: &str,
        ) -> magnetar::Result<()> {
            self.gate.pause_immediately_before_persist().await;
            self.inner.set_password_hash(actor, password_hash).await
        }

        async fn mark_email_verified(
            &self,
            user_id: &str,
            at: chrono::DateTime<Utc>,
        ) -> magnetar::Result<()> {
            self.inner.mark_email_verified(user_id, at).await
        }

        async fn lock_if_unlocked_by_email(
            &self,
            email: &str,
            locked_at: chrono::DateTime<Utc>,
            window_start: chrono::DateTime<Utc>,
        ) -> magnetar::Result<bool> {
            self.inner
                .lock_if_unlocked_by_email(email, locked_at, window_start)
                .await
        }

        async fn set_locked_at_by_email(
            &self,
            email: &str,
            locked_at: Option<chrono::DateTime<Utc>>,
        ) -> magnetar::Result<()> {
            self.inner.set_locked_at_by_email(email, locked_at).await
        }
    }

    #[cfg(all(feature = "passkey", feature = "magic-link"))]
    struct GatedPasskeyEnrollment {
        inner: Arc<dyn PasskeyStore>,
        gate: PersistenceGate,
    }

    #[cfg(all(feature = "passkey", feature = "magic-link"))]
    #[async_trait::async_trait]
    impl PasskeyStore for GatedPasskeyEnrollment {
        async fn insert_passkey(
            &self,
            actor: &CredentialActor,
            credential_id_b64: &str,
            envelope_json: &str,
        ) -> magnetar::Result<PasskeyRow> {
            self.gate.pause_immediately_before_persist().await;
            self.inner
                .insert_passkey(actor, credential_id_b64, envelope_json)
                .await
        }

        async fn passkeys_for_user(&self, user_id: &str) -> magnetar::Result<Vec<PasskeyRow>> {
            self.inner.passkeys_for_user(user_id).await
        }

        async fn find_user_by_credential(
            &self,
            credential_id_b64: &str,
        ) -> magnetar::Result<Option<PasskeyRow>> {
            self.inner.find_user_by_credential(credential_id_b64).await
        }

        async fn update_passkey_envelope(
            &self,
            actor: &CredentialActor,
            credential_id_b64: &str,
            envelope_json: &str,
        ) -> magnetar::Result<()> {
            self.inner
                .update_passkey_envelope(actor, credential_id_b64, envelope_json)
                .await
        }
    }

    #[cfg(all(feature = "oauth", feature = "magic-link"))]
    struct GatedExplicitOAuthLink {
        inner: Arc<dyn LinkedAccountStore>,
        gate: PersistenceGate,
    }

    #[cfg(all(feature = "oauth", feature = "magic-link"))]
    #[async_trait::async_trait]
    impl LinkedAccountStore for GatedExplicitOAuthLink {
        async fn create(
            &self,
            actor: &CredentialActor,
            input: NewLinkedAccount,
        ) -> magnetar::Result<LinkedAccountRecord> {
            self.gate.pause_immediately_before_persist().await;
            self.inner.create(actor, input).await
        }

        async fn validate_actor(&self, actor: &CredentialActor) -> magnetar::Result<()> {
            self.inner.validate_actor(actor).await
        }

        async fn find_by_provider_subject(
            &self,
            provider: &str,
            provider_account_id: &str,
        ) -> magnetar::Result<Option<LinkedAccountRecord>> {
            self.inner
                .find_by_provider_subject(provider, provider_account_id)
                .await
        }
    }

    #[cfg(all(feature = "two-factor", feature = "magic-link"))]
    struct GatedTotpEnrollment {
        inner: Arc<dyn TwoFactorStore>,
        gate: PersistenceGate,
    }

    #[cfg(all(feature = "two-factor", feature = "magic-link"))]
    #[async_trait::async_trait]
    impl TwoFactorStore for GatedTotpEnrollment {
        async fn find_enrollment(&self, user_id: &str) -> magnetar::Result<Option<TwoFactorRow>> {
            self.inner.find_enrollment(user_id).await
        }

        async fn begin_enrollment(
            &self,
            actor: &CredentialActor,
            secret: &[u8],
            recovery_codes: Option<&[u8]>,
        ) -> magnetar::Result<bool> {
            self.gate.pause_immediately_before_persist().await;
            self.inner
                .begin_enrollment(actor, secret, recovery_codes)
                .await
        }

        async fn set_confirmed(
            &self,
            actor: &CredentialActor,
            at: chrono::DateTime<Utc>,
        ) -> magnetar::Result<bool> {
            self.inner.set_confirmed(actor, at).await
        }

        async fn claim_timestep(&self, user_id: &str, matched_step: i64) -> magnetar::Result<bool> {
            self.inner.claim_timestep(user_id, matched_step).await
        }

        async fn swap_recovery_codes(
            &self,
            user_id: &str,
            expected: &[u8],
            next: Option<&[u8]>,
        ) -> magnetar::Result<bool> {
            self.inner
                .swap_recovery_codes(user_id, expected, next)
                .await
        }

        async fn rotate_enrollment(
            &self,
            actor: &CredentialActor,
            claim: TwoFactorProofClaim,
            secret: &[u8],
            recovery_codes: Option<&[u8]>,
        ) -> magnetar::Result<bool> {
            self.inner
                .rotate_enrollment(actor, claim, secret, recovery_codes)
                .await
        }

        async fn regenerate_recovery_codes(
            &self,
            actor: &CredentialActor,
            claim: TwoFactorProofClaim,
            next: &[u8],
        ) -> magnetar::Result<bool> {
            self.inner
                .regenerate_recovery_codes(actor, claim, next)
                .await
        }

        async fn delete_enrollment(&self, actor: &CredentialActor) -> magnetar::Result<bool> {
            self.inner.delete_enrollment(actor).await
        }
    }

    #[cfg(all(
        feature = "magic-link",
        any(
            feature = "password",
            feature = "passkey",
            feature = "oauth",
            feature = "two-factor"
        )
    ))]
    async fn commit_magic_link_first_proof(database: &sea_orm::DatabaseConnection) {
        let token = SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())
            .issue(IssueToken {
                user_id: "1".to_owned(),
                purpose: "magic-link".to_owned(),
                ttl: Duration::from_secs(900),
            })
            .await
            .unwrap();
        let proof =
            SqlFirstEmailProofStore::new(database.clone(), Arc::new(AeadEncryptor::new([31; 32])));

        let commit = proof
            .apply(FirstEmailProofMutation::MagicLink {
                token: PresentedToken(token.plaintext),
            })
            .await
            .unwrap()
            .into_commit()
            .unwrap();

        assert!(commit.first_proof);
    }

    #[cfg(all(feature = "password", feature = "magic-link"))]
    #[tokio::test]
    async fn password_write_started_before_first_proof_cannot_restore_password_after_commit() {
        let (database, _reset_token) = seeded_squatted_account().await;
        let gate = PersistenceGate::new();
        let store = Arc::new(GatedPasswordWrite {
            inner: Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())),
            gate: gate.clone(),
        });
        let actor = CredentialActor::from_session(&verified_seeded_session(&database).await);
        let write = tokio::spawn({
            let store = Arc::clone(&store);
            async move {
                let actor = actor;
                store
                    .set_password_hash(&actor, "late-authenticated-password-hash")
                    .await
            }
        });

        gate.wait_until_ready_to_persist().await;
        commit_magic_link_first_proof(&database).await;
        gate.release_write().await;
        let error = write.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            magnetar::Error::NotFound { resource, identifier }
                if resource == "credential actor" && identifier == "expired or revoked"
        ));

        let user = users::Entity::find_by_id(1)
            .one(&database)
            .await
            .unwrap()
            .unwrap();
        assert!(
            user.password_hash.is_none(),
            "a password write authorized before first proof must not survive its cleanup"
        );
    }

    #[cfg(all(feature = "passkey", feature = "magic-link"))]
    #[tokio::test]
    async fn passkey_enrollment_started_before_first_proof_cannot_restore_passkey_after_commit() {
        let (database, _reset_token) = seeded_squatted_account().await;
        let gate = PersistenceGate::new();
        let store = Arc::new(GatedPasskeyEnrollment {
            inner: Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())),
            gate: gate.clone(),
        });
        let actor = CredentialActor::from_session(&verified_seeded_session(&database).await);
        let write = tokio::spawn({
            let store = Arc::clone(&store);
            async move {
                store
                    .insert_passkey(&actor, "late-passkey-credential", r#"{"passkey":"late"}"#)
                    .await
            }
        });

        gate.wait_until_ready_to_persist().await;
        commit_magic_link_first_proof(&database).await;
        gate.release_write().await;
        let error = write.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            magnetar::Error::NotFound { resource, identifier }
                if resource == "credential actor" && identifier == "expired or revoked"
        ));

        assert!(
            store.passkeys_for_user("1").await.unwrap().is_empty(),
            "a passkey enrollment authorized before first proof must not survive its cleanup"
        );
    }

    #[cfg(all(feature = "oauth", feature = "magic-link"))]
    #[tokio::test]
    async fn explicit_oauth_link_started_before_first_proof_cannot_restore_link_after_commit() {
        let (database, _reset_token) = seeded_squatted_account().await;
        let gate = PersistenceGate::new();
        let store = Arc::new(GatedExplicitOAuthLink {
            inner: Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(database.clone())),
            gate: gate.clone(),
        });
        let actor = CredentialActor::from_session(&verified_seeded_session(&database).await);
        let write = tokio::spawn({
            let store = Arc::clone(&store);
            let actor = actor.clone();
            async move {
                store
                    .create(
                        &actor,
                        NewLinkedAccount {
                            user_id: "1".to_owned(),
                            provider: "github".to_owned(),
                            provider_account_id: "late-explicit-link".to_owned(),
                        },
                    )
                    .await
            }
        });

        gate.wait_until_ready_to_persist().await;
        commit_magic_link_first_proof(&database).await;
        gate.release_write().await;
        let error = write.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            magnetar::Error::NotFound { resource, identifier }
                if resource == "credential actor" && identifier == "expired or revoked"
        ));

        assert!(
            store
                .find_by_provider_subject("github", "late-explicit-link")
                .await
                .unwrap()
                .is_none(),
            "an explicit OAuth link authorized before first proof must not survive its cleanup"
        );
    }

    #[cfg(all(feature = "two-factor", feature = "magic-link"))]
    #[tokio::test]
    async fn totp_enrollment_started_before_first_proof_cannot_restore_factor_after_commit() {
        let (database, _reset_token) = seeded_squatted_account().await;
        let gate = PersistenceGate::new();
        let store = Arc::new(GatedTotpEnrollment {
            inner: Arc::new(magnetar::default_schema::sql_two_factor::SqlTwoFactorStore(
                database.clone(),
            )),
            gate: gate.clone(),
        });
        let actor = CredentialActor::from_session(&verified_seeded_session(&database).await);
        let write = tokio::spawn({
            let store = Arc::clone(&store);
            async move {
                store
                    .begin_enrollment(&actor, b"late-totp-secret", Some(b"late-recovery-codes"))
                    .await
            }
        });

        gate.wait_until_ready_to_persist().await;
        commit_magic_link_first_proof(&database).await;
        gate.release_write().await;
        let error = write.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            magnetar::Error::NotFound { resource, identifier }
                if resource == "credential actor" && identifier == "expired or revoked"
        ));

        assert!(
            store.find_enrollment("1").await.unwrap().is_none(),
            "a TOTP enrollment authorized before first proof must not survive its cleanup"
        );
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
                None,
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
    #[cfg(all(feature = "password", feature = "two-factor"))]
    #[tokio::test]
    async fn reset_after_squatter_totp_yields_session_allowed() {
        use magnetar::auth::{FactorGate, OpaqueFactorGate, SignInDecision};
        use magnetar::default_schema::sql_stores::SqlSessionStore;
        use magnetar::default_schema::sql_two_factor::SqlTwoFactorStore;
        use magnetar::password::{
            LockoutConfig, LockoutService, PasswordHashConfig, PasswordVerifier,
            StandardPasswordHashDriver,
        };
        use magnetar::plugins::password::{
            PasswordAttempt, PasswordAuthProvider, PasswordAuthService,
        };
        use magnetar::sessions::{OpaqueConfig, OpaqueSessionProvider, SessionMetadata};
        use magnetar::two_factor::{TwoFactorConfig, TwoFactorService};

        let (database, token) = seeded_squatted_account().await;
        let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(database.clone()));
        let crypto = Arc::new(AeadEncryptor::new([26; 32]));
        let verifier = Arc::new(
            PasswordVerifier::new(
                Arc::new(StandardPasswordHashDriver),
                PasswordHashConfig {
                    bcrypt_cost: 4,
                    argon2_memory_kib: 8,
                    argon2_iterations: 1,
                    argon2_parallelism: 1,
                },
            )
            .unwrap(),
        );
        let replacement = "mailbox owner replacement";
        let replacement_hash = verifier
            .mint_target(&SecretString::from(replacement.to_owned()))
            .unwrap();
        let first_proof = SqlFirstEmailProofStore::new(database.clone(), crypto.clone());
        first_proof
            .apply(FirstEmailProofMutation::PasswordReset {
                token: PresentedToken(token.plaintext),
                expected_user_id: Some("1".to_owned()),
                new_password_hash: SecretString::from(replacement_hash),
            })
            .await
            .unwrap()
            .into_commit()
            .unwrap();

        let lockout = Arc::new(LockoutService::new(
            storage.clone(),
            storage.clone(),
            LockoutConfig::default(),
        ));
        let two_factor = Arc::new(TwoFactorService::new(
            Arc::new(SqlTwoFactorStore(database.clone())),
            storage.clone(),
            lockout,
            crypto.clone(),
            TwoFactorConfig::default(),
        ));
        let sessions = Arc::new(OpaqueSessionProvider::new(
            Arc::new(SqlSessionStore(database)),
            OpaqueConfig::default(),
        ));
        let gate = OpaqueFactorGate::new(storage.clone(), two_factor, crypto, sessions);
        let provider = PasswordAuthService::new(storage.clone(), storage, verifier);
        let principal = provider
            .authenticate(PasswordAttempt {
                email: "victim@example.test".to_owned(),
                password: SecretString::from(replacement.to_owned()),
                metadata: SessionMetadata::default(),
            })
            .await
            .unwrap();
        let context = principal.context().clone();

        assert!(matches!(
            gate.complete_sign_in(principal, context).await.unwrap(),
            SignInDecision::SessionAllowed(_)
        ));
    }
}
