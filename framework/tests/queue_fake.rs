use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobQueued;
use suprnova::queue::testing::{
    assert_pushed, assert_pushed_later, install_fake, pushed_with_available_at, pushed_with_id,
};
use suprnova::{FrameworkError, Job, Queue, async_trait};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Greet {
    name: String,
}

#[async_trait]
impl Job for Greet {
    fn job_name() -> &'static str {
        "Greet"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
async fn queue_fake_captures_pushed_jobs_without_running_them() {
    let _guard = install_fake();
    Queue::push(Greet {
        name: "Lucas".into(),
    })
    .await
    .unwrap();
    Queue::push(Greet { name: "Ada".into() }).await.unwrap();
    assert_pushed::<Greet>(|g| g.name == "Lucas");
    assert_pushed::<Greet>(|g| g.name == "Ada");
}

#[tokio::test]
async fn queue_fake_isolates_per_test() {
    let _guard = install_fake();
    Queue::push(Greet {
        name: "Solo".into(),
    })
    .await
    .unwrap();
    assert_pushed::<Greet>(|g| g.name == "Solo");
}

#[tokio::test]
async fn queue_fake_records_available_at_for_delayed_pushes() {
    let _guard = install_fake();
    let now = Utc::now();
    let in_an_hour = now + ChronoDuration::hours(1);

    // Immediate push records ~now.
    Queue::push(Greet {
        name: "instant".into(),
    })
    .await
    .unwrap();

    // Delayed push must record the dispatched timestamp, not now.
    Queue::push_later(
        Greet {
            name: "scheduled".into(),
        },
        in_an_hour,
    )
    .await
    .unwrap();

    let entries = pushed_with_available_at::<Greet>();
    assert_eq!(entries.len(), 2, "two pushes captured");

    let scheduled = entries
        .iter()
        .find(|(g, _)| g.name == "scheduled")
        .expect("delayed push present");
    assert_eq!(
        scheduled.1, in_an_hour,
        "fake must record the dispatched available_at exactly"
    );

    let instant = entries
        .iter()
        .find(|(g, _)| g.name == "instant")
        .expect("instant push present");
    let drift = (instant.1 - now).num_milliseconds().abs();
    assert!(
        drift < 1_000,
        "Queue::push records ~now (drift {drift}ms must be small)"
    );

    // Predicate helper also exercises both fields.
    assert_pushed_later::<Greet>(|g, t| g.name == "scheduled" && t == in_an_hour);
}

// ---- the fake stamps an envelope id (Laravel 13.25 #60966) ---------------

#[tokio::test]
async fn queue_fake_stamps_a_distinct_id_on_every_push() {
    let _guard = install_fake();
    Queue::push(Greet { name: "one".into() }).await.unwrap();
    Queue::push(Greet { name: "two".into() }).await.unwrap();

    let entries = pushed_with_id::<Greet>();
    assert_eq!(entries.len(), 2, "two pushes captured");
    assert_ne!(entries[0].1, entries[1].1, "ids must be distinct");
    assert!(
        !entries[0].1.is_nil() && !entries[1].1.is_nil(),
        "ids must not be the nil UUID"
    );
    assert_eq!(entries[0].0.name, "one");
    assert_eq!(entries[1].0.name, "two");
}

#[tokio::test]
async fn queue_fake_id_matches_the_job_queued_event_id() {
    // The point of the id: a test can join what the fake captured to
    // what a listener saw. Under the fake there is no driver, so the
    // fake is what emits the pair - with the id it recorded.
    let _queue = install_fake();
    let _events = EventFacade::fake();

    Queue::push(Greet {
        name: "correlated".into(),
    })
    .await
    .unwrap();

    let entries = pushed_with_id::<Greet>();
    assert_eq!(entries.len(), 1);

    let queued = dispatched::<JobQueued>(|e| e.job_name == "Greet");
    assert_eq!(queued.len(), 1, "the fake emits exactly one JobQueued");
    assert_eq!(
        queued[0].id, entries[0].1,
        "the captured push and the event must name the same envelope"
    );
}

#[tokio::test]
async fn queue_fake_delayed_push_also_carries_an_id() {
    let _guard = install_fake();
    let when = Utc::now() + ChronoDuration::hours(2);
    Queue::push_later(
        Greet {
            name: "later".into(),
        },
        when,
    )
    .await
    .unwrap();

    let entries = pushed_with_id::<Greet>();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].1.is_nil());
}
