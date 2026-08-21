//! Closed low-cardinality telemetry labels for trusted observability adapters.

use std::fmt;

use crate::error::{ErrorCategory, RecoveryInstruction, SafeDiagnosticCode};
use crate::identity::ContentDigest;

/// Closed operation phase suitable for metrics and span labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryEvent {
    /// Canonical control input parsing.
    CanonicalParsing,
    /// Signed snapshot integrity and compatibility verification.
    SnapshotVerification,
    /// Public seed promotion.
    SeedPromotion,
    /// Instance revision claim or outcome lookup.
    LedgerClaim,
    /// Request control-envelope validation.
    RequestProtocol,
    /// Response control-envelope validation.
    ResponseProtocol,
    /// Pure response-application planning.
    ResponseOrdering,
}

impl TelemetryEvent {
    /// Complete bounded event vocabulary.
    pub const ALL: &[Self] = &[
        Self::CanonicalParsing,
        Self::SnapshotVerification,
        Self::SeedPromotion,
        Self::LedgerClaim,
        Self::RequestProtocol,
        Self::ResponseProtocol,
        Self::ResponseOrdering,
    ];

    /// Returns the stable low-cardinality label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalParsing => "canonical_parsing",
            Self::SnapshotVerification => "snapshot_verification",
            Self::SeedPromotion => "seed_promotion",
            Self::LedgerClaim => "ledger_claim",
            Self::RequestProtocol => "request_protocol",
            Self::ResponseProtocol => "response_protocol",
            Self::ResponseOrdering => "response_ordering",
        }
    }
}

/// Closed coarse outcome suitable for metrics and span labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryOutcome {
    /// Boundary accepted the operation.
    Accepted,
    /// Exact duplicate observed prior compatible work.
    Duplicate,
    /// Boundary safely rejected the operation.
    Rejected,
    /// Operation requires current island authority.
    RefreshRequired,
    /// Provider or invariant failure stopped the operation.
    Failed,
}

impl TelemetryOutcome {
    /// Complete bounded outcome vocabulary.
    pub const ALL: &[Self] = &[
        Self::Accepted,
        Self::Duplicate,
        Self::Rejected,
        Self::RefreshRequired,
        Self::Failed,
    ];

    /// Returns the stable low-cardinality label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Duplicate => "duplicate",
            Self::Rejected => "rejected",
            Self::RefreshRequired => "refresh_required",
            Self::Failed => "failed",
        }
    }
}

/// Closed label set with one optional fixed-width digest prefix.
pub struct TelemetryLabels {
    event: TelemetryEvent,
    outcome: TelemetryOutcome,
    category: ErrorCategory,
    recovery: RecoveryInstruction,
    detail: SafeDiagnosticCode,
    context_digest: Option<String>,
}

impl TelemetryLabels {
    /// Creates labels exclusively from closed enums and an already-safe digest.
    #[must_use]
    pub fn new(
        event: TelemetryEvent,
        outcome: TelemetryOutcome,
        category: ErrorCategory,
        recovery: RecoveryInstruction,
        detail: SafeDiagnosticCode,
        context_digest: Option<&ContentDigest>,
    ) -> Self {
        Self {
            event,
            outcome,
            category,
            recovery,
            detail,
            context_digest: context_digest.map(short_digest),
        }
    }

    /// Materializes a bounded adapter-neutral key/value list.
    #[must_use]
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![
            ("event", self.event.as_str().to_owned()),
            ("outcome", self.outcome.as_str().to_owned()),
            ("category", self.category.as_str().to_owned()),
            ("recovery", self.recovery.as_str().to_owned()),
            ("detail", self.detail.as_str().to_owned()),
        ];
        if let Some(digest) = &self.context_digest {
            pairs.push(("context_digest", digest.clone()));
        }
        pairs
    }
}

impl fmt::Debug for TelemetryLabels {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.to_pairs()).finish()
    }
}

fn short_digest(digest: &ContentDigest) -> String {
    digest.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
