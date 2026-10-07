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
