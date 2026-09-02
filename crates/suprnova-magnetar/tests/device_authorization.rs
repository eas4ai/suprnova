//! RFC 8628 device authorization suite (Task 4).

#![cfg(all(
    feature = "oauth",
    feature = "device-authorization",
    feature = "seaorm-sqlite"
))]

#[path = "fixtures/grants_harness.rs"]
mod grants_harness;
#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::Utc;
use magnetar::abuse::AbusePolicy;
use magnetar::crypto::{CryptoPurpose, Encryptor};
use magnetar::oauth::device::{
    DeviceApprovalOutcome, DeviceAuthorizationConfig, DeviceAuthorizationService,
    DeviceCeremonyStatus, DevicePollOutcome,
};
use magnetar::sessions::SessionMetadata;
use magnetar::storage::{
    CeremonyRecord, CeremonyStore, CredentialActor, DeviceStore, NewCeremony, SeaOrmStorage,
};
use magnetar::{Error, Result};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use secrecy::ExposeSecret;

use grants_harness::create_user;

const TRANSIENT_GRANT_DECODE_ERROR: &str = "forced transient session grant decode failure";

struct FailOnceGrantDecryptor {
    inner: Arc<dyn Encryptor>,
    fail_next_grant_decrypt: AtomicBool,
}

impl Encryptor for FailOnceGrantDecryptor {
    fn encrypt(&self, purpose: CryptoPurpose, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.inner.encrypt(purpose, plaintext)
    }

    fn decrypt(&self, purpose: CryptoPurpose, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if purpose == CryptoPurpose::SessionGrant
            && self.fail_next_grant_decrypt.swap(false, Ordering::SeqCst)
        {
            return Err(Error::Internal {
                message: TRANSIENT_GRANT_DECODE_ERROR.to_owned(),
            });
        }
        self.inner.decrypt(purpose, ciphertext)
    }
}

struct ReplaceGrantAfterPeek {
    inner: Arc<SeaOrmStorage<storage_schema::StorageSchema>>,
    replace_next_grant: AtomicBool,
}

#[async_trait]
impl CeremonyStore for ReplaceGrantAfterPeek {
    async fn create(&self, input: NewCeremony) -> Result<CeremonyRecord> {
        self.inner.create(input).await
    }

    async fn consume(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>> {
        self.inner.consume(selector, kind).await
    }

    async fn peek(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>> {
        let record = self.inner.peek(selector, kind).await?;
        if kind == "device-authorization-grant"
            && let Some(observed) = record.as_ref()
            && self.replace_next_grant.swap(false, Ordering::SeqCst)
        {
            let consumed =
                self.inner
                    .consume(selector, kind)
                    .await?
                    .ok_or_else(|| Error::Internal {
                        message: "replacement seam lost the observed grant".to_owned(),
                    })?;
            if consumed.id != observed.id {
                return Err(Error::Internal {
                    message: "replacement seam consumed a different grant".to_owned(),
                });
            }
            self.inner
                .create(NewCeremony {
                    selector: observed.selector.clone(),
                    kind: observed.kind.clone(),
                    state: observed.state.clone(),
                    payload: observed.payload.clone(),
                    expires_at: observed.expires_at,
                })
                .await?;
        }
        Ok(record)
    }

    async fn transition(
        &self,
        selector: &str,
        kind: &str,
        expected: &str,
        next: &str,
    ) -> Result<bool> {
        self.inner.transition(selector, kind, expected, next).await
    }

    async fn transition_and_consume(
        &self,
        transition_selector: &str,
        transition_kind: &str,
        expected: &str,
        next: &str,
        consume_selector: &str,
        consume_kind: &str,
    ) -> Result<Option<CeremonyRecord>> {
        self.inner
            .transition_and_consume(
                transition_selector,
                transition_kind,
                expected,
                next,
                consume_selector,
                consume_kind,
            )
            .await
    }

    async fn transition_and_consume_exact(
        &self,
        transition_selector: &str,
        transition_kind: &str,
        expected: &str,
        next: &str,
        consume_selector: &str,
        consume_kind: &str,
        consume_id: &str,
    ) -> Result<Option<CeremonyRecord>> {
        self.inner
            .transition_and_consume_exact(
                transition_selector,
                transition_kind,
                expected,
                next,
                consume_selector,
                consume_kind,
                consume_id,
            )
            .await
    }
}

async fn actor(
    h: &grants_harness::GrantsHarness,
    user_id: &str,
    session_id: &str,
) -> CredentialActor {
    storage_schema::credential_actor(&h.oauth.db, user_id, 0, session_id).await
}

async fn service(h: &grants_harness::GrantsHarness) -> DeviceAuthorizationService {
    service_with_config(h, DeviceAuthorizationConfig::default()).await
}

async fn service_with_config(
    h: &grants_harness::GrantsHarness,
    config: DeviceAuthorizationConfig,
) -> DeviceAuthorizationService {
    DeviceAuthorizationService::new(
        h.storage(),
        h.storage(),
        h.storage(),
        h.gate.clone(),
        h.sessions.clone(),
        h.oauth.limiter.clone(),
        h.oauth.encryptor.clone(),
        config,
    )
}

// --- issue_code --------------------------------------------------------

#[test]
fn device_authorization_exposes_no_oauth_client_or_scope_surface() {
    let source = include_str!("../src/oauth/device.rs");
    for forbidden in [
        "pub struct DeviceClient",
        "pub struct DeviceClientRegistry",
        "allowed_scopes",
        "pub client_id:",
        "pub scopes:",
        "issue_code(&self, client_id",
        "ClientAuthentication",
    ] {
        assert!(
            !source.contains(forbidden),
            "first-party device login must not expose `{forbidden}`"
        );
    }
}

#[tokio::test]
async fn issue_code_and_verify_expose_only_first_party_device_state() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();
    assert!(!issued.user_code.is_empty());
    assert!(issued.interval > 0);
    assert!(issued.expires_in > 0);

    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Pending);
    assert!(display.expires_at > Utc::now());
}

