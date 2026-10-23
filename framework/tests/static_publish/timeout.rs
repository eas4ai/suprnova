use bytes::Bytes;
use std::time::Duration;
use suprnova::{
    FrameworkError,
    static_publish::{PageObject, PublishConfig, Publisher},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pub_003_serialization_timeout_is_logged_and_persisted_for_status() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_003_serialization_timeout_is_logged_and_persisted_for_status",
    )
    .await
    {
        return;
    }
    use tracing::instrument::WithSubscriber as _;
    let _container = suprnova::testing::TestContainer::fake();
    let disk = opendal::Operator::new(opendal::services::Memory::default()).unwrap();
    let page = |text: &str| {
        PageObject::new("/about", Bytes::from(format!("<p>{text}</p>")), Bytes::from(serde_json::json!({"component":"About","props":{"text":text},"url":"/about","version":"v1"}).to_string())).unwrap()
    };
    let writer = Publisher::from_operator(
        disk.clone(),
        "v1",
        PublishConfig {
            timeout: Duration::from_millis(400),
            ..PublishConfig::default()
        },
    )
    .unwrap();
    writer.publish(vec![page("previous")]).await.unwrap();
    let before = disk.read("_pub/manifest.json").await.unwrap().to_bytes();
    let runtime = suprnova::static_publish::StaticPublication::install(
        &suprnova::Router::new(),
        writer.clone(),
    )
    .unwrap();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, parked) = std::sync::mpsc::channel();
    let first_writer = writer.clone();
    let first = tokio::spawn(async move {
        first_writer
            .publish_with(|| async move {
                let _ = entered.send(());
                // A synchronous renderer cannot be preempted by the forward timer.
                tokio::task::block_in_place(|| parked.recv_timeout(Duration::from_secs(5)))
                    .unwrap();
                Err(FrameworkError::internal("parked render released"))
            })
            .await
    });
    started.await.unwrap();
    let logs = tempfile::NamedTempFile::new().unwrap();
    let log_writer = logs.reopen().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || log_writer.try_clone().unwrap())
        .finish();
    let started = tokio::time::Instant::now();
    let result = writer
        .publish(vec![page("second")])
        .with_subscriber(subscriber)
        .await;
    let elapsed = started.elapsed();
    let status = runtime.status().await;
    let fresh = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default())
        .unwrap()
        .status()
        .await;
    let command = suprnova::console::test(["static:status"]).run().await;
    let log_text = std::fs::read_to_string(logs.path()).unwrap();
    let unchanged = disk.read("_pub/manifest.json").await.unwrap().to_bytes() == before;
    release.send(()).unwrap();
    assert!(first.await.unwrap().is_err());
    let cause = result.unwrap_err().to_string();
    assert!(
        cause.contains("publication serialization timed out"),
        "{cause}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "cleanup exceeded its budget: {elapsed:?}"
    );
    assert!(unchanged);
    assert_eq!(status.outcome, "failed");
    assert!(
        status
            .last_outcome
            .contains("publication serialization timed out")
    );
    assert_eq!(fresh.outcome, "failed");
    assert!(
        fresh
            .last_outcome
            .contains("publication serialization timed out")
    );
    command.assert_successful();
    assert!(
        command.output().contains("failed")
            && command
                .output()
                .contains("publication serialization timed out"),
        "{}",
        command.output()
    );
    assert!(
        log_text
            .lines()
            .any(|line| line.contains("static publication failed")
                && line.contains("publication serialization timed out")),
        "{log_text}"
    );
}
