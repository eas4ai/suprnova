//! Recovery checks use storage outcomes and a semantic conditional-write backend.
use super::*;

async fn delayed(
    mode: usize,
) -> (
    Publisher,
    Operator,
    Arc<Probe>,
    Arc<MemoryCdnDriver>,
    String,
    Result<suprnova::static_publish::PublishOutcome, FrameworkError>,
) {
    let disk = memory();
    let state = Arc::new(Probe::default());
    state.force_cas.store(true, Ordering::SeqCst);
    let driver = Arc::new(MemoryCdnDriver::default());
    let publisher = Publisher::from_operator(
        disk.clone().layer(ProbeLayer(state.clone())),
        "v1",
        PublishConfig {
            timeout: Duration::from_millis(400),
            ..Default::default()
        },
    )
    .unwrap()
    .with_cdn(driver.clone());
    let previous = publisher.publish(vec![page("old")]).await.unwrap().version;
    state.commit_mode.store(mode, Ordering::SeqCst);
    let result = publisher.publish(vec![page("next")]).await;
    (publisher, disk, state, driver, previous, result)
}
fn rollback_events(driver: &MemoryCdnDriver) -> Vec<String> {
    driver
        .events()
        .unwrap()
        .into_iter()
        .filter(|event| event.starts_with("rollback:"))
        .collect()
}
#[tokio::test]
async fn pub_003_round2_1_accepted_put_with_late_reply_is_success() {
    let (publisher, disk, _, driver, previous, result) = delayed(1).await;
    let outcome = result.expect("a durable manifest commit succeeds despite a late reply");
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&manifest_bytes(&disk).await).unwrap();
    assert_eq!(manifest.live, outcome.version);
    assert_ne!(manifest.live, previous);
    assert!(rollback_events(&driver).is_empty());
    let status: serde_json::Value =
        serde_json::from_slice(&disk.read("_pub/status.json").await.unwrap().to_bytes()).unwrap();
    assert_eq!(status["outcome"], "success");
    assert_eq!(publisher.status().await.live_version, manifest.live);
}
#[tokio::test]
async fn pub_003_round2_1_rejected_put_with_late_reply_rolls_back() {
    let (_, disk, _, driver, previous, result) = delayed(2).await;
    assert!(result.is_err());
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&manifest_bytes(&disk).await).unwrap();
    assert_eq!(manifest.live, previous);
    assert_eq!(
        rollback_events(&driver),
        vec![format!("rollback:{previous}")]
    );
}
#[tokio::test]
async fn pub_003_round2_1_unreadable_commit_is_unknown_then_checks_selection() {
    let (publisher, disk, state, driver, previous, result) = delayed(3).await;
    assert!(result.is_err());
    assert!(
        rollback_events(&driver).is_empty(),
        "unknown outcomes cannot roll back the CDN"
    );
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&manifest_bytes(&disk).await).unwrap();
    let status: serde_json::Value =
        serde_json::from_slice(&disk.read("_pub/status.json").await.unwrap().to_bytes()).unwrap();
    assert_eq!(status["outcome"], "unknown");
    assert_eq!(status["previous_version"], previous);
    assert_eq!(status["attempted_version"], manifest.live);
    let switches = driver
        .events()
        .unwrap()
        .into_iter()
        .filter(|event| event.starts_with("switch:"))
        .count();
    state.fail_manifest_read.store(false, Ordering::SeqCst);
    publisher.publish(vec![page("next")]).await.unwrap();
    assert_eq!(
        driver
            .events()
            .unwrap()
            .into_iter()
            .filter(|event| event.starts_with("switch:"))
            .count(),
        switches,
        "a known selection already matching the durable manifest needs no switch"
    );
}
#[tokio::test]
async fn pub_003_round2_2_semantic_cas_rejects_stale_writes() {
    let state = Arc::new(Probe::default());
    state.force_cas.store(true, Ordering::SeqCst);
    let disk = memory().layer(ProbeLayer(state));
    disk.write_with("object", "one")
        .if_not_exists(true)
        .await
        .unwrap();
    let etag = disk
        .stat("object")
        .await
        .unwrap()
        .etag()
        .unwrap()
        .to_string();
    disk.write_with("object", "two")
        .if_match(&etag)
        .await
        .unwrap();
    let failure = disk
        .write_with("object", "stale")
        .if_match(&etag)
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), opendal::ErrorKind::ConditionNotMatch);
    assert_eq!(
        disk.read("object").await.unwrap().to_bytes(),
        Bytes::from_static(b"two")
    );
    assert_eq!(
        disk.write_with("object", "replace")
            .if_not_exists(true)
            .await
            .unwrap_err()
            .kind(),
        opendal::ErrorKind::ConditionNotMatch
    );
}
#[tokio::test]
async fn pub_003_round2_2_manifest_cas_rejects_a_change_after_validation() {
    let raw = memory();
    let state = Arc::new(Probe::default());
    state.force_cas.store(true, Ordering::SeqCst);
    let p = Publisher::from_operator(
        raw.clone().layer(ProbeLayer(state.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap();
    p.publish(vec![page("old")]).await.unwrap();
    let mut old = p.manifest().await.unwrap();
    old.versions.get_mut(&old.live).unwrap().published_at_ms += 1;
    let foreign = Bytes::from(serde_json::to_vec(&old).unwrap());
    *state.inject_manifest.lock().unwrap() = Some((raw.clone(), foreign.clone()));
    assert!(p.publish(vec![page("new")]).await.is_err());
    assert_eq!(manifest_bytes(&raw).await, foreign);
    assert_eq!(state.cas_rejections.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pub_003_round2_2_clock_margin_refuses_an_early_taker() {
    let raw = memory();
    let state = Arc::new(Probe::default());
    state.force_cas.store(true, Ordering::SeqCst);
    let disk = raw.clone().layer(ProbeLayer(state));
    let p = Publisher::from_operator(
        disk.clone(),
        "v1",
        PublishConfig {
            timeout: Duration::from_millis(400),
            ..Default::default()
        },
    )
    .unwrap();
    p.publish_with(|| async {
        let lease = highest_lease(&raw).await;
        let expires = lease["expires_at_ms"].as_i64().unwrap();
        for at in [
            chrono::Utc::now() + chrono::Duration::seconds(9),
            chrono::DateTime::from_timestamp_millis(expires + 9000).unwrap(),
        ] {
            let contender =
                Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
            let result = suprnova::testing::TestClock::scope(at, |_| async {
                contender.publish(vec![page("taker")]).await
            })
            .await;
            assert!(
                result.is_err(),
                "a taker inside the configured skew margin must be refused"
            );
        }
        Ok(vec![page("holder")])
    })
    .await
    .unwrap();
}
struct TakeoverDriver {
    disk: Operator,
    ledger: Arc<MemoryCdnDriver>,
    armed: AtomicBool,
    fail_after: bool,
    takeover: Mutex<Option<String>>,
}
#[async_trait]
impl CdnDriver for TakeoverDriver {
    fn capabilities(&self) -> CdnCapabilities {
        Default::default()
    }
    async fn selected_version(&self) -> Result<Option<String>, FrameworkError> {
        self.ledger.selected_version().await
    }
    async fn switch_version(&self, version: &str) -> Result<(), FrameworkError> {
        self.ledger.switch_version(version).await?;
        if self.armed.swap(false, Ordering::SeqCst) {
            let record = highest_lease(&self.disk).await;
            let at = chrono::DateTime::from_timestamp_millis(
                record["expires_at_ms"].as_i64().unwrap() + 10000,
            )
            .unwrap();
            let next = Publisher::from_operator(self.disk.clone(), "v1", PublishConfig::default())
                .unwrap()
                .with_cdn(self.ledger.clone());
            let result = suprnova::testing::TestClock::scope(at, |_| async {
                next.publish(vec![page("new owner")]).await
            })
            .await?;
            *self.takeover.lock().unwrap() = Some(result.version);
            if self.fail_after {
                return Err(FrameworkError::internal("old switch reply failed"));
            }
        }
        Ok(())
    }
    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        self.ledger.rollback_version(previous).await
    }
    async fn purge(&self, _: Purge) -> Result<(), FrameworkError> {
        Ok(())
    }
}
#[tokio::test]
async fn pub_003_round2_2_takeover_fences_old_commit_and_rollback() {
    for fail_after in [false, true] {
        let state = Arc::new(Probe::default());
        state.force_cas.store(true, Ordering::SeqCst);
        let disk = memory().layer(ProbeLayer(state));
        let ledger = Arc::new(MemoryCdnDriver::default());
        let driver = Arc::new(TakeoverDriver {
            disk: disk.clone(),
            ledger: ledger.clone(),
            armed: AtomicBool::new(false),
            fail_after,
            takeover: Mutex::new(None),
        });
        let p = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default())
            .unwrap()
            .with_cdn(driver.clone());
        p.publish(vec![page("original")]).await.unwrap();
        driver.armed.store(true, Ordering::SeqCst);
        let failure = p.publish(vec![page("displaced")]).await.unwrap_err();
        let live = p.manifest().await.unwrap().live;
        assert_eq!(Some(live.clone()), *driver.takeover.lock().unwrap());
        assert!(
            rollback_events(&ledger).is_empty(),
            "a displaced holder must not roll back the new owner"
        );
        assert_eq!(
            ledger
                .events()
                .unwrap()
                .into_iter()
                .rfind(|event| event.starts_with("switch:"))
                .unwrap(),
            format!("switch:{live}")
        );
        let owner = highest_lease(&disk).await;
        assert!(
            failure
                .to_string()
                .contains(owner["owner"].as_str().unwrap()),
            "{failure}"
        );
        assert_eq!(
            p.status().await.live_version,
            live,
            "status must name the current owner's live manifest"
        );
        assert!(
            p.status()
                .await
                .last_outcome
                .contains(owner["owner"].as_str().unwrap())
        );
    }
}
#[tokio::test]
#[serial_test::serial]
async fn pub_003_round2_2_clock_skew_setting_is_loaded() {
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&["STATIC_PUBLISH_CLOCK_SKEW"]);
    crate::env_snapshot::set_env("STATIC_PUBLISH_CLOCK_SKEW", Some("60"));
    let (publisher, disk) = publisher("v1");
    publisher
        .publish_with(|| async {
            let lease = highest_lease(&disk).await;
            assert!(
                lease["expires_at_ms"].as_i64().unwrap()
                    > chrono::Utc::now().timestamp_millis() + 300000,
                "skew setting must extend the usable lease budget"
            );
            Ok(vec![page("configured")])
        })
        .await
        .unwrap();
    crate::env_snapshot::set_env("STATIC_PUBLISH_CLOCK_SKEW", Some("invalid"));
    assert!(
        Publisher::from_operator(memory(), "v1", PublishConfig::default()).is_err(),
        "invalid skew must be diagnosed"
    );
}

#[tokio::test]
async fn pub_003_round4_1_cached_success_cannot_skip_another_publishers_unknown_switch() {
    let raw = memory();
    let probe = Arc::new(Probe::default());
    let driver = Arc::new(MemoryCdnDriver::default());
    let a = Publisher::from_operator(raw.clone(), "v1", PublishConfig::default())
        .unwrap()
        .with_cdn(driver.clone());
    let old = a.publish(vec![page("old")]).await.unwrap().version;
    assert_eq!(a.status().await.outcome, "success");
    let b = Publisher::from_operator(
        raw.clone().layer(ProbeLayer(probe.clone())),
        "v1",
        PublishConfig::default(),
    )
    .unwrap()
    .with_cdn(driver.clone());
    probe.commit_mode.store(4, Ordering::SeqCst);
    assert!(b.publish(vec![page("uncommitted")]).await.is_err());
    assert_eq!(b.status().await.outcome, "unknown");
    assert_eq!(highest_lease(&raw).await["expires_at_ms"], 0);
    assert_ne!(
        driver.events().unwrap().last().unwrap(),
        &format!("switch:{old}")
    );
    let events = driver.events().unwrap().len();
    a.publish_with(|| async {
        assert_eq!(
            driver.selected_version().await.unwrap().as_deref(),
            Some(old.as_str())
        );
        assert_eq!(
            a.status().await.outcome,
            "pending",
            "success cannot precede reconciliation"
        );
        Ok(vec![page("old")])
    })
    .await
    .unwrap();
    assert_eq!(
        driver.events().unwrap()[events..],
        [format!("switch:{old}")],
        "unchanged A must repair the actual CDN selection despite its cached success"
    );
    assert_eq!(a.status().await.outcome, "success");
    assert_eq!(a.manifest().await.unwrap().live, old);
}

#[tokio::test]
async fn pub_003_round4_1_unknown_driver_selection_always_realigns() {
    struct Unknown(Arc<MemoryCdnDriver>);
    #[async_trait]
    impl CdnDriver for Unknown {
        fn capabilities(&self) -> CdnCapabilities {
            Default::default()
        }
        async fn switch_version(&self, version: &str) -> Result<(), FrameworkError> {
            self.0.switch_version(version).await
        }
        async fn purge(&self, _: Purge) -> Result<(), FrameworkError> {
            Ok(())
        }
    }
    let ledger = Arc::new(MemoryCdnDriver::default());
    let (p, _) = publisher("v1");
    let p = p.with_cdn(Arc::new(Unknown(ledger.clone())));
    let first = p.publish(vec![page("one")]).await.unwrap();
    let events = ledger.events().unwrap().len();
    p.publish(vec![page("one")]).await.unwrap();
    assert_eq!(
        ledger.events().unwrap()[events..],
        [format!("switch:{}", first.version)]
    );
}
