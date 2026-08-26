#![cfg(feature = "filesystem")]

//! Integration tests for read-through disks.
//!
//! Every test takes a `Storage::fake()` guard first, exactly like
//! `framework/tests/filesystem.rs`: the guard serializes against every other
//! fake-using test through a process-wide mutex and wipes the global disk
//! registry on drop, so these tests can register named disks under the default
//! parallel test runner without colliding.

use suprnova::filesystem::streaming::copy_between_disks;
use suprnova::{DiskExt, ReadThroughConfig, Storage};

/// Register `primary` + `fallback` memory disks and an `assets` read-through
/// disk over them. Returns nothing; call `Storage::disk(...)` for each handle.
fn register_memory_pair() {
    Storage::register_memory("primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");
}

#[tokio::test]
async fn a_fallback_only_read_is_promoted_onto_the_primary() {
    let _guard = Storage::fake();
    register_memory_pair();

    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    let bytes = assets.read("cold.txt").await.expect("read resolves");
    assert_eq!(&bytes.to_vec(), b"cold bytes");

    let primary = Storage::disk("primary").expect("primary disk");
    assert_eq!(
        &primary
            .read("cold.txt")
            .await
            .expect("the fallback hit was promoted")
            .to_vec(),
        b"cold bytes",
        "a fallback hit must be written through to the primary"
    );
}

#[tokio::test]
async fn a_promoted_object_is_served_from_the_primary_afterwards() {
    let _guard = Storage::fake();
    register_memory_pair();

    let fallback = Storage::disk("fallback").expect("fallback disk");
    fallback
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    assets.read("cold.txt").await.expect("first read promotes");

    // Remove the fallback copy. If the second read still succeeds, it can only
    // have come from the primary - which is the whole point of promotion.
    fallback
        .delete("cold.txt")
        .await
        .expect("drop the fallback copy");

    let bytes = assets.read("cold.txt").await.expect("second read");
    assert_eq!(&bytes.to_vec(), b"cold bytes");
}

#[tokio::test]
async fn a_primary_hit_never_consults_the_fallback() {
    let _guard = Storage::fake();
    register_memory_pair();

    // Both disks hold the path, with different contents. A read that returns
    // the primary's bytes proves the primary answered without the fallback
    // shadowing it.
    Storage::disk("primary")
        .expect("primary disk")
        .write("both.txt", "primary bytes")
        .await
        .expect("seed the primary");
    Storage::disk("fallback")
        .expect("fallback disk")
        .write("both.txt", "fallback bytes")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    assert_eq!(
        &assets
            .read("both.txt")
            .await
            .expect("read resolves")
            .to_vec(),
        b"primary bytes",
        "the primary owns the object, so the fallback must not answer"
    );
}

#[tokio::test]
async fn a_ranged_read_resolves_and_promotes_from_the_fallback() {
    let _guard = Storage::fake();
    register_memory_pair();

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "0123456789")
        .await
        .expect("seed the fallback");

    // No `chunk` option, so opendal drives this through the reader's `open`
    // path: one streaming reader for the requested range.
    let assets = Storage::disk("assets").expect("read-through disk");
    let bytes = assets
        .read_with("cold.txt")
        .range(2..7)
        .await
        .expect("ranged read resolves");
    assert_eq!(
        &bytes.to_vec(),
        b"23456",
        "a ranged read must return only the requested slice"
    );

    assert_eq!(
        &Storage::disk("primary")
            .expect("primary disk")
            .read("cold.txt")
            .await
            .expect("the ranged hit was promoted")
            .to_vec(),
        b"0123456789",
        "promotion writes the whole object, not just the requested range"
    );
}

