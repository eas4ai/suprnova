use std::sync::Arc;

use async_trait::async_trait;
use magnetar::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofStore,
    NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use magnetar::storage::PresentedToken;
use secrecy::SecretString;

struct FakeFirstEmailProofStore;

#[async_trait]
impl FirstEmailProofStore for FakeFirstEmailProofStore {
    async fn apply(
        &self,
        mutation: FirstEmailProofMutation,
    ) -> magnetar::Result<FirstEmailProofCommit> {
        let kind = match mutation {
            FirstEmailProofMutation::PasswordReset { .. } => FirstEmailProofKind::PasswordReset,
            FirstEmailProofMutation::MagicLink { .. } => FirstEmailProofKind::MagicLink,
            FirstEmailProofMutation::OAuthEmailCompletion { .. } => {
                FirstEmailProofKind::OAuthEmailCompletion
            }
        };
        Ok(FirstEmailProofCommit {
            user_id: "42".to_owned(),
            kind,
            first_proof: true,
            auth_epoch: 1,
            revoked_sessions: 0,
            revoked_remember_rows: 0,
        })
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
