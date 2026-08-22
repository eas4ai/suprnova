//! Synthetic host context assembled through the complete production validator.

use std::collections::BTreeMap;
use std::sync::Arc;

use suprnova_live::action::ActionAuthorizationPort;
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostContextError,
    HostScopeFacts, LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalog,
    MountSelection, TrustedLiveRequestContext,
};
use suprnova_live::identity::UnixMillis;

/// Dev-only ergonomic builder that never bypasses production host validation.
pub struct SyntheticLiveRequestContextBuilder {
    catalog: MountCatalog,
    selection: MountSelection,
    scope: HostScopeFacts,
    now: UnixMillis,
    expires_at: UnixMillis,
    overrides: BTreeMap<CheckKind, CheckFact>,
    action_authorization: Option<Arc<dyn ActionAuthorizationPort>>,
}

impl SyntheticLiveRequestContextBuilder {
    /// Creates a complete passed-check fixture for one catalog selection and scope.
    #[must_use]
    pub fn new(
        catalog: MountCatalog,
        selection: MountSelection,
        scope: HostScopeFacts,
        now: UnixMillis,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            catalog,
            selection,
            scope,
            now,
            expires_at,
            overrides: BTreeMap::new(),
            action_authorization: None,
        }
    }

    /// Replaces one synthetic disposition while still using production validation.
    #[must_use]
    pub fn with_check(mut self, kind: CheckKind, fact: CheckFact) -> Self {
        self.overrides.insert(kind, fact);
        self
    }

    /// Installs a conformance current-authorization provider without bypassing host validation.
    #[must_use]
    pub fn with_action_authorization(
        mut self,
        authorization: Arc<dyn ActionAuthorizationPort>,
    ) -> Self {
        self.action_authorization = Some(authorization);
        self
    }

    /// Runs the complete production catalog, check, expiry, and capability validator.
    pub fn build(self) -> Result<TrustedLiveRequestContext, HostContextError> {
        let mut capabilities = HostCapabilities::bound_to(self.scope.clone());
        if let Some(authorization) = self.action_authorization {
            capabilities = capabilities.with_action_authorization(authorization);
        }
        let current_route = self.selection.route().clone();
        let current_slot = self.selection.slot().clone();
        let mut checks = HostCheckFacts::new();
        for kind in CheckKind::ALL {
            checks.record(
                kind,
                self.overrides
                    .get(&kind)
                    .copied()
                    .unwrap_or_else(|| CheckFact::new(CheckDisposition::Passed, self.expires_at)),
            )?;
        }
        let candidate = LiveRequestContextCandidate::new(
            current_route,
            current_slot,
            self.selection,
            self.scope,
            checks,
            capabilities,
            self.expires_at,
        );
        LiveRequestContextValidator::new(300_000)?.validate(&self.catalog, candidate, self.now)
    }
}