#[tokio::test]
async fn a_chunked_read_resolves_and_promotes_from_the_fallback() {
    let _guard = Storage::fake();
    register_memory_pair();
    Storage::register_memory("scratch");

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.bin", "chunked cold bytes")
        .await
        .expect("seed the fallback");

    // `copy_between_disks` sets `.chunk(..)`, which drives opendal's chunked
    // reader - the reader's `read` path rather than its `open` path.
    let copied = copy_between_disks("assets", "cold.bin", "scratch", "warm.bin")
        .await
        .expect("chunked copy off a read-through disk succeeds");
    assert_eq!(copied, "chunked cold bytes".len() as u64);

    assert_eq!(
        &Storage::disk("scratch")
            .expect("scratch disk")
            .read("warm.bin")
            .await
            .expect("destination holds the copy")
            .to_vec(),
        b"chunked cold bytes"
    );
    assert_eq!(
        &Storage::disk("primary")
            .expect("primary disk")
            .read("cold.bin")
            .await
            .expect("the chunked hit was promoted")
            .to_vec(),
        b"chunked cold bytes",
        "a chunked read must promote the fallback hit too"
    );
}

#[tokio::test]
async fn existence_and_metadata_see_fallback_only_objects() {
    let _guard = Storage::fake();
    register_memory_pair();

    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    assert!(
        assets.exists("cold.txt").await.expect("exists answers"),
        "a fallback-only object must be visible to exists"
    );
    assert!(
        assets
            .file_exists("cold.txt")
            .await
            .expect("file_exists answers"),
        "a fallback-only object must be visible to file_exists"
    );
    assert_eq!(
        assets.size("cold.txt").await.expect("size answers"),
        "cold bytes".len() as u64,
        "metadata must be answered by whichever disk holds the object"
    );
    assert!(
        assets.missing("absent.txt").await.expect("missing answers"),
        "an object on neither disk is still missing"
    );
}

#[tokio::test]
async fn a_read_missing_from_both_disks_fails_as_not_found() {
    let _guard = Storage::fake();
    register_memory_pair();

    let assets = Storage::disk("assets").expect("read-through disk");
    let err = assets
        .read("absent.txt")
        .await
        .expect_err("an object on neither disk cannot be read");
    assert_eq!(
        err.kind(),
        opendal::ErrorKind::NotFound,
        "a miss on both disks is a plain NotFound, got: {err}"
    );

    assert!(
        !Storage::disk("primary")
            .expect("primary disk")
            .exists("absent.txt")
            .await
            .expect("primary exists answers"),
        "a failed read must not create anything on the primary"
    );
}

#[tokio::test]
async fn writes_land_on_the_primary_only() {
    let _guard = Storage::fake();
    register_memory_pair();

    let assets = Storage::disk("assets").expect("read-through disk");
    assets
        .write("hot.txt", "hot bytes")
        .await
        .expect("write succeeds");

    assert_eq!(
        &Storage::disk("primary")
            .expect("primary disk")
            .read("hot.txt")
            .await
            .expect("primary holds the write")
            .to_vec(),
        b"hot bytes"
    );
    assert!(
        !Storage::disk("fallback")
            .expect("fallback disk")
            .exists("hot.txt")
            .await
            .expect("fallback exists answers"),
        "a write must never touch the fallback disk"
    );
}

#[tokio::test]
async fn listing_shows_primary_entries_only() {
    let _guard = Storage::fake();
    register_memory_pair();

    Storage::disk("primary")
        .expect("primary disk")
        .write("hot.txt", "hot")
        .await
        .expect("seed the primary");
    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold")
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    let files = assets.files("", false).await.expect("listing succeeds");
    assert_eq!(
        files,
        vec!["hot.txt".to_string()],
        "listing is primary-only; fallback entries stay invisible"
    );
}

#[tokio::test]
async fn delete_removes_the_object_from_both_disks() {
    let _guard = Storage::fake();
    register_memory_pair();

    let primary = Storage::disk("primary").expect("primary disk");
    let fallback = Storage::disk("fallback").expect("fallback disk");
    primary
        .write("doomed.txt", "primary copy")
        .await
        .expect("seed the primary");
    fallback
        .write("doomed.txt", "fallback copy")
        .await
        .expect("seed the fallback");

    Storage::disk("assets")
        .expect("read-through disk")
        .delete("doomed.txt")
        .await
        .expect("delete succeeds");

    assert!(
        !primary.exists("doomed.txt").await.expect("primary exists"),
        "delete must remove the primary copy"
    );
    assert!(
        !fallback
            .exists("doomed.txt")
            .await
            .expect("fallback exists"),
        "delete must remove the fallback copy too, or the next read resurrects it"
    );
}