#[tokio::test]
async fn verify_accepts_lowercase_and_dehyphenated_user_code() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();

    // RFC 8628 §6.1: transcription-tolerant entry -- case and the display
    // hyphen are both normalized before the lookup.
    let lower = issued.user_code.to_lowercase();
    let display = svc.verify(&lower).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Pending);

    let no_hyphen: String = issued.user_code.chars().filter(|c| *c != '-').collect();
    let display = svc.verify(&no_hyphen.to_lowercase()).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Pending);
}

#[tokio::test]
async fn verify_surfaces_decided_state_instead_of_a_dead_end_prompt() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "ivan@example.test").await;
    let actor = actor(&h, &user_id, "ivan-browser").await;
    let svc = service(&h).await;

    let issued = svc.issue_code().await.unwrap();
    svc.deny(&issued.user_code, &actor).await.unwrap();

    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Denied);
}

#[tokio::test]
async fn verify_on_unknown_user_code_is_not_found_and_never_mutates() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();

    let err = svc.verify("WRONG-CODE").await.unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));

    // The real ceremony is untouched.
    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Pending);
}

// --- approve/deny --------------------------------------------------------

#[tokio::test]
async fn approve_without_two_factor_stores_an_encrypted_one_time_device_session() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "alice@example.test").await;
    let actor = actor(&h, &user_id, "alice-browser").await;
    h.factors.set_enrolled(false);
    let svc = service(&h).await;

    let issued = svc.issue_code().await.unwrap();
    let outcome = svc
        .approve(
            &issued.user_code,
            &actor,
            SessionMetadata {
                user_agent: Some("first-party-cli".to_owned()),
                ip_address: Some("192.0.2.10".to_owned()),
            },
        )
        .await
        .unwrap();
    assert!(matches!(outcome, DeviceApprovalOutcome::Approved));

    let stored = h
        .storage()
        .peek_device(&issued.user_code)
        .await
        .unwrap()
        .expect("approved ceremony remains until the device polls");
    assert!(!stored.payload.is_empty());

    let grant = match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(grant) => grant,
        other => panic!("expected Success, got {other:?}"),
    };
    assert_eq!(grant.user_id(), user_id);
    assert_eq!(
        grant.metadata(),
        &SessionMetadata {
            user_agent: Some("first-party-cli".to_owned()),
            ip_address: Some("192.0.2.10".to_owned()),
        }
    );

    let bearer = grant.into_bearer();
    let token = bearer.expose_token_once();
    assert!(
        !stored
            .payload
            .windows(token.expose_secret().len())
            .any(|window| window == token.expose_secret().as_bytes()),
        "the persisted device grant must be encrypted, not a plaintext bearer"
    );
    let verified = h
        .sessions
        .verify_bearer(token.expose_secret())
        .await
        .expect("poll returns a real Magnetar session");
    assert_eq!(verified.user_id(), user_id);

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        other => panic!("expected ExpiredToken on redemption replay, got {other:?}"),
    }
}

