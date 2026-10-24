//! Broadcast notification channel.
//!
//! Delivers a notification to WebSocket subscribers by publishing it to the
//! application's [`BroadcastHub`] (Phase 7B). The per-recipient
//! `route_for("broadcast")` value is the broadcast channel name, the
//! notification's type name is the event, and its `data()` is the payload,
//! unless the notification implements [`NotificationBroadcast`], whose
//! [`BroadcastMessage`] supplies the payload and may route the publish
//! through the queue.

use crate::broadcasting::{BroadcastEnvelope, BroadcastHub};
use crate::container::App;
use crate::error::FrameworkError;
use crate::lock;
use crate::notifications::events::BroadcastNotificationCreated;
use crate::notifications::{Channel, DynNotification, Notification};
use crate::queue::{EnvelopeOverrides, Job, Queue};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Broadcast channel - publishes notifications to the application's
/// [`BroadcastHub`] so WebSocket subscribers receive them in real time.
///
/// The hub is resolved from the container at delivery time
/// (`App::make::<dyn BroadcastHub>()`), the same way the WS handler and SSE
/// bridge obtain it. Bind one at boot with
/// `App::bind::<dyn BroadcastHub>(Arc::clone(&hub))`.
///
/// # The message
///
/// A notification that implements [`NotificationBroadcast`] and is
/// registered with [`register_broadcast_renderer`] supplies a
/// [`BroadcastMessage`]: its data is the payload, and a message that names
/// a queue ([`BroadcastMessage::on_queue`]) or a connection
/// ([`BroadcastMessage::on_connection`]) is published by a queued
/// [`BroadcastNotificationJob`] on that queue and connection instead of at
/// once. Any other notification publishes its `data()` at once.
///
/// After the publish, or the push of the job, the channel dispatches
/// [`BroadcastNotificationCreated`], as Laravel's `BroadcastChannel` does.
///
/// # Dispatch semantics (load-bearing - do not "simplify" back to `Ok`)
///
/// [`Channel::deliver`] returns `Err` when **no** `BroadcastHub` is bound in
/// the container for a message published at once, and the queued job fails
/// the same way on the worker. The [`crate::notifications`] dispatcher
/// breaks on the first channel error, so this short-circuits the rest of
/// the notification's channels - **by design**. A notification that
/// declares `"broadcast"` in an app that never wired a hub is a
/// misconfiguration that must surface, not be silently dropped. (This type
/// was previously a stub that returned `Ok(())` without delivering
/// anything; that silent success was the bug being fixed.)
///
/// When a hub **is** bound - the normal case - `deliver` publishes and
/// returns `Ok(())`, so broadcast never short-circuits a correctly-configured
/// app. Publishing to a channel with zero live subscribers is not an error.
#[derive(Default)]
pub struct BroadcastChannel;

impl BroadcastChannel {
    /// Build a new `BroadcastChannel`. Stateless - the bound `BroadcastHub` is resolved per-call.
    pub fn new() -> Self {
        Self
    }
}

/// What a broadcast notification publishes, and how. Mirrors Laravel's
/// `BroadcastMessage`, whose `onQueue` and `onConnection` route the
/// broadcast through the queue.
///
/// A message that names neither a queue nor a connection is published at
/// once, as the channel always has; Laravel queues every broadcast, and
/// Suprnova keeps the at-once publish for the message that does not ask.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BroadcastMessage {
    /// The payload subscribers receive.
    pub data: Value,
    /// The queue the publishing job is pushed to, when it is queued.
    pub queue: Option<String>,
    /// The queue connection the publishing job is pushed to, when it is
    /// queued.
    pub connection: Option<String>,
}

impl BroadcastMessage {
    /// A message carrying `data`, published at once.
    pub fn new(data: Value) -> Self {
        Self {
            data,
            queue: None,
            connection: None,
        }
    }

    /// Publish through a job on the queue `queue`.
    pub fn on_queue(mut self, queue: impl Into<String>) -> Self {
        self.queue = Some(queue.into());
        self
    }

    /// Publish through a job on the queue connection `connection`.
    pub fn on_connection(mut self, connection: impl Into<String>) -> Self {
        self.connection = Some(connection.into());
        self
    }

    /// Whether the message is published by a queued job.
    fn is_queued(&self) -> bool {
        self.queue.is_some() || self.connection.is_some()
    }
}

/// Opt-in trait for a notification that supplies its own broadcast
/// message. Mirrors Laravel's `toBroadcast($notifiable)`.
///
/// No `Notifiable` argument, for the reason
/// [`NotificationMailable::to_mail`](crate::notifications::channels::mail::NotificationMailable::to_mail)
/// gives: the queued path does not keep the recipient, so anything the
/// message needs rides on the notification's own fields.
///
/// Register each implementor once at boot with
/// [`register_broadcast_renderer::<N>()`](register_broadcast_renderer); the
/// [`BroadcastChannel`] looks the renderer up by the notification's name.
pub trait NotificationBroadcast: Notification {
    /// The message the broadcast channel publishes for this notification.
    fn to_broadcast(&self) -> BroadcastMessage;
}

/// Renderer function pointer, as the mail channel keeps one per
/// notification name: registered renderers are stateless.
type BroadcastRendererFn = fn(&dyn DynNotification) -> Result<BroadcastMessage, FrameworkError>;

