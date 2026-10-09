//! A manual binding of an `#[injectable]` survives boot without its
//! constructor running.
//!
//! `App::boot_services` promises that a binding installed before boot is
//! kept. It kept the value, but it still ran the generated constructor
//! first, so the constructor's own dependencies had to resolve: an
//! application that bound a complete `OverriddenReport` by hand, and never
//! registered the `ReportStore` only that constructor reads, failed boot.
//!
//! The case needs a dependency that is missing at boot, and every other
//! test in this binary boots services with it present. So the check runs
//! in a child process, where `provide_report_store` registers nothing.

use std::process::Command;

use suprnova::{App, injectable};

const CHILD_MODE: &str = "SUPRNOVA_INJECTABLE_OVERRIDE_CHILD";

/// What only the generated constructor of `OverriddenReport` reads.
#[derive(Clone)]
pub struct ReportStore;

/// Registers `ReportStore` at boot, except in the child process, where it is
/// absent the way an unused dependency of an overridden service is.
fn provide_report_store() -> Result<(), String> {
    if std::env::var_os(CHILD_MODE).is_none() {
        App::singleton_if_absent(ReportStore);
    }
    Ok(())
}

suprnova::inventory::submit! {
    suprnova::container::provider::SingletonEntry {
        register: provide_report_store,
        name: "ReportStore (injectable_override fixture)",
    }
}

#[injectable]
pub struct OverriddenReport {
    #[inject]
    store: ReportStore,
}

#[test]
fn overridden_injectable_child() {
    if std::env::var(CHILD_MODE).is_err() {
        return;
    }
    App::init();
    App::singleton(OverriddenReport { store: ReportStore });
    App::boot_services().expect("a bound injectable needs no constructor at boot");
    let report = App::resolve::<OverriddenReport>().expect("the manual binding is kept");
    let ReportStore = report.store;
}

#[test]
fn a_bound_injectable_does_not_need_its_constructor_dependencies_at_boot() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "injectable_override::overridden_injectable_child",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .output()
        .expect("spawn the child");

    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "child filter matched no test; stdout:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}
