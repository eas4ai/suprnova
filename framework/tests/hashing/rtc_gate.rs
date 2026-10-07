//! RTC-001: password hash work runs under one process-wide limit, and the
//! excess waits as tasks. A hasher that waits for the test's signal shows
//! how many pieces of work are inside the hasher at once; with the limit at
//! two, a third piece must not enter until one of the first two returns.
//!
//! Each test installs a hasher and sets `HASH_MAX_CONCURRENCY`, both of
//! which last for the life of the process, so each runs alone in a child
//! process (see `own_process`). Under nextest every test already has its
//! own process; the child costs one more start of the binary.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use suprnova::FrameworkError;
use suprnova::hashing::{self, Algorithm, HashConfig, Hasher};

/// A hasher that counts the pieces of work inside it and holds each until
/// the test opens the gate.
struct Gated {
    entered: Arc<AtomicUsize>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Hasher for Gated {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Bcrypt
    }

    fn hash(&self, password: &str) -> Result<String, FrameworkError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let (open, signal) = &*self.gate;
        let mut open = open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*open {
            open = signal
                .wait(open)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        Ok(format!("$gated${password}"))
    }

    fn verify(&self, password: &str, hash: &str) -> Result<bool, FrameworkError> {
        Ok(hash == format!("$gated${password}"))
    }

    fn needs_rehash(&self, _hash: &str) -> bool {
        false
    }
}

async fn wait_until(entered: &AtomicUsize, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while entered.load(Ordering::SeqCst) < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "only {} pieces of hash work entered the hasher",
            entered.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Set `HASH_MAX_CONCURRENCY` and install the gated hasher. Returns the
/// count of pieces inside the hasher and the gate that holds them.
fn install_gated(limit: &str) -> (Arc<AtomicUsize>, Arc<(Mutex<bool>, Condvar)>) {
    // SAFETY: the calling test runs alone in a child process (nextest, or
    // the child `own_process_async::delegate` starts under plain `cargo
    // test`), and nothing else in it reads the environment while this call
    // runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", limit) };
    let entered = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    hashing::set_default_driver(Box::new(Gated {
        entered: entered.clone(),
        gate: gate.clone(),
    }))
    .expect("install the gated hasher");
    (entered, gate)
}

/// Let every piece of work inside the gated hasher return.
fn open_gate(gate: &(Mutex<bool>, Condvar)) {
    let (open, signal) = gate;
    *open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    signal.notify_all();
}

/// RTC-001: with `HASH_MAX_CONCURRENCY=2`, the third piece of hash work
/// waits outside the hasher until one of the first two returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_hash_work_past_the_limit_waits_as_a_task() {
    if crate::own_process_async::delegate(
        module_path!(),
        "rtc_hash_work_past_the_limit_waits_as_a_task",
    )
    .await
    {
        return;
    }
    let (entered, gate) = install_gated("2");

    let work: Vec<_> = (0..3)
        .map(|i| tokio::spawn(async move { hashing::hash_async(&format!("password {i}")).await }))
        .collect();
    wait_until(&entered, 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let inside = entered.load(Ordering::SeqCst);

    // Open the gate before judging, so every blocking task returns and the
    // runtime can shut down whatever the verdict is.
    open_gate(&gate);
    for piece in work {
        let hash = piece.await.expect("the task ran").expect("the hash");
        assert!(hash.starts_with("$gated$"));
    }
    assert_eq!(
        entered.load(Ordering::SeqCst),
        3,
        "every piece ran once the gate opened"
    );
    assert_eq!(
        inside, 2,
        "{inside} pieces of hash work were inside the hasher while two held the permits"
    );
}

/// RTC-001: Magnetar's hash work waits under the same limit. With
/// `HASH_MAX_CONCURRENCY=2` and two framework pieces inside the hasher, a
/// piece sent through Magnetar's `run_hash_work` does not start until the
/// gate opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_magnetar_hash_work_shares_the_limit() {
    if crate::own_process_async::delegate(module_path!(), "rtc_magnetar_hash_work_shares_the_limit")
        .await
    {
        return;
    }
    let (entered, gate) = install_gated("2");

    let framework: Vec<_> = (0..2)
        .map(|i| tokio::spawn(async move { hashing::hash_async(&format!("password {i}")).await }))
        .collect();
    wait_until(&entered, 2).await;

    let magnetar_entered = Arc::new(AtomicBool::new(false));
    let flag = magnetar_entered.clone();
    let magnetar_piece = tokio::spawn(magnetar::password::run_hash_work(move || {
        flag.store(true, Ordering::SeqCst);
        Ok(())
    }));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let entered_early = magnetar_entered.load(Ordering::SeqCst);

    // Open the gate before judging, as above.
    open_gate(&gate);
    for piece in framework {
        piece.await.expect("the task ran").expect("the hash");
    }
    magnetar_piece
        .await
        .expect("the task ran")
        .expect("the Magnetar work");
    assert!(
        magnetar_entered.load(Ordering::SeqCst),
        "the Magnetar piece ran once the gate opened"
    );
    assert!(
        !entered_early,
        "a piece sent through Magnetar ran while the framework's two pieces held the permits"
    );
}

