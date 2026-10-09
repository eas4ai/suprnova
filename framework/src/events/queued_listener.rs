//! Bridge from an in-process event to a durable queue job.
//!
//! [`QueuedListener`] is the crash-durable tier of event handling. The event
//! itself stays in-process (unbounded, not serializable); when it fires, the
//! listener builds a [`Job`] from it and enqueues that job. Durability,
//! retries, and backoff then come from the queue - the job is persisted, so it
//! survives a process crash and is picked up by a worker after restart.
//!
//! Contrast the in-process queued-listener path ([`Event::queued`](super::Event::queued)
//! returning `true`): that is best-effort - bounded and retrying, and drained
//! on graceful shutdown, but its work does NOT survive a crash. Reach for
//! `QueuedListener` when the work must happen no matter what.
//!
//! ```rust,no_run
//! use suprnova::events::{Event, EventFacade, QueuedListener};
//! # use suprnova::queue::Job;
//! # use suprnova::FrameworkError;
//! # use async_trait::async_trait;
//! # use std::sync::Arc;
//! # #[derive(Debug, Clone)]
//! # struct UserRegistered { user_id: i64 }
//! # impl Event for UserRegistered {
//! #     fn event_name() -> &'static str { "UserRegistered" }
//! # }
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct SendWelcomeEmail { user_id: i64 }
//! # #[async_trait]
//! # impl Job for SendWelcomeEmail {
//! #     fn job_name() -> &'static str { "SendWelcomeEmail" }
//! #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
//! # }
//! # async fn ex() {
//! // `UserRegistered` is a normal (unbounded) event; `SendWelcomeEmail` is a Job.
//! EventFacade::listen::<UserRegistered, _>(Arc::new(
//!     QueuedListener::<UserRegistered, SendWelcomeEmail>::new(
//!         |e| SendWelcomeEmail { user_id: e.user_id },
//!     ),
//! ))
//! .await;
//! # }
//! ```
//!
//! Register `QueuedListener` for a synchronous (non-`queued`) event: the
//! durability lives in the queue, so the listener only needs to enqueue -
//! which is fast - and the request that fired the event waits just for that
//! enqueue, not for the job to run.

use super::{Event as EventTrait, Listener};
use crate::FrameworkError;
use crate::queue::{Job, Queue};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

/// Computes the moment a listener's job becomes available from the event.
type DelayUntilFn<E> = Arc<dyn Fn(&E) -> DateTime<Utc> + Send + Sync>;

/// The delay a [`QueuedListener`] registration gives its job.
enum ListenerDelay<E> {
    /// This long after the event is handled.
    For(Duration),
    /// At the moment the closure computes from the event.
    Until(DelayUntilFn<E>),
}

/// A [`Listener`] that turns event `E` into durable job `J` and enqueues it.
/// See the module docs for when to use this versus an in-process queued
/// listener.
///
/// A job that declares [`Job::unique_id`] is pushed with
/// [`Queue::push_unique`], so the unique lock is taken before the job is
/// queued, as Laravel's dispatcher takes it for a unique listener: a second
/// event while the lock is held pushes nothing and dispatches
/// [`UniqueJobSkipped`](crate::queue::events::UniqueJobSkipped), and a job
/// that asks [`Job::unique_until_processing`] releases the lock when a
/// worker starts it. Any other job is pushed with [`Queue::push`].
pub struct QueuedListener<E, J> {
    build: Arc<dyn Fn(&E) -> J + Send + Sync>,
    delay: Option<ListenerDelay<E>>,
    _marker: PhantomData<fn() -> (E, J)>,
}

impl<E, J> QueuedListener<E, J>
where
    E: EventTrait,
    J: Job,
{
    /// Build a listener that maps each `E` to a `J` and enqueues it.
    pub fn new(build: impl Fn(&E) -> J + Send + Sync + 'static) -> Self {
        Self {
            build: Arc::new(build),
            delay: None,
            _marker: PhantomData,
        }
    }

    /// Make each job available `delay` after the event is handled. Wins
    /// over the job's own [`Job::delay`]. Mirrors the `delay` of Laravel's
    /// queued closure and the `$delay` of a queued listener.
    ///
    /// Replaces an earlier [`QueuedListener::delay_until`].
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(ListenerDelay::For(delay));
        self
    }

    /// Make each job available at the moment `until` computes from the
    /// event. Wins over the job's own [`Job::delay`].
    ///
    /// A closure over the event, not a date, because a date fixed when the
    /// listener is registered is in the past for every event a
    /// long-running process handles later. Laravel's queued closure takes a
    /// date for the same purpose. A moment already past makes the job
    /// available at once.
    ///
    /// Replaces an earlier [`QueuedListener::delay`].
    pub fn delay_until(
        mut self,
        until: impl Fn(&E) -> DateTime<Utc> + Send + Sync + 'static,
    ) -> Self {
        self.delay = Some(ListenerDelay::Until(Arc::new(until)));
        self
    }

    /// The moment the job for `event` becomes available, when the
    /// registration gives one.
    fn available_at(&self, event: &E) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        match &self.delay {
            None => Ok(None),
            Some(ListenerDelay::For(delay)) => {
                crate::queue::driver::available_after(*delay).map(Some)
            }
            Some(ListenerDelay::Until(until)) => Ok(Some(until(event))),
        }
    }
}

