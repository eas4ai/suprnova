//! What `install` and the Live ledger boot accept and refuse, and the
//! Database profile through the real middleware.
//!
//! The adapter tests in `sql` and `redis` prove one store against one
//! backend. What they cannot prove is the wiring: that
//! `RenderCache::install` on a profile actually puts those providers on the
//! request path, that a profile whose schema or endpoint is absent fails at
//! boot with one actionable sentence rather than on every request, and that
//! the operator's own sweep dispatches to the configured tier. That is what
//! this module is for.
//!
//! The Redis profile's own end-to-end proof lives in `redis`, with the rest
//! of the tests that need a live Redis, so one filter selects them all.

use bytes::Bytes;
use hyper::Method;
use suprnova::DB;
use suprnova::StatusCode;
use suprnova::live::{LedgerDriver, verify_ledger_driver_for_test};
use suprnova::render_cache::RenderCache;
use suprnova::render_cache::providers::{SqlInstanceRecordStore, SqlRenderStore};
use suprnova::render_cache::{CoordinatorConfig, L1Config, L1Provider, sweep_l1};
use suprnova_live::identity::{InstanceId, ScopeFingerprint};
use suprnova_live::render_cache::entry::EntryKind;
use suprnova_live::render_cache::store::RenderStore;

use crate::render_cache_stitch_support;
use crate::render_cache_tiers_support;
use render_cache_stitch_support::{
    SEED_ONLY_PATH, STITCHED_PATH, boot_on_the_database_profile_for_test,
    dispatch as dispatch_route, handler_renders, island_tag,
};
use render_cache_tiers_support::{
    boot, boot_without_the_tier_tables, fence, install, key, redis_config_on_a_closed_port,
    tier_config,
};

use super::{
    LedgerDriverEnv, a_second_nodes_ledger, instance_row_count,
    mount_and_act_on_the_identity_bound_island, row_count,
};

// --- Profiles: what `install` and the Live ledger boot accept and refuse ---

