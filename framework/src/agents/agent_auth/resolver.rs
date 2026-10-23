//! Agent tokens as MCP callers (AGENT-007, AGENT-010).

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use magnetar::agent_auth::RegistrationRecord;
use magnetar::agent_auth::store::{STATUS_ACTIVE, STATUS_CLAIMED, STATUS_UNCLAIMED};
use magnetar::authorization_server::VerifiedAccess;

use super::MountedAgentAuth;
use crate::agents::oauth::load_user;
use crate::agents::{ResolveError, ToolCaller};
use crate::error::FrameworkError;

/// Turns a verified agent access token into an agent caller, after reading
/// the registration it was issued from.
///
/// The authorization server already checked the token: it is live, for
/// this resource, and its grant was not revoked. The registration check
/// still matters, because a revocation writes the registration first and
/// revokes its grants after: a grant revocation that fails leaves a live
/// grant behind a revoked registration, and a claim whose revocation of
/// the pre-claim grants fails leaves a pre-claim token behind a claimed
/// one. Neither token gets through here.
///
/// | Registration | Answer |
/// |---|---|
/// | gone | [`ResolveError::Invalid`] |
/// | `revoked`, `expired`, `pending` | [`ResolveError::Invalid`] |
/// | `unclaimed`, window open, token bound to no user | the pre-claim scopes the token carries |
/// | `unclaimed`, window passed (the expiry pass has not run yet) | [`ResolveError::Invalid`] |
/// | `active` or `claimed`, token bound to the registration's user | the scopes the token carries that the registration was granted and the configuration still grants after a claim, acting as the user |
/// | `active` or `claimed`, token bound to no user or another (a pre-claim token) | [`ResolveError::Invalid`] |
/// | the store fails | [`ResolveError::Unavailable`] |
///
/// The scopes are the token's, cut to what the registration holds now: the
/// current pre-claim scopes before a claim, and after it the scopes it was
/// granted that the current post-claim scopes still name. A configuration
/// that names fewer scopes applies at once, to a token minted before the
/// change as to a new one.
pub(crate) struct AgentTokens {
    mounted: Arc<MountedAgentAuth>,
}

impl AgentTokens {
    pub(crate) fn new(mounted: Arc<MountedAgentAuth>) -> Self {
        Self { mounted }
    }

    /// The caller `access` stands for.
    pub(crate) async fn caller(&self, access: &VerifiedAccess) -> Result<ToolCaller, ResolveError> {
        let record = match self
            .mounted
            .store
            .find_registration(&access.client_id)
            .await
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Err(ResolveError::Invalid(
                    "The token's agent registration no longer exists.",
                ));
            }
            Err(error) => {
                return Err(ResolveError::Unavailable(FrameworkError::internal(
                    format!("agent auth could not read a token's registration: {error}"),
                )));
            }
        };
        let config = self.mounted.engine.config();
        let entitlement = entitlement(
            &record,
            access.user_id.as_deref(),
            &config.pre_claim_scopes,
            &config.post_claim_scopes,
            crate::clock::now(),
        )?;
        let scopes: Vec<&String> = access
            .scopes
            .iter()
            .filter(|scope| entitlement.contains(scope.as_str()))
            .collect();
        let mut caller = ToolCaller::agent(record.id.clone())
            .scopes(scopes)
            .claimed(record.user_id.is_some());
        if let Some(issuer) = &record.issuer {
            caller = caller.client(issuer.clone());
        }
        if let Some(user_id) = &record.user_id {
            caller = caller.bound_to(load_user(user_id).await?);
        }
        Ok(caller)
    }
}