/// RTC-001: the permit travels with the work. With
/// `HASH_MAX_CONCURRENCY=1`, piece A's caller stops waiting while A is
/// inside the hasher; A's work keeps the permit, so piece B waits outside
/// until its own caller gives up too. Once A returns, its permit is free
/// and neither dropped caller holds one: piece C enters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rtc_the_permit_travels_with_the_work() {
    if crate::own_process_async::delegate(module_path!(), "rtc_the_permit_travels_with_the_work")
        .await
    {
        return;
    }
    let (entered, gate) = install_gated("1");

    let a = tokio::spawn(async { hashing::hash_async("password a").await });
    wait_until(&entered, 1).await;
    // A's caller stops waiting. Its work is still inside the hasher.
    a.abort();
    assert!(
        a.await.expect_err("A's caller was aborted").is_cancelled(),
        "A's caller ended by the abort"
    );

    // B waits for the one permit, and its caller gives up after the bounded
    // wait, dropping B's future.
    let b = tokio::time::timeout(
        Duration::from_millis(300),
        hashing::hash_async("password b"),
    )
    .await;
    let inside_while_a_ran = entered.load(Ordering::SeqCst);

    // Open the gate before judging, so A's work returns and frees its permit.
    open_gate(&gate);
    let c = tokio::spawn(async { hashing::hash_async("password c").await });
    // Only a permit leaked by A or B keeps C out past this deadline.
    wait_until(&entered, inside_while_a_ran + 1).await;
    let hash = c.await.expect("the task ran").expect("the hash");
    assert!(hash.starts_with("$gated$"));

    assert!(b.is_err(), "B finished while A's work held the only permit");
    assert_eq!(
        inside_while_a_ran, 1,
        "{inside_while_a_ran} pieces were inside the hasher while A's work held the only permit after its caller stopped waiting"
    );
}

/// RTC-001: `HASH_MAX_CONCURRENCY` that is not a whole number of at least
/// 1 is refused when the hashing configuration loads, and by the first
/// async hash.
#[test]
fn rtc_hash_max_concurrency_refuses_what_is_not_a_whole_number() {
    crate::own_process::run_alone(
        "rtc_gate::rtc_hash_max_concurrency_refuses_what_is_not_a_whole_number_child",
    );
}

#[test]
fn rtc_hash_max_concurrency_refuses_what_is_not_a_whole_number_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    for value in ["0", "two", "-1", "1.5"] {
        // SAFETY: this test runs alone in a child process (see
        // `own_process`), and nothing else in it reads the environment
        // while this call runs.
        unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", value) };
        let Err(error) = HashConfig::from_env() else {
            panic!("HASH_MAX_CONCURRENCY={value} loaded");
        };
        let message = error.to_string();
        assert!(
            message.contains("HASH_MAX_CONCURRENCY") && message.contains(value),
            "the error names the variable and its value: {message}"
        );
    }

    // The driver is built from the same configuration, so it fails too.
    let Err(error) = hashing::default_driver() else {
        panic!("the default driver loaded with HASH_MAX_CONCURRENCY=1.5");
    };
    assert!(
        error.to_string().contains("HASH_MAX_CONCURRENCY"),
        "{error}"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a runtime");
    let error = runtime
        .block_on(hashing::hash_async("password"))
        .expect_err("the first async hash refuses the setting");
    assert!(
        error.to_string().contains("HASH_MAX_CONCURRENCY"),
        "{error}"
    );
}

/// RTC-001: a whole number of at least 1 loads as the limit, and an unset
/// `HASH_MAX_CONCURRENCY` leaves the limit at the host's parallelism.
#[test]
fn rtc_hash_max_concurrency_loads_a_whole_number() {
    crate::own_process::run_alone("rtc_gate::rtc_hash_max_concurrency_loads_a_whole_number_child");
}

#[test]
fn rtc_hash_max_concurrency_loads_a_whole_number_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    // SAFETY: this test runs alone in a child process (see `own_process`),
    // and nothing else in it reads the environment while this call runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", "3") };
    let config = HashConfig::from_env().expect("HASH_MAX_CONCURRENCY=3 loads");
    assert_eq!(config.max_concurrency.map(|limit| limit.get()), Some(3));

    // SAFETY: as above.
    unsafe { std::env::remove_var("HASH_MAX_CONCURRENCY") };
    let config = HashConfig::from_env().expect("an unset HASH_MAX_CONCURRENCY loads");
    assert_eq!(config.max_concurrency, None);
}

/// RTC-001: the limit is fixed by whichever comes first, the configuration
/// or the first piece of hash work. A `HASH_MAX_CONCURRENCY` read after
/// Magnetar's work fixed the limit at its default cannot take effect, and
/// every async hash says so instead of running under a limit the setting
/// does not name.
#[tokio::test]
async fn rtc_a_limit_that_cannot_take_effect_is_refused() {
    if crate::own_process_async::delegate(
        module_path!(),
        "rtc_a_limit_that_cannot_take_effect_is_refused",
    )
    .await
    {
        return;
    }
    magnetar::password::run_hash_work(|| Ok(()))
        .await
        .expect("Magnetar's work runs under the default limit");
    let default = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    // SAFETY: this test runs alone in a child process (nextest, or the
    // child `own_process_async::delegate` starts under plain `cargo test`),
    // and nothing else in it reads the environment while this call runs.
    unsafe { std::env::set_var("HASH_MAX_CONCURRENCY", (default + 1).to_string()) };

    for attempt in 1..=2 {
        let error = hashing::hash_async("password")
            .await
            .expect_err("a limit that cannot take effect is refused");
        let message = error.to_string();
        assert!(
            message.contains("HASH_MAX_CONCURRENCY") && message.contains("already set"),
            "attempt {attempt} names the setting and the limit already in force: {message}"
        );
    }
}
