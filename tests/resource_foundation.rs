//! Deterministic bounded-resource and lifecycle foundation tests.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use suprnova_live::resource::{
    BoundedQueue, CancellationFlag, HARD_MAX_ACTIVE_PERMITS, HARD_MAX_RESOURCE_BYTES,
    HARD_MAX_RESOURCE_ITEMS, PermitPool, ResourceBounds, ResourceBoundsError, ResourceDiagnostic,
    ResourceError, ResourceOwner, ResourceQueue, Retirement, TailAdmission, TailAdmissionOutcome,
};

struct SecretDebugSentinel;

impl fmt::Debug for SecretDebugSentinel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("resource-debug-secret-sentinel")
    }
}

#[test]
fn bounded_queue_debug_is_metadata_only() {
    let mut queue = BoundedQueue::new(ResourceBounds::new(1, 4).expect("valid bounds"));
    queue
        .try_push(4, SecretDebugSentinel)
        .expect("sentinel fits");

    let debug = format!("{queue:?}");
    assert!(!debug.contains("resource-debug-secret-sentinel"));
    assert!(debug.contains("max_items"));
    assert!(debug.contains("max_bytes"));
    assert!(debug.contains("items"));
    assert!(debug.contains("retained_bytes"));
    assert!(debug.contains("retired"));

    let owner = ResourceOwner::new(ResourceBounds::new(1, 4).expect("valid bounds"));
    owner
        .queue()
        .try_push(4, SecretDebugSentinel)
        .expect("owned sentinel fits");
    assert!(!format!("{owner:?}").contains("resource-debug-secret-sentinel"));
    assert!(!format!("{:?}", owner.queue()).contains("resource-debug-secret-sentinel"));

    struct PayloadWithoutDebug;
    let opaque =
        BoundedQueue::<PayloadWithoutDebug>::new(ResourceBounds::new(1, 1).expect("valid bounds"));
    let _metadata = format!("{opaque:?}");
}

#[test]
fn configured_bounds_are_nonzero_and_cannot_exceed_engine_ceilings() {
    assert_eq!(ResourceBounds::new(0, 1), Err(ResourceBoundsError));
    assert_eq!(ResourceBounds::new(1, 0), Err(ResourceBoundsError));
    assert_eq!(
        ResourceBounds::new(HARD_MAX_RESOURCE_ITEMS + 1, 1),
        Err(ResourceBoundsError)
    );
    assert_eq!(
        ResourceBounds::new(1, HARD_MAX_RESOURCE_BYTES + 1),
        Err(ResourceBoundsError)
    );
    assert_eq!(PermitPool::new(0).unwrap_err(), ResourceBoundsError);
    assert_eq!(
        PermitPool::new(HARD_MAX_ACTIVE_PERMITS + 1).unwrap_err(),
        ResourceBoundsError
    );

    let maximum = ResourceBounds::new(HARD_MAX_RESOURCE_ITEMS, HARD_MAX_RESOURCE_BYTES)
        .expect("hard ceilings are inclusive");
    assert_eq!(maximum.max_items(), HARD_MAX_RESOURCE_ITEMS);
    assert_eq!(maximum.max_bytes(), HARD_MAX_RESOURCE_BYTES);
}

#[test]
fn queue_rejects_an_item_beyond_the_item_cap_without_changing_accounting() {
    let mut queue = BoundedQueue::new(ResourceBounds::new(2, 8).expect("valid bounds"));

    queue.try_push(2, "first").expect("first item fits");
    queue.try_push(2, "second").expect("second item fits");

    assert_eq!(
        queue.try_push(1, "third"),
        Err(ResourceError::ItemsExceeded)
    );
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.retained_bytes(), 4);
}

#[test]
fn queue_rejects_byte_cap_and_overflow_without_reserving_bytes() {
    let mut queue = BoundedQueue::new(ResourceBounds::new(3, 8).expect("valid bounds"));
    queue.try_push(8, "full").expect("exact byte cap fits");

    assert_eq!(queue.try_push(1, "over"), Err(ResourceError::BytesExceeded));
    assert_eq!(queue.retained_bytes(), 8);
    assert_eq!(queue.pop(), Some("full"));
    assert_eq!(queue.retained_bytes(), 0);

    queue.try_push(1, "one").expect("one byte fits");
    assert_eq!(
        queue.try_push(usize::MAX, "overflow"),
        Err(ResourceError::BytesExceeded)
    );
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.retained_bytes(), 1);
}

