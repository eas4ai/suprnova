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
use std::time::Duration as StdDuration;

use magnetar::abuse::AbusePolicy;
use magnetar::oauth::device::{
    DeviceApprovalOutcome, DeviceAuthorizationConfig, DeviceAuthorizationService,
    DeviceCeremonyStatus, DeviceClient, DeviceClientRegistry, DevicePollOutcome,
};
use magnetar::{Error, Result};
use secrecy::ExposeSecret;

use grants_harness::create_user;

fn registry() -> DeviceClientRegistry {
    let mut registry = DeviceClientRegistry::new();
    registry
        .register(DeviceClient {
            client_id: "cli-1".to_owned(),
            display_name: "My Streaming CLI".to_owned(),
            allowed_scopes: vec!["read".to_owned(), "write".to_owned()],
        })
        .unwrap();
    registry
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
        Arc::new(registry()),
        config,
    )
}

// --- issue_code --------------------------------------------------------

#[tokio::test]
async fn issue_code_rejects_unknown_client() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let err = svc.issue_code("no-such-client", "").await.unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));
}

#[tokio::test]
async fn issue_code_rejects_disallowed_scope() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let err = svc.issue_code("cli-1", "read admin").await.unwrap_err();
    assert!(matches!(err, Error::InvalidInput { .. }));
}

#[tokio::test]
async fn issue_code_and_verify_shows_registered_client_binding() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code("cli-1", "read write").await.unwrap();
    assert!(!issued.user_code.is_empty());
    assert!(issued.interval > 0);
    assert!(issued.expires_in > 0);
    // The device_code response itself already carries the display name
    // and granted scopes (spec 09's device-code response contents), not
    // only `verify`.
    assert_eq!(issued.client_display_name, "My Streaming CLI");
    assert_eq!(issued.scopes, vec!["read".to_owned(), "write".to_owned()]);

    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.client_display_name, "My Streaming CLI");
    assert_eq!(display.scopes, vec!["read".to_owned(), "write".to_owned()]);
    assert_eq!(display.status, DeviceCeremonyStatus::Pending);
}

#[tokio::test]
async fn verify_accepts_lowercase_and_dehyphenated_user_code() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

    // RFC 8628 §6.1: transcription-tolerant entry -- case and the display
    // hyphen are both normalized before the lookup.
    let lower = issued.user_code.to_lowercase();
    let display = svc.verify(&lower).await.unwrap();
    assert_eq!(display.client_display_name, "My Streaming CLI");

    let no_hyphen: String = issued.user_code.chars().filter(|c| *c != '-').collect();
    let display = svc.verify(&no_hyphen.to_lowercase()).await.unwrap();
    assert_eq!(display.client_display_name, "My Streaming CLI");
}

#[tokio::test]
async fn verify_surfaces_decided_state_instead_of_a_dead_end_prompt() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "ivan@example.test").await;
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    svc.deny(&issued.user_code, &user_id).await.unwrap();

    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.status, DeviceCeremonyStatus::Denied);
}

#[tokio::test]
async fn verify_on_unknown_user_code_is_not_found_and_never_mutates() {
    let h = grants_harness::harness().await;
    let svc = service(&h).await;
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

    let err = svc.verify("WRONG-CODE").await.unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));

    // The real ceremony is untouched.
    let display = svc.verify(&issued.user_code).await.unwrap();
    assert_eq!(display.client_display_name, "My Streaming CLI");
}

// --- approve/deny --------------------------------------------------------

#[tokio::test]
async fn approve_by_non_enrolled_actor_transitions_to_approved_and_poll_succeeds() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "alice@example.test").await;
    h.factors.set_enrolled(false);
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    let outcome = svc.approve(&issued.user_code, &user_id).await.unwrap();
    match outcome {
        DeviceApprovalOutcome::Approved { approver_session } => {
            assert_eq!(approver_session.user_id(), user_id);
        }
        other => panic!("expected Approved, got {other:?}"),
    }

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(principal) => assert_eq!(principal.user_id(), user_id),
        other => panic!("expected Success, got {other:?}"),
    }
}

#[tokio::test]
async fn approve_by_enrolled_actor_returns_factor_required_and_does_not_transition() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "bob@example.test").await;
    h.factors.set_enrolled(true);
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    let outcome = svc.approve(&issued.user_code, &user_id).await.unwrap();
    assert!(matches!(
        outcome,
        DeviceApprovalOutcome::FactorRequired { .. }
    ));

    // Not approved yet: polling still reports pending.
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AuthorizationPending => {}
        other => panic!("expected AuthorizationPending, got {other:?}"),
    }
}

