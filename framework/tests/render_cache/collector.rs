//! The collector is task-scoped, nestable, cleaned up, and never leaks into
//! detached tasks; framework reads register automatically. This file also
//! pins the bound at its exact edge, the R29 record-key encoding, the
//! incomplete-report signal on an unencodable dependency, and each
//! production read hook that has no other test coverage.
//!
//! Every test below that asserts on the content bucket opens its scope
//! with `collector::begin_handler()`, the marker the Live completion
//! middleware sets on a real request just before the route handler runs.
//! Without it a scope starts in the gate bucket and those reads would be
//! recorded there instead; see the collector's own "Attribution" section.

use serial_test::serial;
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::collector::{self, Collector, current_report};
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};
use suprnova_live::render_cache::generation::MAX_OBSERVATIONS;

#[tokio::test]
async fn observations_stay_within_their_scope_and_detached_tasks_see_none() {
    let report = Collector::scope(async {
        collector::begin_handler();
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
async fn nested_scopes_report_independently() {
    let outer = Collector::scope(async {
        collector::begin_handler();
        collector::observe(DependencyIdentity::table("outer"));
        let inner = Collector::scope(async {
            collector::begin_handler();
            collector::observe(DependencyIdentity::table("inner"));
            collector::current_report().expect("inner")
        })
        .await;
        assert_eq!(inner.observed, vec![DependencyIdentity::table("inner")]);
        collector::current_report().expect("outer")
    })
    .await;
    assert_eq!(outer.observed, vec![DependencyIdentity::table("outer")]);
}

/// Pins the bound at its exact edge rather than merely past it: changing
/// `>=` to `>` in `observe`, or dropping the cap by one, would still pass a
/// "loop 5000 times and assert overflowed" test but would fail this one.
#[tokio::test]
async fn observation_is_exact_at_the_cap_and_overflows_one_past_it() {
    let at_cap = Collector::scope(async {
        collector::begin_handler();
        for index in 0..MAX_OBSERVATIONS - 1 {
            collector::observe(DependencyIdentity::record(
                "t",
                index.to_string().as_bytes(),
            ));
        }
        collector::current_report().expect("report")
    })
    .await;
    assert!(
        !at_cap.context.overflowed,
        "exactly at the cap the report must not be overflowed"
    );
    assert_eq!(at_cap.observed.len(), MAX_OBSERVATIONS - 1);
    assert_eq!(
        at_cap.storable().map(<[_]>::len),
        Some(MAX_OBSERVATIONS - 1)
    );

    let one_past = Collector::scope(async {
        collector::begin_handler();
        for index in 0..MAX_OBSERVATIONS {
            collector::observe(DependencyIdentity::record(
                "t",
                index.to_string().as_bytes(),
            ));
        }
        collector::current_report().expect("report")
    })
    .await;
    assert!(
        one_past.context.overflowed,
        "one identity past the cap must overflow"
    );
    assert!(
        one_past.storable().is_none(),
        "an overflowed report must be unusable, not merely truncated"
    );
}

#[tokio::test]
async fn oversized_table_name_marks_the_report_incomplete_instead_of_vanishing() {
    let long_name = "t".repeat(200);
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::observe_table_read(&long_name);
        collector::current_report().expect("report")
    })
    .await;
    assert!(
        report.context.overflowed,
        "a table name over the bound must mark the report incomplete"
    );
    assert!(report.storable().is_none());
    assert!(
        report.observed.is_empty(),
        "no identity should silently appear for a name that could not be encoded"
    );
}

#[tokio::test]
async fn oversized_record_key_marks_the_report_incomplete_instead_of_vanishing() {
    let huge_key = vec![0u8; 600];
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::observe_record_read("t", &huge_key);
        collector::current_report().expect("report")
    })
    .await;
    assert!(
        report.context.overflowed,
        "a primary key over the bound must mark the report incomplete"
    );
    assert!(report.storable().is_none());
    assert!(report.observed.is_empty());
}

/// Pins the R29 encoding: the write side (Task 12) must build its record
/// identity through [`collector::record_identity`], so this function's
/// output for these three shapes is a contract, not an implementation
/// detail.
#[test]
fn record_identity_pins_the_json_key_encoding_for_three_shapes() {
    assert_eq!(
        collector::record_identity("t", &serde_json::json!(42)),
        Some(DependencyIdentity::record("t", b"42"))
    );
    assert_eq!(
        collector::record_identity("t", &serde_json::json!("abc")),
        Some(DependencyIdentity::record("t", b"\"abc\"")),
        "quotes are retained: a string primary key is not the same bytes as its bare text"
    );
    let id = uuid::Uuid::new_v4();
    let expected = format!("\"{id}\"");
    assert_eq!(
        collector::record_identity("t", &serde_json::to_value(id).expect("uuid to json")),
        Some(DependencyIdentity::record("t", expected.as_bytes()))
    );
}