#[test]
fn queue_is_fifo_and_releases_each_reservation_on_pop() {
    let mut queue = BoundedQueue::new(ResourceBounds::new(3, 10).expect("valid bounds"));
    queue.try_push(2, "first").expect("first item");
    queue.try_push(3, "second").expect("second item");
    queue.try_push(5, "third").expect("third item");

    assert_eq!(queue.pop(), Some("first"));
    assert_eq!(queue.retained_bytes(), 8);
    assert_eq!(queue.pop(), Some("second"));
    assert_eq!(queue.retained_bytes(), 5);
    assert_eq!(queue.pop(), Some("third"));
    assert_eq!(queue.retained_bytes(), 0);
    assert_eq!(queue.pop(), None);
}

#[test]
fn queue_can_replace_only_its_newest_item_with_exact_byte_accounting() {
    let mut queue = BoundedQueue::new(ResourceBounds::new(3, 10).expect("valid bounds"));
    assert_eq!(queue.bounds(), ResourceBounds::new(3, 10).expect("bounds"));
    queue.try_push(2, "first").expect("first item");
    queue.try_push(3, "replaceable").expect("replaceable item");

    assert_eq!(
        queue.try_replace_back(5, "replacement"),
        Ok(Some("replaceable"))
    );
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.retained_bytes(), 7);
    assert_eq!(queue.pop(), Some("first"));
    assert_eq!(queue.pop(), Some("replacement"));

    queue.try_push(8, "full").expect("full item");
    assert_eq!(
        queue.try_replace_back(11, "too-large"),
        Err(ResourceError::BytesExceeded)
    );
    assert_eq!(queue.retained_bytes(), 8);
    assert_eq!(queue.pop(), Some("full"));

    let owner = ResourceOwner::<u8>::new(ResourceBounds::new(2, 4).expect("owner bounds"));
    assert_eq!(
        owner.queue().bounds(),
        ResourceBounds::new(2, 4).expect("owner bounds")
    );
    owner.queue().try_push(1, 1).expect("owned first");
    owner.queue().try_push(1, 2).expect("owned newest");
    assert_eq!(owner.queue().try_replace_back(2, 3), Ok(true));
    assert_eq!(owner.queue().retained_bytes(), 3);
    assert_eq!(owner.queue().pop(), Some(1));
    assert_eq!(owner.queue().pop(), Some(3));
}

#[test]
fn cloned_resource_handles_admit_a_batch_wholly_or_not_at_all() {
    let owner = ResourceOwner::new(ResourceBounds::new(3, 3).expect("valid bounds"));
    let batch_queue = owner.queue().clone();
    let single_queue = owner.queue().clone();
    let start = Arc::new(Barrier::new(3));

    let batch_start = Arc::clone(&start);
    let batch = thread::spawn(move || {
        batch_start.wait();
        batch_queue.try_push_batch(vec![(1, "batch-1"), (1, "batch-2"), (1, "batch-3")])
    });
    let single_start = Arc::clone(&start);
    let single = thread::spawn(move || {
        single_start.wait();
        single_queue.try_push(1, "single")
    });

    start.wait();
    let batch = batch.join().expect("batch worker");
    let single = single.join().expect("single worker");
    match (batch, single) {
        (Ok(()), Err(ResourceError::ItemsExceeded)) => {
            assert_eq!(owner.queue().pop(), Some("batch-1"));
            assert_eq!(owner.queue().pop(), Some("batch-2"));
            assert_eq!(owner.queue().pop(), Some("batch-3"));
        }
        (Err(ResourceError::ItemsExceeded), Ok(())) => {
            assert_eq!(owner.queue().pop(), Some("single"));
        }
        other => panic!("batch admission was not linearizable: {other:?}"),
    }
    assert!(owner.queue().is_empty());
    assert_eq!(owner.queue().retained_bytes(), 0);
}