#[tokio::test]
async fn invalid_read_through_configurations_are_rejected() {
    let _guard = Storage::fake();
    Storage::register_memory("primary");
    Storage::register_memory("fallback");

    let missing_primary = Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: String::new(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect_err("an empty primary is rejected");
    assert!(
        missing_primary.to_string().contains("primary"),
        "the error must name the missing option, got: {missing_primary}"
    );

    let missing_fallback = Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: String::new(),
            ..Default::default()
        },
    )
    .expect_err("an empty fallback is rejected");
    assert!(
        missing_fallback.to_string().contains("fallback"),
        "the error must name the missing option, got: {missing_fallback}"
    );

    let same_disk = Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "primary".into(),
            ..Default::default()
        },
    )
    .expect_err("primary and fallback must differ");
    assert!(
        same_disk.to_string().contains("distinct"),
        "the error must say the disks have to be distinct, got: {same_disk}"
    );

    let self_reference = Storage::register_read_through(
        "primary",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect_err("a read-through disk cannot name itself");
    assert!(
        self_reference.to_string().contains("itself"),
        "the error must say the disk references itself, got: {self_reference}"
    );

    let unknown_disk = Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "nowhere".into(),
            ..Default::default()
        },
    )
    .expect_err("an unregistered disk is rejected");
    assert!(
        unknown_disk.to_string().contains("nowhere"),
        "the error must name the unregistered disk, got: {unknown_disk}"
    );

    assert!(
        Storage::disk("assets").is_err(),
        "no rejected configuration may leave a disk registered"
    );
}

#[tokio::test]
async fn the_path_traversal_guard_still_applies_under_a_read_through_disk() {
    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    // Both roots sit one level below the planted secret, so "../secret.txt"
    // escapes either of them.
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, b"TOP SECRET").expect("plant out-of-root secret");
    for dir in ["primary", "fallback"] {
        std::fs::create_dir_all(tmp.path().join(dir)).expect("create a disk root");
    }
    Storage::register_fs("primary", tmp.path().join("primary")).expect("fs primary");
    Storage::register_fs("fallback", tmp.path().join("fallback")).expect("fs fallback");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");

    let assets = Storage::disk("assets").expect("read-through disk");
    assert!(
        assets.read("../secret.txt").await.is_err(),
        "the traversal guard must still reject a read composed under a read-through disk"
    );
    assert!(
        assets.write("../escaped.txt", "owned").await.is_err(),
        "the traversal guard must still reject a write"
    );
    assert!(
        assets.stat("../secret.txt").await.is_err(),
        "the traversal guard must still reject a stat, without falling through to the fallback"
    );
    assert!(
        assets.delete("../secret.txt").await.is_err(),
        "the traversal guard must still reject a delete"
    );

    assert_eq!(
        std::fs::read(&secret).expect("the secret survives"),
        b"TOP SECRET",
        "no traversal attempt may reach the out-of-root file"
    );
    assert!(
        !tmp.path().join("escaped.txt").exists(),
        "no traversal attempt may create an out-of-root file"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_promotion_is_swallowed_by_default_and_surfaced_on_request() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = Storage::fake();
    let tmp = tempfile::tempdir().expect("tempdir");
    let primary_root = tmp.path().join("primary");
    std::fs::create_dir_all(&primary_root).expect("create the primary root");

    Storage::register_fs("primary", &primary_root).expect("fs primary");
    Storage::register_memory("fallback");
    Storage::disk("fallback")
        .expect("fallback disk")
        .write("cold.txt", "cold bytes")
        .await
        .expect("seed the fallback");

    // Make the primary root unwritable so the promotion write fails while the
    // stat that routes the read still succeeds (0o555 keeps read + traverse).
    std::fs::set_permissions(&primary_root, PermissionsExt::from_mode(0o555))
        .expect("make the primary root read-only");
    let probe = primary_root.join(".probe");
    if std::fs::write(&probe, b"probe").is_ok() {
        // A process that ignores directory modes (running as root, or an
        // exotic filesystem) cannot express this precondition at all.
        let _ = std::fs::remove_file(&probe);
        let _ = std::fs::set_permissions(&primary_root, PermissionsExt::from_mode(0o755));
        eprintln!(
            "skipping: this process can write through a read-only directory, \
             so an unwritable primary cannot be simulated"
        );
        return;
    }

    Storage::register_read_through(
        "swallowing",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");
    let swallowing = Storage::disk("swallowing").expect("read-through disk");
    assert_eq!(
        &swallowing
            .read("cold.txt")
            .await
            .expect("a failed promotion must not fail the read")
            .to_vec(),
        b"cold bytes",
        "by default a read-through disk over an unwritable primary degrades to \
         reading the fallback every time"
    );

    Storage::register_read_through(
        "strict",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            throw_on_promotion_failure: true,
        },
    )
    .expect("strict read-through registration succeeds");
    let strict = Storage::disk("strict").expect("strict read-through disk");
    let err = strict
        .read("cold.txt")
        .await
        .expect_err("throw_on_promotion_failure surfaces the failure");
    let msg = err.to_string();
    assert!(
        msg.contains("promotion") && msg.contains("cold.txt"),
        "the error must name the failure and the path, got: {msg}"
    );

    // Restore the mode so TempDir::drop can clean up.
    let _ = std::fs::set_permissions(&primary_root, PermissionsExt::from_mode(0o755));
}

