use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use suprnova::testing::TestContainer;
use suprnova::{Aggregate, App, DB, EventStore, Projector, Reactor};
use tokio::sync::{Notify, Semaphore};
use tokio::time::timeout;

#[domain_event(name = "es.cancellation", version = 1)]
#[derive(Serialize, Deserialize)]
struct Cancellation {
    before: bool,
    after: bool,
}
struct Gate {
    entered: Notify,
    release: Semaphore,
    finished: Notify,
    before_waited: AtomicBool,
}
impl Gate {
    fn new() -> Self {
        Self {
            entered: Notify::new(),
            release: Semaphore::new(0),
            finished: Notify::new(),
            before_waited: AtomicBool::new(false),
        }
    }
    async fn wait(&self) {
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        self.finished.notify_one();
    }
}
#[suprnova::projector]
#[derive(Default)]
struct BeforeCommit;
#[suprnova::async_trait]
impl Projector for BeforeCommit {
    fn name(&self) -> &'static str {
        "es.cancellation.before"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == "es.cancellation"
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if event
            .decode::<Cancellation>()?
            .is_some_and(|fact| fact.before)
        {
            let gate = App::resolve::<Arc<Gate>>()?;
            if !gate.before_waited.swap(true, Ordering::SeqCst) {
                gate.wait().await;
            }
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
#[suprnova::reactor]
#[derive(Default)]
struct AfterCommit;
#[suprnova::async_trait]
impl Reactor for AfterCommit {
    fn name(&self) -> &'static str {
        "es.cancellation.after"
    }
    async fn react(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if event
            .decode::<Cancellation>()?
            .is_some_and(|fact| fact.after)
        {
            App::resolve::<Arc<Gate>>()?.wait().await;
        }
        Ok(())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_cancelled_after_commit_keeps_instance_bookkeeping() {
    let _db = database().await;
    let gate = Arc::new(Gate::new());
    TestContainer::singleton(gate.clone());
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Cancellation {
        before: false,
        after: true,
    })
    .await
    .unwrap();
    let mut persist = Box::pin(root.persist());
    tokio::select! {
        result = &mut persist => panic!("persist finished before the reactor blocked: {result:?}"),
        _ = gate.entered.notified() => {}
    }
    assert!(timeout(Duration::from_millis(20), persist).await.is_err());
    assert!(!root.is_settled());
    let version = root.persisted_version();
    let pending = root.recorded().len();
    gate.release.add_permits(1);
    timeout(Duration::from_secs(5), gate.finished.notified())
        .await
        .unwrap();
    assert_eq!(version, 1);
    assert_eq!(pending, 0);
    assert_eq!(EventStore::events(root.uuid()).await.unwrap().len(), 1);
    root.record(Cancellation {
        before: false,
        after: false,
    })
    .await
    .unwrap();
    root.persist().await.unwrap();
    assert_eq!(root.persisted_version(), 2);
    assert!(root.recorded().is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_cancelled_before_commit_retains_pending_facts() {
    let _db = database().await;
    let gate = Arc::new(Gate::new());
    TestContainer::singleton(gate.clone());
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Cancellation {
        before: true,
        after: false,
    })
    .await
    .unwrap();
    let mut persist = Box::pin(root.persist());
    tokio::select! {
        result = &mut persist => panic!("persist finished before the projector blocked: {result:?}"),
        _ = gate.entered.notified() => {}
    }
    assert!(timeout(Duration::from_millis(20), persist).await.is_err());
    assert!(
        timeout(Duration::from_secs(5), EventStore::events(root.uuid()))
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert_eq!(root.persisted_version(), 0);
    assert_eq!(root.recorded().len(), 1);
    assert!(!root.is_settled());
    root.record(Cancellation {
        before: false,
        after: false,
    })
    .await
    .unwrap();
    assert!(root.is_settled());
    assert_eq!(root.persisted_version(), 0);
    assert_eq!(root.recorded().len(), 2);
    root.persist().await.unwrap();
    assert_eq!(root.persisted_version(), 2);
    assert!(root.is_settled());
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_persistence_outcomes_have_distinct_types_and_context() {
    let db = database().await;
    let id = uuid();
    let mut first = Aggregate::<Tally>::new(&id).unwrap();
    let mut stale = Aggregate::<Tally>::new(&id).unwrap();
    first.record(RenamedAddition { amount: 1 }).await.unwrap();
    stale.record(RenamedAddition { amount: 2 }).await.unwrap();
    first.persist().await.unwrap();
    let conflict = stale.persist().await.unwrap_err();
    assert_eq!(conflict.status_code(), 409, "{conflict}");
    assert!(conflict.to_string().contains(&id));
    assert!(conflict.to_string().contains("expected version 0"));
    assert!(conflict.to_string().contains("actual version 1"));
    let ownership = DB::transaction(move |_tx| Box::pin(async move { stale.persist().await }))
        .await
        .unwrap_err();
    assert!(
        matches!(ownership, FrameworkError::Internal { .. }),
        "{ownership}"
    );
    assert!(ownership.to_string().contains(&id));
    db.execute_unprepared("CREATE TRIGGER reject_insert BEFORE INSERT ON stored_events BEGIN SELECT RAISE(ABORT, 'insert refused'); END").await.unwrap();
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(RenamedAddition { amount: 1 }).await.unwrap();
    let storage = root.persist().await.unwrap_err();
    assert!(matches!(storage, FrameworkError::Database(_)), "{storage}");
    assert!(storage.to_string().contains(root.uuid()));
    assert!(storage.to_string().contains("attempted version 1"));
    db.execute_unprepared("DROP TRIGGER reject_insert; CREATE TRIGGER reject_lock BEFORE UPDATE ON projector_checkpoints BEGIN SELECT RAISE(ABORT, 'lock refused'); END").await.unwrap();
    let lock = root.persist().await.unwrap_err();
    assert!(matches!(lock, FrameworkError::Database(_)), "{lock}");
    assert!(lock.to_string().contains(root.uuid()));
    assert!(lock.to_string().contains("attempted version 1"));
    assert_eq!(root.persisted_version(), 0);
    assert_eq!(root.recorded().len(), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_cancelled_commit_listener_publishes_durable_bookkeeping() {
    use suprnova::{EventFacade, Listener, TransactionCommitted};
    struct BlockCommit {
        gate: Arc<Gate>,
        waited: AtomicBool,
    }
    #[suprnova::async_trait]
    impl Listener<TransactionCommitted> for BlockCommit {
        async fn handle(&self, _event: &TransactionCommitted) -> Result<(), FrameworkError> {
            if !self.waited.swap(true, Ordering::SeqCst) {
                self.gate.wait().await;
            }
            Ok(())
        }
    }
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            EventFacade::forget::<TransactionCommitted>();
        }
    }
    let _db = database().await;
    let _cleanup = Cleanup;
    let gate = Arc::new(Gate::new());
    EventFacade::listen::<TransactionCommitted, _>(Arc::new(BlockCommit {
        gate: gate.clone(),
        waited: AtomicBool::new(false),
    }))
    .await;
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(RenamedAddition { amount: 2 }).await.unwrap();
    let mut persist = Box::pin(root.persist());
    tokio::select! {
        result = &mut persist => panic!("persist completed before the listener blocked: {result:?}"),
        _ = gate.entered.notified() => {}
    }
    assert!(timeout(Duration::from_millis(20), persist).await.is_err());
    let durable = EventStore::events(root.uuid()).await.unwrap().len();
    assert!(!root.is_settled());
    let version = root.persisted_version();
    let pending = root.recorded().len();
    gate.release.add_permits(1);
    timeout(Duration::from_secs(5), gate.finished.notified())
        .await
        .unwrap();
    assert_eq!(durable, 1);
    assert_eq!(version, 1, "COMMIT returned before the listener ran");
    assert_eq!(pending, 0);
    root.record(RenamedAddition { amount: 3 }).await.unwrap();
    assert!(root.is_settled());
    assert_eq!(root.persisted_version(), 1);
    root.persist().await.unwrap();
    assert_eq!(root.persisted_version(), 2);
    assert_eq!(root.state().total, 5);
    assert!(root.is_settled());
}