#[test]
fn batch_admission_checks_count_bytes_overflow_empty_zero_and_retirement_atomically() {
    let owner = ResourceOwner::new(ResourceBounds::new(3, 4).expect("valid bounds"));

    owner
        .queue()
        .try_push_batch(Vec::<(usize, &'static str)>::new())
        .expect("an active empty batch is a no-op");
    owner
        .queue()
        .try_push_batch(vec![(0, "zero-1"), (0, "zero-2")])
        .expect("zero-byte entries remain item bounded");
    assert_eq!(owner.queue().len(), 2);
    assert_eq!(owner.queue().retained_bytes(), 0);

    assert_eq!(
        owner.queue().try_push_batch(vec![(1, "one"), (1, "two")]),
        Err(ResourceError::ItemsExceeded)
    );
    assert_eq!(owner.queue().len(), 2);
    assert_eq!(owner.queue().retained_bytes(), 0);
    assert_eq!(owner.queue().pop(), Some("zero-1"));
    assert_eq!(owner.queue().pop(), Some("zero-2"));

    assert_eq!(
        owner
            .queue()
            .try_push_batch(vec![(usize::MAX, "huge"), (1, "overflow")]),
        Err(ResourceError::BytesExceeded)
    );
    assert!(owner.queue().is_empty());
    assert_eq!(owner.queue().retained_bytes(), 0);

    owner.queue().try_push(3, "kept").expect("initial item");
    assert_eq!(
        owner.queue().try_push_batch(vec![(1, "fits"), (1, "over")]),
        Err(ResourceError::BytesExceeded)
    );
    assert_eq!(owner.queue().len(), 1);
    assert_eq!(owner.queue().retained_bytes(), 3);
    assert_eq!(owner.queue().pop(), Some("kept"));

    owner.retire();
    assert_eq!(
        owner
            .queue()
            .try_push_batch(Vec::<(usize, &'static str)>::new()),
        Err(ResourceError::Retired)
    );
    assert_eq!(
        owner.queue().try_push_batch(vec![(0, "late")]),
        Err(ResourceError::Retired)
    );
}

#[test]
fn tail_admission_decides_and_mutates_under_one_queue_lock() {
    let owner = ResourceOwner::new(ResourceBounds::new(3, 12).expect("valid bounds"));
    owner
        .queue()
        .try_push(2, "replaceable")
        .expect("initial tail");

    let replacement_queue = owner.queue().clone();
    let competing_queue = owner.queue().clone();
    let inspected = Arc::new(Barrier::new(2));
    let release_decision = Arc::new(Barrier::new(2));
    let replacement_inspected = Arc::clone(&inspected);
    let replacement_release = Arc::clone(&release_decision);
    let replacement = thread::spawn(move || {
        replacement_queue.try_admit_tail_with(3, "replacement", |tail| {
            assert_eq!(tail, Some(&"replaceable"));
            replacement_inspected.wait();
            replacement_release.wait();
            TailAdmission::Replace
        })
    });

    inspected.wait();
    let competing = thread::spawn(move || competing_queue.try_push(4, "later"));
    release_decision.wait();

    assert_eq!(
        replacement.join().expect("replacement worker"),
        Ok(TailAdmissionOutcome::Replaced)
    );
    competing
        .join()
        .expect("competing worker")
        .expect("competing append");
    assert_eq!(owner.queue().len(), 2);
    assert_eq!(owner.queue().retained_bytes(), 7);
    assert_eq!(owner.queue().pop(), Some("replacement"));
    assert_eq!(owner.queue().pop(), Some("later"));

    owner.queue().try_push(1, "identity-a").expect("new tail");
    assert_eq!(
        owner.queue().try_admit_tail_with(1, "identity-b", |tail| {
            if tail == Some(&"identity-b") {
                TailAdmission::Replace
            } else {
                TailAdmission::Append
            }
        }),
        Ok(TailAdmissionOutcome::Appended)
    );
    assert_eq!(owner.queue().pop(), Some("identity-a"));
    assert_eq!(owner.queue().pop(), Some("identity-b"));
}

struct PanickingDecisionDrop {
    queue: ResourceQueue<Self>,
    dropped: mpsc::Sender<usize>,
}

impl Drop for PanickingDecisionDrop {
    fn drop(&mut self) {
        let _ = self.dropped.send(self.queue.len());
    }
}

#[test]
fn panicking_tail_decision_drops_the_unadmitted_value_after_unlock() {
    let owner = ResourceOwner::new(ResourceBounds::new(1, 1).expect("valid bounds"));
    let queue = owner.queue().clone();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();

    thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = queue.try_admit_tail_with(
                1,
                PanickingDecisionDrop {
                    queue: queue.clone(),
                    dropped: dropped_tx,
                },
                |_| panic!("tail-decision-sentinel"),
            );
        }));
        let _ = finished_tx.send(outcome.is_err());
    });

    let dropped = dropped_rx.recv_timeout(Duration::from_millis(500));
    if dropped != Ok(0) {
        std::mem::forget(owner);
        panic!("incoming payload drop re-entered while the queue lock was held: {dropped:?}");
    }
    assert_eq!(
        finished_rx.recv_timeout(Duration::from_millis(500)),
        Ok(true)
    );
    assert!(owner.queue().is_empty());
    assert_eq!(owner.queue().retained_bytes(), 0);
}

