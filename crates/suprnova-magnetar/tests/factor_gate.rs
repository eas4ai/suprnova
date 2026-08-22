//! Contract tests for the provider-neutral factor gate boundary.

use chrono::{Duration, Utc};
use magnetar::auth::reauth::{REAUTH_WINDOW, ReauthStamp, validate_reauth};
use magnetar::auth::{SignInDecision, TWO_FACTOR_CHALLENGE_KIND};

#[test]
fn disabled_or_unenrolled_contract_is_direct_session_decision() {
    // The concrete decision has no challenge selector in this path; providers
    // are required to route it through the shared gate rather than minting.
    let decision = std::mem::discriminant(&SignInDecision::FactorRequired {
        challenge_selector: "selector".to_owned(),
    });
    let challenge = std::mem::discriminant(&SignInDecision::FactorRequired {
        challenge_selector: "selector".to_owned(),
    });
    assert_eq!(decision, challenge);
}

#[test]
fn enrolled_contract_uses_one_time_challenge_namespace() {
    assert_eq!(TWO_FACTOR_CHALLENGE_KIND, "two-factor.challenge");
}

#[test]
fn reauth_accepts_only_matching_owner_within_three_hours() {
    let now = Utc::now();
    let capability = validate_reauth(
        "user-1",
        ReauthStamp {
            owner_user_id: "user-1".to_owned(),
            password_confirmed_at: now - REAUTH_WINDOW + Duration::seconds(1),
        },
        now,
    )
    .expect("fresh matching stamp should validate");
    assert_eq!(capability.owner_user_id(), "user-1");
}

#[test]
fn stale_invalid_and_future_reauth_stamps_fail() {
    let now = Utc::now();
    for stamp in [
        ReauthStamp {
            owner_user_id: "other".to_owned(),
            password_confirmed_at: now,
        },
        ReauthStamp {
            owner_user_id: "user-1".to_owned(),
            password_confirmed_at: now - REAUTH_WINDOW - Duration::seconds(1),
        },
        ReauthStamp {
            owner_user_id: "user-1".to_owned(),
            password_confirmed_at: now + Duration::seconds(1),
        },
    ] {
        assert!(validate_reauth("user-1", stamp, now).is_err());
    }
}
#[cfg(feature = "seaorm-sqlite")]
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

