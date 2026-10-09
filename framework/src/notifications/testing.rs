//! `Notify::fake()` - installs an in-memory recorder that captures
//! dispatched notifications without invoking any channel.
//!
//! Records keyed by `(notification_name, route, channel)`. Suprnova's
//! [`crate::notifications::Notifiable`] trait exposes only `route_for`, not
//! a stable identity/key, so the fake keys recipient-side on the per-channel
//! route value rather than Laravel's `(class, primary_key)` pair. The fake
//! captures the JSON payload so tests can inspect notification contents
//! without re-serializing.
//!
//! Parallel-test safety: [`install_fake`] takes a process-wide serialization
//! mutex for the lifetime of the returned guard, mirroring
//! `Queue::fake()` / `Bus::fake()`.

use super::{Notifiable, Notification};
use crate::FrameworkError;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::any::TypeId;
use std::sync::{Mutex, MutexGuard};

/// One captured dispatch.
#[derive(Clone, Debug)]
pub struct FakeRecord {
    /// `Notification::notification_name()` of the dispatched notification.
    pub notification: String,
    /// Channel name (`"mail"`, `"database"`, …).
    pub channel: String,
    /// Per-channel route returned by the recipient.
    pub route: String,
    /// JSON-serializable payload - the same blob channels would have seen.
    pub data: Value,
}

#[derive(Default)]
struct FakeStore {
    records: Vec<FakeRecord>,
    typed: Vec<TypedRecord>,
}

#[derive(Clone)]
struct TypedRecord {
    notification_type: TypeId,
    payload: Value,
    routes: Vec<String>,
}

static FAKE_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static FAKE: Mutex<Option<FakeStore>> = Mutex::new(None);

fn lock_fake() -> MutexGuard<'static, Option<FakeStore>> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn is_active() -> bool {
    lock_fake().is_some()
}

/// Retain the concrete notification once and keep the existing per-channel records.
pub(crate) fn record_notification<N: Notification, R: Notifiable + ?Sized>(
    recipient: &R,
    notification: &N,
) -> Result<(), FrameworkError> {
    let payload = serde_json::to_value(notification)
        .map_err(|error| FrameworkError::internal(format!("encode fake notification: {error}")))?;
    let data = notification.data();
    let records: Vec<_> = notification
        .channels()
        .into_iter()
        .filter_map(|channel| {
            recipient.route_for(channel).map(|route| FakeRecord {
                notification: N::notification_name().to_owned(),
                channel: channel.to_owned(),
                route,
                data: data.clone(),
            })
        })
        .collect();
    let routes = records.iter().map(|record| record.route.clone()).collect();
    if let Some(store) = lock_fake().as_mut() {
        store.typed.push(TypedRecord {
            notification_type: TypeId::of::<N>(),
            payload,
            routes,
        });
        store.records.extend(records);
    }
    Ok(())
}

/// Install the notify fake for the current test.
///
/// Holds a process-wide serialization lock so parallel tests cannot share
/// the store, and clears the store on drop.
pub fn install_fake() -> NotifyFakeGuard {
    let serial = FAKE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *lock_fake() = Some(FakeStore::default());
    NotifyFakeGuard { _serial: serial }
}

/// RAII guard returned by [`install_fake`]. Clears the fake on drop.
pub struct NotifyFakeGuard {
    _serial: MutexGuard<'static, ()>,
}

impl NotifyFakeGuard {
    /// Inspect full notifications of one type sent to a route, so your tests
    /// can filter fields that the channel payload does not expose.
    /// Each send or queue call appears once even when several channels use the route.
    /// Serialization errors are returned rather than silently losing a record.
    pub fn sent<N: Notification>(
        &self,
        route: &str,
        predicate: impl Fn(&N) -> bool,
    ) -> Result<Vec<N>, FrameworkError> {
        let records = lock_fake()
            .as_ref()
            .ok_or_else(|| FrameworkError::internal("Notify::fake() must be active"))?
            .typed
            .iter()
            .filter(|record| {
                record.notification_type == TypeId::of::<N>()
                    && record.routes.iter().any(|candidate| candidate == route)
            })
            .cloned()
            .collect::<Vec<_>>();
        // Release the mutex before deserialization or application callbacks.
        let mut matches = Vec::new();
        for record in records {
            let notification: N = serde_json::from_value(record.payload).map_err(|error| {
                FrameworkError::internal(format!("decode fake notification: {error}"))
            })?;
            if predicate(&notification) {
                matches.push(notification);
            }
        }
        Ok(matches)
    }