#[test]
fn predicate_removal_releases_exact_items_and_bytes_without_touching_siblings() {
    let owner = ResourceOwner::new(ResourceBounds::new(4, 12).expect("valid bounds"));
    owner.queue().try_push(2, (1, "first")).expect("first");
    owner.queue().try_push(3, (2, "sibling")).expect("sibling");
    owner.queue().try_push(4, (1, "second")).expect("second");
    assert!(owner.queue().any(|(membership, _)| *membership == 2));
    assert!(!owner.queue().any(|(membership, _)| *membership == 3));

    let removed = owner.queue().remove_if(|(membership, _)| *membership == 1);
    assert_eq!(removed, (2, 6));
    assert_eq!(owner.queue().len(), 1);
    assert_eq!(owner.queue().retained_bytes(), 3);
    assert_eq!(owner.queue().pop(), Some((2, "sibling")));

    assert_eq!(owner.queue().remove_if(|_| true), (0, 0));
    owner.retire();
    assert_eq!(owner.queue().remove_if(|_| true), (0, 0));
}

#[test]
fn permit_exhaustion_release_and_reuse_are_exact() {
    let pool = PermitPool::new(2).expect("valid permit bound");
    let first = pool.try_acquire().expect("first permit");
    let second = pool.try_acquire().expect("second permit");

    assert_eq!(pool.active(), 2);
    assert_eq!(
        pool.try_acquire().unwrap_err(),
        ResourceError::PermitsExceeded
    );

    first.release();
    assert_eq!(pool.active(), 1);
    let replacement = pool.try_acquire().expect("released permit is reusable");
    assert_eq!(pool.active(), 2);

    drop(second);
    drop(replacement);
    assert_eq!(pool.active(), 0);
    assert_eq!(pool.available(), 2);
}

#[test]
fn concurrent_permit_saturation_never_exceeds_the_cap_and_releases_every_permit() {
    const CAPACITY: usize = 4;
    const WORKERS: usize = 8;

    let pool = PermitPool::new(CAPACITY).expect("valid permit bound");
    let start = Arc::new(Barrier::new(WORKERS + 1));
    let attempted = Arc::new(Barrier::new(WORKERS + 1));
    let release = Arc::new(Barrier::new(WORKERS + 1));
    let successes = Arc::new(AtomicUsize::new(0));
    let maximum_active = Arc::new(AtomicUsize::new(0));

    let workers = (0..WORKERS)
        .map(|_| {
            let pool = pool.clone();
            let start = Arc::clone(&start);
            let attempted = Arc::clone(&attempted);
            let release = Arc::clone(&release);
            let successes = Arc::clone(&successes);
            let maximum_active = Arc::clone(&maximum_active);
            thread::spawn(move || {
                start.wait();
                let permit = pool.try_acquire().ok();
                if permit.is_some() {
                    successes.fetch_add(1, Ordering::AcqRel);
                    maximum_active.fetch_max(pool.active(), Ordering::AcqRel);
                }
                attempted.wait();
                release.wait();
                drop(permit);
            })
        })
        .collect::<Vec<_>>();

    start.wait();
    attempted.wait();
    assert_eq!(successes.load(Ordering::Acquire), CAPACITY);
    assert_eq!(pool.active(), CAPACITY);
    assert!(maximum_active.load(Ordering::Acquire) <= CAPACITY);
    release.wait();
    for worker in workers {
        worker.join().expect("permit worker");
    }
    assert_eq!(pool.active(), 0);
    assert_eq!(pool.available(), CAPACITY);

    let permits = (0..CAPACITY)
        .map(|_| pool.try_acquire().expect("every permit is reusable"))
        .collect::<Vec<_>>();
    assert_eq!(
        pool.try_acquire().unwrap_err(),
        ResourceError::PermitsExceeded
    );
    drop(permits);
    assert_eq!(pool.active(), 0);
    assert_eq!(pool.available(), CAPACITY);
}

