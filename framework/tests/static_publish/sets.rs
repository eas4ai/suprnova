//! Asset sets share unchanged bytes while keeping relative references together.
use super::*;

fn rendered(text: &str, css: &'static [u8]) -> PageObject {
    let mut rendered = page(text);
    rendered.json =
        Bytes::from(serde_json::json!({"props":{"text":text,"css":"/theme.css"}}).to_string());
    let embedded = std::str::from_utf8(&rendered.json)
        .unwrap()
        .replace('/', "\\/");
    rendered.html = Bytes::from(format!(
        r#"<p>{text}</p><link rel="stylesheet" href="/theme.css"><script type="application/json">{embedded}</script>"#
    ));
    for (key, bytes, mime) in [
        ("theme.css", css, "text/css"),
        ("bg.png", b"pixels".as_slice(), "image/png"),
        ("bg-AbC12345.png", b"fingerprint".as_slice(), "image/png"),
    ] {
        rendered
            .assets
            .insert(key.into(), (Bytes::from_static(bytes), mime.into()));
    }
    rendered
}
fn set_id(manifest: &suprnova::static_publish::Manifest) -> String {
    let value = serde_json::to_value(&manifest.versions[&manifest.live]).unwrap();
    value["asset_set_digest"]
        .as_str()
        .expect("each revision records its asset set digest")
        .to_string()
}
fn set_writes(probe: &Probe) -> usize {
    probe
        .writes
        .lock()
        .unwrap()
        .iter()
        .filter(|key| key.contains("/_assets/"))
        .count()
}

#[tokio::test]
#[serial_test::serial]
async fn pub_002_round3_page_changes_reuse_the_asset_set_without_writes_or_copies() {
    let _container = suprnova::testing::TestContainer::fake();
    let router = asset_router();
    let probe = Arc::new(Probe::default());
    probe.force_copy.store(true, Ordering::SeqCst);
    let disk = memory().layer(ProbeLayer(probe.clone()));
    let p = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
    p.publish(vec![rendered("one", b"body{background:url(./bg.png)}")])
        .await
        .unwrap();
    let first = p.manifest().await.unwrap();
    let writes = set_writes(&probe);
    let copies = probe.copies.load(Ordering::SeqCst);
    p.publish(vec![rendered("two", b"body{background:url(./bg.png)}")])
        .await
        .unwrap();
    let second = p.manifest().await.unwrap();
    assert_ne!(first.live, second.live);
    assert_eq!(
        set_writes(&probe),
        writes,
        "page changes must not upload mutable assets"
    );
    assert_eq!(
        probe.copies.load(Ordering::SeqCst),
        copies,
        "page changes must not copy mutable assets"
    );
    let set = set_id(&first);
    assert_eq!(set_id(&second), set);
    let path = format!("/_pub/v1/_assets/{set}/bg.png");
    suprnova::static_publish::StaticPublication::install(&router, p).unwrap();
    let response = suprnova::testing::TestClient::new(router, suprnova::MiddlewareRegistry::new())
        .get(path)
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.body_text(), "pixels");
}