#[tokio::test]
async fn issued_transition_failure_does_not_destroy_device_grant() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "transition-failure@example.test").await;
    let actor = actor(&h, &user_id, "transition-failure-browser").await;
    h.factors.set_enrolled(false);
    let svc = service(&h).await;

    let issued = svc.issue_code().await.unwrap();
    let outcome = svc
        .approve(&issued.user_code, &actor, SessionMetadata::default())
        .await
        .unwrap();
    assert!(matches!(outcome, DeviceApprovalOutcome::Approved));

    let approved = h
        .storage()
        .peek_device(&issued.user_code)
        .await
        .unwrap()
        .expect("approved ceremony remains until the device polls");
    let approved_state = approved.state.clone();
    let grant_selector = approved_state
        .strip_prefix("approved:")
        .expect("approved state carries its grant selector")
        .to_owned();

    h.oauth
        .db
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TRIGGER fail_device_issued_transition
             BEFORE UPDATE OF state ON storage_ceremonies
             WHEN OLD.kind = 'device-authorization'
              AND NEW.kind = 'device-authorization'
              AND OLD.state <> 'issued'
              AND NEW.state = 'issued'
             BEGIN
                 SELECT RAISE(ABORT, 'forced device transition failure');
             END"
            .to_owned(),
        ))
        .await
        .expect("install scoped transition failure trigger");

    svc.poll(issued.device_code.expose_secret())
        .await
        .expect_err("issued transition failure must surface");

    let after_failure = h
        .storage()
        .peek_device(&issued.user_code)
        .await
        .unwrap()
        .expect("failed transition preserves the approved device ceremony");
    assert_eq!(after_failure.state, approved_state);
    assert!(
        h.storage()
            .peek(&grant_selector, "device-authorization-grant")
            .await
            .unwrap()
            .is_some(),
        "failed transition must preserve the one-time device grant"
    );

    h.oauth
        .db
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER fail_device_issued_transition".to_owned(),
        ))
        .await
        .expect("remove transition failure trigger before retry");

    let grant = match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(grant) => grant,
        other => panic!("expected Success after retry, got {other:?}"),
    };
    assert_eq!(grant.user_id(), user_id);

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        other => panic!("expected ExpiredToken on redemption replay, got {other:?}"),
    }
}

#[tokio::test]
async fn transient_grant_decode_failure_keeps_approved_device_grant_retryable() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "grant-decode-retry@example.test").await;
    let actor = actor(&h, &user_id, "grant-decode-retry-browser").await;
    h.factors.set_enrolled(false);
    let encryptor = Arc::new(FailOnceGrantDecryptor {
        inner: h.oauth.encryptor.clone(),
        fail_next_grant_decrypt: AtomicBool::new(true),
    });
    let svc = DeviceAuthorizationService::new(
        h.storage(),
        h.storage(),
        h.storage(),
        h.gate.clone(),
        h.sessions.clone(),
        h.oauth.limiter.clone(),
        encryptor,
        DeviceAuthorizationConfig::default(),
    );

    let issued = svc.issue_code().await.unwrap();
    let outcome = svc
        .approve(&issued.user_code, &actor, SessionMetadata::default())
        .await
        .unwrap();
    assert!(matches!(outcome, DeviceApprovalOutcome::Approved));

    let error = svc
        .poll(issued.device_code.expose_secret())
        .await
        .expect_err("the first session grant decode must surface its local failure");
    assert!(
        matches!(error, Error::Internal { message } if message == TRANSIENT_GRANT_DECODE_ERROR),
        "the forced local decode error must be preserved"
    );

    let grant = match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(grant) => grant,
        other => panic!("expected Success after the transient decode failure, got {other:?}"),
    };
    assert_eq!(grant.user_id(), user_id);
}

