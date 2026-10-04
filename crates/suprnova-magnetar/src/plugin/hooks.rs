//! Post-commit lifecycle hooks and optional durable-delivery seam.

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
}

/// Stable post-commit lifecycle event.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LifecycleEvent {
    /// Stable idempotency key for the committed mutation.
    pub mutation_id: String,
    /// Mutation kind.
    pub kind: LifecycleEventKind,
    /// Application-owned user identifier.
    pub user_id: String,
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
        }
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
