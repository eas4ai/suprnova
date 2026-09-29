#![cfg(feature = "testing")]

//! `suprnova::clock` and `TestClock`: the clock the framework reads can be
//! moved by a test, so expiry, idle timeouts and pruning are tested without
//! sleeping.
//!
//! The first group tests the clock itself. The second group shows the point
//! of it on three subsystems: a signed URL that expires, a session that idles
//! out, and a soft-deleted model that becomes prunable.

use std::sync::{Arc, Barrier};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use suprnova::eloquent::Prunable;
use suprnova::http::Request;
use suprnova::routing::SignatureVerdict;
use suprnova::routing::url::{signature_verdict, signed_url};
use suprnova::session::{DatabaseSessionDriver, SessionData, SessionStore};
use suprnova::testing::{TestClock, TestDatabase};
use suprnova::{Model, attrs, model};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("in range")
}

// ---- The clock ----------------------------------------------------------

#[test]
fn a_frozen_clock_stands_still() {
    let clock = TestClock::freeze();

    let first = suprnova::clock::now();
    let second = suprnova::clock::now();

    assert_eq!(first, second);
    assert_eq!(clock.now(), first);
}

#[test]
fn travel_to_and_advance_move_the_clock() {
    let clock = TestClock::travel_to(at(1_000_000_000));
    assert_eq!(suprnova::clock::now(), at(1_000_000_000));

    clock.advance(Duration::minutes(90));
    assert_eq!(suprnova::clock::now(), at(1_000_000_000 + 5_400));

    clock.advance(Duration::minutes(-90));
    assert_eq!(suprnova::clock::now(), at(1_000_000_000));

    clock.set(at(1_500_000_000));
    assert_eq!(suprnova::clock::now(), at(1_500_000_000));
}

#[test]
fn guards_nest_and_give_the_outer_time_back() {
    let outer = TestClock::travel_to(at(1_000_000_000));
    {
        let _inner = TestClock::travel_to(at(2_000_000_000));
        assert_eq!(suprnova::clock::now(), at(2_000_000_000));
    }
    assert_eq!(suprnova::clock::now(), at(1_000_000_000));

    drop(outer);
    let after = suprnova::clock::now();
    assert!(
        (after - Utc::now()).num_seconds().abs() < 60,
        "with no guard left the system clock answers"
    );
}

#[test]
fn a_panic_inside_a_guard_gives_the_clock_back() {
    let outcome = std::panic::catch_unwind(|| {
        let _clock = TestClock::travel_to(at(1_000_000_000));
        assert_eq!(suprnova::clock::now(), at(1_000_000_000));
        std::panic::resume_unwind(Box::new("test panic"));
    });
    assert!(outcome.is_err());

    assert!(
        (suprnova::clock::now() - Utc::now()).num_seconds().abs() < 60,
        "the unwound guard restored the system clock"
    );
}

#[test]
fn two_threads_with_two_clocks_do_not_see_each_other() {
    let barrier = Arc::new(Barrier::new(2));
    let spawn = |secs: i64| {
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let _clock = TestClock::travel_to(at(secs));
            // Both threads hold their clock before either one reads.
            barrier.wait();
            let seen = suprnova::clock::now();
            barrier.wait();
            seen
        })
    };
    let one = spawn(1_000_000_000);
    let two = spawn(2_000_000_000);

    assert_eq!(one.join().expect("thread one"), at(1_000_000_000));
    assert_eq!(two.join().expect("thread two"), at(2_000_000_000));
}