#[tokio::test]
async fn poll_rejects_stale_preflight_when_grant_is_replaced_before_atomic_redemption() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "grant-replacement@example.test").await;
    let actor = actor(&h, &user_id, "grant-replacement-browser").await;
    h.factors.set_enrolled(false);
    let ceremonies = Arc::new(ReplaceGrantAfterPeek {
        inner: h.storage(),
        replace_next_grant: AtomicBool::new(true),
    });
    let svc = DeviceAuthorizationService::new(
        ceremonies,
        h.storage(),
        h.storage(),
        h.gate.clone(),
        h.sessions.clone(),
        h.oauth.limiter.clone(),
        h.oauth.encryptor.clone(),
        DeviceAuthorizationConfig::default(),
    );

    let issued = svc.issue_code().await.unwrap();
    let outcome = svc
        .approve(&issued.user_code, &actor, SessionMetadata::default())
        .await
        .unwrap();
    assert!(matches!(outcome, DeviceApprovalOutcome::Approved));

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        DevicePollOutcome::Success(_) => {
            panic!("a stale preflight must not redeem its replacement, got Success")
        }
        other => panic!("a stale preflight must not redeem its replacement, got {other:?}"),
    }
    let approved = h
        .storage()
        .peek_device(&issued.user_code)
        .await
        .unwrap()
        .expect("replacement mismatch must preserve the approved ceremony");
    let grant_selector = approved
        .state
        .strip_prefix("approved:")
        .expect("approved state retains its grant selector");
    assert!(
        h.storage()
            .peek(grant_selector, "device-authorization-grant")
            .await
            .unwrap()
            .is_some(),
        "replacement mismatch must preserve grant B"
    );

    let grant = match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(grant) => grant,
        other => panic!("preserved replacement grant must remain retryable, got {other:?}"),
    };
    assert_eq!(grant.user_id(), user_id);
}

#[tokio::test]
async fn complete_approval_binds_factor_challenge_to_original_user_code_and_actor() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "bob@example.test").await;
    let bob_actor = actor(&h, &user_id, "bob-browser").await;
    let other_user_id = create_user(&h.storage(), "mallory@example.test").await;
    let other_actor = actor(&h, &other_user_id, "mallory-browser").await;
    h.factors.set_enrolled(true);
    h.factors.set_code("123456");
    let svc = service(&h).await;

    let bound = svc.issue_code().await.unwrap();
    let untouched = svc.issue_code().await.unwrap();
    let selector = match svc
        .approve(&bound.user_code, &bob_actor, SessionMetadata::default())
        .await
        .unwrap()
    {
        DeviceApprovalOutcome::FactorRequired { challenge_selector } => challenge_selector,
        other => panic!("expected FactorRequired, got {other:?}"),
    };

    assert!(
        svc.complete_approval(&selector, "123456", &other_actor)
            .await
            .is_err()
    );
    match svc.poll(bound.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("wrong actor must leave the bound request pending, got {other:?}"),
    }

    svc.complete_approval(&selector, "123456", &bob_actor)
        .await
        .unwrap();
    match svc.poll(bound.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(grant) => assert_eq!(grant.user_id(), user_id),
        other => panic!("completed challenge must release the bound grant, got {other:?}"),
    }
    match svc
        .poll(untouched.device_code.expose_secret())
        .await
        .unwrap()
    {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("challenge must not approve another user_code, got {other:?}"),
    }
}

#[tokio::test]
async fn denial_fences_a_pending_factor_continuation_before_proof_claim_or_session_mint() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "denied-race@example.test").await;
    let actor = actor(&h, &user_id, "denied-race-browser").await;
    h.factors.set_enrolled(true);
    h.factors.set_code("123456");
    let svc = service(&h).await;
    let sessions_before = h.sessions.list_for_user(&user_id).await.unwrap().len();

    let denied = svc.issue_code().await.unwrap();
    let selector = match svc
        .approve(&denied.user_code, &actor, SessionMetadata::default())
        .await
        .unwrap()
    {
        DeviceApprovalOutcome::FactorRequired { challenge_selector } => challenge_selector,
        other => panic!("expected FactorRequired, got {other:?}"),
    };
    svc.deny(&denied.user_code, &actor).await.unwrap();

    let err = svc
        .complete_approval(&selector, "123456", &actor)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Conflict { .. }),
        "denial must win over a stale factor continuation, got {err:?}"
    );
    assert_eq!(
        h.factors.claim_count(),
        0,
        "a stale continuation must be fenced before claiming the factor proof"
    );
    assert_eq!(
        h.sessions.list_for_user(&user_id).await.unwrap().len(),
        sessions_before,
        "the losing completion must not mint a session or device grant"
    );
    match svc.poll(denied.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AccessDenied => {}
        other => panic!("denied device code must remain terminal, got {other:?}"),
    }

    let claimable = svc.issue_code().await.unwrap();
    let claimable_selector = match svc
        .approve(&claimable.user_code, &actor, SessionMetadata::default())
        .await
        .unwrap()
    {
        DeviceApprovalOutcome::FactorRequired { challenge_selector } => challenge_selector,
        other => panic!("expected FactorRequired, got {other:?}"),
    };
    svc.complete_approval(&claimable_selector, "123456", &actor)
        .await
        .expect("unclaimed proof remains usable for another live challenge");
    assert_eq!(
        h.factors.claim_count(),
        1,
        "only the live challenge may claim the proof"
    );
    match svc
        .poll(claimable.device_code.expose_secret())
        .await
        .unwrap()
    {
        DevicePollOutcome::Success(grant) => assert_eq!(grant.user_id(), user_id),
        other => panic!("live challenge must release its device grant, got {other:?}"),
    }
}

