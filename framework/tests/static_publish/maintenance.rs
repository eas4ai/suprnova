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

fn asset_page(text: &str) -> PageObject {
    let mut page = page(text);
    page.html = Bytes::from(format!(
        "<p>{text}</p><link rel=\"stylesheet\" href=\"/theme.css\">"
    ));
    page.assets.insert(
        "theme.css".into(),
        (
            Bytes::from(format!("body{{background:url(./bg.png)}}/*{text}*/")),
            "text/css".into(),
        ),
    );
    page.assets.insert(
        "bg.png".into(),
        (Bytes::from_static(b"pixels"), "image/png".into()),
    );
    page
}
#[tokio::test]
async fn pub_008_round4_3_failed_deletion_drops_references_before_set_reuse_and_orphan_sweep() {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    let disk = raw.clone().layer(ProbeLayer(probe.clone()));
    let p = Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap();
    let mut sets = Vec::new();
    for text in ["a", "b"] {
        let rendered = asset_page(text);
        let id = p.publish(vec![rendered.clone()]).await.unwrap().version;
        sets.push((id, rendered));
    }
    let live = p.publish(vec![asset_page("c")]).await.unwrap().version;
    let mut manifest = p.manifest().await.unwrap();
    sets.sort_by_key(|(id, _)| manifest.versions[id].asset_set_digest.clone());
    let root = |id: &str| {
        format!(
            "_pub/v1/_assets/{}/",
            manifest.versions[id].asset_set_digest
        )
    };
    let reused_root = root(&sets[0].0);
    let orphan_root = root(&sets[1].0);
    for (id, _) in &sets {
        manifest.versions.get_mut(id).unwrap().retired_at_ms = Some(0);
    }
    raw.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    *probe.refuse_delete.lock().unwrap() = Some(format!("{reused_root}theme.css"));
    *probe.observe_delete_manifest.lock().unwrap() = Some(raw.clone());
    assert!(
        p.prune()
            .await
            .unwrap_err()
            .to_string()
            .contains("injected object deletion")
    );
    let pruned = p.manifest().await.unwrap();
    assert_eq!(
        pruned.versions.len(),
        1,
        "expired records must be committed before deletion can fail"
    );
    assert_eq!(pruned.live, live);
    for bytes in probe.deletion_manifests.lock().unwrap().iter() {
        let observed: suprnova::static_publish::Manifest = serde_json::from_slice(bytes).unwrap();
        assert_eq!(
            observed, pruned,
            "the first deletion must already observe the pruned manifest"
        );
    }
    assert!(!raw.exists(&format!("{reused_root}bg.png")).await.unwrap());
    assert!(
        raw.exists(&format!("{reused_root}theme.css"))
            .await
            .unwrap()
    );
    assert!(raw.exists(&format!("{orphan_root}bg.png")).await.unwrap());
    *probe.observe_delete_manifest.lock().unwrap() = None;
    let writes = probe
        .writes
        .lock()
        .unwrap()
        .iter()
        .filter(|key| key.starts_with(&reused_root))
        .count();
    let outcome = p.publish(vec![sets[0].1.clone()]).await.unwrap();
    assert_eq!(
        probe
            .writes
            .lock()
            .unwrap()
            .iter()
            .filter(|key| key.starts_with(&reused_root))
            .count()
            - writes,
        2,
        "an unreferenced set must write every member again"
    );
    for key in ["theme.css", "bg.png"] {
        assert!(raw.exists(&format!("{reused_root}{key}")).await.unwrap());
    }
    assert!(
        outcome
            .prune
            .removed
            .contains(&format!("{orphan_root}bg.png")),
        "the following retention pass sweeps orphaned set directories"
    );
    assert!(
        raw.list_with(&orphan_root)
            .recursive(true)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        outcome
            .prune
            .removed
            .iter()
            .all(|key| !key.starts_with(&reused_root))
    );
}

#[tokio::test]
async fn pub_008_round4_3_refused_prune_manifest_leaves_all_objects_present() {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    let p = Publisher::from_operator(
        raw.clone().layer(ProbeLayer(probe.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap();
    let old = p.publish(vec![asset_page("old")]).await.unwrap().version;
    p.publish(vec![asset_page("live")]).await.unwrap();
    let mut manifest = p.manifest().await.unwrap();
    manifest.versions.get_mut(&old).unwrap().retired_at_ms = Some(0);
    raw.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let before = probe.deleted.lock().unwrap().len();
    *probe.refuse.lock().unwrap() = Some("_pub/manifest.json".into());
    assert!(p.prune().await.is_err());
    assert_eq!(
        probe.deleted.lock().unwrap().len() - before,
        1,
        "only the lease release may delete after a failed prune commit"
    );
    assert_eq!(p.manifest().await.unwrap(), manifest);
    for object in manifest
        .versions
        .values()
        .flat_map(|version| &version.objects)
    {
        assert!(
            raw.exists(&object.path).await.unwrap(),
            "manifest must not reference deleted {}",
            object.path
        );
    }
}

async fn stalled_status_releases_lease(render_failure: bool) {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    probe.stall_status.store(true, Ordering::SeqCst);
    probe.delay_lease_release.store(true, Ordering::SeqCst);
    let p = Publisher::from_operator(
        raw.clone().layer(ProbeLayer(probe)),
        "v1",
        PublishConfig {
            timeout: Duration::from_millis(400),
            clock_skew: Duration::ZERO,
            ..Default::default()
        },
    )
    .unwrap();
    let result = p
        .publish_with(|| async {
            if render_failure {
                Err(FrameworkError::internal("intentional render failure"))
            } else {
                Ok(vec![page("success")])
            }
        })
        .await;
    assert_eq!(result.is_err(), render_failure);
    assert!(
        !raw.exists("_pub/lease.json").await.unwrap(),
        "slow status persistence must not exhaust the lease release budget"
    );
    assert_eq!(
        p.status().await.outcome,
        if render_failure { "failed" } else { "success" }
    );
}
#[tokio::test(start_paused = true)]
async fn pub_003_round4_4_success_releases_lease_before_stalled_status() {
    stalled_status_releases_lease(false).await;
}
#[tokio::test(start_paused = true)]
async fn pub_003_round4_4_render_failure_releases_lease_before_stalled_status() {
    stalled_status_releases_lease(true).await;
}
