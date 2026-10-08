//! RTC-001: installing the Magnetar engines hands `HASH_MAX_CONCURRENCY` to
//! the process-wide hash work limit before any Magnetar hash work can run.
//! The limit is fixed by whichever comes first, the configuration or the
//! first piece of work, so a sign-in that came before the application's
//! first framework hash would otherwise fix it at the host's parallelism.

use std::num::NonZeroUsize;

use sea_orm::Database;
use suprnova::{Crypt, EncryptionKey, MagnetarConfig, init_magnetar};

/// Runs in its own process: it sets `HASH_MAX_CONCURRENCY`, fixes the
/// process-wide hash work limit and installs the Magnetar engines, all of
/// which last for the life of the process.
#[tokio::test]
async fn rtc_installing_magnetar_applies_hash_max_concurrency() {
    if crate::own_process_async::delegate(
        module_path!(),
        "rtc_installing_magnetar_applies_hash_max_concurrency",
    )
    .await
    {
        return;
    }
    Crypt::init(EncryptionKey::generate());

    // SAFETY: this test runs alone in a child process (nextest, or the
    // child `own_process_async::delegate` starts under plain `cargo test`),
    // and nothing else in it reads the environment while this call runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", "0") };
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    let error = init_magnetar(MagnetarConfig::from_sea_orm(connection))
        .await
        .expect_err("HASH_MAX_CONCURRENCY=0 refuses the installation");
    assert!(
        error.to_string().contains("HASH_MAX_CONCURRENCY"),
        "the error names the variable: {error}"
    );

    // SAFETY: as above.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", "3") };
    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    init_magnetar(MagnetarConfig::from_sea_orm(connection))
        .await
        .expect("install the default Magnetar engines");

    let three = NonZeroUsize::new(3).expect("non-zero");
    let four = NonZeroUsize::new(4).expect("non-zero");
    magnetar::password::configure_hash_work_limit(three)
        .expect("the installation fixed the limit at HASH_MAX_CONCURRENCY");
    let error =
        magnetar::password::configure_hash_work_limit(four).expect_err("the limit is fixed once");
    assert!(
        error.to_string().contains("already set to 3"),
        "the limit the installation fixed is 3: {error}"
    );
}

/// RTC-001: building the verifier at installation warms one dummy hash per
/// format, and that is hash work too. With the one permit held, the
/// installation waits for it instead of hashing inline on the runtime.
#[tokio::test]
async fn rtc_the_installations_warm_up_hashes_wait_for_a_permit() {
    if crate::own_process_async::delegate(
        module_path!(),
        "rtc_the_installations_warm_up_hashes_wait_for_a_permit",
    )
    .await
    {
        return;
    }
    Crypt::init(EncryptionKey::generate());
    let one = NonZeroUsize::new(1).expect("non-zero");
    magnetar::password::configure_hash_work_limit(one).expect("the limit is free to fix");
    // SAFETY: this test runs alone in a child process (nextest, or the
    // child `own_process_async::delegate` starts under plain `cargo test`),
    // and nothing else in it reads the environment while this call runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", "1") };

    // Hold the one permit until the test lets go.
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = tokio::spawn(magnetar::password::run_hash_work(move || {
        entered_tx.send(()).expect("the test listens");
        release_rx.recv().expect("the test releases the permit");
        Ok(())
    }));
    tokio::task::spawn_blocking(move || entered_rx.recv())
        .await
        .expect("join")
        .expect("the holder entered the hasher");

    let connection = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    let install = tokio::spawn(init_magnetar(MagnetarConfig::from_sea_orm(connection)));
    // Either the installation reaches the gate and waits, or it finishes
    // with the permit still held. Decide which, release the holder, and
    // only then judge: a failing assertion must never leave the holder
    // blocked on the pool.
    let finished_early = loop {
        if install.is_finished() {
            break true;
        }
        if magnetar::password::hash_work_waiting() >= 1 {
            break false;
        }
        tokio::task::yield_now().await;
    };
    release_tx.send(()).expect("the holder listens");
    holder.await.expect("join").expect("the held work returns");
    let installed = install.await.expect("join");
    assert!(
        !finished_early,
        "the installation finished while the only permit was held: its warm-up hashes ran outside the limit"
    );
    installed.expect("install the default Magnetar engines once the permit is free");
}
