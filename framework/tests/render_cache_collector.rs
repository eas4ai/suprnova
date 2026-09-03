//! The collector is task-scoped, nestable, cleaned up, and never leaks into
//! detached tasks; framework reads register automatically.

use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::collector::{self, Collector};

#[tokio::test]
async fn observations_stay_within_their_scope_and_detached_tasks_see_none() {
    let report = Collector::scope(async {
        collector::observe_table_read("posts");
        collector::observe_record_read("posts", b"42");
        collector::observe_principal_read();
        let detached = tokio::spawn(async {
            collector::observe(DependencyIdentity::table("leak"));
            collector::is_active()
        });
        assert!(
            !detached.await.expect("join"),
            "a detached task has no collector"
        );
        collector::current_report().expect("active")
    })
    .await;
    assert!(
        report
            .observed
            .contains(&DependencyIdentity::table("posts"))
    );
    assert!(
        report
            .observed
            .contains(&DependencyIdentity::record("posts", b"42"))
    );
    assert!(!report.observed.contains(&DependencyIdentity::table("leak")));
    assert!(report.context.principal_read);
    assert!(
        collector::current_report().is_none(),
        "nothing leaks past the scope"
    );
}

#[tokio::test]
async fn nested_scopes_report_independently_and_bounded_observation_fails_closed() {
    let outer = Collector::scope(async {
        collector::observe(DependencyIdentity::table("outer"));
        let inner = Collector::scope(async {
            collector::observe(DependencyIdentity::table("inner"));
            collector::current_report().expect("inner")
        })
        .await;
        assert_eq!(inner.observed, vec![DependencyIdentity::table("inner")]);
        collector::current_report().expect("outer")
    })
    .await;
    assert_eq!(outer.observed, vec![DependencyIdentity::table("outer")]);
    let overflow = Collector::scope(async {
        for index in 0..5_000 {
            collector::observe(DependencyIdentity::record(
                "t",
                index.to_string().as_bytes(),
            ));
        }
        collector::current_report().expect("report")
    })
    .await;
    assert!(
        overflow.context.overflowed,
        "past the bound the report is marked so the response is not stored"
    );
}

#[tokio::test]
async fn framework_reads_register_automatically() {
    let report = Collector::scope(async {
        let _ = suprnova::Lang::locale();
        let _ = suprnova::Auth::check();
        let _ = suprnova::session::session();
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::Locale));
    assert!(report.context.principal_read);
    assert!(report.context.session_read);
}