static BROADCAST_RENDERERS: RwLock<Option<HashMap<&'static str, BroadcastRendererFn>>> =
    RwLock::new(None);

/// Register a notification's [`NotificationBroadcast::to_broadcast`] with
/// the [`BroadcastChannel`], keyed by `Notification::notification_name()`,
/// the way [`register_mail_renderer`](crate::notifications::channels::mail::register_mail_renderer)
/// registers mail. A notification that is not registered publishes its
/// `data()` at once.
///
/// Re-registering the same name replaces the earlier renderer
/// (last-write-wins), as the other notification registries do.
///
/// # Errors
///
/// Fails only when the renderer registry's lock is poisoned.
pub fn register_broadcast_renderer<N: NotificationBroadcast>() -> Result<(), FrameworkError> {
    let renderer: BroadcastRendererFn = |notification| {
        // The notification itself, so `to_broadcast` sees every field.
        if let Some(n) = notification
            .as_any()
            .and_then(|any| any.downcast_ref::<N>())
        {
            return Ok(n.to_broadcast());
        }
        // A different type under the same name, or a hand-written
        // `DynNotification`: decode `N` from the public payload, the only
        // shape there is.
        let n: N = serde_json::from_value(notification.data()).map_err(|e| {
            FrameworkError::internal(format!("decode {}: {e}", N::notification_name()))
        })?;
        Ok(n.to_broadcast())
    };
    let mut g = lock::write(&BROADCAST_RENDERERS, "notification broadcast renderers")?;
    g.get_or_insert_with(HashMap::new)
        .insert(N::notification_name(), renderer);
    Ok(())
}

/// The message for `notification`: its registered renderer's, or its
/// `data()` published at once.
fn message_for(notification: &dyn DynNotification) -> Result<BroadcastMessage, FrameworkError> {
    let renderer = lock::read(&BROADCAST_RENDERERS, "notification broadcast renderers")?
        .as_ref()
        .and_then(|renderers| renderers.get(notification.name()).copied());
    match renderer {
        Some(render) => render(notification),
        None => Ok(BroadcastMessage::new(notification.data())),
    }
}

/// The hub bound in the container, or the error that names how to bind one.
fn bound_hub() -> Result<Arc<dyn BroadcastHub>, FrameworkError> {
    App::make::<dyn BroadcastHub>().ok_or_else(|| {
        FrameworkError::internal(
            "broadcast notification channel requires a BroadcastHub bound in the \
             container - call `App::bind::<dyn BroadcastHub>(Arc::clone(&hub))` at boot, \
             or drop \"broadcast\" from the notification's channels()",
        )
    })
}

/// Publish `envelope` to the bound hub.
async fn publish(envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
    let hub = bound_hub()?;
    tracing::debug!(
        channel = %envelope.channel,
        event = %envelope.event,
        "publishing broadcast notification to hub"
    );
    // Propagate hub publish failures: a cross-process fanout loss
    // is real and the notification dispatcher should surface it,
    // not swallow it.
    hub.publish(envelope).await
}

#[async_trait]
impl Channel for BroadcastChannel {
    fn name(&self) -> &'static str {
        "broadcast"
    }

    async fn deliver(
        &self,
        route: &str,
        notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        let message = message_for(notification)?;
        let event = notification.name().to_string();
        if message.is_queued() {
            let overrides = EnvelopeOverrides {
                queue: message.queue.clone(),
                connection: message.connection.clone(),
                ..Default::default()
            };
            Queue::push_with(
                BroadcastNotificationJob {
                    channel: route.to_string(),
                    event: event.clone(),
                    data: message.data.clone(),
                },
                overrides,
            )
            .await?;
        } else {
            publish(BroadcastEnvelope::new(
                route.to_string(),
                event.clone(),
                message.data.clone(),
            ))
            .await?;
        }
        let created = BroadcastNotificationCreated {
            notification: event,
            route: route.to_string(),
            data: message.data,
            connection: message.connection,
            queue: message.queue,
        };
        if let Err(e) = crate::events::EventFacade::dispatch_best_effort(created).await {
            tracing::warn!(
                channel = %route,
                error = %e,
                "a BroadcastNotificationCreated listener failed; the broadcast itself \
                 already went out"
            );
        }
        Ok(())
    }
}

/// The queued publish of a broadcast notification whose
/// [`BroadcastMessage`] names a queue or a connection. The worker publishes
/// it to the hub bound in its own container, and fails, to be retried, when
/// none is bound.
///
/// The framework registers this job with the worker, as it registers the
/// mail and notification jobs, so an application never calls
/// `register_job` for it. `Queue::fake()` records it, so a test asserts a
/// queued broadcast with
/// `assert_pushed_on_queue::<BroadcastNotificationJob>(queue, ...)`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BroadcastNotificationJob {
    /// The broadcast channel name.
    pub channel: String,
    /// The event name subscribers receive: the notification's name.
    pub event: String,
    /// The payload subscribers receive.
    pub data: Value,
}

#[async_trait]
impl Job for BroadcastNotificationJob {
    fn job_name() -> &'static str {
        "Suprnova::BroadcastNotification"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        publish(BroadcastEnvelope::new(self.channel, self.event, self.data)).await
    }
}
