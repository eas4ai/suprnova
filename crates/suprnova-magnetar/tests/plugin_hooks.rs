//! Lifecycle event contracts: stable mutation ids and idempotent consumers.

use std::collections::HashSet;

use magnetar::plugin::{LifecycleEvent, LifecycleEventKind};

struct IdempotentConsumer {
    seen: HashSet<String>,
}

impl IdempotentConsumer {
    fn deliver(&mut self, event: LifecycleEvent) -> bool {
        self.seen.insert(event.mutation_id)
    }
}

#[test]
fn duplicate_post_commit_delivery_is_idempotent_by_mutation_id() {
    let event = LifecycleEvent::new("mutation-7", LifecycleEventKind::UserCreated, "user-1");
    let mut consumer = IdempotentConsumer {
        seen: HashSet::new(),
    };
    assert!(consumer.deliver(event.clone()));
    assert!(!consumer.deliver(event));
    assert_eq!(consumer.seen.len(), 1);
}

#[test]
fn lifecycle_event_preserves_kind_and_user_across_retries() {
    let event = LifecycleEvent::new("mutation-8", LifecycleEventKind::SessionDeleted, "user-2");
    let retry = event.clone();
    assert_eq!(event, retry);
    assert_eq!(retry.kind, LifecycleEventKind::SessionDeleted);
    assert_eq!(retry.user_id, "user-2");
}
