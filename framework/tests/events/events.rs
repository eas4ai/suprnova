//! Integration tests for the events subsystem and error→event bridge.

use suprnova::{ErrorOccurred, EventFacade, FrameworkError, HttpResponse};
use tokio::sync::Mutex;

// Tests in this file share the global event fake store, so they must
// run serially. Use `tokio::sync::Mutex` so the guard can be safely
// held across `.await` points.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn server_error_dispatches_error_occurred() {
    let _serial = TEST_LOCK.lock().await;
    let _guard = EventFacade::fake();

    let err = FrameworkError::internal("boom");
    let _resp: HttpResponse = err.into();

    // The dispatch is spawned; yield + small sleep to let it land.
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    suprnova::events::testing::assert_dispatched::<ErrorOccurred>(|e| {
        e.status_code == 500 && e.error_message.contains("boom")
    });
}

#[tokio::test]
async fn client_error_does_not_dispatch_error_occurred() {
    let _serial = TEST_LOCK.lock().await;
    let _guard = EventFacade::fake();

    let err = FrameworkError::param("name");
    let _resp: HttpResponse = err.into();

    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    suprnova::events::testing::assert_not_dispatched::<ErrorOccurred>(|_| true);
}

/// A queued event whose listener runs until the test releases it.
#[derive(Debug, Clone)]
struct HeldEvent;

impl suprnova::Event for HeldEvent {
    fn event_name() -> &'static str {
        "events.HeldEvent"
    }

    fn queued() -> bool {
        true
    }
}

struct HeldListener {
    started: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}

#[suprnova::async_trait]
impl suprnova::Listener<HeldEvent> for HeldListener {
    async fn handle(&self, _event: &HeldEvent) -> Result<(), FrameworkError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(())
    }
}

/// A dispatcher with one held listener running, and the handle that
/// releases it.
async fn dispatcher_with_a_held_listener() -> (
    suprnova::EventDispatcher,
    std::sync::Arc<tokio::sync::Notify>,
) {
    let dispatcher = suprnova::EventDispatcher::new();
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    dispatcher
        .listen::<HeldEvent, _>(std::sync::Arc::new(HeldListener {
            started: started.clone(),
            release: release.clone(),
        }))
        .await;
    dispatcher
        .dispatch(HeldEvent)
        .await
        .expect("the queued event is admitted");
    started.notified().await;
    (dispatcher, release)
}

/// Two drains of one dispatcher at once. The first takes the running
/// listener out of the task set; the second found the set empty and
/// reported the dispatcher drained while the listener still ran. A
/// console command's end and a server's shutdown both trust that report
/// before the runtime drops the listener.
#[tokio::test]
async fn a_second_drain_waits_for_a_listener_the_first_drain_holds() {
    let (dispatcher, release) = dispatcher_with_a_held_listener().await;

    let first = dispatcher.drain_queued(std::time::Duration::from_secs(5));
    tokio::pin!(first);
    assert!(futures::poll!(first.as_mut()).is_pending());
    let second = dispatcher.drain_queued(std::time::Duration::from_secs(5));
    tokio::pin!(second);
    assert!(
        futures::poll!(second.as_mut()).is_pending(),
        "the second drain reported the dispatcher drained while the first held a running listener"
    );

    release.notify_one();
    let (first, second) = tokio::join!(first, second);
    assert_eq!(
        (first, second),
        (0, 0),
        "both drains saw the listener finish"
    );
}

/// The second drain's deadline counts the listener the first drain holds:
/// it is still running, wherever its task is kept.
#[tokio::test(start_paused = true)]
async fn a_second_drain_counts_a_listener_the_first_drain_holds_at_its_deadline() {
    let (dispatcher, release) = dispatcher_with_a_held_listener().await;

    let first = dispatcher.drain_queued(std::time::Duration::from_secs(60));
    tokio::pin!(first);
    assert!(futures::poll!(first.as_mut()).is_pending());
    let remaining = dispatcher
        .drain_queued(std::time::Duration::from_secs(1))
        .await;
    assert_eq!(
        remaining, 1,
        "the second drain's deadline left out the listener the first drain holds"
    );

    release.notify_one();
    assert_eq!(first.await, 0, "the first drain saw the listener finish");
}