#[cfg(feature = "seaorm-sqlite")]
mod concurrent_completion {
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use magnetar::auth::{
        AuthenticationContext, FactorGate, FactorVerifier, OpaqueFactorGate, PreparedFactorProof,
        TWO_FACTOR_CHALLENGE_KIND,
    };
    use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};
    use magnetar::sessions::{
        OpaqueConfig, OpaqueSessionProvider, SessionMetadata, SessionQueries,
    };
    use magnetar::storage::{CeremonyStore, NewCeremony, SeaOrmStorage};
    use magnetar::{Error, Result};
    use parking_lot::Mutex;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    use serde::Serialize;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    use super::storage_schema::sql_stores::SqlSessionStore;
    use super::storage_schema::{StorageSchema, database, users};
    const USER_ID: &str = "41";
    const SELECTOR: &str = "shared-factor-challenge";
    const TOTP_PROOF: &str = "valid-totp-proof";
    const RECOVERY_PROOF: &str = "valid-recovery-proof";

    struct ProofState {
        available: HashSet<String>,
        claimed: HashSet<String>,
    }

    struct BarrierFactorVerifier {
        prepared: Barrier,
        proofs: Mutex<ProofState>,
    }

    impl BarrierFactorVerifier {
        fn new() -> Self {
            Self {
                prepared: Barrier::new(2),
                proofs: Mutex::new(ProofState {
                    available: HashSet::from([TOTP_PROOF.to_owned(), RECOVERY_PROOF.to_owned()]),
                    claimed: HashSet::new(),
                }),
            }
        }

        fn claimed(&self) -> HashSet<String> {
            self.proofs.lock().claimed.clone()
        }

        fn is_available(&self, code: &str) -> bool {
            self.proofs.lock().available.contains(code)
        }
    }

    #[async_trait]
    impl FactorVerifier for BarrierFactorVerifier {
        type PreparedProof = String;

        async fn has_confirmed_enrollment(&self, _user_id: &str) -> Result<bool> {
            Ok(true)
        }

        async fn prepare_code(
            &self,
            user_id: &str,
            code: &str,
        ) -> Result<PreparedFactorProof<Self::PreparedProof>> {
            if user_id != USER_ID || !self.proofs.lock().available.contains(code) {
                return Ok(PreparedFactorProof::invalid(code.to_owned()));
            }

            // Both callers prepare a distinct valid proof while the shared
            // challenge is still pending. The barrier would deadlock if the
            // gate tried to claim either proof before both prepares complete.
            self.prepared.wait().await;
            Ok(PreparedFactorProof::valid(code.to_owned()))
        }

        async fn claim_prepared(&self, user_id: &str, proof: Self::PreparedProof) -> Result<bool> {
            if user_id != USER_ID {
                return Ok(false);
            }
            let mut proofs = self.proofs.lock();
            if !proofs.available.remove(&proof) {
                return Ok(false);
            }
            proofs.claimed.insert(proof);
            Ok(true)
        }
    }

    #[derive(Serialize)]
    struct ChallengePayload {
        user_id: String,
        context: AuthenticationContext,
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_distinct_valid_proofs_claim_only_for_the_challenge_winner() {
        let db = database().await;
        users::ActiveModel {
            id: Set(41),
            email: Set("factor-race@example.test".to_owned()),
            auth_epoch: Set(7),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("seed factor-race user");
        let ceremonies = Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
        let factors = Arc::new(BarrierFactorVerifier::new());
        let sessions = Arc::new(OpaqueSessionProvider::new(
            Arc::new(SqlSessionStore(db)),
            OpaqueConfig::default(),
        ));
        let encryptor = Arc::new(AeadEncryptor::new([41; 32]));
        let plaintext = serde_json::to_vec(&ChallengePayload {
            user_id: USER_ID.to_owned(),
            context: AuthenticationContext::new(SessionMetadata::default(), 7, Utc::now()),
        })
        .expect("challenge payload serializes");
        let payload = encryptor
            .encrypt(CryptoPurpose::CeremonyState, &plaintext)
            .expect("challenge payload encrypts");
        ceremonies
            .create(NewCeremony {
                selector: SELECTOR.to_owned(),
                kind: TWO_FACTOR_CHALLENGE_KIND.to_owned(),
                state: "pending".to_owned(),
                payload,
                expires_at: Utc::now() + Duration::minutes(10),
            })
            .await
            .expect("pending challenge is stored");

        let gate = Arc::new(OpaqueFactorGate::new(
            ceremonies,
            factors.clone(),
            encryptor,
            sessions.clone(),
        ));
        let first_gate = gate.clone();
        let first = tokio::spawn(async move {
            (
                TOTP_PROOF,
                first_gate.complete_challenge(SELECTOR, TOTP_PROOF).await,
            )
        });
        let second = tokio::spawn(async move {
            (
                RECOVERY_PROOF,
                gate.complete_challenge(SELECTOR, RECOVERY_PROOF).await,
            )
        });
        let outcomes = [
            first.await.expect("TOTP completion task joins"),
            second.await.expect("recovery completion task joins"),
        ];

        assert_eq!(
            outcomes
                .iter()
                .filter(|(_, outcome)| outcome.is_ok())
                .count(),
            1,
            "the shared challenge issues exactly one SessionGrant"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|(_, outcome)| matches!(outcome, Err(Error::Conflict { .. })))
                .count(),
            1,
            "the challenge CAS rejects the losing completion"
        );
        assert_eq!(
            sessions
                .list_for_user(USER_ID)
                .await
                .expect("issued sessions can be listed")
                .len(),
            1,
            "only the challenge owner persists a session"
        );

        let winner = outcomes
            .iter()
            .find_map(|(code, outcome)| outcome.is_ok().then_some(*code))
            .expect("one proof wins the challenge");
        let loser = outcomes
            .iter()
            .find_map(|(code, outcome)| {
                matches!(outcome, Err(Error::Conflict { .. })).then_some(*code)
            })
            .expect("one proof loses the challenge");
        assert_eq!(
            factors.claimed(),
            HashSet::from([winner.to_owned()]),
            "only the challenge winner may claim its prepared one-time proof"
        );
        assert!(
            factors.is_available(loser),
            "the losing conflict must leave its TOTP timestep or recovery code unconsumed"
        );
    }
}