// `suprnova::Lang` exists only with the `localization` feature, which the
// minimal profile checked by `scripts/check-feature-matrix.sh` leaves off.
// The whole test is gated rather than only the `Lang::locale()` line,
// because the locale read is one of the three framework reads this test
// exists to pin and a version of it missing that read would be a different,
// weaker test.
#[cfg(feature = "localization")]
#[tokio::test]
async fn framework_reads_register_automatically() {
    let report = Collector::scope(async {
        collector::begin_handler();
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

#[tokio::test]
async fn gate_inspect_observes_authorization() {
    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = suprnova::Gate::inspect("render-cache-collector-probe-action", &(), &());
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.context.authorization_read);
}

#[tokio::test]
async fn gate_inspect_async_observes_authorization() {
    let report = Collector::scope(async {
        collector::begin_handler();
        let _ =
            suprnova::Gate::inspect_async("render-cache-collector-probe-action", &(), &()).await;
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.context.authorization_read);
}

/// `raw` bypasses `inspect` entirely (it preserves the "undefined" case as
/// `None` instead of normalizing to a default deny), so it needs its own
/// hook rather than inheriting `inspect`'s. This test fails if that hook is
/// removed: nothing else on this call path sets `authorization_read`.
#[tokio::test]
async fn gate_raw_observes_authorization() {
    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = suprnova::Gate::raw("render-cache-collector-probe-action", &(), &());
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.context.authorization_read);
}

/// Async sibling of [`gate_raw_observes_authorization`]; same reasoning.
#[tokio::test]
async fn gate_raw_async_observes_authorization() {
    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = suprnova::Gate::raw_async("render-cache-collector-probe-action", &(), &()).await;
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.context.authorization_read);
}

// ---- Attribution: gate, content, slot ---------------------------------
//
// A scope starts in the gate bucket; `begin_handler` switches it to
// content; `slot_scope` counts reads and records them nowhere. Only a
// stitched route classifies from content alone - every other route folds
// the gate bucket back in and sees exactly the undivided report it saw
// before attribution existed.

#[tokio::test]
async fn reads_before_begin_handler_land_in_the_gate_bucket_and_after_it_in_content() {
    let report = Collector::scope(async {
        collector::observe_principal_value("alice");
        collector::observe_table_read("users");
        collector::begin_handler();
        collector::observe_table_read("posts");
        collector::observe_session_read();
        current_report().expect("report")
    })
    .await;
    assert!(report.gate.context.principal_read);
    assert!(report.gate.context.principal_material.contains("alice"));
    assert_eq!(
        report.gate.observed.len(),
        1,
        "the users read is a gate read"
    );
    assert!(!report.context.principal_read, "content saw no principal");
    assert!(report.context.session_read);
    assert_eq!(report.observed.len(), 1, "the posts read is a content read");
}

#[tokio::test]
async fn begin_handler_is_idempotent_and_a_noop_outside_a_scope() {
    collector::begin_handler();
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::observe_table_read("a");
        collector::begin_handler();
        collector::observe_table_read("b");
        current_report().expect("report")
    })
    .await;
    assert_eq!(report.observed.len(), 2);
    assert!(report.gate.observed.is_empty());
}

#[tokio::test]
async fn slot_scope_reads_are_counted_and_recorded_nowhere_else() {
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::slot_scope(async {
            collector::observe_principal_value("alice");
            collector::observe_table_read("counters");
            collector::observe_session_read();
        })
        .await;
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    assert_eq!(report.slot_reads, 3);
    assert!(!report.context.principal_read);
    assert!(!report.context.session_read);
    assert_eq!(report.observed.len(), 1);
    assert!(report.gate.observed.is_empty());
}

#[tokio::test]
async fn a_nested_slot_scope_stays_in_the_slot_bucket_and_restores_exactly_once() {
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::slot_scope(async {
            collector::observe_table_read("outer_slot");
            collector::slot_scope(async {
                collector::observe_table_read("inner_slot");
            })
            .await;
            // Still inside the outer slot scope: the inner one restored the
            // slot bucket it found, not the content bucket underneath both.
            collector::observe_table_read("after_inner");
        })
        .await;
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    assert_eq!(
        report.slot_reads, 3,
        "every read inside either scope is a slot read"
    );
    assert_eq!(
        report.observed.len(),
        1,
        "only the read after the outermost scope is recorded, and it is a content read"
    );
    assert!(
        report.gate.observed.is_empty(),
        "the outermost scope restored content, not gate"
    );
}