#[tokio::test]
async fn a_thread_clock_holds_across_an_await() {
    let _clock = TestClock::travel_to(at(1_000_000_000));

    tokio::task::yield_now().await;

    assert_eq!(suprnova::clock::now(), at(1_000_000_000));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scope_holds_across_awaits_on_a_multi_thread_runtime() {
    let seen = TestClock::scope(at(1_000_000_000), |handle| async move {
        let mut seen = Vec::new();
        for _ in 0..4 {
            tokio::task::yield_now().await;
            seen.push(suprnova::clock::now());
        }
        handle.advance(Duration::hours(1));
        tokio::task::yield_now().await;
        seen.push(suprnova::clock::now());
        seen
    })
    .await;

    assert_eq!(&seen[..4], &[at(1_000_000_000); 4]);
    assert_eq!(seen[4], at(1_000_000_000 + 3_600));
    assert!(
        (suprnova::clock::now() - Utc::now()).num_seconds().abs() < 60,
        "outside the scope the system clock answers"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spawned_task_sees_the_scope_through_the_handle() {
    TestClock::scope(at(1_000_000_000), |handle| async move {
        let inherited = tokio::spawn(async { suprnova::clock::now() })
            .await
            .expect("task");
        assert!(
            (inherited - Utc::now()).num_seconds().abs() < 60,
            "a bare spawned task does not inherit the task-local"
        );

        let mover = handle.clone();
        let entered =
            tokio::spawn(async move { mover.run(async { suprnova::clock::now() }).await })
                .await
                .expect("task");
        assert_eq!(entered, at(1_000_000_000));
    })
    .await;
}

// ---- A signed URL that expires ------------------------------------------

#[test]
fn a_signed_url_expires_when_the_clock_passes_its_deadline() {
    suprnova::testing::install_test_encryption_key();
    let clock = TestClock::travel_to(at(1_900_000_000));
    let deadline = (suprnova::clock::now() + Duration::minutes(10)).timestamp();
    let url = signed_url("/download?file=report", Some(deadline)).expect("sign");
    let verdict = || signature_verdict(&Request::for_test("GET", &url)).expect("verdict");

    assert_eq!(verdict(), SignatureVerdict::Valid);

    clock.advance(Duration::minutes(9));
    assert_eq!(verdict(), SignatureVerdict::Valid);

    clock.advance(Duration::minutes(2));
    assert_eq!(verdict(), SignatureVerdict::Expired);
}

// ---- A session that idles out -------------------------------------------

#[tokio::test]
async fn a_session_idles_out_when_the_clock_passes_its_lifetime() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE sessions (\
            id TEXT PRIMARY KEY, \
            user_id TEXT NULL, \
            payload TEXT NOT NULL, \
            csrf_token TEXT NOT NULL, \
            last_activity TEXT NOT NULL\
         )",
    )
    .await
    .expect("sessions table");

    let clock = TestClock::travel_to(at(1_900_000_000));
    let driver = DatabaseSessionDriver::new(std::time::Duration::from_secs(3600));
    driver
        .write(&SessionData::new("idle-sess".into(), "csrf".into()))
        .await
        .expect("write");

    clock.advance(Duration::minutes(59));
    assert!(
        driver.read("idle-sess").await.expect("read").is_some(),
        "inside the lifetime the session reads"
    );

    clock.advance(Duration::minutes(2));
    assert!(
        driver.read("idle-sess").await.expect("read").is_none(),
        "past the lifetime the session is gone"
    );
}

// ---- A soft-deleted model that becomes prunable -------------------------

#[model(table = "clk_accounts", soft_deletes, fillable = ["name"])]
pub struct ClkAccount {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[suprnova::prunable]
#[async_trait]
impl Prunable for ClkAccount {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().only_trashed().filter_op(
            "deleted_at",
            "<",
            (suprnova::clock::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}

#[tokio::test]
async fn a_soft_deleted_model_becomes_prunable_when_the_clock_passes_the_window() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE clk_accounts (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT, \
            deleted_at TEXT\
         )",
    )
    .await
    .expect("accounts table");

    // The delete runs under the test clock, so the stamp the model macro
    // writes is the travelled time, not the wall clock.
    let deleted_on = at(1_900_000_000);
    let clock = TestClock::travel_to(deleted_on);
    let account = ClkAccount::create(attrs! { name: "closed" })
        .await
        .expect("create");
    account.delete().await.expect("soft delete");
    let stamped = ClkAccount::with_trashed()
        .filter("name", "closed")
        .first()
        .await
        .expect("read")
        .expect("the trashed row")
        .deleted_at;
    assert_eq!(stamped, Some(deleted_on), "the soft delete reads the clock");

    clock.set(deleted_on + Duration::days(10));
    let early = suprnova::eloquent::prune_one("ClkAccount", true)
        .await
        .expect("dry run");
    assert_eq!(early, Some(0), "ten days after the delete nothing is due");

    clock.set(deleted_on + Duration::days(31));
    let due = suprnova::eloquent::prune_one("ClkAccount", true)
        .await
        .expect("dry run");
    assert_eq!(due, Some(1), "thirty-one days after the delete it is due");

    let pruned = suprnova::eloquent::prune_one("ClkAccount", false)
        .await
        .expect("prune");
    assert_eq!(pruned, Some(1));
    assert!(
        !ClkAccount::with_trashed()
            .filter("name", "closed")
            .exists()
            .await
            .expect("exists")
    );
}

// ---- A scheduled task without a time zone -------------------------------

#[test]
fn a_schedule_without_a_time_zone_is_due_at_the_travelled_time() {
    use chrono::{Local, TimeZone};
    use suprnova::schedule::CronExpression;

    let expression = CronExpression::parse("0 3 * * *").expect("cron");
    let three = Local
        .with_ymd_and_hms(2031, 5, 6, 3, 0, 0)
        .single()
        .expect("03:00 exists once in the local time zone")
        .with_timezone(&Utc);

    let clock = TestClock::travel_to(three);
    assert!(expression.is_due(), "due at 03:00 of the clock");
    clock.set(three + Duration::minutes(1));
    assert!(!expression.is_due(), "not due a minute later");
}