/// What `record` holds now for a token bound to `token_user`, under the
/// configuration's current pre-claim and post-claim scopes, or why the
/// token no longer works.
fn entitlement<'a>(
    record: &'a RegistrationRecord,
    token_user: Option<&str>,
    pre_claim_scopes: &'a [String],
    post_claim_scopes: &[String],
    now: DateTime<Utc>,
) -> Result<BTreeSet<&'a str>, ResolveError> {
    match record.status.as_str() {
        STATUS_UNCLAIMED => {
            let open = record.claim_expires_at.is_some_and(|window| window > now);
            if !open || token_user.is_some() || record.user_id.is_some() {
                return Err(ResolveError::Invalid("The agent registration expired."));
            }
            Ok(pre_claim_scopes.iter().map(String::as_str).collect())
        }
        STATUS_ACTIVE | STATUS_CLAIMED => {
            if record.user_id.is_none() || token_user != record.user_id.as_deref() {
                return Err(ResolveError::Invalid(
                    "The token was issued before the agent registration was claimed.",
                ));
            }
            Ok(record
                .scopes
                .iter()
                .filter(|scope| post_claim_scopes.contains(scope))
                .map(String::as_str)
                .collect())
        }
        _ => Err(ResolveError::Invalid(
            "The agent registration was revoked or has ended.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magnetar::agent_auth::store::{STATUS_EXPIRED, STATUS_PENDING, STATUS_REVOKED};

    fn record(status: &str, user: Option<&str>, window_open: bool) -> RegistrationRecord {
        let now = crate::clock::now();
        RegistrationRecord {
            id: "reg_1".to_owned(),
            kind: "anonymous".to_owned(),
            status: status.to_owned(),
            user_id: user.map(str::to_owned),
            claim_email: None,
            claim_user_id: None,
            claim_token_hash: None,
            claim_expires_at: Some(if window_open {
                now + chrono::Duration::hours(1)
            } else {
                now - chrono::Duration::seconds(1)
            }),
            attempt_id: None,
            attempt_token_hash: None,
            user_code_hash: None,
            user_code_expires_at: None,
            user_code_failures: 0,
            last_polled_at: None,
            delegation_key: None,
            issuer: None,
            subject: None,
            scopes: vec!["posts:read".to_owned(), "posts:write".to_owned()],
            assertion_version: 1,
            auth_epoch: None,
            client_address: None,
            cleanup_pending: false,
            created_at: now,
            claimed_at: None,
            revoked_at: None,
        }
    }

    fn held(result: Result<BTreeSet<&str>, ResolveError>) -> Vec<String> {
        match result {
            Ok(scopes) => scopes.into_iter().map(str::to_owned).collect(),
            Err(error) => panic!("expected scopes, got {error:?}"),
        }
    }

    fn refused(result: Result<BTreeSet<&str>, ResolveError>) -> &'static str {
        match result {
            Err(ResolveError::Invalid(reason)) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn agent_010_resolver_maps_every_registration_status() {
        let pre = vec!["posts:read".to_owned()];
        let post = vec!["posts:read".to_owned(), "posts:write".to_owned()];
        let now = crate::clock::now();
        let unclaimed = record(STATUS_UNCLAIMED, None, true);
        assert_eq!(
            held(entitlement(&unclaimed, None, &pre, &post, now)),
            ["posts:read"]
        );
        let lapsed = record(STATUS_UNCLAIMED, None, false);
        assert_eq!(
            refused(entitlement(&lapsed, None, &pre, &post, now)),
            "The agent registration expired."
        );
        for status in [STATUS_ACTIVE, STATUS_CLAIMED] {
            let bound = record(status, Some("u1"), true);
            assert_eq!(
                held(entitlement(&bound, Some("u1"), &pre, &post, now)),
                ["posts:read", "posts:write"]
            );
            // A narrowed configuration cuts what a bound registration
            // holds at once.
            assert_eq!(
                held(entitlement(&bound, Some("u1"), &pre, &pre, now)),
                ["posts:read"]
            );
            // A pre-claim token left behind a claim, and a token of another
            // user, both stop here.
            assert_eq!(
                refused(entitlement(&bound, None, &pre, &post, now)),
                "The token was issued before the agent registration was claimed."
            );
            assert!(matches!(
                entitlement(&bound, Some("u2"), &pre, &post, now),
                Err(ResolveError::Invalid(_))
            ));
        }
        for status in [STATUS_REVOKED, STATUS_EXPIRED, STATUS_PENDING] {
            let ended = record(status, Some("u1"), true);
            assert_eq!(
                refused(entitlement(&ended, Some("u1"), &pre, &post, now)),
                "The agent registration was revoked or has ended."
            );
        }
    }
}
