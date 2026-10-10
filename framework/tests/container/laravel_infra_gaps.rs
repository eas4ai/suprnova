//! Laravel infrastructure gaps owned by the container suite: `#[service]`
//! bindings chosen by environment, and lazy singletons.
//!
//! An environment test runs its body alone in a child process started with
//! `APP_ENV` set, because `#[service]` bindings land in the process-global
//! container and the environment is read from the process environment.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use suprnova::container::Container;
use suprnova::testing::TestContainer;
use suprnova::{App, service};

// --- #[service] bindings chosen by environment -----------------------------

#[service(impl = RealMailer, bind(FakeMailer, env = ["testing"]))]
pub trait Mailer {
    fn name(&self) -> &'static str;
}

#[derive(Default)]
pub struct RealMailer;
impl Mailer for RealMailer {
    fn name(&self) -> &'static str {
        "real"
    }
}

#[derive(Default)]
pub struct FakeMailer;
impl Mailer for FakeMailer {
    fn name(&self) -> &'static str {
        "fake"
    }
}

#[service(
    impl = RealStorage,
    bind(LocalStorage, env = ["local"]),
    bind(StagingStorage, env = ["stag*", "qa-*"]),
    bind(SecondStorage, env = ["staging"])
)]
pub trait Storage {
    fn name(&self) -> &'static str;
}

#[derive(Default)]
pub struct RealStorage;
impl Storage for RealStorage {
    fn name(&self) -> &'static str {
        "real"
    }
}

#[derive(Default)]
pub struct LocalStorage;
impl Storage for LocalStorage {
    fn name(&self) -> &'static str {
        "local"
    }
}

#[derive(Default)]
pub struct StagingStorage;
impl Storage for StagingStorage {
    fn name(&self) -> &'static str {
        "staging"
    }
}

#[derive(Default)]
pub struct SecondStorage;
impl Storage for SecondStorage {
    fn name(&self) -> &'static str {
        "second"
    }
}

/// A service with environment entries and no `impl`: outside its
/// environments it is not bound at all.
#[service(bind(DebugProbe, env = ["local", "testing"]))]
pub trait Probe {
    fn name(&self) -> &'static str;
}

#[derive(Default)]
pub struct DebugProbe;
impl Probe for DebugProbe {
    fn name(&self) -> &'static str {
        "debug"
    }
}

/// Run the `_child` test `name` alone in a child process whose `APP_ENV`
/// is `app_env`, and fail unless it ran and passed.
fn run_in(app_env: &str, name: &str) {
    let child = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(name)
            .env("APP_ENV", app_env)
            .spawn()
            .expect("spawn the child process")
    };
    let output = child.wait_with_output().expect("wait for the child");
    crate::own_process::assert_child_passed(&output);
}

fn booted() {
    App::boot_services().expect("the services boot");
}

#[test]
fn testing_binds_the_entry_for_testing() {
    run_in(
        "testing",
        "laravel_infra_gaps::testing_binds_the_entry_for_testing_child",
    );
}

#[test]
fn testing_binds_the_entry_for_testing_child() {
    if !crate::own_process::is_child() {
        return;
    }
    booted();
    assert_eq!(App::make::<dyn Mailer>().unwrap().name(), "fake");
    assert_eq!(App::make::<dyn Storage>().unwrap().name(), "real");
    assert_eq!(App::make::<dyn Probe>().unwrap().name(), "debug");
}

#[test]
fn production_binds_impl_when_no_entry_matches() {
    run_in(
        "production",
        "laravel_infra_gaps::production_binds_impl_when_no_entry_matches_child",
    );
}

#[test]
fn production_binds_impl_when_no_entry_matches_child() {
    if !crate::own_process::is_child() {
        return;
    }
    booted();
    assert_eq!(App::make::<dyn Mailer>().unwrap().name(), "real");
    assert_eq!(
        App::make::<dyn Storage>().unwrap().name(),
        "real",
        "an env = [\"local\"] entry is not chosen in production"
    );
    assert!(
        App::make::<dyn Probe>().is_none(),
        "a service without `impl` binds nothing when no entry matches"
    );
}

#[test]
fn a_star_matches_any_run_and_the_first_matching_entry_wins() {
    run_in(
        "staging",
        "laravel_infra_gaps::a_star_matches_any_run_and_the_first_matching_entry_wins_child",
    );
}

#[test]
fn a_star_matches_any_run_and_the_first_matching_entry_wins_child() {
    if !crate::own_process::is_child() {
        return;
    }
    booted();
    assert_eq!(App::make::<dyn Storage>().unwrap().name(), "staging");
}

#[test]
fn a_custom_environment_matches_a_pattern() {
    run_in(
        "qa-eu",
        "laravel_infra_gaps::a_custom_environment_matches_a_pattern_child",
    );
}

#[test]
fn a_custom_environment_matches_a_pattern_child() {
    if !crate::own_process::is_child() {
        return;
    }
    booted();
    assert_eq!(App::make::<dyn Storage>().unwrap().name(), "staging");
    assert_eq!(App::make::<dyn Mailer>().unwrap().name(), "real");
}

