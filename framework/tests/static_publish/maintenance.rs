//! Maintenance must stop when another process owns the publication lease.
use super::*;

async fn wait_count(counter: &AtomicUsize, event: &tokio::sync::Notify, count: usize) {
    while counter.load(Ordering::SeqCst) < count {
        event.notified().await;
    }
}

#[tokio::test(start_paused = true)]
async fn pub_003_round4_2_non_cas_takers_settle_before_one_can_render() {
    let raw = memory();
    raw.write(
        "_pub/lease.json",
        br#"{"owner":"crashed","expires_at_ms":0}"#.to_vec(),
    )
    .await
    .unwrap();
    let probe = Arc::new(Probe::default());
    let race = Arc::new(LeaseRace::default());
    *probe.lease_race.lock().unwrap() = Some(race.clone());
    let disk = raw.clone().layer(ProbeLayer(probe));
    let render_release = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for i in 0..2 {
        let publisher = Publisher::from_operator(
            disk.clone(),
            "v1",
            PublishConfig {
                clock_skew: Duration::from_millis(100),
                timeout: Duration::from_secs(2),
                ..Default::default()
            },
        )
        .unwrap();
        let render_release = render_release.clone();
        let started = started.clone();
        tasks.push(tokio::spawn(async move {
            publisher
                .publish_with(|| async {
                    started.fetch_add(1, Ordering::SeqCst);
                    let _permit = render_release.acquire().await.unwrap();
                    Err::<Vec<PageObject>, _>(FrameworkError::internal(
                        "render observed acquired lease",
                    ))
                })
                .await
        }));
        wait_count(&race.entered, &race.entered_event, i + 1).await;
    }
    race.gates[0].notify_one();
    wait_count(&race.created, &race.created_event, 1).await;
    // Let the first holder verify and enter render in the unfixed implementation.
    tokio::time::sleep(Duration::from_millis(1)).await;
    race.gates[1].notify_one();
    wait_count(&race.created, &race.created_event, 2).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    render_release.add_permits(2);
    let mut failures = Vec::new();
    for task in tasks {
        failures.push(task.await.unwrap().unwrap_err().to_string());
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "only the taker that survives the settle verification can render"
    );
    assert_eq!(
        failures
            .iter()
            .filter(|failure| failure.contains("held"))
            .count(),
        1,
        "{failures:?}"
    );
    assert_eq!(
        failures
            .iter()
            .filter(|failure| failure.contains("render observed"))
            .count(),
        1
    );
}

#[tokio::test]
async fn pub_008_round4_2_prune_stops_before_deletion_after_lease_replacement() {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    let disk = raw.clone().layer(ProbeLayer(probe.clone()));
    let p = Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap();
    let old = p.publish(vec![page("one")]).await.unwrap().version;
    p.publish(vec![page("two")]).await.unwrap();
    let mut manifest = p.manifest().await.unwrap();
    manifest.versions.get_mut(&old).unwrap().retired_at_ms = Some(0);
    raw.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let before = probe.deleted.lock().unwrap().len();
    *probe.replace_lease_on_list.lock().unwrap() = Some(raw.clone());
    let result = p.prune().await;
    assert!(result.is_err(), "a displaced prune must refuse: {result:?}");
    assert!(result.unwrap_err().to_string().contains("replacement"));
    assert_eq!(
        probe.deleted.lock().unwrap().len(),
        before,
        "no deletion may follow replacement"
    );
    assert_eq!(p.manifest().await.unwrap(), manifest);
    for object in manifest
        .versions
        .values()
        .flat_map(|version| &version.objects)
    {
        assert!(raw.exists(&object.path).await.unwrap());
    }
}
