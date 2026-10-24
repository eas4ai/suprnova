//! Server-only authorization of verified v2 child-parameter capabilities.

use std::error::Error;
use std::fmt;

use super::VerifiedChildParametersV2;
use crate::canonical::CanonicalValue;
use crate::identity::{InstanceId, Revision};
use crate::ledger::{LedgerError, LiveInstanceLedger};
use crate::snapshot::VerifiedInstanceV1;

/// Closed reason server-side child eligibility was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildParameterEligibilityErrorKind {
    /// The verified parameter and parent snapshot authorities do not match.
    BindingMismatch,
    /// The signed parent snapshot does not own this exact child tuple.
    CompositionLineageMismatch,
    /// No current accepted parent authority exists in the ledger.
    ParentAuthorityMissing,
    /// The ledger has accepted a different parent revision.
    ParentRevisionMismatch,
    /// The provider could not supply the authorization metadata read.
    ProviderUnavailable,
}

impl ChildParameterEligibilityErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BindingMismatch => "child_parameter_parent_binding_mismatch",
            Self::CompositionLineageMismatch => "child_parameter_composition_lineage_mismatch",
            Self::ParentAuthorityMissing => "child_parameter_parent_authority_missing",
            Self::ParentRevisionMismatch => "child_parameter_parent_revision_mismatch",
            Self::ProviderUnavailable => "child_parameter_authority_provider_unavailable",
        }
    }
}

/// Redacted server eligibility error with an optional causal provider error.
pub struct ChildParameterEligibilityError {
    kind: ChildParameterEligibilityErrorKind,
    source: Option<LedgerError>,
}

impl ChildParameterEligibilityError {
    const fn new(kind: ChildParameterEligibilityErrorKind) -> Self {
        Self { kind, source: None }
    }

    const fn provider(source: LedgerError) -> Self {
        Self {
            kind: ChildParameterEligibilityErrorKind::ProviderUnavailable,
            source: Some(source),
        }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(&self) -> ChildParameterEligibilityErrorKind {
        self.kind
    }
}

impl fmt::Display for ChildParameterEligibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ChildParameterEligibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ChildParameterEligibilityError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as &dyn Error)
    }
}

/// Exact-child parameters authorized by signed lineage and current ledger state.
#[derive(Clone)]
pub struct EligibleChildParametersV2 {
    verified: VerifiedChildParametersV2,
}

impl EligibleChildParametersV2 {
    const fn new(verified: VerifiedChildParametersV2) -> Self {
        Self { verified }
    }

    /// Returns the typed canonical parameters.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        self.verified.parameters()
    }

    /// Returns the exact child instance authorized for delivery.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        self.verified.child_instance()
    }

    /// Returns the accepted parent revision authorizing this delivery.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.verified.parent_revision()
    }
}

impl fmt::Debug for EligibleChildParametersV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<EligibleChildParametersV2:redacted>")
    }
}

/// Authorizes one verified v2 capability against signed composition and ledger state.
pub async fn authorize_child_parameters_v2(
    verified: &VerifiedChildParametersV2,
    parent: &VerifiedInstanceV1,
    ledger: &dyn LiveInstanceLedger,
) -> Result<EligibleChildParametersV2, ChildParameterEligibilityError> {
    let parameters = &verified.body;
    let parent_body = parent.body();
    if &parameters.parent_scope != parent_body.scope()
        || &parameters.parent_instance != parent_body.instance_id()
        || parameters.parent_revision != parent_body.revision()
    {
        return Err(ChildParameterEligibilityError::new(
            ChildParameterEligibilityErrorKind::BindingMismatch,
        ));
    }

    let owns_exact_child = parent_body.composition_lineage().is_some_and(|lineage| {
        lineage.children().iter().any(|child| {
            child.parent_instance() == &parameters.parent_instance
                && child.parent_revision() == parameters.parent_revision
                && child.child_key() == verified.child_key()
                && child.child_contract() == &parameters.child_contract
                && child.child_instance() == &parameters.child_instance
        })
    });
    if !owns_exact_child {
        return Err(ChildParameterEligibilityError::new(
            ChildParameterEligibilityErrorKind::CompositionLineageMismatch,
        ));
    }

    match ledger
        .current_accepted_revision(&parameters.parent_scope, &parameters.parent_instance)
        .await
        .map_err(ChildParameterEligibilityError::provider)?
    {
        Some(revision) if revision == parameters.parent_revision => {
            Ok(EligibleChildParametersV2::new(verified.clone()))
        }
        Some(_) => Err(ChildParameterEligibilityError::new(
            ChildParameterEligibilityErrorKind::ParentRevisionMismatch,
        )),
        None => Err(ChildParameterEligibilityError::new(
            ChildParameterEligibilityErrorKind::ParentAuthorityMissing,
        )),
    }
}