#[test]
fn a_binding_installed_before_boot_still_wins() {
    run_in(
        "testing",
        "laravel_infra_gaps::a_binding_installed_before_boot_still_wins_child",
    );
}

#[test]
fn a_binding_installed_before_boot_still_wins_child() {
    if !crate::own_process::is_child() {
        return;
    }
    App::bind::<dyn Mailer>(Arc::new(RealMailer));
    booted();
    assert_eq!(App::make::<dyn Mailer>().unwrap().name(), "real");
}

// --- Lazy singletons --------------------------------------------------------

#[derive(Debug)]
struct Expensive {
    built: usize,
}

#[test]
fn a_lazy_singleton_is_built_on_its_first_resolve_and_shared() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    App::singleton_lazy(move || {
        let built = counted.fetch_add(1, Ordering::SeqCst) + 1;
        Arc::new(Expensive { built })
    });
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "the factory ran before a resolve"
    );

    let first = App::get::<Arc<Expensive>>().expect("resolves");
    let second = App::get::<Arc<Expensive>>().expect("resolves");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the factory ran twice");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.built, 1);
}

trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        42
    }
}

#[test]
fn a_lazy_singleton_can_be_a_trait_object() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    App::singleton_lazy(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Arc::new(FixedClock) as Arc<dyn Clock>
    });
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    let first = App::make::<dyn Clock>().expect("resolves");
    let second = App::resolve_make::<dyn Clock>().expect("resolves");
    assert_eq!(first.now(), 42);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[derive(Clone, Debug)]
struct SelfReferential {
    nested: Option<String>,
}

#[test]
fn a_factory_that_resolves_its_own_type_answers_an_error() {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        App::singleton_lazy(|| SelfReferential {
            nested: App::resolve::<SelfReferential>()
                .err()
                .map(|e| e.to_string()),
        });
        let outer = App::resolve::<SelfReferential>();
        let _ = done.send(outer.map(|value| value.nested));
    });
    let outcome = finished
        .recv_timeout(Duration::from_secs(10))
        .expect("resolving a factory that resolves itself hung");
    let nested = outcome
        .expect("the outer resolve answers the built value")
        .expect("the nested resolve answered an error");
    assert!(nested.contains("SelfReferential"), "{nested}");
}

#[derive(Clone, Debug, PartialEq)]
struct Configured(&'static str);

#[test]
fn singleton_lazy_if_absent_registers_once_and_answers_whether_it_did() {
    let runs = Arc::new(AtomicUsize::new(0));
    let first_runs = runs.clone();
    let second_runs = runs.clone();
    assert!(App::singleton_lazy_if_absent(move || {
        first_runs.fetch_add(1, Ordering::SeqCst);
        Configured("first")
    }));
    assert!(!App::singleton_lazy_if_absent(move || {
        second_runs.fetch_add(1, Ordering::SeqCst);
        Configured("second")
    }));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert_eq!(App::get::<Configured>(), Some(Configured("first")));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[derive(Clone, Debug, PartialEq)]
struct AlreadyThere(u8);

#[test]
fn singleton_lazy_if_absent_keeps_a_binding_already_there() {
    App::singleton(AlreadyThere(1));
    assert!(!App::singleton_lazy_if_absent(|| AlreadyThere(2)));
    assert_eq!(App::get::<AlreadyThere>(), Some(AlreadyThere(1)));
}

#[derive(Debug)]
struct Contended;

#[test]
fn two_threads_resolving_at_once_share_one_build() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let (started, building) = mpsc::channel::<()>();
    let (release, go) = mpsc::channel::<()>();
    let go = std::sync::Mutex::new(go);
    App::singleton_lazy(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let _ = started.send(());
        let _ = go
            .lock()
            .expect("go lock")
            .recv_timeout(Duration::from_secs(10));
        Arc::new(Contended)
    });
    let first = std::thread::spawn(App::get::<Arc<Contended>>);
    building
        .recv_timeout(Duration::from_secs(10))
        .expect("the first resolve started building");
    let second = std::thread::spawn(App::get::<Arc<Contended>>);
    // Give the second resolver time to reach the value being built.
    std::thread::sleep(Duration::from_millis(50));
    release.send(()).expect("release the factory");
    let first = first.join().expect("first thread").expect("resolves");
    let second = second.join().expect("second thread").expect("resolves");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn a_container_builds_a_lazy_singleton_on_its_first_resolve() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let mut container = Container::new();
    container.singleton_lazy(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        Configured("container")
    });
    assert!(!container.singleton_lazy_if_absent(|| Configured("again")));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    assert_eq!(container.get::<Configured>(), Some(Configured("container")));
    assert_eq!(container.get::<Configured>(), Some(Configured("container")));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[derive(Clone, Debug, PartialEq)]
struct TestOnly(&'static str);

#[test]
fn a_test_container_takes_a_lazy_singleton() {
    let _guard = TestContainer::fake();
    TestContainer::singleton_lazy(|| TestOnly("fake"));
    assert_eq!(App::get::<TestOnly>(), Some(TestOnly("fake")));
}
