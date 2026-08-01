//! One place that knows how a Suprnova process is asked to stop.
//!
//! Two signals mean "stop", and which one arrives depends entirely on who
//! is doing the asking. A developer at a terminal sends SIGINT. Every
//! automated supervisor — `docker stop`, Coolify, systemd, Kubernetes —
//! sends SIGTERM. A process that listens for only one of them is correct
//! in exactly one of those situations.
//!
//! This module exists because the framework got that split wrong: the HTTP
//! server listened for both, and the `schedule:work`, `queue:work` and
//! `workflow:work` daemons listened only for SIGINT. Each of the three
//! already had a careful bounded drain sitting behind its `select!`, and
//! none of it had ever run in a container.
//!
//! # Why a listener and not a future
//!
//! The obvious fix — build a combined signal future inside each loop's
//! `select!` — reintroduces a bug the server already fixed. A signal
//! future only observes signals delivered *after* it registers, so
//! rebuilding one every iteration leaves a window: a signal arriving
//! between dropping the old future and constructing the new one is lost,
//! and the loop keeps running as though nothing happened.
//!
//! So the signal is observed once, in a task that outlives the loop, and
//! published through a `watch` channel. `watch` carries state rather than
//! merely waking parked waiters, which is what makes a waiter constructed
//! *after* the signal still see it.
//!
//! # PID 1
//!
//! Installing a handler is not optional politeness. The kernel does not
//! apply default signal dispositions to PID 1, so an unhandled SIGTERM to
//! PID 1 is discarded rather than fatal — and `CMD ["app", "queue:work"]`
//! makes the process PID 1. Without a handler the container does not die
//! promptly and unpleasantly; it does not die at all until the supervisor
//! gives up and sends SIGKILL, taking in-flight work with it.

/// Which signal asked the process to stop.
///
/// Carried through rather than collapsed to `()` so shutdown logs name the
/// real cause. "Shutting down (SIGTERM)" and "shutting down (Ctrl-C)" send
/// an operator to very different places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownSignal {
    /// SIGINT — a developer pressing Ctrl-C.
    Interrupt,
    /// SIGTERM — a supervisor stopping the process.
    Terminate,
}

impl ShutdownSignal {
    /// Human-facing name, for the one log line that reports the cause.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Interrupt => "Ctrl-C",
            Self::Terminate => "SIGTERM",
        }
    }
}

/// A handle to the process-wide shutdown signal.
///
/// Cheap to clone; every clone observes the same signal, including one
/// created after it already fired.
#[derive(Clone)]
pub(crate) struct ShutdownListener {
    rx: tokio::sync::watch::Receiver<Option<ShutdownSignal>>,
}

impl ShutdownListener {
    /// Resolve once shutdown has been signalled — immediately if it
    /// already was.
    ///
    /// Safe to call repeatedly inside a loop: each call clones a fresh
    /// receiver, and `wait_for` inspects the current value before parking.
    pub(crate) async fn fired(&self) -> ShutdownSignal {
        let mut rx = self.rx.clone();
        match rx.wait_for(|fired| fired.is_some()).await {
            Ok(seen) => seen.expect("wait_for only returns once the value is Some"),
            // The sender is dropped only if the listener task was aborted,
            // which happens at process teardown. Treating that as a
            // terminate request is the safe reading: the alternative is a
            // future that never resolves, which would hang the very loop
            // that is trying to exit.
            Err(_) => ShutdownSignal::Terminate,
        }
    }

    /// The `()`-returning form, for call sites that race this against
    /// other work and do not care which signal won.
    pub(crate) async fn fired_unit(&self) {
        let _ = self.fired().await;
    }
}

/// Listen for SIGINT and SIGTERM once, in a task, and publish the result.
///
/// Call this once per long-running loop, outside the loop, and clone the
/// handle wherever it is needed. Calling it per iteration would recreate
/// the missed-signal window this exists to close.
pub(crate) fn spawn_shutdown_listener() -> ShutdownListener {
    let (tx, rx) = tokio::sync::watch::channel(None);
    tokio::spawn(async move {
        let signal = tokio::select! {
            _ = tokio::signal::ctrl_c() => ShutdownSignal::Interrupt,
            _ = wait_terminate() => ShutdownSignal::Terminate,
        };
        tracing::info!(signal = signal.as_str(), "shutdown signal received");
        let _ = tx.send(Some(signal));
    });
    ShutdownListener { rx }
}

/// Wait for SIGTERM on Unix. On non-Unix platforms returns a future that
/// never resolves, so the `tokio::select!` arm stays parked.
#[cfg(unix)]
async fn wait_terminate() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut sig) => {
            sig.recv().await;
        }
        Err(err) => {
            tracing::warn!(
                ?err,
                "failed to install SIGTERM handler; Ctrl-C is still honored"
            );
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_terminate() {
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a listener that can be fired by hand, so the loops that
    /// consume one are testable without raising real signals at the test
    /// process — which would take the test runner down with them.
    fn riggable() -> (
        tokio::sync::watch::Sender<Option<ShutdownSignal>>,
        ShutdownListener,
    ) {
        let (tx, rx) = tokio::sync::watch::channel(None);
        (tx, ShutdownListener { rx })
    }

    /// The property that makes one listener correct for a whole loop. The
    /// shape this replaced rebuilt its signal future every iteration, and
    /// a freshly built one only observes signals delivered after it
    /// registers — so a signal arriving between two iterations was lost.
    #[tokio::test]
    async fn a_waiter_that_arrives_after_the_signal_still_observes_it() {
        let (tx, listener) = riggable();
        tx.send(Some(ShutdownSignal::Terminate))
            .expect("receiver is alive");

        let seen = tokio::time::timeout(std::time::Duration::from_secs(5), listener.fired())
            .await
            .expect(
                "a waiter built after the signal fired must resolve; if it parks, \
                 the loop can miss a shutdown that already happened",
            );
        assert_eq!(seen, ShutdownSignal::Terminate);
    }

    /// A waiter already parked when the signal arrives sees it too — the
    /// ordinary case, asserted so a future refactor cannot fix one
    /// direction by breaking the other.
    #[tokio::test]
    async fn a_waiter_parked_before_the_signal_observes_it() {
        let (tx, listener) = riggable();
        let waiter = tokio::spawn(async move { listener.fired().await });

        tokio::task::yield_now().await;
        tx.send(Some(ShutdownSignal::Interrupt))
            .expect("receiver is alive");

        let seen = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("parked waiter must resolve once the signal fires")
            .expect("waiter task must not panic");
        assert_eq!(seen, ShutdownSignal::Interrupt);
    }

    /// Which signal arrived has to survive the trip, because it is the
    /// only thing distinguishing "a developer stopped this" from "the
    /// orchestrator is replacing this pod" in the logs.
    #[tokio::test]
    async fn the_signal_that_fired_is_reported_not_collapsed() {
        for signal in [ShutdownSignal::Interrupt, ShutdownSignal::Terminate] {
            let (tx, listener) = riggable();
            tx.send(Some(signal)).expect("receiver is alive");
            assert_eq!(listener.fired().await, signal);
        }
    }

    /// A dropped sender must not park the waiter forever. The loop calling
    /// this is trying to exit; a future that never resolves would hang it.
    #[tokio::test]
    async fn a_dropped_sender_resolves_rather_than_hanging() {
        let (tx, listener) = riggable();
        drop(tx);

        let seen = tokio::time::timeout(std::time::Duration::from_secs(5), listener.fired())
            .await
            .expect("a dropped sender must not leave the waiter parked");
        assert_eq!(seen, ShutdownSignal::Terminate);
    }
}
