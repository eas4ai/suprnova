//! Deterministic bounded-resource and lifecycle foundation tests.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use suprnova_live::resource::{
    BoundedQueue, CancellationFlag, HARD_MAX_ACTIVE_PERMITS, HARD_MAX_RESOURCE_BYTES,
    HARD_MAX_RESOURCE_ITEMS, PermitPool, ResourceBounds, ResourceBoundsError, ResourceDiagnostic,
    ResourceError, ResourceOwner, ResourceQueue, Retirement,
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
