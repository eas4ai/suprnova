//! `Bus::fake()` - installs a capture-only recorder.
//!
//! `install_fake()` acquires a process-wide serialization mutex for the
//! lifetime of the returned `BusFakeGuard` so parallel tests calling
//! `Bus::fake()` cannot clobber each other's recorded-command store. This
//! matches [`crate::events::testing`] and [`crate::queue::testing`].
//!
//! Tests that interleave real-dispatch and fake-dispatch within the same
//! binary (as `framework/tests/bus.rs` does) still need their own
//! `#[serial_test::serial]` annotation: a real-dispatch test does not
//! acquire `FAKE_SERIAL`, so it can race a parallel fake-dispatch test and
//! see `is_active() == true`. `FAKE_SERIAL` removes the cross-fake hazard,
//! not the real-vs-fake one.

use crate::bus::command::Command;
use crate::error::FrameworkError;
use once_cell::sync::Lazy;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

#[derive(Default)]
struct FakeStore {
    dispatched: HashMap<TypeId, Vec<serde_json::Value>>,
    dispatched_sync: HashMap<TypeId, Vec<serde_json::Value>>,
    dispatched_after_response: HashMap<TypeId, Vec<serde_json::Value>>,
}

pub(super) enum DispatchMode {
    Regular,
    Sync,
    AfterResponse,
}

/// Process-wide serializer: only one test may hold the bus fake at a time.
static FAKE_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static FAKE: Mutex<Option<FakeStore>> = Mutex::new(None);

/// Poison-safe access to the fake store. A panicking test poisons the mutex;
/// reading through the poison keeps the next test from inheriting the abort.
fn lock_fake() -> MutexGuard<'static, Option<FakeStore>> {
    FAKE.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn is_active() -> bool {
    lock_fake().is_some()
}

pub(super) fn record<C: Command>(cmd: &C, mode: DispatchMode) -> Result<(), FrameworkError> {
    let payload = serde_json::to_value(cmd)
        .map_err(|e| FrameworkError::internal(format!("bus encode: {e}")))?;
    let mut g = lock_fake();
    if let Some(s) = g.as_mut() {
        let commands = match mode {
            DispatchMode::Regular => &mut s.dispatched,
            DispatchMode::Sync => &mut s.dispatched_sync,
            DispatchMode::AfterResponse => &mut s.dispatched_after_response,
        };
        commands.entry(TypeId::of::<C>()).or_default().push(payload);
    }
    Ok(())
}

fn commands<C: Command>(mode: DispatchMode) -> Vec<C> {
    let payloads = {
        let g = lock_fake();
        let store = g.as_ref().expect("Bus::fake() must be active");
        let commands = match mode {
            DispatchMode::Regular => &store.dispatched,
            DispatchMode::Sync => &store.dispatched_sync,
            DispatchMode::AfterResponse => &store.dispatched_after_response,
        };
        commands
            .get(&TypeId::of::<C>())
            .cloned()
            .unwrap_or_default()
    };
    payloads
        .into_iter()
        .map(|payload| {
            serde_json::from_value(payload).unwrap_or_else(|error| {
                panic!("cannot decode captured {}: {error}", C::command_name())
            })
        })
        .collect()
}

/// Return captured ordinary commands in order so your test can filter their typed data.
pub fn dispatched<C: Command>() -> Vec<C> {
    commands(DispatchMode::Regular)
}

/// Return captured synchronous commands so your test can inspect that dispatch mode alone.
pub fn dispatched_sync<C: Command>() -> Vec<C> {
    commands(DispatchMode::Sync)
}

/// Return captured deferred commands so your test can inspect work scheduled after the response.
pub fn dispatched_after_response<C: Command>() -> Vec<C> {
    commands(DispatchMode::AfterResponse)
}

/// Install a fake Bus that captures dispatched commands instead of running handlers.
///
/// Returns a guard that removes the fake when dropped. The guard also holds
/// a process-wide serialization lock so parallel `#[tokio::test]`s that call
/// `Bus::fake()` cannot interleave with each other's captured-store.
pub fn install_fake() -> BusFakeGuard {
    let serial = FAKE_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    *lock_fake() = Some(FakeStore::default());
    BusFakeGuard { _serial: serial }
}

/// RAII guard returned by [`install_fake`]. Resets the fake on drop and
/// releases the process-wide fake serializer.
pub struct BusFakeGuard {
    _serial: MutexGuard<'static, ()>,
}

impl Drop for BusFakeGuard {
    fn drop(&mut self) {
        // Tolerate poisoning: if a test assertion panicked while holding
        // the lock, this drop still needs to clear the fake so the next
        // test starts fresh. Reading through the poison avoids a
        // panic-in-destructor abort that would mask the original failure.
        *lock_fake() = None;
    }
}

/// Count dispatched commands of type `C` matching `pred`. Shared core for the
/// assertion helpers below. Panics if the fake is not active.
fn count_matching<C: Command>(pred: &impl Fn(&C) -> bool) -> usize {
    let g = lock_fake();
    let store = g.as_ref().expect("Bus::fake() must be active");
    store
        .dispatched
        .get(&TypeId::of::<C>())
        .map(|b| {
            b.iter()
                .filter_map(|p| serde_json::from_value::<C>(p.clone()).ok())
                .filter(|c| pred(c))
                .count()
        })
        .unwrap_or(0)
}

/// Assert that at least one command of type `C` was dispatched matching `pred`.
///
/// Panics if the fake is not active or no matching command was found.
pub fn assert_dispatched<C: Command>(pred: impl Fn(&C) -> bool) {
    let count = count_matching::<C>(&pred);
    assert!(
        count > 0,
        "expected at least one dispatched {}",
        C::command_name()
    );
}

/// Assert that NO command of type `C` matching `pred` was dispatched.
///
/// Panics if the fake is not active or a matching command was found.
pub fn assert_not_dispatched<C: Command>(pred: impl Fn(&C) -> bool) {
    let count = count_matching::<C>(&pred);
    assert_eq!(
        count,
        0,
        "expected no dispatched {} but found {count}",
        C::command_name()
    );
}

/// Assert that EXACTLY `expected` commands of type `C` matching `pred` were
/// dispatched.
///
/// Panics if the fake is not active or the count does not match.
pub fn assert_dispatched_times<C: Command>(pred: impl Fn(&C) -> bool, expected: usize) {
    let actual = count_matching::<C>(&pred);
    assert_eq!(
        actual,
        expected,
        "expected {expected} dispatched {} but found {actual}",
        C::command_name()
    );
}

/// Assert that NO commands of any type were dispatched under the active fake.
///
/// Panics if the fake is not active or any command was dispatched.
pub fn assert_nothing_dispatched() {
    let total: usize = {
        let g = lock_fake();
        let store = g.as_ref().expect("Bus::fake() must be active");
        store
            .dispatched
            .values()
            .chain(store.dispatched_sync.values())
            .chain(store.dispatched_after_response.values())
            .map(Vec::len)
            .sum()
    };
    assert_eq!(
        total, 0,
        "expected no dispatched commands but found {total}"
    );
}
