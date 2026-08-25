//! Opaque host capability bindings without raw request credentials.

use std::fmt;
use std::sync::Arc;

use crate::action::ActionAuthorizationPort;
use crate::async_updates::{SubscriptionAuthorizationPort, SubscriptionCredentialPort};
use crate::identity::{ContentDigest, IdentityError, ScopeFingerprint};
use crate::upload::UploadAuthorizationPort;

macro_rules! host_fingerprint {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Eq, PartialEq)]
        pub struct $name(ContentDigest);

        impl $name {
            /// Constructs a purpose-specific opaque fingerprint from 256-bit host output.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdentityError> {
                ContentDigest::from_bytes(bytes).map(Self)
            }

            pub(crate) const fn digest(&self) -> &ContentDigest {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!("<", stringify!($name), ">"))
            }
        }
    };
}

host_fingerprint!(
    /// Purpose-specific digest of the current resolved principal.
    PrincipalFingerprint
);
host_fingerprint!(
    /// Purpose-specific digest of the current host session.
    SessionFingerprint
);
host_fingerprint!(
    /// Purpose-specific digest of the current resolved tenant.
    TenantFingerprint
);

/// Normalized current identity facts supplied by the host adapter.
#[derive(Clone, Eq, PartialEq)]
pub struct HostScopeFacts {
    scope: ScopeFingerprint,
    session: Option<SessionFingerprint>,
    principal: Option<PrincipalFingerprint>,
    tenant: Option<TenantFingerprint>,
}

impl HostScopeFacts {
    /// Groups the aggregate scope with independently comparable host identities.
    #[must_use]
    pub const fn new(
        scope: ScopeFingerprint,
        session: Option<SessionFingerprint>,
        principal: Option<PrincipalFingerprint>,
        tenant: Option<TenantFingerprint>,
    ) -> Self {
        Self {
            scope,
            session,
            principal,
            tenant,
        }
    }

    /// Returns the purpose-specific aggregate scope.
    #[must_use]
    pub const fn scope(&self) -> &ScopeFingerprint {
        &self.scope
    }

    /// Returns the current session fingerprint when the route is stateful.
    #[must_use]
    pub const fn session(&self) -> Option<&SessionFingerprint> {
        self.session.as_ref()
    }

    /// Returns the current authenticated principal fingerprint.
    #[must_use]
    pub const fn principal(&self) -> Option<&PrincipalFingerprint> {
        self.principal.as_ref()
    }

    /// Returns the current tenant fingerprint when the route is tenant-scoped.
    #[must_use]
    pub const fn tenant(&self) -> Option<&TenantFingerprint> {
        self.tenant.as_ref()
    }
}

impl fmt::Debug for HostScopeFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<HostScopeFacts:redacted>")
    }
}

/// Opaque host services bound to exactly one normalized request scope.
///
/// Later kernel stages extend this value with authorization, transaction, and
/// application-service ports. The binding exists now so a capability cannot be
/// reused across a principal, session, tenant, or aggregate scope.
#[derive(Clone)]
pub struct HostCapabilities {
    scope: HostScopeFacts,
    action_authorization: Option<Arc<dyn ActionAuthorizationPort>>,
    upload_authorization: Option<Arc<dyn UploadAuthorizationPort>>,
    subscription_authorization: Option<Arc<dyn SubscriptionAuthorizationPort>>,
    subscription_credentials: Option<Arc<dyn SubscriptionCredentialPort>>,
}

impl HostCapabilities {
    /// Binds host-owned capabilities to the current normalized request facts.
    #[must_use]
    pub const fn bound_to(scope: HostScopeFacts) -> Self {
        Self {
            scope,
            action_authorization: None,
            upload_authorization: None,
            subscription_authorization: None,
            subscription_credentials: None,
        }
    }

    /// Installs the host-owned current authorization provider for protected actions.
    #[must_use]
    pub fn with_action_authorization(
        mut self,
        authorization: Arc<dyn ActionAuthorizationPort>,
    ) -> Self {
        self.action_authorization = Some(authorization);
        self
    }

    /// Installs the host-owned current authorization provider for upload controls.
    #[must_use]
    pub fn with_upload_authorization(
        mut self,
        authorization: Arc<dyn UploadAuthorizationPort>,
    ) -> Self {
        self.upload_authorization = Some(authorization);
        self
    }

    /// Installs current authorization for asynchronous subscription boundaries.
    #[must_use]
    pub fn with_subscription_authorization(
        mut self,
        authorization: Arc<dyn SubscriptionAuthorizationPort>,
    ) -> Self {
        self.subscription_authorization = Some(authorization);
        self
    }

    /// Installs the host-owned descriptor-scoped transport credential provider.
    #[must_use]
    pub fn with_subscription_credentials(
        mut self,
        credentials: Arc<dyn SubscriptionCredentialPort>,
    ) -> Self {
        self.subscription_credentials = Some(credentials);
        self
    }

    pub(crate) const fn scope(&self) -> &HostScopeFacts {
        &self.scope
    }

    pub(crate) fn action_authorization(&self) -> Option<&dyn ActionAuthorizationPort> {
        self.action_authorization.as_deref()
    }

    pub(crate) fn upload_authorization(&self) -> Option<&dyn UploadAuthorizationPort> {
        self.upload_authorization.as_deref()
    }

    pub(crate) fn subscription_authorization(&self) -> Option<&dyn SubscriptionAuthorizationPort> {
        self.subscription_authorization.as_deref()
    }

    pub(crate) fn subscription_credentials(&self) -> Option<&dyn SubscriptionCredentialPort> {
        self.subscription_credentials.as_deref()
    }
}

impl fmt::Debug for HostCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<HostCapabilities:redacted>")
    }
}
