//! Schedule-wide elections and interruption between tasks.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::testing::{TestContainer, TestContainerGuard};
use suprnova::{CacheStore, FrameworkError, InMemoryCache, Schedule};

fn cache() -> TestContainerGuard {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    guard
}

#[tokio::test]
async fn schedule_wide_election_preserves_every_server_opt_out() {
    let _clock = suprnova::clock::TestClock::freeze();
    let _cache = cache();
    let elected = Arc::new(AtomicUsize::new(0));
    let every = Arc::new(AtomicUsize::new(0));
    let names = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    for background in [false, true] {
        let mut schedule = Schedule::new();
        let count = elected.clone();
        let mut task = schedule
            .call(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .name(&names[0]);
        if background {
            task = task.run_in_background();
        }
        schedule.add(task);
        let count = every.clone();
        schedule.add(
            schedule
                .call(move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .name(&names[1])
                .on_one_server()
                .on_every_server(),
        );
        schedule.always_on_one_server();
        for (_, result) in schedule.run_due_tasks().await {
            result.expect("run");
        }
    }
    assert_eq!(elected.load(Ordering::SeqCst), 1);
    assert_eq!(every.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tasks_registered_after_the_default_inherit_it_and_last_override_wins() {
    let mut schedule = Schedule::new();
    schedule.always_on_one_server();
    schedule.add(schedule.call(|| async { Ok(()) }).name("inherits"));
    schedule.add(
        schedule
            .call(|| async { Ok(()) })
            .name("opts-out")
            .on_every_server(),
    );
    schedule.add(
        schedule
            .call(|| async { Ok(()) })
            .name("opts-back-in")
            .on_every_server()
            .on_one_server(),
    );
    assert!(schedule.tasks()[0].on_one_server);
    assert!(!schedule.tasks()[1].on_one_server);
    assert!(schedule.tasks()[2].on_one_server);
}

#[tokio::test]
async fn interrupt_after_first_task_stops_second_and_next_run_clears_mark() {
    let _cache = cache();
    let start = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(
        !Schedule::has_been_interrupted_since(start)
            .await
            .expect("no mark")
    );
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .call(|| async { Schedule::interrupt().await })
            .name("interrupt"),
    );
    schedule.add(
        schedule
            .call(|| async { panic!("second task must not start") })
            .name("second"),
    );
    let results = schedule.run_due_tasks().await;
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());
    assert!(
        Schedule::has_been_interrupted_since(start)
            .await
            .expect("mark")
    );
    assert!(
        !Schedule::has_been_interrupted_since(chrono::Utc::now() + chrono::Duration::seconds(1))
            .await
            .expect("older mark")
    );
    let mut next = Schedule::new();
    next.add(next.call(|| async { Ok(()) }).name("fresh"));
    assert_eq!(next.run_due_tasks().await.len(), 1);
    assert!(
        !Schedule::has_been_interrupted_since(start)
            .await
            .expect("cleared")
    );
}

/// A cache whose I/O fails, so interruption cannot quietly report success.
struct BrokenCache {
    allow_forget: bool,
}

#[suprnova::async_trait]
impl CacheStore for BrokenCache {
    async fn get_raw(&self, _: &str) -> Result<Option<String>, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn put_raw(&self, _: &str, _: &str, _: Option<Duration>) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn has(&self, _: &str) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn forget(&self, _: &str) -> Result<bool, FrameworkError> {
        if self.allow_forget {
            Ok(false)
        } else {
            Err(FrameworkError::internal("cache unavailable"))
        }
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn increment(&self, _: &str, _: i64) -> Result<i64, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn decrement(&self, _: &str, _: i64) -> Result<i64, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn tagged_put_raw(
        &self,
        _: &[&str],
        _: &str,
        _: &str,
        _: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn flush_tags(&self, _: &[&str]) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn acquire_lock(&self, _: &str, _: Duration) -> Result<Option<String>, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn release_lock(&self, _: &str, _: &str) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn refresh_lock(&self, _: &str, _: &str, _: Duration) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
    async fn touch(&self, _: &str, _: Duration) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal("cache unavailable"))
    }
}

#[tokio::test]
async fn cache_failure_is_returned_and_run_fails_closed() {
    let _scope = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(BrokenCache {
        allow_forget: false,
    }));
    assert!(Schedule::interrupt().await.is_err());
    assert!(
        Schedule::has_been_interrupted_since(chrono::Utc::now())
            .await
            .is_err()
    );
    let mut schedule = Schedule::new();
    schedule.add(schedule.call(|| async { panic!("cache failure must stop the run") }));
    let results = schedule.run_due_tasks().await;
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_err());
    TestContainer::bind::<dyn CacheStore>(Arc::new(BrokenCache { allow_forget: true }));
    let mut schedule = Schedule::new();
    schedule.add(schedule.call(|| async { Ok(()) }).name("first"));
    schedule.add(
        schedule
            .call(|| async { panic!("cache read failure must stop the run") })
            .name("second"),
    );
    let results = schedule.run_due_tasks().await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "first");
    assert!(results[0].1.is_ok());
    assert_eq!(results[1].0, "<schedule>");
    assert!(results[1].1.is_err());
}

#[test]
fn interrupt_command_writes_the_mark() {
    const CHILD: &str = "SUPRNOVA_D1_INTERRUPT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "laravel_delta::interrupt_command_writes_the_mark",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("APP_ENV", "testing")
            .output()
            .expect("child");
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    suprnova::boot::load_env().expect("configuration before runtime");
    let _scope = cache();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let start = chrono::Utc::now() - chrono::Duration::seconds(1);
            suprnova::Application::new()
                .run_with_args(["app", "schedule:interrupt"])
                .await
                .expect("interrupt command");
            assert!(
                Schedule::has_been_interrupted_since(start)
                    .await
                    .expect("cache mark")
            );
        });
}
