//! Deterministic bounded-resource and lifecycle foundation tests.

use suprnova_live::resource::{
    BoundedQueue, CancellationFlag, HARD_MAX_ACTIVE_PERMITS, HARD_MAX_RESOURCE_BYTES,
    HARD_MAX_RESOURCE_ITEMS, PermitPool, ResourceBounds, ResourceBoundsError, ResourceDiagnostic,
    ResourceError, ResourceOwner, Retirement,
};

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