/// Register an fs primary rooted in a fresh tempdir, a memory fallback, and an
/// `assets` read-through disk over them. The tempdir is returned so the caller
/// keeps it alive for the length of the test.
fn register_local_primary_pair() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let primary_root = tmp.path().join("primary");
    std::fs::create_dir_all(&primary_root).expect("create the primary root");
    Storage::register_fs("primary", &primary_root).expect("fs primary");
    Storage::register_memory("fallback");
    Storage::register_read_through(
        "assets",
        ReadThroughConfig {
            primary: "primary".into(),
            fallback: "fallback".into(),
            ..Default::default()
        },
    )
    .expect("read-through registration succeeds");
    tmp
}

/// Assert that no reader can ever see the promoted object on the primary at a
/// length other than its full one.
///
/// A local filesystem creates the target file and fills it in place, so a
/// promotion written straight to the target is visible at every intermediate
/// length for the duration of the write. That matters because the promoted path
/// is exactly the object a second cold reader routes by existence: it sees the
/// partial file, delegates to the primary, and gets a short body with no error
/// attached. Sampling the primary while a promotion runs is the direct
/// observation of that, and it does not depend on winning a race.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_promotion_is_never_visible_on_the_primary_at_a_partial_length() {
    let _guard = Storage::fake();
    let _tmp = register_local_primary_pair();

    // Large enough that the write spans many milliseconds, so the sampling loop
    // below takes a meaningful number of looks at the primary.
    let payload = vec![b'x'; 64 * 1024 * 1024];
    let full_len = payload.len() as u64;
    Storage::disk("fallback")
        .expect("fallback disk")
        .write("big.bin", payload)
        .await
        .expect("seed the fallback");

    let assets = Storage::disk("assets").expect("read-through disk");
    let promoter = assets.clone();
    let handle =
        tokio::spawn(async move { promoter.read("big.bin").await.expect("promote").len() });

    let primary = Storage::disk("primary").expect("primary disk");
    let mut samples = 0usize;
    let mut partial = Vec::new();
    while !handle.is_finished() && samples < 4096 {
        tokio::time::sleep(std::time::Duration::from_micros(250)).await;
        samples += 1;
        if let Ok(metadata) = primary.stat("big.bin").await
            && metadata.content_length() != full_len
        {
            partial.push(metadata.content_length());
        }
    }

    let promoted = handle.await.expect("the promoting task did not panic");
    assert_eq!(
        promoted as u64, full_len,
        "the promoter read the whole object"
    );
    // How many looks this gets is a property of the host, not of the code under
    // test, so it is reported rather than asserted: a faster disk would turn a
    // correct implementation red. The mechanism itself is pinned deterministically
    // by `read_through`'s own `a_promotion_on_a_rename_capable_primary_is_staged_and_renamed`.
    eprintln!("sampled the primary {samples} times during the promotion");
    assert!(
        partial.is_empty(),
        "the promoted object was visible at a partial length: {:?}",
        &partial.iter().take(8).collect::<Vec<_>>()
    );

    let listed = primary.files("", true).await.expect("listing succeeds");
    assert_eq!(
        listed,
        vec!["big.bin".to_string()],
        "a promotion must leave no staging object behind"
    );
}