#[tokio::test]
async fn deny_is_terminal_and_never_issues_a_device_session() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "carol@example.test").await;
    let actor = actor(&h, &user_id, "carol-browser").await;
    let svc = service(&h).await;

    let issued = svc.issue_code().await.unwrap();
    svc.deny(&issued.user_code, &actor).await.unwrap();

    for _ in 0..2 {
        match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
            DevicePollOutcome::AccessDenied => {}
            other => panic!("expected terminal AccessDenied, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn approve_and_deny_are_exactly_one_winner_under_concurrency() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "dave@example.test").await;
    let actor = actor(&h, &user_id, "dave-browser").await;
    h.factors.set_enrolled(false);
    let svc = Arc::new(service(&h).await);

    let issued = svc.issue_code().await.unwrap();
    let user_code = issued.user_code.clone();

    let a = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            svc.approve(&user_code, &actor, SessionMetadata::default())
                .await
        })
    };
    let b = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let actor = actor.clone();
        tokio::spawn(async move { svc.deny(&user_code, &actor).await })
    };
    let (a, b): (Result<DeviceApprovalOutcome>, Result<()>) = (a.await.unwrap(), b.await.unwrap());

    let winners =
        usize::from(matches!(a, Ok(DeviceApprovalOutcome::Approved))) + usize::from(b.is_ok());
    assert_eq!(winners, 1, "approve={a:?} deny={b:?}");
}

#[tokio::test]
async fn approve_wrong_user_code_is_not_found_and_never_mutates() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "erin@example.test").await;
    let actor = actor(&h, &user_id, "erin-browser").await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();

    let err = svc
        .approve("WRONG-CODE", &actor, SessionMetadata::default())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("expected still-pending, got {other:?}"),
    }
}

// --- poll: expired / already-issued -------------------------------------

#[tokio::test]
async fn poll_unknown_device_code_is_expired_token() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    match svc.poll("unknown-device-code").await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        other => panic!("expected ExpiredToken, got {other:?}"),
    }
}

#[tokio::test]
async fn poll_on_unknown_device_code_still_consults_the_abuse_limiter() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;

    // Enumerating device_codes must not be a free action: the limiter is
    // consulted before the lookup even happens, so a rejected permit is
    // reported as slow_down (using the configured base interval, since
    // there is no per-ceremony record to read an escalated one from)
    // rather than the code's existence leaking through an early return.
    h.oauth.limiter.set_mode(oauth_harness::LimiterMode::Reject);
    match svc.poll("guessed-device-code").await.unwrap() {
        DevicePollOutcome::SlowDown { interval } => {
            assert_eq!(
                interval,
                DeviceAuthorizationConfig::default().poll_interval.as_secs()
            );
        }
        other => panic!("expected SlowDown from a rejected limiter permit, got {other:?}"),
    }
}

