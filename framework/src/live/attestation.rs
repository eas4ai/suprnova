//! Request-bound security evidence minted only by framework-owned stages.

use std::fmt;

use sha2::{Digest, Sha256};
use suprnova_live::host::{CheckDisposition, PolicyReason};
use suprnova_live::identity::UnixMillis;
use uuid::Uuid;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct LiveRequestIdentity(Uuid);

impl LiveRequestIdentity {
    pub(crate) fn fresh() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Debug for LiveRequestIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveRequestIdentity:redacted>")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveOperation {
    Document,
    Action,
    Upload,
    SseControl,
    WebSocketHandshake,
}

impl LiveOperation {
    pub(crate) const fn requires_csrf(self) -> bool {
        matches!(self, Self::Action | Self::Upload)
    }

    const fn label(self) -> &'static [u8] {
        match self {
            Self::Document => b"document",
            Self::Action => b"action",
            Self::Upload => b"upload",
            Self::SseControl => b"sse-control",
            Self::WebSocketHandshake => b"websocket-handshake",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecurityCheck {
    Origin,
    Csrf,
    Session,
    Principal,
    Tenant,
    Proxy,
    RateLimit,
    Middleware,
}

impl SecurityCheck {
    pub(crate) const ALL: [Self; 8] = [
        Self::Origin,
        Self::Csrf,
        Self::Session,
        Self::Principal,
        Self::Tenant,
        Self::Proxy,
        Self::RateLimit,
        Self::Middleware,
    ];

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Origin => 0,
            Self::Csrf => 1,
            Self::Session => 2,
            Self::Principal => 3,
            Self::Tenant => 4,
            Self::Proxy => 5,
            Self::RateLimit => 6,
            Self::Middleware => 7,
        }
    }

    const fn execution_rank(self) -> u8 {
        match self {
            Self::Proxy => 0,
            Self::Session => 1,
            Self::Origin => 2,
            Self::Csrf => 3,
            Self::Principal => 4,
            Self::Tenant => 5,
            Self::RateLimit => 6,
            Self::Middleware => 7,
        }
    }
}

struct RequestBinding {
    request: LiveRequestIdentity,
    operation: LiveOperation,
    digest: [u8; 32],
    expires_at: UnixMillis,
}

struct Evidence {
    disposition: CheckDisposition,
    binding: [u8; 32],
    fact_fingerprint: Option<[u8; 32]>,
}

#[derive(Clone, Copy)]
pub(crate) struct AttestedFact {
    pub(crate) disposition: CheckDisposition,
    pub(crate) fingerprint: Option<[u8; 32]>,
    pub(crate) expires_at: UnixMillis,
}

/// Non-cloneable evidence collection carried by exactly one request value.
pub(crate) struct LiveSecurityAttestation {
    binding: Option<RequestBinding>,
    evidence: [Option<Evidence>; 8],
    last_execution_rank: Option<u8>,
    order_valid: bool,
}

impl LiveSecurityAttestation {
    pub(crate) fn new() -> Self {
        Self {
            binding: None,
            evidence: std::array::from_fn(|_| None),
            last_execution_rank: None,
            order_valid: true,
        }
    }

    pub(crate) fn prepare(
        &mut self,
        request: LiveRequestIdentity,
        route_pattern: &str,
        operation: LiveOperation,
        expires_at: UnixMillis,
    ) -> bool {
        if self.binding.is_some() || expires_at.get() == 0 {
            self.order_valid = false;
            return false;
        }
        let digest = binding_digest(request, route_pattern, operation);
        self.binding = Some(RequestBinding {
            request,
            operation,
            digest,
            expires_at,
        });
        true
    }

    pub(crate) fn operation(&self, request: LiveRequestIdentity) -> Option<LiveOperation> {
        self.binding
            .as_ref()
            .filter(|binding| binding.request == request)
            .map(|binding| binding.operation)
    }

    pub(crate) fn record_passed(
        &mut self,
        request: LiveRequestIdentity,
        check: SecurityCheck,
        fact: Option<&[u8]>,
    ) -> bool {
        self.record(
            request,
            check,
            CheckDisposition::Passed,
            fact.map(|value| purpose_fingerprint(check, value)),
            true,
        )
    }

    /// Records a check the server proved before any middleware could run.
    ///
    /// The WebSocket upgrade path verifies `Origin` ahead of the chain, so
    /// that evidence must not claim a position in the middleware order.
    pub(crate) fn record_passed_before_chain(
        &mut self,
        request: LiveRequestIdentity,
        check: SecurityCheck,
    ) -> bool {
        self.record(request, check, CheckDisposition::Passed, None, false)
    }