/// Seed a cold object and read it from several parallel tasks, asserting none
/// of them observes a partially promoted object and that the staging path a
/// rename-capable primary uses is always cleaned up.
async fn assert_concurrent_cold_reads_are_whole(rounds: usize, payload_len: usize, readers: usize) {
    let fallback = Storage::disk("fallback").expect("fallback disk");
    let assets = Storage::disk("assets").expect("read-through disk");
    let payload = vec![b'x'; payload_len];

    for round in 0..rounds {
        let path = format!("cold-{round}.bin");
        fallback
            .write(path.as_str(), payload.clone())
            .await
            .expect("seed the fallback");

        let handles: Vec<_> = (0..readers)
            .map(|reader| {
                let disk = assets.clone();
                let path = path.clone();
                tokio::spawn(async move {
                    // Stagger the readers so some of them arrive while another
                    // is midway through its promotion.
                    tokio::time::sleep(std::time::Duration::from_micros(200 * reader as u64)).await;
                    disk.read(path.as_str()).await
                })
            })
            .collect();

        for (reader, handle) in handles.into_iter().enumerate() {
            let bytes = handle
                .await
                .expect("the reader task did not panic")
                .expect("the read resolves");
            assert_eq!(
                bytes.len(),
                payload_len,
                "round {round}, reader {reader}: read a partially promoted object"
            );
        }
    }

    let listed = Storage::disk("primary")
        .expect("primary disk")
        .files("", true)
        .await
        .expect("listing succeeds");
    assert!(
        listed.iter().all(|name| !name.contains("suprnova-promote")),
        "a promotion must leave no staging object behind, found: {listed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_cold_reads_over_a_local_primary_are_never_partial() {
    let _guard = Storage::fake();
    let _tmp = register_local_primary_pair();

    assert_concurrent_cold_reads_are_whole(8, 4 * 1024 * 1024, 4).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_cold_reads_over_a_memory_primary_are_never_partial() {
    let _guard = Storage::fake();
    register_memory_pair();

    assert_concurrent_cold_reads_are_whole(8, 4 * 1024 * 1024, 4).await;
}

#[tokio::test]
async fn a_promotion_carries_the_fallback_objects_content_metadata() {
    let _guard = Storage::fake();
    register_memory_pair();

    Storage::disk("fallback")
        .expect("fallback disk")
        .write_with("logo.png", "pretend png bytes")
        .content_type("image/png")
        .cache_control("public, max-age=31536000")
        .content_disposition("inline; filename=\"logo.png\"")
        .content_encoding("identity")
        .await
        .expect("seed the fallback with content metadata");

    Storage::disk("assets")
        .expect("read-through disk")
        .read("logo.png")
        .await
        .expect("read promotes");

    let promoted = Storage::disk("primary")
        .expect("primary disk")
        .stat("logo.png")
        .await
        .expect("the primary holds the promoted object");
    assert_eq!(
        promoted.content_type(),
        Some("image/png"),
        "a promotion that drops Content-Type loses it permanently"
    );
    assert_eq!(promoted.cache_control(), Some("public, max-age=31536000"));
    assert_eq!(
        promoted.content_disposition(),
        Some("inline; filename=\"logo.png\"")
    );
    assert_eq!(promoted.content_encoding(), Some("identity"));
}