#[tokio::test]
async fn deny_transitions_to_denied_and_poll_reports_access_denied() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "carol@example.test").await;
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    svc.deny(&issued.user_code, &user_id).await.unwrap();

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::AccessDenied => {}
        other => panic!("expected AccessDenied, got {other:?}"),
    }
}

#[tokio::test]
async fn approve_and_deny_are_exactly_one_winner_under_concurrency() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "dave@example.test").await;
    h.factors.set_enrolled(false);
    let svc = Arc::new(service(&h).await);

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    let user_code = issued.user_code.clone();

    let a = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let user_id = user_id.clone();
        tokio::spawn(async move { svc.approve(&user_code, &user_id).await })
    };
    let b = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let user_id = user_id.clone();
        tokio::spawn(async move { svc.deny(&user_code, &user_id).await })
    };
    let (a, b): (Result<DeviceApprovalOutcome>, Result<()>) = (a.await.unwrap(), b.await.unwrap());

    // Exactly one of the two decisions won.
    let winners = usize::from(matches!(a, Ok(DeviceApprovalOutcome::Approved { .. })))
        + usize::from(b.is_ok());
    assert_eq!(winners, 1, "approve={a:?} deny={b:?}");
}

#[tokio::test]
async fn losing_the_approve_cas_after_the_gate_ran_revokes_the_orphaned_session() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "heidi@example.test").await;
    h.factors.set_enrolled(false);
    let svc = Arc::new(service(&h).await);

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    let user_code = issued.user_code.clone();

    // Two concurrent approve() calls for the same user: both pass the
    // gate (both mint a real session), only one wins the CAS.
    let a = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let user_id = user_id.clone();
        tokio::spawn(async move { svc.approve(&user_code, &user_id).await })
    };
    let b = {
        let svc = svc.clone();
        let user_code = user_code.clone();
        let user_id = user_id.clone();
        tokio::spawn(async move { svc.approve(&user_code, &user_id).await })
    };
    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    let winners = usize::from(matches!(a, Ok(DeviceApprovalOutcome::Approved { .. })))
        + usize::from(matches!(b, Ok(DeviceApprovalOutcome::Approved { .. })));
    assert_eq!(winners, 1, "a={a:?} b={b:?}");

    // The winner's session is live; the loser's was best-effort revoked --
    // exactly one active session survives, not two orphaned rows.
    let active = h.sessions.list_for_user(&user_id).await.unwrap();
    assert_eq!(
        active.len(),
        1,
        "the losing CAS attempt's session must be cleaned up, not left live"
    );
}

#[tokio::test]
async fn approve_wrong_user_code_is_not_found_and_never_mutates() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "erin@example.test").await;
    let svc = service(&h).await;
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

    let err = svc.approve("WRONG-CODE", &user_id).await.unwrap_err();
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
async fn poll_after_success_is_expired_token_single_shot() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "frank@example.test").await;
    h.factors.set_enrolled(false);
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    svc.approve(&issued.user_code, &user_id).await.unwrap();

    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::Success(_) => {}
        other => panic!("expected Success, got {other:?}"),
    }
    // Terminal states resolve immediately -- redemption replay is not
    // masked behind a SlowDown even though this second poll is immediate.
    match svc.poll(issued.device_code.expose_secret()).await.unwrap() {
        DevicePollOutcome::ExpiredToken => {}
        other => panic!("expected ExpiredToken on redemption replay, got {other:?}"),
    }
}

#[tokio::test]
async fn denied_ceremony_reports_access_denied_even_when_polled_immediately_twice() {
    let h = grants_harness::harness().await;
    let user_id = create_user(&h.storage(), "grace@example.test").await;
    let svc = service(&h).await;

    let issued = svc.issue_code("cli-1", "read").await.unwrap();
    // A first poll while still pending consumes the "first poll always
    // proceeds" allowance and records a last-poll timestamp.
    svc.poll(issued.device_code.expose_secret()).await.unwrap();
    svc.deny(&issued.user_code, &user_id).await.unwrap();

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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();
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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();
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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();

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
    let issued = svc.issue_code("cli-1", "read").await.unwrap();
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