    pub(crate) fn record_not_required(
        &mut self,
        request: LiveRequestIdentity,
        check: SecurityCheck,
        reason: PolicyReason,
    ) -> bool {
        if !reason_matches(check, reason) {
            self.order_valid = false;
            return false;
        }
        self.record(
            request,
            check,
            CheckDisposition::NotRequired(reason),
            None,
            false,
        )
    }

    fn record(
        &mut self,
        request: LiveRequestIdentity,
        check: SecurityCheck,
        disposition: CheckDisposition,
        fact_fingerprint: Option<[u8; 32]>,
        enforce_execution_order: bool,
    ) -> bool {
        let Some(binding) = self.binding.as_ref() else {
            return false;
        };
        if binding.request != request {
            self.order_valid = false;
            return false;
        }
        let index = check.index();
        if self.evidence[index].is_some() {
            self.order_valid = false;
            return false;
        }
        if enforce_execution_order {
            let rank = check.execution_rank();
            if self.last_execution_rank.is_some_and(|last| rank < last) {
                self.order_valid = false;
            }
            self.last_execution_rank = Some(rank);
        }
        self.evidence[index] = Some(Evidence {
            disposition,
            binding: binding.digest,
            fact_fingerprint,
        });
        self.order_valid
    }

    pub(crate) fn present(&self) -> u8 {
        self.evidence
            .iter()
            .enumerate()
            .fold(0_u8, |bits, (index, evidence)| {
                bits | (u8::from(evidence.is_some()) << index)
            })
    }

    pub(crate) fn disposition(&self, check: SecurityCheck) -> Option<CheckDisposition> {
        self.evidence[check.index()]
            .as_ref()
            .filter(|evidence| {
                self.binding
                    .as_ref()
                    .is_some_and(|binding| evidence.binding == binding.digest)
            })
            .map(|evidence| evidence.disposition)
    }

    pub(crate) fn fact(
        &self,
        request: LiveRequestIdentity,
        check: SecurityCheck,
    ) -> Option<AttestedFact> {
        let binding = self
            .binding
            .as_ref()
            .filter(|binding| binding.request == request)?;
        let evidence = self.evidence[check.index()]
            .as_ref()
            .filter(|evidence| evidence.binding == binding.digest)?;
        Some(AttestedFact {
            disposition: evidence.disposition,
            fingerprint: evidence.fact_fingerprint,
            expires_at: binding.expires_at,
        })
    }

    pub(crate) fn expires_at(&self, request: LiveRequestIdentity) -> Option<UnixMillis> {
        self.binding
            .as_ref()
            .filter(|binding| binding.request == request)
            .map(|binding| binding.expires_at)
    }

    pub(crate) const fn order_valid(&self) -> bool {
        self.order_valid
    }

    #[cfg(feature = "testing")]
    pub(crate) fn remove_for_test(&mut self, check: SecurityCheck) {
        self.evidence[check.index()] = None;
    }
}

fn reason_matches(check: SecurityCheck, reason: PolicyReason) -> bool {
    matches!(
        (check, reason),
        (SecurityCheck::Origin, PolicyReason::TrustedInternalOrigin)
            | (SecurityCheck::Csrf, PolicyReason::StatelessCsrfPolicy)
            | (SecurityCheck::Session, PolicyReason::StatelessRequest)
            | (SecurityCheck::Principal, PolicyReason::AnonymousPrincipal)
            | (SecurityCheck::Tenant, PolicyReason::TenantlessRoute)
            | (SecurityCheck::Proxy, PolicyReason::DirectPeer)
            | (SecurityCheck::RateLimit, PolicyReason::UpstreamRateLimited)
            | (
                SecurityCheck::Middleware,
                PolicyReason::NoAdditionalMiddleware
            )
    )
}

impl Default for LiveSecurityAttestation {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LiveSecurityAttestation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveSecurityAttestation:redacted>")
    }
}

fn binding_digest(
    request: LiveRequestIdentity,
    route_pattern: &str,
    operation: LiveOperation,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/request-binding/v1\0");
    digest.update(request.0.as_bytes());
    digest.update(operation.label());
    digest.update([0]);
    digest.update(route_pattern.as_bytes());
    digest.finalize().into()
}

pub(super) fn purpose_fingerprint(check: SecurityCheck, value: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/request-fact/v1\0");
    digest.update([check.index() as u8]);
    digest.update(value);
    digest.finalize().into()
}
