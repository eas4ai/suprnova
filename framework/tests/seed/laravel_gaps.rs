//! Nested seeder execution and the db:seed invocation boundary.

use serial_test::serial;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::{App, FrameworkError, Seeder, SeederParams, async_trait, attrs, console, seed};

static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static RETRIES: AtomicUsize = AtomicUsize::new(0);

struct Users;
#[async_trait]
impl Seeder for Users {
    fn name() -> &'static str {
        "UserSeeder"
    }
    async fn run() -> Result<(), FrameworkError> {
        ORDER.lock().unwrap().push("users");
        Ok(())
    }
}
struct Posts;
#[async_trait]
impl Seeder for Posts {
    fn name() -> &'static str {
        "PostSeeder"
    }
    async fn run() -> Result<(), FrameworkError> {
        ORDER.lock().unwrap().push("posts");
        Ok(())
    }
}
struct Service(String);
struct Parameterized;
#[async_trait]
impl Seeder for Parameterized {
    fn name() -> &'static str {
        "Parameterized"
    }
    async fn run() -> Result<(), FrameworkError> {
        Err(FrameworkError::bad_request("parameters required"))
    }
    async fn run_with(params: SeederParams) -> Result<(), FrameworkError> {
        let service = App::make::<Service>()
            .ok_or_else(|| FrameworkError::internal("missing seed service"))?;
        if params.get("prefix").and_then(|value| value.as_str()) != Some(service.0.as_str()) {
            return Err(FrameworkError::bad_request("wrong prefix parameter"));
        }
        ORDER.lock().unwrap().push("parameters");
        Ok(())
    }
}
struct Root;
#[async_trait]
impl Seeder for Root {
    fn name() -> &'static str {
        "Root"
    }
    async fn run() -> Result<(), FrameworkError> {
        ORDER.lock().unwrap().push("root");
        seed::call(["UserSeeder", "PostSeeder"]).await?;
        seed::call_once(["UserSeeder", "PostSeeder", "UserSeeder"]).await?;
        seed::call_with("Parameterized", attrs! { prefix: "seed" }).await
    }
}
struct QuietRoot;
#[async_trait]
impl Seeder for QuietRoot {
    fn name() -> &'static str {
        "QuietRoot"
    }
    async fn run() -> Result<(), FrameworkError> {
        seed::call_silent(["PostSeeder", "UserSeeder"]).await
    }
}
struct OnceRoot;
#[async_trait]
impl Seeder for OnceRoot {
    fn name() -> &'static str {
        "OnceRoot"
    }
    async fn run() -> Result<(), FrameworkError> {
        seed::call_once(["UserSeeder", "UserSeeder", "PostSeeder"]).await?;
        seed::call_once(["PostSeeder"]).await
    }
}
struct Failing;
#[async_trait]
impl Seeder for Failing {
    fn name() -> &'static str {
        "Failing"
    }
    async fn run() -> Result<(), FrameworkError> {
        ORDER.lock().unwrap().push("failing");
        Err(FrameworkError::internal("seed failure"))
    }
}
struct FailureRoot;
#[async_trait]
impl Seeder for FailureRoot {
    fn name() -> &'static str {
        "FailureRoot"
    }
    async fn run() -> Result<(), FrameworkError> {
        seed::call(["UserSeeder", "Failing", "PostSeeder"]).await
    }
}
struct Retry;
#[async_trait]
impl Seeder for Retry {
    fn name() -> &'static str {
        "Retry"
    }
    async fn run() -> Result<(), FrameworkError> {
        if RETRIES.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(FrameworkError::internal("try again"))
        } else {
            ORDER.lock().unwrap().push("retry");
            Ok(())
        }
    }
}
struct RetryRoot;
#[async_trait]
impl Seeder for RetryRoot {
    fn name() -> &'static str {
        "RetryRoot"
    }
    async fn run() -> Result<(), FrameworkError> {
        assert!(seed::call_once(["Retry"]).await.is_err());
        seed::call_once(["Retry", "Retry"]).await
    }
}
struct Recursive;
#[async_trait]
impl Seeder for Recursive {
    fn name() -> &'static str {
        "Recursive"
    }
    async fn run() -> Result<(), FrameworkError> {
        seed::call(["Recursive"]).await
    }
}
struct Writer;
#[async_trait]
impl Seeder for Writer {
    fn name() -> &'static str {
        "Writer"
    }
    async fn run() -> Result<(), FrameworkError> {
        suprnova::DB::unprepared("INSERT INTO seed_rows (name) VALUES ('seed')").await?;
        Ok(())
    }
}

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        seed::clear();
    }
}
fn setup() -> Reset {
    seed::clear();
    ORDER.lock().unwrap().clear();
    RETRIES.store(0, Ordering::SeqCst);
    seed::register::<Posts>();
    seed::register::<Users>();
    seed::register::<Parameterized>();
    seed::register::<Failing>();
    Reset
}
fn order() -> Vec<&'static str> {
    ORDER.lock().unwrap().clone()
}
fn assert_progress(output: &str, names: &[&str]) {
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), names.len() * 2, "{output}");
    for (pair, name) in lines.as_chunks::<2>().0.iter().zip(names) {
        assert_eq!(pair[0], format!("RUNNING {name}"));
        let duration = pair[1]
            .strip_prefix(&format!("DONE {name} ("))
            .unwrap()
            .strip_suffix(" ms)")
            .unwrap();
        assert!(duration.parse::<u128>().is_ok(), "{output}");
    }
}