#[async_trait]
impl<E, J> Listener<E> for QueuedListener<E, J>
where
    E: EventTrait,
    J: Job,
{
    /// Build the job and push it. A duplicate of a unique job answers
    /// `Ok(())`: the event was handled, and the job already waiting does
    /// the work.
    async fn handle(&self, event: &E) -> Result<(), FrameworkError> {
        let job = (self.build)(event);
        let available_at = self.available_at(event)?;
        if job.unique_id().is_some() {
            // `Ok(false)` is a duplicate, reported by `UniqueJobSkipped`.
            match available_at {
                Some(at) => Queue::push_unique_later(job, at).await?,
                None => Queue::push_unique(job).await?,
            };
            return Ok(());
        }
        match available_at {
            Some(at) => Queue::push_later(job, at).await,
            None => Queue::push(job).await,
        }
    }
}

/// Derives a debounce id from the event, so a per-entity window is a decision
/// the listener registration makes rather than a property of the job.
type DebounceKeyFn<E> = Arc<dyn Fn(&E) -> String + Send + Sync>;

/// A [`Listener`] that turns event `E` into durable job `J` and enqueues it
/// with a debounce window, so a burst of events becomes one run.
///
/// Reach for this when the window is a property of the **registration** rather
/// than of the job - a job that is debounced everywhere it is dispatched should
/// declare [`Job::debounce_for`](crate::queue::Job::debounce_for) instead, and
/// a plain [`QueuedListener`] will honor it. This is the shape Laravel's
/// `#[DebounceFor]` attribute on a listener expresses, with the debounce id
/// derived from the event rather than from the job.
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use std::time::Duration;
/// # use suprnova::events::{Event, EventFacade, DebouncedListener};
/// # use suprnova::queue::Job;
/// # use suprnova::FrameworkError;
/// # #[derive(Debug, Clone)]
/// # struct OrderUpdated { order_id: u32 }
/// # impl Event for OrderUpdated { fn event_name() -> &'static str { "OrderUpdated" } }
/// # #[derive(serde::Serialize, serde::Deserialize)]
/// # struct ReindexOrder { order_id: u32 }
/// # #[suprnova::async_trait]
/// # impl Job for ReindexOrder {
/// #     fn job_name() -> &'static str { "ReindexOrder" }
/// #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
/// # }
/// # async fn ex() {
/// EventFacade::listen::<OrderUpdated, _>(Arc::new(
///     DebouncedListener::<OrderUpdated, ReindexOrder>::new(
///         Duration::from_secs(30),
///         |e| ReindexOrder { order_id: e.order_id },
///     )
///     .max_wait(Duration::from_secs(300))
///     .keyed_by(|e| e.order_id.to_string()),
/// ))
/// .await;
/// # }
/// ```
pub struct DebouncedListener<E, J> {
    build: Arc<dyn Fn(&E) -> J + Send + Sync>,
    key: Option<DebounceKeyFn<E>>,
    window: std::time::Duration,
    max_wait: Option<std::time::Duration>,
    _marker: PhantomData<fn() -> (E, J)>,
}

impl<E, J> DebouncedListener<E, J>
where
    E: EventTrait,
    J: Job,
{
    /// Build a listener that maps each `E` to a `J` and enqueues it, collapsing
    /// a burst into one run `window` after the most recent event.
    pub fn new(
        window: std::time::Duration,
        build: impl Fn(&E) -> J + Send + Sync + 'static,
    ) -> Self {
        Self {
            build: Arc::new(build),
            key: None,
            window,
            max_wait: None,
            _marker: PhantomData,
        }
    }

    /// Bound how long a continuous burst may defer the run.
    pub fn max_wait(mut self, max_wait: std::time::Duration) -> Self {
        self.max_wait = Some(max_wait);
        self
    }

    /// Derive the debounce id from the event, so bursts for different entities
    /// collapse independently. Without this, every event of type `E` shares one
    /// window.
    pub fn keyed_by(mut self, key: impl Fn(&E) -> String + Send + Sync + 'static) -> Self {
        self.key = Some(Arc::new(key));
        self
    }
}

#[async_trait]
impl<E, J> Listener<E> for DebouncedListener<E, J>
where
    E: EventTrait,
    J: Job,
{
    async fn handle(&self, event: &E) -> Result<(), FrameworkError> {
        let job = (self.build)(event);
        let mut options = crate::queue::DebounceOptions::new(self.window);
        if let Some(max_wait) = self.max_wait {
            options = options.max_wait(max_wait);
        }
        if let Some(key) = self.key.as_ref() {
            options = options.id(key(event));
        }
        Queue::push_debounced(job, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::testing;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    struct UserRegistered {
        user_id: i64,
    }
    impl EventTrait for UserRegistered {
        fn event_name() -> &'static str {
            "UserRegistered"
        }
    }

    #[derive(Serialize, Deserialize)]
    struct SendWelcome {
        user_id: i64,
    }
    #[async_trait]
    impl Job for SendWelcome {
        fn job_name() -> &'static str {
            "SendWelcome"
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn handle_builds_job_from_event_and_enqueues_it() {
        let _fake = testing::install_fake();
        let listener = QueuedListener::<UserRegistered, SendWelcome>::new(|e| SendWelcome {
            user_id: e.user_id,
        });
        listener
            .handle(&UserRegistered { user_id: 42 })
            .await
            .unwrap();
        testing::assert_pushed::<SendWelcome>(|j| j.user_id == 42);
    }

    #[tokio::test]
    async fn dispatched_event_routes_through_the_listener_to_the_queue() {
        use crate::events::EventDispatcher;
        let _fake = testing::install_fake();
        let d = EventDispatcher::new();
        d.listen::<UserRegistered, _>(Arc::new(
            QueuedListener::<UserRegistered, SendWelcome>::new(|e| SendWelcome {
                user_id: e.user_id,
            }),
        ))
        .await;
        d.dispatch(UserRegistered { user_id: 7 }).await.unwrap();
        testing::assert_pushed::<SendWelcome>(|j| j.user_id == 7);
    }
}