#[test]
fn cancellation_is_idempotent_and_visible_to_clones() {
    let flag = CancellationFlag::new();
    let observer = flag.clone();

    assert!(!flag.is_canceled());
    assert!(flag.cancel());
    assert!(observer.is_canceled());
    assert!(!observer.cancel());
    assert!(flag.is_canceled());
}

#[test]
fn retiring_an_owner_cancels_and_drains_exactly_once() {
    let owner = ResourceOwner::new(ResourceBounds::new(2, 8).expect("valid bounds"));
    owner.queue().try_push(4, "first").expect("first item fits");
    owner
        .queue()
        .try_push(4, "second")
        .expect("second item fits");

    assert_eq!(
        owner.retire(),
        Retirement {
            canceled: true,
            drained_items: 2,
            drained_bytes: 8,
        }
    );
    assert_eq!(owner.retire(), Retirement::already_retired());
    assert_eq!(
        owner.queue().try_push(1, "late"),
        Err(ResourceError::Retired)
    );
    assert_eq!(owner.queue().len(), 0);
    assert_eq!(owner.queue().retained_bytes(), 0);
    assert!(owner.cancellation().is_canceled());
}

#[test]
fn owner_pop_and_retirement_release_every_byte_once() {
    let owner = ResourceOwner::new(ResourceBounds::new(3, 12).expect("valid bounds"));
    owner.queue().try_push(3, "popped").expect("popped item");
    owner.queue().try_push(4, "drained").expect("drained item");

    assert_eq!(owner.queue().pop(), Some("popped"));
    assert_eq!(owner.queue().retained_bytes(), 4);
    assert_eq!(
        owner.retire(),
        Retirement {
            canceled: true,
            drained_items: 1,
            drained_bytes: 4,
        }
    );
    assert_eq!(owner.queue().retained_bytes(), 0);
}

#[test]
fn concurrent_admission_and_retirement_have_only_linearizable_outcomes() {
    const WORKERS: usize = 16;

    let owner = Arc::new(ResourceOwner::new(
        ResourceBounds::new(WORKERS, WORKERS).expect("valid bounds"),
    ));
    let start = Arc::new(Barrier::new(WORKERS + 1));
    let retired = Arc::new(Barrier::new(WORKERS + 1));
    let workers = (0..WORKERS)
        .map(|value| {
            let owner = Arc::clone(&owner);
            let start = Arc::clone(&start);
            let retired = Arc::clone(&retired);
            thread::spawn(move || {
                start.wait();
                let raced = owner.queue().try_push(1, value);
                retired.wait();
                let after_retirement = owner.queue().try_push(1, value + WORKERS);
                (raced, after_retirement)
            })
        })
        .collect::<Vec<_>>();

    start.wait();
    let retirement = owner.retire();
    retired.wait();

    let mut accepted = 0;
    for worker in workers {
        let (raced, after_retirement) = worker.join().expect("admission worker");
        match raced {
            Ok(()) => accepted += 1,
            Err(ResourceError::Retired) => {}
            Err(other) => panic!("unexpected racing admission outcome: {other}"),
        }
        assert_eq!(after_retirement, Err(ResourceError::Retired));
    }

    assert!(retirement.canceled);
    assert_eq!(retirement.drained_items, accepted);
    assert_eq!(retirement.drained_bytes, accepted);
    assert!(owner.queue().is_retired());
    assert_eq!(owner.queue().len(), 0);
    assert_eq!(owner.queue().retained_bytes(), 0);
    assert_eq!(owner.retire(), Retirement::already_retired());
}

struct ReentrantDropObservation {
    drops: AtomicUsize,
    saw_retired: AtomicBool,
    saw_empty: AtomicBool,
    saw_zero_bytes: AtomicBool,
    saw_empty_pop: AtomicBool,
}

impl ReentrantDropObservation {
    fn new() -> Self {
        Self {
            drops: AtomicUsize::new(0),
            saw_retired: AtomicBool::new(false),
            saw_empty: AtomicBool::new(false),
            saw_zero_bytes: AtomicBool::new(false),
            saw_empty_pop: AtomicBool::new(false),
        }
    }

    fn assert_retired_once(&self) {
        assert_eq!(self.drops.load(Ordering::Acquire), 1);
        assert!(self.saw_retired.load(Ordering::Acquire));
        assert!(self.saw_empty.load(Ordering::Acquire));
        assert!(self.saw_zero_bytes.load(Ordering::Acquire));
        assert!(self.saw_empty_pop.load(Ordering::Acquire));
    }
}