#[tokio::test]
async fn a_slot_scope_that_begins_the_handler_inside_it_restores_to_content() {
    let report = Collector::scope(async {
        // The scope starts in the gate bucket, as a real request does, and
        // the handler boundary is crossed while the slot scope is open.
        collector::slot_scope(async {
            collector::begin_handler();
            collector::observe_table_read("counters");
        })
        .await;
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    assert!(report.handler_began);
    assert_eq!(report.slot_reads, 1);
    assert_eq!(
        report.observed.len(),
        1,
        "the read after the scope is a content read: restoring the gate bucket the scope \
         found would have recorded it as a gate read the handler never made"
    );
    assert!(report.gate.observed.is_empty());
}

#[tokio::test]
async fn folding_the_gate_into_content_reproduces_the_undivided_report() {
    let mut report = Collector::scope(async {
        collector::observe_principal_value("alice");
        collector::observe_table_read("users");
        collector::begin_handler();
        collector::observe_table_read("users");
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    report.fold_gate_into_content();
    assert!(report.context.principal_read);
    assert!(report.context.principal_material.contains("alice"));
    let names: Vec<String> = report
        .observed
        .iter()
        .map(|identity| format!("{identity:?}"))
        .collect();
    assert_eq!(
        report.observed.len(),
        2,
        "users deduplicated across buckets, gate first: {names:?}"
    );
    assert!(report.gate.observed.is_empty());
}

/// Task 8: the shared bound counts distinct identities, not read events.
/// A gate that reads exactly what the handler goes on to read folds into
/// one observation per identity, so it must cost the budget one slot per
/// identity and not two - otherwise a report whose folded set sits well
/// inside the window is declared unstorable and the route silently stops
/// being cached.
#[tokio::test]
async fn an_identity_read_in_both_buckets_costs_the_bound_one_slot_not_two() {
    let mut report = Collector::scope(async {
        // No `begin_handler` yet, so these land in the gate bucket.
        for index in 0..MAX_OBSERVATIONS - 1 {
            collector::observe(DependencyIdentity::record(
                "t",
                index.to_string().as_bytes(),
            ));
        }
        collector::begin_handler();
        for index in 0..MAX_OBSERVATIONS - 1 {
            collector::observe(DependencyIdentity::record(
                "t",
                index.to_string().as_bytes(),
            ));
        }
        current_report().expect("report")
    })
    .await;
    assert!(
        !report.context.overflowed,
        "the same identities read in both buckets are one observation each, not two"
    );
    report.fold_gate_into_content();
    assert_eq!(
        report.storable().map(<[_]>::len),
        Some(MAX_OBSERVATIONS - 1),
        "the folded set is exactly the distinct identities the two buckets saw"
    );
}

/// The other half of the bound: counting distinct identities must not
/// become counting nothing. One identity past the cap still overflows, and
/// it does so whichever bucket the last one landed in.
#[tokio::test]
async fn a_distinct_identity_past_the_bound_still_overflows_across_buckets() {
    let report = Collector::scope(async {
        for index in 0..MAX_OBSERVATIONS - 1 {
            collector::observe(DependencyIdentity::record(
                "gate",
                index.to_string().as_bytes(),
            ));
        }
        collector::begin_handler();
        collector::observe(DependencyIdentity::record("content", b"0"));
        current_report().expect("report")
    })
    .await;
    assert!(
        report.context.overflowed,
        "the cap counts distinct identities, and this request read one too many"
    );
    assert!(report.storable().is_none());
}

/// Task 8: a read the collector cannot name, made inside a slot, is a slot
/// read - counted where slot reads are counted and recorded nowhere. It
/// must not spoil the shell's report: a slot's reads are re-run on every
/// hit and are in no bucket, so they say nothing about what the stored
/// shell depends on, and a route that keeps an identity-bound island
/// without stitching is already declined whole.
#[tokio::test]
async fn an_unnameable_read_inside_a_slot_is_counted_there_and_spoils_no_report() {
    let long_name = "t".repeat(200);
    let report = Collector::scope(async {
        collector::begin_handler();
        collector::observe_table_read("posts");
        collector::slot_scope(async {
            collector::observe_unobservable_read();
            collector::observe_table_read(&long_name);
        })
        .await;
        current_report().expect("report")
    })
    .await;
    assert!(
        !report.context.overflowed,
        "a slot's unnameable read is not the shell's problem"
    );
    assert_eq!(
        report.storable().map(<[_]>::len),
        Some(1),
        "the shell still stores on exactly what the handler read"
    );
    assert_eq!(
        report.slot_reads, 2,
        "both unnameable reads are counted where every slot read is counted"
    );

    // The control: the same two reads outside a slot still spoil the report,
    // so the exemption is the slot bucket and not the reads themselves.
    let spoiled = Collector::scope(async {
        collector::begin_handler();
        collector::observe_unobservable_read();
        current_report().expect("report")
    })
    .await;
    assert!(spoiled.context.overflowed);
    assert!(spoiled.storable().is_none());
    let spoiled = Collector::scope(async {
        collector::begin_handler();
        collector::observe_table_read(&long_name);
        current_report().expect("report")
    })
    .await;
    assert!(spoiled.context.overflowed);
    assert!(spoiled.storable().is_none());
}

#[tokio::test]
async fn overflow_inside_any_bucket_marks_the_whole_report() {
    let report = Collector::scope(async {
        collector::observe_unobservable_read();
        collector::begin_handler();
        current_report().expect("report")
    })
    .await;
    assert!(report.context.overflowed);
    assert!(report.storable().is_none());
}

#[tokio::test]
async fn a_scope_whose_handler_never_began_says_so_and_folding_keeps_the_answer() {
    let mut report = Collector::scope(async {
        collector::observe_principal_value("alice");
        collector::observe_table_read("users");
        current_report().expect("report")
    })
    .await;
    assert!(
        !report.handler_began,
        "nothing marked the handler boundary, so every read was a gate read"
    );
    report.fold_gate_into_content();
    assert!(
        !report.handler_began,
        "folding moves reads between buckets; it does not invent a handler"
    );
}

#[tokio::test]
async fn a_scope_that_began_its_handler_says_so_and_folding_keeps_the_answer() {
    let mut report = Collector::scope(async {
        collector::observe_table_read("users");
        collector::begin_handler();
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    assert!(report.handler_began);
    report.fold_gate_into_content();
    assert!(
        report.handler_began,
        "the flag records what the request did, not which bucket a read landed in"
    );
}

// ---- Eloquent read-seam coverage -------------------------------------
//
// Each test below exercises exactly one production hook end to end
// against a real (in-memory) database, rather than calling the collector
// functions directly, so removing the corresponding hook line makes the
// test fail rather than merely narrowing coverage.

const TABLE: &str = "render_cache_collector_probe";

#[model(table = "render_cache_collector_probe", timestamps = false)]
pub struct Probe {
    pub id: i64,
    pub name: String,
    pub amount: f64,
}

async fn migrate(db: &TestDatabase) {
    db.execute_unprepared(
        r#"CREATE TABLE render_cache_collector_probe (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            amount REAL NOT NULL
        )"#,
    )
    .await
    .expect("create table");
}

#[tokio::test]
#[serial]
async fn eloquent_get_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::query().get().await.expect("get");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_aggregate_value_observes_the_table_through_count() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::query().count().await.expect("count");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

/// The table is not migrated on purpose: the COUNT phase fails and
/// `paginate_using` returns before it ever reaches its page-phase `get()`
/// call. If this were observed only through `get()` (as it would be if
/// `paginate_using`'s own hook were deleted), the report would see
/// nothing here.
#[tokio::test]
#[serial]
async fn eloquent_paginate_using_observes_the_table_even_when_the_count_query_fails() {
    let _db = TestDatabase::sqlite_memory().await.expect("sqlite");

    let report = Collector::scope(async {
        collector::begin_handler();
        let result = Probe::query().paginate_using("page", 10).await;
        assert!(
            result.is_err(),
            "the COUNT query must fail against a table that was never created"
        );
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_find_observes_the_table_and_the_record() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    let created = Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::find(created.id).await.expect("find");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
    let expected_record = DependencyIdentity::record(TABLE, created.id.to_string().as_bytes());
    assert!(
        report.observed.contains(&expected_record),
        "find must observe the record built from the model's own JSON-encoded primary key"
    );
}

#[tokio::test]
#[serial]
async fn eloquent_find_many_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    let a = Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");
    let b = Probe::create(attrs!(name: "b", amount: 2.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::find_many([a.id, b.id]).await.expect("find_many");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_all_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::all().await.expect("all");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_aggregate_optional_observes_the_table_through_min() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _: Option<f64> = Probe::query().min("amount").await.expect("min");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_value_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _: Option<f64> = Probe::query().value("amount").await.expect("value");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_pluck_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _: Vec<String> = Probe::query().pluck("name").await.expect("pluck");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_pluck_keyed_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _: std::collections::HashMap<i64, String> = Probe::query()
            .pluck_keyed("id", "name")
            .await
            .expect("pluck_keyed");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_model_keys_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _ = Probe::query().model_keys().await.expect("model_keys");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}

#[tokio::test]
#[serial]
async fn eloquent_sole_value_observes_the_table() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;
    Probe::create(attrs!(name: "a", amount: 1.0))
        .await
        .expect("create");

    let report = Collector::scope(async {
        collector::begin_handler();
        let _: f64 = Probe::query()
            .sole_value("amount")
            .await
            .expect("sole_value");
        collector::current_report().expect("report")
    })
    .await;
    assert!(report.observed.contains(&DependencyIdentity::table(TABLE)));
}