#[tokio::test]
#[serial]
async fn root_calls_in_order_with_parameters_services_and_progress() {
    let _reset = setup();
    let _container = TestContainer::fake();
    TestContainer::bind(std::sync::Arc::new(Service("seed".into())));
    seed::register_root::<Root>().unwrap();
    let run = console::test(["db:seed"]).run().await;
    run.assert_successful();
    assert_eq!(order(), vec!["root", "users", "posts", "parameters"]);
    assert_progress(run.output(), &["UserSeeder", "PostSeeder", "Parameterized"]);
    let run = console::test(["db:seed"]).run().await;
    run.assert_successful();
    assert_eq!(
        order(),
        vec![
            "root",
            "users",
            "posts",
            "parameters",
            "root",
            "users",
            "posts",
            "parameters"
        ]
    );
}

#[tokio::test]
#[serial]
async fn silent_calls_keep_console_output_empty() {
    let _reset = setup();
    seed::register_root::<QuietRoot>().unwrap();
    let run = console::test(["db:seed"]).run().await;
    run.assert_successful();
    assert_eq!(run.output(), "");
    assert_eq!(order(), vec!["posts", "users"]);
    seed::call_silent(Vec::<&str>::new()).await.unwrap();
}

#[tokio::test]
#[serial]
async fn once_skips_duplicates_and_resets_between_invocations() {
    let _reset = setup();
    seed::register_root::<OnceRoot>().unwrap();
    for _ in 0..2 {
        let run = console::test(["db:seed"]).run().await;
        run.assert_successful();
        assert_progress(run.output(), &["UserSeeder", "PostSeeder"]);
    }
    assert_eq!(order(), vec!["users", "posts", "users", "posts"]);
}

#[tokio::test]
#[serial]
async fn failed_once_can_retry_and_recursive_calls_are_errors() {
    let _reset = setup();
    seed::register::<Retry>();
    seed::register_root::<RetryRoot>().unwrap();
    let run = console::test(["db:seed"]).run().await;
    run.assert_successful();
    assert_eq!(RETRIES.load(Ordering::SeqCst), 2);
    assert_eq!(order(), vec!["retry"]);
    assert_eq!(run.output().matches("RUNNING Retry\n").count(), 2);
    assert_eq!(run.output().matches("DONE Retry (").count(), 1);
    seed::register_root::<Recursive>().unwrap();
    let run = console::test(["db:seed"]).run().await;
    run.assert_failed()
        .assert_errors_contain("recursive seeder call");
}