struct ReentrantDrop {
    queue: ResourceQueue<Self>,
    observation: Arc<ReentrantDropObservation>,
}

impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        self.observation
            .saw_retired
            .store(self.queue.is_retired(), Ordering::Release);
        self.observation
            .saw_empty
            .store(self.queue.is_empty(), Ordering::Release);
        self.observation
            .saw_zero_bytes
            .store(self.queue.retained_bytes() == 0, Ordering::Release);
        self.observation
            .saw_empty_pop
            .store(self.queue.pop().is_none(), Ordering::Release);
        self.observation.drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn reentrant_payload_drop_can_query_the_retired_queue_without_deadlock() {
    let observation = Arc::new(ReentrantDropObservation::new());
    let owner = ResourceOwner::new(ResourceBounds::new(1, 1).expect("valid bounds"));
    owner
        .queue()
        .try_push(
            1,
            ReentrantDrop {
                queue: owner.queue().clone(),
                observation: Arc::clone(&observation),
            },
        )
        .expect("reentrant payload fits");

    assert_eq!(owner.retire().drained_items, 1);
    observation.assert_retired_once();

    let implicit_observation = Arc::new(ReentrantDropObservation::new());
    let surviving_queue = {
        let implicit_owner = ResourceOwner::new(ResourceBounds::new(1, 1).expect("valid bounds"));
        let queue = implicit_owner.queue().clone();
        implicit_owner
            .queue()
            .try_push(
                1,
                ReentrantDrop {
                    queue: queue.clone(),
                    observation: Arc::clone(&implicit_observation),
                },
            )
            .expect("implicit reentrant payload fits");
        queue
    };
    implicit_observation.assert_retired_once();
    assert!(surviving_queue.is_retired());
}

struct ConfigurableDropPanic {
    panic_on_drop: bool,
}

impl Drop for ConfigurableDropPanic {
    fn drop(&mut self) {
        assert!(!self.panic_on_drop, "resource-payload-drop-sentinel");
    }
}

#[test]
fn panicking_payload_drop_does_not_poison_retired_queue_handles() {
    let owner = ResourceOwner::new(ResourceBounds::new(1, 1).expect("valid bounds"));
    let queue = owner.queue().clone();
    queue
        .try_push(
            1,
            ConfigurableDropPanic {
                panic_on_drop: true,
            },
        )
        .expect("panicking payload fits");

    let panic = catch_unwind(AssertUnwindSafe(|| owner.retire()));
    assert!(panic.is_err());
    assert!(queue.is_retired());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.retained_bytes(), 0);
    assert_eq!(
        queue.try_push(
            1,
            ConfigurableDropPanic {
                panic_on_drop: false,
            },
        ),
        Err(ResourceError::Retired)
    );
}

#[test]
fn dropping_an_owner_retires_every_cloned_lifecycle_handle() {
    let (queue, cancellation) = {
        let owner = ResourceOwner::new(ResourceBounds::new(1, 4).expect("valid bounds"));
        owner.queue().try_push(4, "owned").expect("owned item");
        (owner.queue().clone(), owner.cancellation())
    };

    assert!(queue.is_retired());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.retained_bytes(), 0);
    assert!(cancellation.is_canceled());
    assert_eq!(queue.try_push(1, "late"), Err(ResourceError::Retired));
}

#[test]
fn resource_diagnostics_are_closed_and_low_cardinality() {
    assert_eq!(ResourceDiagnostic::ALL.len(), 7);
    assert!(
        ResourceDiagnostic::ALL
            .iter()
            .all(|diagnostic| diagnostic.as_str().len() <= 32)
    );
    assert_eq!(
        ResourceError::ItemsExceeded.diagnostic(),
        ResourceDiagnostic::ItemsExceeded
    );
    assert_eq!(
        ResourceError::BytesExceeded.diagnostic(),
        ResourceDiagnostic::BytesExceeded
    );
    assert_eq!(
        ResourceError::PermitsExceeded.diagnostic(),
        ResourceDiagnostic::PermitsExceeded
    );
    assert_eq!(
        ResourceError::Retired.diagnostic(),
        ResourceDiagnostic::Retired
    );
}

#[test]
fn shared_primitives_are_send_and_sync_for_executor_neutral_ownership() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<CancellationFlag>();
    assert_send_sync::<PermitPool>();
    assert_send_sync::<ResourceOwner<String>>();
}