#[tokio::test]
async fn pub_002_round3_css_change_writes_a_complete_new_set_with_relative_dependencies() {
    use sha2::{Digest, Sha256};
    for copy in [false, true] {
        let probe = Arc::new(Probe::default());
        probe.force_copy.store(copy, Ordering::SeqCst);
        let disk = memory().layer(ProbeLayer(probe.clone()));
        let p = Publisher::from_operator(disk.clone(), "v1", PublishConfig::default()).unwrap();
        p.publish(vec![rendered("one", b"body{background:url(./bg.png)}")])
            .await
            .unwrap();
        let first = p.manifest().await.unwrap();
        let css = b"body{color:red;background:url(./bg.png)}";
        let next = rendered("two", css);
        let pairs: Vec<_> = next
            .assets
            .iter()
            .map(|(path, (bytes, _))| (path, hex::encode(Sha256::digest(bytes))))
            .collect();
        let expected = hex::encode(Sha256::digest(serde_json::to_vec(&pairs).unwrap()));
        let writes = set_writes(&probe);
        p.publish(vec![next]).await.unwrap();
        let second = p.manifest().await.unwrap();
        let set = set_id(&second);
        assert_ne!(set_id(&first), set);
        assert_eq!(
            set, expected,
            "digest uses sorted public paths and content digests"
        );
        let root = format!("_pub/v1/_assets/{set}");
        for (key, expected) in [
            ("theme.css", css.as_slice()),
            ("bg.png", b"pixels".as_slice()),
            ("bg-AbC12345.png", b"fingerprint".as_slice()),
        ] {
            assert_eq!(
                disk.read(&format!("{root}/{key}"))
                    .await
                    .unwrap()
                    .to_bytes(),
                expected
            );
        }
        let css_url = url::Url::parse(&format!("https://public.test/{root}/theme.css")).unwrap();
        assert_eq!(
            css_url.join("./bg.png").unwrap().path(),
            format!("/{root}/bg.png")
        );
        let live = &second.versions[&second.live];
        for object in live.objects.iter().filter(|o| !o.asset) {
            let body = disk.read(&object.path).await.unwrap().to_bytes();
            assert!(String::from_utf8_lossy(&body).contains(&format!("/{root}/theme.css")));
        }
        if copy {
            assert_eq!(
                probe.copies.load(Ordering::SeqCst),
                2,
                "unchanged set members use server-side copy"
            );
            assert_eq!(set_writes(&probe) - writes, 1);
        } else {
            assert_eq!(probe.copies.load(Ordering::SeqCst), 0);
            assert_eq!(set_writes(&probe) - writes, 3);
        }
    }
}

#[tokio::test]
async fn pub_008_round3_prune_keeps_shared_sets_until_the_last_reference_retires() {
    let disk = memory();
    let p = Publisher::from_operator(
        disk.clone(),
        "v1",
        PublishConfig {
            grace_period: Duration::from_secs(3600),
            ..Default::default()
        },
    )
    .unwrap();
    p.publish(vec![rendered("one", b"body{background:url(./bg.png)}")])
        .await
        .unwrap();
    let first = p.manifest().await.unwrap();
    p.publish(vec![rendered("two", b"body{background:url(./bg.png)}")])
        .await
        .unwrap();
    let second = p.manifest().await.unwrap();
    let old_bg = first.versions[&first.live]
        .objects
        .iter()
        .find(|o| o.url == "/bg.png")
        .unwrap()
        .path
        .clone();
    assert_eq!(
        second.versions[&second.live]
            .objects
            .iter()
            .find(|o| o.url == "/bg.png")
            .unwrap()
            .path,
        old_bg,
        "retained revisions share the same set directory"
    );
    p.publish(vec![rendered(
        "three",
        b"body{color:red;background:url(./bg.png)}",
    )])
    .await
    .unwrap();
    let mut manifest = p.manifest().await.unwrap();
    let old_extra = format!("{}extra.bin", old_bg.strip_suffix("bg.png").unwrap());
    disk.write(&old_extra, "extra member").await.unwrap();
    let live_extra = format!("_pub/v1/_assets/{}/extra.bin", set_id(&manifest));
    disk.write(&live_extra, "live member").await.unwrap();
    manifest
        .versions
        .get_mut(&first.live)
        .unwrap()
        .retired_at_ms = Some(0);
    disk.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let kept = p.prune().await.unwrap();
    assert!(kept.kept.contains(&old_bg));
    assert!(kept.kept.contains(&old_extra));
    assert!(disk.exists(&old_bg).await.unwrap());
    let mut manifest = p.manifest().await.unwrap();
    manifest
        .versions
        .get_mut(&second.live)
        .unwrap()
        .retired_at_ms = Some(0);
    disk.write("_pub/manifest.json", serde_json::to_vec(&manifest).unwrap())
        .await
        .unwrap();
    let removed = p.prune().await.unwrap();
    assert!(removed.removed.contains(&old_bg));
    assert!(removed.removed.contains(&old_extra));
    assert!(removed.kept.contains(&live_extra));
    assert!(disk.exists(&live_extra).await.unwrap());
    assert!(!disk.exists(&old_bg).await.unwrap());
    let live = &manifest.versions[&manifest.live];
    for asset in live.objects.iter().filter(|o| o.asset) {
        assert!(disk.exists(&asset.path).await.unwrap());
        assert!(!removed.removed.contains(&asset.path));
    }
}