#[tokio::test]
async fn the_database_profile_refuses_to_install_without_the_tier_migration() {
    let _db = boot_without_the_tier_tables().await;

    let refused = install(tier_config(
        L1Config::Database {
            max_bytes: 1024 * 1024,
        },
        CoordinatorConfig::Local {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a database L1 tier without its table must not install");
    let message = refused.to_string();
    assert!(
        message.contains("m20260906_000000_create_render_cache_tier_tables"),
        "the refusal names the migration to add: {message}"
    );

    // The coordinator reaches a different table of the same migration, so it
    // has to be refused on its own too - a database coordinator in front of
    // a file L1 is a legitimate shape, and it still needs the tables.
    let refused = install(tier_config(
        L1Config::Disabled,
        CoordinatorConfig::Database {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a database coordinator without its table must not install");
    assert!(
        refused
            .to_string()
            .contains("m20260906_000000_create_render_cache_tier_tables"),
        "{refused}"
    );
}

/// The `L1Provider::Database` arm of the sweep every operator reaches, over
/// a provider this test built.
///
/// [`sweep_l1`] is the body of `RenderCache::sweep` with the runtime lookup
/// lifted out, and it is what this calls: installing a runtime to reach one
/// `match` arm would bind a *process* singleton that every other test in this
/// binary can see, and `RenderCache::sweep` adds nothing over `sweep_l1` but
/// that lookup and the file tier's epoch read.
///
/// The epoch argument is deliberately a value no ledger would return: the
/// database arm must not consult it, and passing one proves the arm ignores
/// what it is given rather than merely that no ledger was queried.
#[tokio::test]
async fn the_database_l1_provider_sweeps_its_own_rows_through_the_facade_body() {
    let _db = boot().await;
    let provider = L1Provider::Database(SqlRenderStore::new(1024 * 1024));

    // A retention of zero makes each row due by the database's own clock the
    // moment it is written, so nothing here waits on a timer: the sweep's
    // `now` is read after the publication's.
    for pattern in ["/facade-a", "/facade-b"] {
        provider
            .publish(
                &key(pattern),
                Bytes::from_static(b"due immediately"),
                fence(1, 1),
                1_000,
                0,
            )
            .await
            .expect("publish");
    }
    assert_eq!(row_count().await, 2);

    let swept = sweep_l1(&provider, 1_000, u64::MAX)
        .await
        .expect("the database tier sweeps");
    assert_eq!(swept.removed, 2);
    assert!(!swept.more_remain);
    assert_eq!(row_count().await, 0);
}

#[tokio::test]
async fn a_redis_profile_whose_endpoint_answers_nothing_refuses_to_install() {
    let _db = boot().await;
    let closed = redis_config_on_a_closed_port();

    let refused = install(tier_config(
        L1Config::Redis {
            url: closed.url.clone(),
            prefix: closed.prefix.clone(),
            max_bytes: 1024 * 1024,
        },
        CoordinatorConfig::Local {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a Redis tier nothing answers must not install");
    let message = refused.to_string();
    assert!(
        message.contains("RENDER_CACHE_REDIS_URL"),
        "the refusal names the setting to fix: {message}"
    );
    assert!(!message.contains("127.0.0.1"), "{message}");
    assert!(!message.contains(&closed.url), "{message}");

    // And the coordinator alone is refused on the same terms.
    let refused = install(tier_config(
        L1Config::Disabled,
        CoordinatorConfig::Redis {
            url: closed.url.clone(),
            prefix: closed.prefix.clone(),
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a Redis coordinator nothing answers must not install");
    assert!(
        refused.to_string().contains("RENDER_CACHE_REDIS_URL"),
        "{refused}"
    );
}

#[tokio::test]
async fn the_live_database_ledger_driver_needs_the_same_migration() {
    let _db = boot_without_the_tier_tables().await;
    let refused = verify_ledger_driver_for_test(&LedgerDriver::Database)
        .await
        .expect_err("a database ledger without its tables must not boot");
    let message = refused.to_string();
    assert!(message.contains("LIVE_LEDGER_DRIVER"), "{message}");
    assert!(
        message.contains("m20260906_000000_create_render_cache_tier_tables"),
        "{message}"
    );
}

#[tokio::test]
async fn the_live_database_ledger_driver_boots_once_the_migration_is_applied() {
    let _db = boot().await;
    verify_ledger_driver_for_test(&LedgerDriver::Database)
        .await
        .expect("the database ledger driver boots against the tier tables");
}

// --- End to end: the Database profile through the middleware and the Live
// --- ledger driver ---
//
// A single-node middleware test proves the wiring - that `RenderCache::install`
// on a profile puts these providers on the request path - and the Live ledger
// driver test proves that an ordinary action commits a revision a second
// handle over the same database reads. Two nodes are two handles over one
// backend, for the reason the adapter modules give: the RenderCache runtime
// and the Live runtime are process singletons, so a second runtime is not
// something a test process can have.

#[tokio::test]
#[serial_test::serial]
async fn the_database_profile_publishes_to_sql_l1_serves_from_it_and_sweeps_through_the_facade() {
    let harness = boot_on_the_database_profile_for_test().await;

    // The publication reaches the shared table, not only this process's L0.
    let first = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        row_count().await,
        1,
        "the entry is a row every node can read"
    );

    // With L0 emptied and the epoch untouched, the next request derives the
    // same key and can only be answered from L1.
    RenderCache::clear_l0_for_test();
    assert!(
        RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
            .await
            .is_none(),
        "precondition: nothing is left in memory"
    );
    let before = handler_renders(SEED_ONLY_PATH);
    let hit = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(hit.status, StatusCode::OK, "{}", hit.text());
    assert_eq!(
        handler_renders(SEED_ONLY_PATH),
        before,
        "an L1 hit runs no handler"
    );
    let promoted = RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
        .await
        .expect("and the entry it served is promoted back into L0");
    assert_eq!(
        promoted.kind,
        EntryKind::Complete,
        "a seed-only route caches whole, with no slot left to fill"
    );
    // A hit is only a hit if it answers with what was published. No handler
    // ran between the two requests, so identical bytes are evidence that the
    // row round-tripped, not that the route renders the same thing twice.
    assert_eq!(
        hit.body, first.body,
        "the L1 hit serves the bytes the first response published"
    );

    // A stitched document's Composite entry round-trips through the same
    // table: the shell comes back from SQL and the island inside it is
    // mounted here and now, for whoever is asking.
    let a1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(a1.status, StatusCode::OK, "{}", a1.text());
    let stored = RenderCache::inspect_route_for_test(STITCHED_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(stored.slots, 1);
    assert_eq!(row_count().await, 2);

    RenderCache::clear_l0_for_test();
    let before = handler_renders(STITCHED_PATH);
    let b1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(b1.status, StatusCode::OK, "{}", b1.text());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "the composite entry came back from SQL and the handler never ran"
    );
    let island_a = island_tag(&a1.text(), "stitch-counter").to_owned();
    let island_b = island_tag(&b1.text(), "stitch-counter").to_owned();
    assert_ne!(
        island_a, island_b,
        "each principal's island is mounted for that principal"
    );
    let shell = |text: &str| text.replace(island_tag(text, "stitch-counter"), "");
    assert_eq!(
        shell(&a1.text()),
        shell(&b1.text()),
        "and the shell around it is the bytes the leader stored"
    );

    // `RenderCache::sweep` is the database arm here. Nothing published above
    // is due yet, and a row another node published with a retention of zero
    // is due by the database's own clock the instant it was written - so this
    // waits on nothing and still proves the facade reached the arm that reads
    // store time rather than a directory or a no-op.
    let swept = RenderCache::sweep().await.expect("sweep");
    assert_eq!(swept.removed, 0);
    assert!(!swept.more_remain);
    assert_eq!(row_count().await, 2);

    SqlRenderStore::new(1024 * 1024)
        .publish(
            &key("/another-node"),
            Bytes::from_static(b"due immediately"),
            fence(1, 1),
            1_000,
            0,
        )
        .await
        .expect("publish");
    assert_eq!(row_count().await, 3);
    let swept = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        swept.removed, 1,
        "the facade dispatched to the database arm"
    );
    assert!(!swept.more_remain);
    assert_eq!(
        row_count().await,
        2,
        "and the rows that are still live are left alone"
    );
}

/// The scope and instance identities of the single row in
/// `suprnova_live_instances`, decoded from the hex the columns store.
///
/// Read out of the table rather than out of the snapshot: the columns are
/// what a second node addresses a record by, so taking the identities from
/// there is what makes the read that follows a genuine second reader of this
/// row rather than a second decoding of the first reader's own state.
async fn the_only_instance_identity() -> (ScopeFingerprint, InstanceId) {
    assert_eq!(instance_row_count().await, 1, "exactly one mounted island");
    let scope: String = DB::scalar("SELECT scope FROM suprnova_live_instances", vec![])
        .await
        .expect("the stored scope");
    let instance: String = DB::scalar("SELECT instance FROM suprnova_live_instances", vec![])
        .await
        .expect("the stored instance identity");
    (
        ScopeFingerprint::from_bytes(&hex::decode(scope).expect("hex")).expect("a scope"),
        InstanceId::from_bytes(&hex::decode(instance).expect("hex")).expect("an instance identity"),
    )
}

#[tokio::test]
#[serial_test::serial]
async fn live_actions_on_the_database_ledger_driver_advance_a_revision_a_second_node_reads() {
    let _env = crate::env_lock::lock_env_async().await;
    let _driver = LedgerDriverEnv::set(&[("LIVE_LEDGER_DRIVER", "database".to_owned())]);
    let _db = boot().await;

    let (_mounted, committed) = mount_and_act_on_the_identity_bound_island().await;

    let (scope, instance) = the_only_instance_identity().await;
    assert_eq!(
        a_second_nodes_ledger(SqlInstanceRecordStore::new())
            .current_accepted_revision(&scope, &instance)
            .await
            .expect("the second node's read"),
        Some(committed),
        "a second process reads the revision this one committed, out of the database"
    );
}