    /// Assert that a notification of this type reaches the route and matches
    /// your predicate, while the untyped free functions remain available.
    pub fn assert_sent_to<N: Notification>(&self, route: &str, predicate: impl Fn(&N) -> bool) {
        let matches = self
            .sent::<N>(route, predicate)
            .expect("decode recorded notifications");
        assert!(
            !matches.is_empty(),
            "expected at least one {} notification sent to {route}",
            N::notification_name()
        );
    }

    /// Assert that this concrete notification type never reaches the route,
    /// so another type with the same stored name cannot satisfy the check.
    pub fn assert_not_sent_to<N: Notification>(&self, route: &str) {
        let matches = self
            .sent::<N>(route, |_| true)
            .expect("decode recorded notifications");
        assert!(
            matches.is_empty(),
            "expected no {} notifications sent to {route}",
            N::notification_name()
        );
    }
}

impl Drop for NotifyFakeGuard {
    fn drop(&mut self) {
        *lock_fake() = None;
    }
}

/// Every recorded dispatch since [`install_fake`] in insertion order. Use
/// this when you need full custody of the data instead of the convenience
/// asserters below.
pub fn recorded() -> Vec<FakeRecord> {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    store.records.clone()
}

/// Assert at least one dispatch matched `pred`. Panics if the fake is
/// inactive or no match was found.
pub fn assert_sent(pred: impl Fn(&FakeRecord) -> bool) {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    let count = store.records.iter().filter(|r| pred(r)).count();
    assert!(count > 0, "expected at least one dispatched notification");
}

/// Laravel-shape `assertSentTo` keyed on per-channel route. Asserts the
/// named notification was dispatched to a recipient whose `route_for(any)`
/// equals `route`. Channel-agnostic - pass [`assert_sent_to_on`] to pin
/// the channel.
pub fn assert_sent_to(route: &str, notification_name: &str) {
    assert_sent(|r| r.route == route && r.notification == notification_name);
}

/// Like [`assert_sent_to`] but also pins the channel.
pub fn assert_sent_to_on(route: &str, channel: &str, notification_name: &str) {
    assert_sent(|r| {
        r.route == route && r.channel == channel && r.notification == notification_name
    });
}

/// Assert the named notification was dispatched at least once on any
/// channel. Convenient when the test only cares that *some* delivery
/// happened.
pub fn assert_sent_named(notification_name: &str) {
    assert_sent(|r| r.notification == notification_name);
}

/// Assert exactly `expected` records matched `pred`.
pub fn assert_sent_times(notification_name: &str, expected: usize) {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    let actual = store
        .records
        .iter()
        .filter(|r| r.notification == notification_name)
        .count();
    assert_eq!(
        actual, expected,
        "expected {expected} dispatched {notification_name} but found {actual}"
    );
}

/// Assert no notifications were dispatched.
pub fn assert_nothing_sent() {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    assert_eq!(
        store.records.len(),
        0,
        "expected no dispatched notifications but found {}",
        store.records.len()
    );
}

/// Assert exactly `expected` notifications were dispatched, across all
/// types and channels. Mirrors Laravel's `Notification::assertCount`.
pub fn assert_count(expected: usize) {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    assert_eq!(
        store.records.len(),
        expected,
        "expected {expected} dispatched notifications but found {}",
        store.records.len()
    );
}

/// Assert no notifications were dispatched to the given route. Mirrors
/// `Notification::assertNothingSentTo`.
pub fn assert_nothing_sent_to(route: &str) {
    let g = lock_fake();
    let store = g.as_ref().expect("Notify::fake() must be active");
    let count = store.records.iter().filter(|r| r.route == route).count();
    assert_eq!(
        count, 0,
        "expected no notifications dispatched to {route} but found {count}"
    );
}
