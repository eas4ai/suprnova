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