#[tokio::test]
#[serial]
async fn failure_stops_the_list_without_a_done_line_or_later_calls() {
    let _reset = setup();
    seed::register_root::<FailureRoot>().unwrap();
    let run = console::test(["db:seed"]).run().await;
    run.assert_failed().assert_errors_contain("seed failure");
    assert_eq!(order(), vec!["users", "failing"]);
    assert!(run.output().contains("RUNNING Failing"));
    assert!(!run.output().contains("DONE Failing"));
    assert!(!run.output().contains("PostSeeder"));
    let run = console::test(["db:seed", "--class=Unknown"]).run().await;
    run.assert_failed()
        .assert_errors_contain("no seeder registered");
    assert_eq!(run.output(), "");
    let _container = TestContainer::fake();
    TestContainer::bind(std::sync::Arc::new(Service("seed".into())));
    let error = seed::call_with("Parameterized", attrs! {})
        .await
        .unwrap_err();
    assert!(error.to_string().contains("wrong prefix parameter"));
    let error = seed::call_with("Parameterized", attrs! { prefix: 42 })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("wrong prefix parameter"));
}

#[tokio::test]
#[serial]
async fn database_option_routes_only_this_invocation_and_restores_after_failure() {
    let _reset = setup();
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared("CREATE TABLE seed_rows (name TEXT)")
        .await
        .unwrap();
    let reporting = suprnova::DbConnection::connect(
        &suprnova::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .build(),
    )
    .await
    .unwrap();
    use sea_orm::ConnectionTrait;
    reporting
        .inner()
        .execute_unprepared("CREATE TABLE seed_rows (name TEXT)")
        .await
        .unwrap();
    suprnova::ConnectionRegistry::register_existing("reporting", reporting.clone())
        .await
        .unwrap();
    seed::register_root::<Writer>().unwrap();
    for args in [
        vec!["db:seed", "--database", "reporting"],
        vec!["db:seed", "--class=Writer", "--database=reporting"],
    ] {
        console::test(args).run().await.assert_successful();
    }
    assert_eq!(
        db.fetch_one("SELECT COUNT(*) AS n FROM seed_rows", vec![])
            .await
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap(),
        0
    );
    let row = reporting
        .inner()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) AS n FROM seed_rows",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
    assert_eq!(
        suprnova::DB::default_connection(),
        suprnova::PRIMARY_CONNECTION_NAME
    );
    console::test(["db:seed", "--database=reporting", "--class=Failing"])
        .run()
        .await
        .assert_failed();
    assert_eq!(
        suprnova::DB::default_connection(),
        suprnova::PRIMARY_CONNECTION_NAME
    );
    console::test(["db:seed", "--database=missing"])
        .run()
        .await
        .assert_failed()
        .assert_errors_contain("not registered");
    console::test(["db:seed"]).run().await.assert_successful();
    assert_eq!(
        db.fetch_one("SELECT COUNT(*) AS n FROM seed_rows", vec![])
            .await
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap(),
        1
    );
}

#[tokio::test]
#[serial]
async fn malformed_options_do_not_run_a_seeder() {
    let _reset = setup();
    seed::register_root::<Users>().unwrap();
    for args in [
        vec!["--database"],
        vec!["--database="],
        vec!["--database", "--force"],
        vec!["--class"],
        vec!["--class="],
        vec!["--typo"],
        vec!["--class=UserSeeder", "PostSeeder"],
    ] {
        console::test(std::iter::once("db:seed").chain(args))
            .run()
            .await
            .assert_failed();
    }
    assert!(order().is_empty());
}

#[test]
fn production_without_force_refuses_and_force_runs_under_app_env() {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "laravel_gaps::production_child",
            "--ignored",
            "--nocapture",
        ])
        .env("APP_ENV", "production")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
#[ignore = "driven by the production environment test in a separate process"]
async fn production_child() {
    assert_eq!(std::env::var("APP_ENV").unwrap(), "production");
    let _reset = setup();
    seed::register_root::<Users>().unwrap();
    console::test(["db:seed"])
        .run()
        .await
        .assert_failed()
        .assert_errors_contain("--force");
    assert!(order().is_empty());
    console::test(["db:seed", "--force"])
        .run()
        .await
        .assert_successful();
    assert_eq!(order(), vec!["users"]);
    console::test(["db:seed", "--class=UserSeeder", "--force"])
        .run()
        .await
        .assert_successful();
    assert_eq!(order(), vec!["users", "users"]);
}
