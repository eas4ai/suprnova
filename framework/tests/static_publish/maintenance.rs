//! Maintenance must stop when another process owns the publication lease.
use super::*;

async fn wait_count(counter: &AtomicUsize, event: &tokio::sync::Notify, count: usize) {
    fixture_wait("acquisition progress", async {
        while counter.load(Ordering::SeqCst) < count {
            event.notified().await;
        }
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn pub_003_round5_1_delayed_taker_cannot_displace_a_settled_holder() {
    let raw = memory();
    raw.write(
        &lease_key(1),
        br#"{"owner":"crashed","epoch":1,"expires_at_ms":0}"#.to_vec(),
    )
    .await
    .unwrap();
    let probe = Arc::new(Probe::default());
    let race = Arc::new(LeaseRace::default());
    *probe.lease_race.lock().unwrap() = Some(race.clone());
    let disk = raw.clone().layer(ProbeLayer(probe.clone()));
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
                    let _permit = fixture_wait("holder render release", render_release.acquire())
                        .await
                        .unwrap();
                    Err::<Vec<PageObject>, _>(FrameworkError::internal(
                        "render observed acquired lease",
                    ))
                })
                .await
        }));
        wait_count(&race.entered, &race.entered_event, i + 1).await;
        assert_eq!(
            *race.conditional.lock().unwrap(),
            vec![true; i + 1],
            "lease acquisition must request exclusive creation"
        );
    }
    // B acquires while A's create remains stalled beyond the old settle window.
    race.gates[1].notify_one();
    wait_count(&race.created, &race.created_event, 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(started.load(Ordering::SeqCst), 1);
    let winner = highest_lease(&raw).await;
    assert_eq!(winner["epoch"], 2);
    race.gates[0].notify_one();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let acquisition_deletes = probe.deleted.lock().unwrap().len();
    render_release.add_permits(2);
    let mut failures = Vec::new();
    for task in tasks {
        failures.push(
            fixture_wait("lease contender completion", task)
                .await
                .unwrap()
                .unwrap_err()
                .to_string(),
        );
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "the delayed taker must not acquire after another holder settles"
    );
    assert_eq!(
        acquisition_deletes, 0,
        "neither acquisition deletes a lease"
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
async fn pub_008_round5_2_prune_stops_when_a_higher_epoch_appears() {
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
        0,
        "a failed prune commit permits no deletion; release overwrites its own epoch"
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
    assert_eq!(
        highest_lease(&raw).await["expires_at_ms"],
        0,
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

#[tokio::test]
async fn pub_003_round5_2_crashed_epoch_advances_and_fences_the_stale_holder() {
    let raw = memory();
    raw.write(
        &lease_key(5),
        serde_json::json!({"owner":"crashed", "epoch":5, "expires_at_ms":0}).to_string(),
    )
    .await
    .unwrap();
    let probe = Arc::new(Probe::default());
    let disk = raw.clone().layer(ProbeLayer(probe.clone()));
    let holder = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
    let next = Publisher::from_operator(disk, "v1", PublishConfig::default()).unwrap();
    let mut committed = None;
    let failure = holder
        .publish_with(|| async {
            let record = highest_lease(&raw).await;
            assert_eq!(record["epoch"], 6);
            let at = chrono::DateTime::from_timestamp_millis(
                record["expires_at_ms"].as_i64().unwrap() + 10000,
            )
            .unwrap();
            let outcome = suprnova::testing::TestClock::scope(at, |_| async {
                next.publish(vec![page("successor")]).await
            })
            .await
            .unwrap();
            assert_eq!(highest_lease(&raw).await["epoch"], 7);
            committed = Some(outcome.version);
            Ok(vec![page("stale holder")])
        })
        .await
        .unwrap_err();
    assert!(failure.to_string().contains("epoch 7"), "{failure}");
    assert_eq!(holder.manifest().await.unwrap().live, committed.unwrap());
    let current = highest_lease(&raw).await;
    assert_eq!(current["epoch"], 7);
    assert_eq!(current["expires_at_ms"], 0);
    assert!(
        probe
            .deleted
            .lock()
            .unwrap()
            .iter()
            .all(|path| path != &lease_key(7))
    );
}

#[tokio::test]
async fn pub_008_round5_3_release_preserves_epochs_and_prune_keeps_the_highest() {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    let p = Publisher::from_operator(
        raw.clone().layer(ProbeLayer(probe.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap();
    let mut acquired_owner = None;
    p.publish_with(|| async {
        let record = highest_lease(&raw).await;
        acquired_owner = Some(record["owner"].clone());
        assert_eq!(record["epoch"], 1);
        assert_ne!(record["expires_at_ms"], 0);
        Ok(vec![page("one")])
    })
    .await
    .unwrap();
    let first = highest_lease(&raw).await;
    assert_eq!(first["epoch"], 1);
    assert_eq!(first["expires_at_ms"], 0);
    assert_eq!(Some(first["owner"].clone()), acquired_owner);
    assert!(probe.deleted.lock().unwrap().is_empty());
    assert!(
        p.publish_with(|| async {
            assert_eq!(highest_lease(&raw).await["epoch"], 2);
            Err::<Vec<PageObject>, _>(FrameworkError::internal("stop before maintenance"))
        })
        .await
        .is_err()
    );
    assert!(raw.exists(&lease_key(1)).await.unwrap());
    assert_eq!(highest_lease(&raw).await["expires_at_ms"], 0);
    let report = p.prune().await.unwrap();
    assert!(report.removed.contains(&lease_key(1)));
    assert!(report.removed.contains(&lease_key(2)));
    assert!(report.kept.contains(&lease_key(3)));
    assert!(!report.removed.contains(&lease_key(3)));
    let current = highest_lease(&raw).await;
    assert_eq!(current["epoch"], 3);
    assert_eq!(current["expires_at_ms"], 0);
    assert_eq!(raw.list("_pub/lease/").await.unwrap().len(), 1);

    assert!(
        probe.conditional_writes.lock().unwrap().is_empty(),
        "memory epochs do not need conditional replacement"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn pub_003_round5_4_foreign_lease_keys_are_ignored_and_epochs_are_not_public_assets() {
    let _container = suprnova::testing::TestContainer::fake();
    let raw = memory();
    let foreign = [
        "_pub/lease/readme.json".to_string(),
        "_pub/lease/00000000000000000002.json.tmp".into(),
        "_pub/lease/999999999999999999999.json".into(),
        format!("_pub/lease/_assets/{}/foreign", "f".repeat(64)),
    ];
    for key in &foreign {
        raw.write(key, "foreign bytes").await.unwrap();
    }
    let p = Publisher::from_operator(raw.clone(), "v1", PublishConfig::default()).unwrap();
    p.publish(vec![page("public")]).await.unwrap();
    assert_eq!(highest_lease(&raw).await["epoch"], 1);
    for key in &foreign {
        assert!(raw.exists(key).await.unwrap());
    }
    let original = manifest_bytes(&raw).await;
    let mut manifest = p.manifest().await.unwrap();
    let bytes = raw.read(&lease_key(1)).await.unwrap().to_bytes();
    manifest
        .versions
        .get_mut(&manifest.live)
        .unwrap()
        .objects
        .push(suprnova::static_publish::ObjectEntry {
            path: lease_key(1),
            url: format!("/{}", lease_key(1)),
            asset: true,
            digest: "not-public".into(),
            asset_version: "v1".into(),
            dependency_tags: vec![],
            headers: std::collections::BTreeMap::from([
                ("content-type".into(), "application/json".into()),
                ("content-length".into(), bytes.len().to_string()),
            ]),
        });
    raw.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let router: suprnova::Router = suprnova::Router::new()
        .get("/_pub/lease/00000000000000000001.json", |_| async {
            Ok(suprnova::HttpResponse::text("lease fallback must not run"))
        })
        .into();
    suprnova::static_publish::StaticPublication::install(&router, p.clone()).unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    let response = client.get(format!("/{}", lease_key(1))).send().await;
    response.assert_status(404);
    assert!(!response.body_text().contains("expires_at_ms"));
    raw.write("_pub/manifest.json", original).await.unwrap();
    p.prune().await.unwrap();
    assert_eq!(highest_lease(&raw).await["epoch"], 2);
    for key in &foreign {
        assert!(
            raw.exists(key).await.unwrap(),
            "foreign lease keys are not orphan-swept"
        );
    }
}

#[tokio::test]
async fn pub_003_round5_epoch_exhaustion_returns_an_error_without_reusing_a_key() {
    let raw = memory();
    raw.write(
        &lease_key(u64::MAX),
        serde_json::json!({"owner":"last", "epoch":u64::MAX, "expires_at_ms":0}).to_string(),
    )
    .await
    .unwrap();
    let p = Publisher::from_operator(raw.clone(), "v1", PublishConfig::default()).unwrap();
    assert!(
        p.publish(vec![page("refused")])
            .await
            .unwrap_err()
            .to_string()
            .contains("epoch is exhausted")
    );
    assert_eq!(highest_lease(&raw).await["epoch"], u64::MAX);
    assert_eq!(raw.list("_pub/lease/").await.unwrap().len(), 1);
}

#[tokio::test]
async fn pub_003_round6_1_fs_atomic_staging_cannot_replace_an_acquired_epoch() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let raw = Operator::new(
        services::Fs::default()
            .root(root.path().to_str().unwrap())
            .atomic_write_dir(staging.path().to_str().unwrap()),
    )
    .unwrap();
    raw.write(
        &lease_key(1),
        br#"{"owner":"crashed","epoch":1,"expires_at_ms":0}"#.to_vec(),
    )
    .await
    .unwrap();
    let race = Arc::new(FsLeaseRace::default());
    let probe = Arc::new(Probe::default());
    *probe.fs_lease_race.lock().unwrap() = Some(race.clone());
    let disk = raw.clone().layer(ProbeLayer(probe));
    let started = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let publisher = Publisher::from_operator(
            disk.clone(),
            "v1",
            PublishConfig {
                timeout: Duration::from_secs(15),
                ..Default::default()
            },
        )
        .unwrap();
        let (race, started) = (race.clone(), started.clone());
        tasks.push(tokio::spawn(async move {
            publisher
                .publish_with(|| async {
                    started.fetch_add(1, Ordering::SeqCst);
                    race.holder_started.notify_one();
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Err::<Vec<PageObject>, _>(FrameworkError::internal(
                        "render observed acquired lease",
                    ))
                })
                .await
        }));
    }
    let mut failures = Vec::new();
    for task in tasks {
        failures.push(
            tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .expect("Fs contenders must finish")
                .unwrap()
                .unwrap_err()
                .to_string(),
        );
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "exclusive Fs create must fence the delayed rename: {failures:?}"
    );
    assert_eq!(
        failures
            .iter()
            .filter(|failure| failure.contains("held"))
            .count(),
        1,
        "{failures:?}"
    );
}

#[tokio::test]
async fn pub_003_round6_1_fs_with_an_unsafe_reserved_staging_entry_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".suprnova-atomic"), b"occupied").unwrap();
    let disk = Operator::new(
        services::Fs::default()
            .root(root.path().to_str().unwrap())
            .atomic_write_dir(staging.path().to_str().unwrap()),
    )
    .unwrap();
    let result = Publisher::from_operator(disk, "v1", PublishConfig::default());
    let failure = result
        .err()
        .expect("an Fs root that cannot support exclusive publication must refuse")
        .to_string();
    assert!(
        failure.contains("atomic_write_dir") && failure.contains("Storage::register_fs"),
        "{failure}"
    );
}

#[test]
fn pub_002_round6_2_reserved_asset_versions_refuse_both_configuration_paths() {
    for version in [
        "lease",
        "manifest.json",
        "lease.json",
        "_private",
        "_",
        "build/nested",
    ] {
        let direct = Publisher::from_operator(memory(), version, PublishConfig::default());
        let configured = Publisher::new("round6-not-registered", version, PublishConfig::default());
        for result in [direct, configured] {
            let failure = result
                .err()
                .unwrap_or_else(|| panic!("reserved version {version} must refuse"))
                .to_string();
            for expected in ["reserved", "lease", "manifest.json", "lease.json", "_", "/"] {
                assert!(failure.contains(expected), "{version}: {failure}");
            }
        }
    }
}

#[tokio::test]
async fn pub_008_round6_2_normal_asset_version_still_publishes_serves_and_prunes() {
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::Storage::register_memory("round6-normal-pages");
    let raw = suprnova::Storage::disk("round6-normal-pages").unwrap();
    let p = Publisher::new(
        "round6-normal-pages",
        "build-20261022",
        PublishConfig::default(),
    )
    .unwrap();
    let old = p.publish(vec![asset_page("old")]).await.unwrap().version;
    p.publish(vec![asset_page("live")]).await.unwrap();
    let mut manifest = p.manifest().await.unwrap();
    let live = manifest.live.clone();
    let asset = manifest.versions[&live]
        .objects
        .iter()
        .find(|entry| entry.asset && entry.path.ends_with("bg.png"))
        .unwrap()
        .clone();
    let router = suprnova::Router::new();
    suprnova::static_publish::StaticPublication::install(&router, p.clone()).unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    client
        .get(format!("/{}", asset.path))
        .send()
        .await
        .assert_ok()
        .assert_see("pixels");
    manifest.versions.get_mut(&old).unwrap().retired_at_ms = Some(0);
    raw.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let removed = p.prune().await.unwrap();
    assert!(
        removed
            .removed
            .iter()
            .any(|path| path.starts_with(&format!("_pub/{old}/")))
    );
    assert!(!removed.removed.contains(&asset.path));
    assert!(raw.exists(&asset.path).await.unwrap());
    client
        .get(format!("/{}", asset.path))
        .send()
        .await
        .assert_ok()
        .assert_see("pixels");
    let retained = p.manifest().await.unwrap();
    assert_eq!(retained.live, live);
    assert_eq!(retained.versions.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn pub_003_round6_3_fixture_observes_unconditional_acquisition_attempts() {
    let race = Arc::new(LeaseRace::default());
    let state = Arc::new(Probe::default());
    *state.lease_race.lock().unwrap() = Some(race.clone());
    let disk = memory().layer(ProbeLayer(state));
    let task = tokio::spawn(async move {
        disk.write(&lease_key(2), b"unconditional write".to_vec())
            .await
            .unwrap();
    });
    fixture_wait(
        "unconditional acquisition attempt to enter its gate",
        async {
            while race.entered.load(Ordering::SeqCst) < 1 {
                race.entered_event.notified().await;
            }
        },
    )
    .await;
    assert_eq!(
        *race.conditional.lock().unwrap(),
        vec![false],
        "the fixture records the conditional flag independently of gating"
    );
    race.gates[0].notify_one();
    fixture_wait("unconditional acquisition writer to finish", task)
        .await
        .unwrap();
    assert_eq!(race.created.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pub_003_round6_4_reserved_lease_url_precedes_application_routing_and_is_documented() {
    let _container = suprnova::testing::TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router: suprnova::Router = suprnova::Router::new()
        .get("/_pub/lease/application", move |_| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(suprnova::HttpResponse::text(
                    "unreachable application route",
                ))
            }
        })
        .into();
    suprnova::static_publish::StaticPublication::install(&router, publisher("v1").0).unwrap();
    let client = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new());
    let response = client.get("/_pub/lease/application").send().await;
    response.assert_status(404);
    assert_eq!(response.header("Cache-Control"), Some("no-store"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let manual = include_str!("../../../manual/static-publication.md").replace('\n', " ");
    assert!(
        manual.contains("Installing static publication reserves the `/_pub/lease/` URL prefix for\nGET and HEAD requests.")
    );
    assert!(manual.contains("receives 404 before application\nrouting"));
    assert!(manual.contains("Other methods are not intercepted."));
    assert!(manual.contains("an application route declared there is never reached"));
}
