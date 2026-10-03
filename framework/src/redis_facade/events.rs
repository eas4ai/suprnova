//! The command events: what `Redis::listen` and
//! `Redis::listen_for_failures` hear while events are enabled.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

/// A command a connection ran, as [`Redis::listen`](super::Redis::listen)
/// hears it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RedisCommandExecuted {
    /// The name of the connection that ran it.
    pub connection: String,
    /// The command, in upper case: `SET`, `LRANGE`.
    pub command: String,
    /// Its arguments, each as text (bytes that are not UTF-8 are replaced).
    pub arguments: Vec<String>,
    /// How long the server took to answer, retries included.
    pub duration: Duration,
}

/// A command that failed, as
/// [`Redis::listen_for_failures`](super::Redis::listen_for_failures) hears it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RedisCommandFailed {
    /// The name of the connection that ran it.
    pub connection: String,
    /// The command, in upper case.
    pub command: String,
    /// Its arguments, each as text.
    pub arguments: Vec<String>,
    /// The error, as the server or the connection gave it.
    pub error: String,
    /// How long the command ran before it failed.
    pub duration: Duration,
}

type Listener<E> = Arc<dyn Fn(&E) + Send + Sync>;

static ENABLED: AtomicBool = AtomicBool::new(false);
static EXECUTED: RwLock<Vec<Listener<RedisCommandExecuted>>> = RwLock::new(Vec::new());
static FAILED: RwLock<Vec<Listener<RedisCommandFailed>>> = RwLock::new(Vec::new());

pub(crate) fn enable(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub(crate) fn listen(listener: Listener<RedisCommandExecuted>) {
    EXECUTED
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .push(listener);
}

pub(crate) fn listen_for_failures(listener: Listener<RedisCommandFailed>) {
    FAILED
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .push(listener);
}

// The listeners are cloned out of the lock before they run, so a listener
// may add another without deadlocking.

pub(crate) fn executed(event: &RedisCommandExecuted) {
    let listeners = EXECUTED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for listener in listeners {
        listener(event);
    }
}

pub(crate) fn failed(event: &RedisCommandFailed) {
    let listeners = FAILED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for listener in listeners {
        listener(event);
    }
}
