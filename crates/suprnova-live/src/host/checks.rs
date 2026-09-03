//! Exact typed dispositions for every host request-authenticity check.

use std::collections::BTreeMap;

use crate::identity::UnixMillis;

use super::{HostContextError, HostContextErrorKind};

/// Closed host authenticity checks required before Live kernel entry.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CheckKind {
    /// Current origin or cross-origin deployment policy.
    Origin,
    /// Current state-changing request CSRF policy.
    Csrf,
    /// Current session validity and rotation state.
    Session,
    /// Current principal resolution.
    Principal,
    /// Current tenant resolution.
    Tenant,
    /// Trusted proxy and transport normalization.
    Proxy,
    /// Current promotion/operation rate admission.
    RateLimit,
    /// Completion of route-specific middleware prerequisites.
    Middleware,
}

impl CheckKind {
    /// Exact checks a conforming host adapter must account for.
    pub const ALL: [Self; 8] = [
        Self::Origin,
        Self::Csrf,
        Self::Session,
        Self::Principal,
        Self::Tenant,
        Self::Proxy,
        Self::RateLimit,
        Self::Middleware,
    ];
}

/// Policy-declared reason one exact host check does not apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyReason {
    /// A trusted internal request path does not use browser origin policy.
    TrustedInternalOrigin,
    /// The configured request class is explicitly outside CSRF policy.
    StatelessCsrfPolicy,
    /// The route is explicitly stateless.
    StatelessRequest,
    /// The route explicitly permits an anonymous principal.
    AnonymousPrincipal,
    /// The route is explicitly outside tenant scoping.
    TenantlessRoute,
    /// The request arrived from a direct peer without proxy interpretation.
    DirectPeer,
    /// A declared upstream boundary completed rate admission.
    UpstreamRateLimited,
    /// The route declares no additional middleware prerequisites.
    NoAdditionalMiddleware,
}

/// Accepted outcome of one configured host authenticity check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckDisposition {
    /// The host performed and passed the check for the current request.
    Passed,
    /// Route policy explicitly declares why this exact check is inapplicable.
    NotRequired(PolicyReason),
}

/// One time-bounded host check fact before complete-context validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckFact {
    disposition: CheckDisposition,
    expires_at: UnixMillis,
}

impl CheckFact {
    /// Records a typed disposition and its exclusive validity deadline.
    #[must_use]
    pub const fn new(disposition: CheckDisposition, expires_at: UnixMillis) -> Self {
        Self {
            disposition,
            expires_at,
        }
    }
}

/// Incomplete host check facts that carry no authority until validation.
#[derive(Clone, Debug, Default)]
pub struct HostCheckFacts {
    facts: BTreeMap<CheckKind, CheckFact>,
}

impl HostCheckFacts {
    /// Creates an empty non-authoritative check collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            facts: BTreeMap::new(),
        }
    }

    /// Records one check exactly once.
    pub fn record(&mut self, kind: CheckKind, fact: CheckFact) -> Result<(), HostContextError> {
        if self.facts.insert(kind, fact).is_some() {
            return Err(HostContextError::new(HostContextErrorKind::DuplicateCheck));
        }
        Ok(())
    }

    /// Requires every security check to be present, unexpired at `now`, and
    /// carrying a disposition its kind permits.
    ///
    /// Mount-bound requests validate through
    /// [`LiveRequestContextValidator`](super::LiveRequestContextValidator);
    /// a host boundary without a mount, such as a document transport, uses
    /// this to refuse a request whose middleware chain skipped a check.
    pub fn require_complete(&self, now: UnixMillis) -> Result<(), HostContextError> {
        RequiredChecks::validate(self.clone(), now, UnixMillis::new(u64::MAX)).map(|_| ())
    }
}

/// Complete validated check dispositions retained by trusted request context.
#[derive(Clone, Debug)]
pub struct RequiredChecks {
    dispositions: BTreeMap<CheckKind, CheckDisposition>,
}

impl RequiredChecks {
    pub(crate) fn validate(
        facts: HostCheckFacts,
        now: UnixMillis,
        mut effective_expiry: UnixMillis,
    ) -> Result<(Self, UnixMillis), HostContextError> {
        let mut dispositions = BTreeMap::new();
        for kind in CheckKind::ALL {
            let fact = facts
                .facts
                .get(&kind)
                .ok_or_else(|| HostContextError::new(HostContextErrorKind::MissingCheck))?;
            if fact.expires_at <= now {
                return Err(HostContextError::new(HostContextErrorKind::CheckExpired));
            }
            if !disposition_matches(kind, fact.disposition) {
                return Err(HostContextError::new(
                    HostContextErrorKind::InvalidCheckDisposition,
                ));
            }
            effective_expiry = effective_expiry.min(fact.expires_at);
            dispositions.insert(kind, fact.disposition);
        }
        Ok((Self { dispositions }, effective_expiry))
    }

    /// Returns the validated disposition for one exact check.
    #[must_use]
    pub fn get(&self, kind: CheckKind) -> CheckDisposition {
        self.dispositions[&kind]
    }
}

fn disposition_matches(kind: CheckKind, disposition: CheckDisposition) -> bool {
    matches!(
        (kind, disposition),
        (_, CheckDisposition::Passed)
            | (
                CheckKind::Origin,
                CheckDisposition::NotRequired(PolicyReason::TrustedInternalOrigin)
            )
            | (
                CheckKind::Csrf,
                CheckDisposition::NotRequired(PolicyReason::StatelessCsrfPolicy)
            )
            | (
                CheckKind::Session,
                CheckDisposition::NotRequired(PolicyReason::StatelessRequest)
            )
            | (
                CheckKind::Principal,
                CheckDisposition::NotRequired(PolicyReason::AnonymousPrincipal)
            )
            | (
                CheckKind::Tenant,
                CheckDisposition::NotRequired(PolicyReason::TenantlessRoute)
            )
            | (
                CheckKind::Proxy,
                CheckDisposition::NotRequired(PolicyReason::DirectPeer)
            )
            | (
                CheckKind::RateLimit,
                CheckDisposition::NotRequired(PolicyReason::UpstreamRateLimited),
            )
            | (
                CheckKind::Middleware,
                CheckDisposition::NotRequired(PolicyReason::NoAdditionalMiddleware),
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_facts(expires_at: u64) -> HostCheckFacts {
        let mut facts = HostCheckFacts::new();
        for kind in CheckKind::ALL {
            facts
                .record(
                    kind,
                    CheckFact::new(CheckDisposition::Passed, UnixMillis::new(expires_at)),
                )
                .expect("record once");
        }
        facts
    }

    #[test]
    fn complete_unexpired_facts_are_required_without_a_mount() {
        complete_facts(10)
            .require_complete(UnixMillis::new(5))
            .expect("complete facts pass");
        let expired = complete_facts(5)
            .require_complete(UnixMillis::new(5))
            .expect_err("an expired fact fails closed");
        assert_eq!(expired.kind(), HostContextErrorKind::CheckExpired);
        for missing in CheckKind::ALL {
            let mut facts = HostCheckFacts::new();
            for kind in CheckKind::ALL {
                if kind == missing {
                    continue;
                }
                facts
                    .record(
                        kind,
                        CheckFact::new(CheckDisposition::Passed, UnixMillis::new(10)),
                    )
                    .expect("record once");
            }
            let error = facts
                .require_complete(UnixMillis::new(5))
                .expect_err("a skipped check fails closed");
            assert_eq!(
                error.kind(),
                HostContextErrorKind::MissingCheck,
                "{missing:?}"
            );
        }
    }
}
