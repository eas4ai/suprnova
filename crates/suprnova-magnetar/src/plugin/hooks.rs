//! Post-commit lifecycle hooks and optional durable-delivery seam.

use std::collections::BTreeMap;

use async_trait::async_trait;

use super::context::HookContext;
use super::error::PluginResult;
use crate::schema::AuthSchema;

/// Lifecycle mutation kinds emitted after a successful commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleEventKind {
    /// A user row was created.
    UserCreated,
    /// A user row was deleted.
    UserDeleted,
    /// A session was established.
    SessionCreated,
    /// A session was deleted or revoked.
    SessionDeleted,
    /// Agent auth created an agent registration (auth.md
    /// `registration.created`).
    AgentRegistrationCreated,
    /// Agent auth signed an identity assertion (`assertion.issued`).
    AgentAssertionIssued,
    /// The token endpoint issued an agent access token (`token.issued`).
    AgentTokenIssued,
    /// An agent access token was revoked (`token.revoked`).
    AgentTokenRevoked,
    /// A claim ceremony was started (`claim.requested`).
    AgentClaimRequested,
    /// A claim user code was minted (`user_code.minted`).
    AgentUserCodeMinted,
    /// A signed-in user confirmed a claim (`claim.confirmed`). The event's
    /// user is the claiming user, so the application can move what the
    /// agent created to them.
    AgentClaimConfirmed,
    /// An unclaimed agent registration passed its window
    /// (`registration.expired`).
    AgentRegistrationExpired,
    /// An agent registration was revoked (`registration.revoked`).
    AgentRegistrationRevoked,
}

impl LifecycleEventKind {
    /// The auth.md audit event name of an agent auth kind, such as
    /// `registration.created`; `None` for the other kinds.
    #[must_use]
    pub fn audit_name(self) -> Option<&'static str> {
        Some(match self {
            Self::AgentRegistrationCreated => "registration.created",
            Self::AgentAssertionIssued => "assertion.issued",
            Self::AgentTokenIssued => "token.issued",
            Self::AgentTokenRevoked => "token.revoked",
            Self::AgentClaimRequested => "claim.requested",
            Self::AgentUserCodeMinted => "user_code.minted",
            Self::AgentClaimConfirmed => "claim.confirmed",
            Self::AgentRegistrationExpired => "registration.expired",
            Self::AgentRegistrationRevoked => "registration.revoked",
            Self::UserCreated | Self::UserDeleted | Self::SessionCreated | Self::SessionDeleted => {
                return None;
            }
        })
    }
}

/// Stable post-commit lifecycle event.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LifecycleEvent {
    /// Stable idempotency key for the committed mutation.
    pub mutation_id: String,
    /// Mutation kind.
    pub kind: LifecycleEventKind,
    /// Application-owned user identifier. Empty for an agent auth event
    /// about a registration no user has claimed.
    pub user_id: String,
    /// What else the event records, by name: agent auth events carry the
    /// registration, the client address and the fields auth.md recommends
    /// for each kind. Empty for the other kinds.
    pub attributes: BTreeMap<String, String>,
}

impl LifecycleEvent {
    /// Construct an event. Hosts should reject empty identity fields before
    /// committing a mutation.
    pub fn new(
        mutation_id: impl Into<String>,
        kind: LifecycleEventKind,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            mutation_id: mutation_id.into(),
            kind,
            user_id: user_id.into(),
            attributes: BTreeMap::new(),
        }
    }

    /// Record one attribute on the event.
    #[must_use]
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }
}

/// One lifecycle callback owned by a plugin.
#[async_trait]
pub trait LifecycleHook<S: AuthSchema>: Send + Sync {
    /// Deliver one post-commit event.
    async fn on_event(
        &self,
        context: HookContext<'_, S>,
        event: LifecycleEvent,
    ) -> PluginResult<()>;
}

/// Optional host integration for durable delivery.
///
/// The SDK does not implement an outbox. Hosts may enqueue stable events after
/// commit and later call [`super::registry::PluginRegistry::dispatch_lifecycle`]
/// with the same mutation id.
#[async_trait]
pub trait DurableLifecycleDelivery: Send + Sync {
    /// Persist an event for retry after process failure.
    async fn enqueue(&self, event: LifecycleEvent) -> PluginResult<()>;
}