#[tokio::test]
async fn denied_ceremony_reports_access_denied_even_when_polled_immediately_twice() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "grace@example.test").await;
    let actor = actor(&h, &user_id, "grace-browser").await;
    let svc = service(&h).await;

    let issued = svc.issue_code().await.unwrap();
    // A first poll while still pending consumes the "first poll always
    // proceeds" allowance and records a last-poll timestamp.
    svc.poll(issued.device_code.expose_secret()).await.unwrap();
    svc.deny(&issued.user_code, &actor).await.unwrap();

    // RFC 8628 §3.5: slow_down is a variant of authorization_pending, not
    // a general rate limit -- a decided ceremony reports its real terminal
    // outcome immediately, even polled back-to-back with no wait.
    for _ in 0..2 {
        match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
            DevicePollOutcome::AccessDenied => {}
            other => panic!("expected AccessDenied, not masked by slow_down, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn issued_codes_expire_and_then_poll_and_verify_report_not_found() {
    let h = grants_harness::harness().await;
    let svc = service_with_config(
        &h,
        DeviceAuthorizationConfig {
            code_ttl: StdDuration::from_millis(20),
            ..DeviceAuthorizationConfig::default()
        },
    )
    .await;
    let issued = svc.issue_code().await.unwrap();
    tokio::time::sleep(StdDuration::from_millis(60)).await;

    assert!(svc.verify(&issued.user_code).await.is_err());
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        other => panic!("expected ExpiredToken, got {other:?}"),
    }
}

// --- poll: slow_down escalation and abuse limiter ------------------------

#[tokio::test]
async fn rapid_repeat_polls_yield_slow_down_and_escalate_the_interval() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();
    assert_eq!(issued.interval, 5, "default base interval");

    // First poll always proceeds (no prior poll to compare against).
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("expected AuthorizationPending on first poll, got {other:?}"),
    }
    // A second, immediate poll is faster than the interval -> slow_down,
    // and the server-communicated interval escalates by 5s (RFC 8628
    // §3.5) so a well-behaved client's next wait actually grows.
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::SlowDown { interval } => assert_eq!(interval, 10),
        other => panic!("expected SlowDown, got {other:?}"),
    }
    // A third, also-immediate poll is still faster than the escalated
    // interval -> slow_down again, escalating further.
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::SlowDown { interval } => assert_eq!(interval, 15),
        other => panic!("expected SlowDown again, got {other:?}"),
    }
}

#[tokio::test]
async fn interval_escalation_is_clamped_to_the_remaining_code_ttl() {
    let h = grants_harness::harness().await;
    // A base interval (5s) larger than the code's own remaining lifetime
    // (3s) forces the very first escalation to need clamping.
    let svc = service_with_config(
        &h,
        DeviceAuthorizationConfig {
            code_ttl: StdDuration::from_secs(3),
            ..DeviceAuthorizationConfig::default()
        },
    )
    .await;
    let issued = svc.issue_code().await.unwrap();

    svc.poll(issued.device_code.expose_secret()).await.unwrap();
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::SlowDown { interval } => {
            assert!(
                interval <= 3,
                "escalated interval {interval} must not exceed the code's remaining ttl"
            );
        }
        other => panic!("expected SlowDown, got {other:?}"),
    }
}

#[tokio::test]
async fn abuse_limiter_rejection_yields_slow_down() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();

    h.oauth.limiter.set_mode(oauth_harness::LimiterMode::Reject);
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::SlowDown { interval } => assert_eq!(interval, issued.interval),
        other => panic!("expected SlowDown from a rejected limiter permit, got {other:?}"),
    }
}
#[tokio::test]
async fn abuse_limiter_backend_failure_fails_closed() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code().await.unwrap();

    h.oauth.limiter.set_mode(oauth_harness::LimiterMode::Error);
    let err = svc
        .poll(issued.device_code.expose_secret())
        .await
        .unwrap_err();
    // A limiter backend failure must never be silently treated as allowed,
    // nor conflated with an unrelated store/decrypt failure.
    assert!(
        matches!(err, Error::DependencyUnavailable { .. }),
        "a limiter backend outage must surface as the dependency failure, got {err:?}"
    );
}

#[tokio::test]
async fn poll_abuse_policy_is_configurable() {
    let h = grants_harness::harness().await;
    let svc = service_with_config(
        &h,
        DeviceAuthorizationConfig {
            poll_abuse_policy: AbusePolicy {
                max_requests: 5,
                window: StdDuration::from_secs(1),
            },
            ..DeviceAuthorizationConfig::default()
        },
    )
    .await;
    let issued = svc.issue_code().await.unwrap();
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("expected AuthorizationPending, got {other:?}"),
    }
    // The configured policy and the per-device_code key actually reached
    // the limiter -- not just that some call was made under some policy.
    let acquired = h.oauth.limiter.acquired.lock();
    assert_eq!(
        acquired.last(),
        Some(&(
            format!("device-poll:{}", issued.device_code.expose_secret()),
            5
        )),
        "the configured policy and per-code key must reach the limiter"
    );
}
