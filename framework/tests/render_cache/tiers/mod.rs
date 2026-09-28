//! Tier 1 and Tier 2: the database-backed and Redis-backed L1 render store,
//! rebuild lease store, and Live instance record store, the tier migration,
//! and the profiles that install them.
//!
//! One submodule per backend, because the test path is what the gate scripts
//! select on. `sql` carries everything the database tier proves - SQLite
//! unconditionally, PostgreSQL and MySQL through its `live_postgres_*` and
//! `live_mysql_*` tests; `redis` carries everything the Redis tier proves,
//! through `live_redis_*` plus the two tests that need no Redis at all; and
//! `profiles` carries what `install` and the Live ledger boot accept and
//! refuse, and the Database profile through the real middleware.
//! `scripts/check-postgres.sh`, `check-mysql.sh`, and `check-redis.sh`
//! select `tiers::sql::live_postgres`, `tiers::sql::live_mysql`, and
//! `tiers::redis::live_redis`.
//!
//! Every test drives an adapter against a real backend, never a mock. Two
//! handles over one backend stand in for two nodes, which is the only way to
//! prove cross-node behaviour when the runtime that would use it is a
//! process singleton.
//!
//! What stays here rather than in a submodule is what more than one of them
//! needs: the store-time seam the ledger conformance suite runs through, the
//! two row counters that read a table rather than a store, the environment
//! and dogfood glue the Live ledger driver tests share, and the three
//! two-node proofs that are one proof against two stores - written once here
//! and called from `sql` and `redis` with each tier's own handles.

pub mod profiles;
pub mod redis;
pub mod sql;

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::Value;
use suprnova::DB;
use suprnova::StatusCode;
use suprnova::live::production_ledger_limits;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::render_cache::providers::{
    RedisInstanceRecordStore, RedisLeaseStore, SqlInstanceRecordStore, SqlLeaseStore,
};
use suprnova_live::clock::{Clock, SystemClock};
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{
    CasOutcome, DistributedInstanceLedger, InstanceRecordKey, InstanceRecordStore, LedgerError,
    LiveInstanceLedger, PromotionRecordKey, StoredRecord,
};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};
use suprnova_live::render_cache::{
    FencedLeaseCoordinator, LeaseAttempt, LeaseStore, RenderCacheErrorKind,
};
use suprnova_live_test_support::{ControlledClock, ledger_conformance};

use crate::live_dogfood_support;
use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, PRIVATE_DOCUMENT_PATH, build_public_router,
    dispatch as dispatch_live, get as live_get, private_action_request, production_middleware,
    session_cookie,
};

/// Rows currently in the entries table, counted in SQL rather than through
/// the store, so a test can tell "the store reports a miss" from "the row
/// is gone".
async fn row_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM suprnova_render_entries", vec![])
        .await
        .expect("count the entries table")
}

/// Rows currently in the instances table, counted in SQL rather than
/// through the store, so a test can tell "the store reports absence" from
/// "the row is gone".
async fn instance_row_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM suprnova_live_instances", vec![])
        .await
        .expect("count the instances table")
}

/// The lease id and store expiry of an attempt that was expected to win.
fn acquired(attempt: LeaseAttempt) -> (u64, u64) {
    match attempt {
        LeaseAttempt::Acquired {
            lease_id,
            expires_at_ms,
        } => (lease_id, expires_at_ms),
        LeaseAttempt::Held => panic!("the lease was expected to be acquired"),
    }
}

/// A lease store whose view of store time a test can move forward.
///
/// The distributed lease adapters carry the same doc-hidden seam their
/// record stores do, and the two-node lease proof below is written against
/// this rather than against either of them, so one proof runs over the
/// database store and the Redis store.
trait OffsetLeaseStore: LeaseStore {
    fn set_time_offset_for_test(&self, offset_ms: u64);
}

impl OffsetLeaseStore for SqlLeaseStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method, not this trait method: an
        // inherent method wins over a trait method of the same name, so this
        // is a delegation rather than the infinite recursion it reads as.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

impl OffsetLeaseStore for RedisLeaseStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method; see the note on the database
        // store's copy of this delegation.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

// --- Two nodes over one backend, proved once and run against both ---
//
// The three proofs below are the same proof twice over: what differs
// between the database tier and the Redis tier is which store the handles
// are, never what two nodes are entitled to do. Writing them once is what
// keeps the tiers answering the same contract rather than two contracts
// that happen to look alike; each caller adds whatever only its own backend
// can show (a raw row count, for instance) after the shared body returns.

/// One entry, one fence: the node the store admits leads and publishes, the
/// other bypasses and publishes nothing, and the fence refuses the bypassing
/// node even if it tries.
async fn assert_two_nodes_publish_one_entry_and_the_bypassing_node_publishes_nothing<L, R>(
    leader: &FencedLeaseCoordinator<L>,
    peer: &FencedLeaseCoordinator<L>,
    leader_entries: &R,
    peer_entries: &R,
    key: &RenderKey,
) where
    L: LeaseStore,
    R: RenderStore,
{
    let RebuildAdmission::Lead(lease) = leader.admit(key, 1, 0).await.expect("admit") else {
        panic!("the node the store admits leads");
    };
    assert!(
        matches!(
            peer.admit(key, 1, 0).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "the other node renders without publishing"
    );

    let fence = leader
        .publish_token(&lease, 0)
        .await
        .expect("the leader mints exactly one fence");
    assert_eq!(
        leader_entries
            .publish(
                key,
                Bytes::from_static(b"the leader's bytes"),
                fence,
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "one publication is accepted per fence"
    );
    leader.release(*lease).await.expect("release");

    // A `Bypass` carries no lease, so the bypassing node has nothing to mint
    // a fence from and never reaches `publish` at all - which is exactly what
    // the middleware does with it. Were it to publish anyway, under the only
    // fence it could have observed, the store refuses it: an equal fence does
    // not supersede.
    assert_eq!(
        peer_entries
            .publish(
                key,
                Bytes::from_static(b"the bypassing node's bytes"),
                fence,
                2_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced
    );

    assert_eq!(
        peer_entries.inspect().await.expect("inspect").entries,
        1,
        "one fence, one entry, one publication"
    );
    let stored = peer_entries
        .get(key)
        .await
        .expect("get")
        .expect("both nodes read the same entry");
    assert_eq!(
        stored.bytes.as_ref(),
        b"the leader's bytes",
        "the bytes every node serves are the leader's"
    );
    assert_eq!(stored.fence, fence);
    assert_eq!(stored.published_at_ms, 1_000);
}

/// A leader whose lease elapsed by store time is taken over, mints nothing
/// afterwards, and the takeover's bytes are the ones that stand.
async fn assert_an_elapsed_leader_is_fenced_and_the_takeover_publishes_instead<L, R>(
    dying_store: &Arc<L>,
    taking_store: &Arc<L>,
    entries: &R,
    key: &RenderKey,
) where
    L: OffsetLeaseStore + 'static,
    R: RenderStore,
{
    let limits = LocalCoordinatorLimits {
        lease_ms: 1_000,
        max_waiters: 4,
    };
    let dying = FencedLeaseCoordinator::new(Arc::clone(dying_store), limits);
    let taking = FencedLeaseCoordinator::new(Arc::clone(taking_store), limits);

    let RebuildAdmission::Lead(dead) = dying.admit(key, 1, 0).await.expect("admit") else {
        panic!("the first node leads");
    };

    // Store time, never a node's: both handles read the backend's own clock,
    // and moving their offsets is what a real pair of nodes reaches by the
    // leader simply not coming back before its lease ran out. Nothing waits.
    dying_store.set_time_offset_for_test(2_000);
    taking_store.set_time_offset_for_test(2_000);

    let RebuildAdmission::Lead(taken) = taking.admit(key, 1, 5_000).await.expect("admit") else {
        panic!("an elapsed lease is taken over");
    };
    let fence = taking.publish_token(&taken, 5_000).await.expect("token");
    assert_eq!(
        fence.token, 1,
        "the leader died before it minted anything, so this is the key's first \
         token - tokens count publications, not tenures"
    );
    assert_eq!(
        entries
            .publish(
                key,
                Bytes::from_static(b"the takeover's bytes"),
                fence,
                5_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    taking.release(*taken).await.expect("release");

    // The former leader finishes its render and comes back to publish. It is
    // refused before it reaches the store at all, and its result is discarded.
    let refused = dying
        .publish_token(&dead, 5_000)
        .await
        .expect_err("an elapsed lease mints nothing");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    // And its own clock buys it nothing: `publish_token` is answered by store
    // time, so asking as though no time had passed is refused identically.
    let refused = dying
        .publish_token(&dead, 0)
        .await
        .expect_err("a node clock never extends a distributed lease");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    dying.release(*dead).await.expect("release");

    assert_eq!(
        entries
            .get(key)
            .await
            .expect("get")
            .expect("a hit")
            .bytes
            .as_ref(),
        b"the takeover's bytes",
        "the entry is the node that held the lease when it published"
    );
}

/// A restart takes every handle and every byte of in-process state with it,
/// and the record is still there at the version the last process left it at.
///
/// `open` is what a restart is here: it is called once for the process that
/// writes, whose handle is dropped before it is called again for the process
/// that comes up next. A caller passes its own constructor, so the second
/// handle is genuinely built after the first one is gone.
async fn assert_a_restarted_node_reads_the_records_the_previous_one_left<S, F, Fut>(
    key: &InstanceRecordKey,
    expires_at: UnixMillis,
    open: F,
) where
    S: InstanceRecordStore,
    F: Fn() -> Fut,
    Fut: Future<Output = S>,
{
    // Everything the previous process did, inside its own scope: a restart
    // takes every handle with it, and the scope ending is what stands in for
    // that here.
    {
        let before = open().await;
        assert!(
            before
                .insert_if_absent(key, b"written before the restart", expires_at)
                .await
                .expect("insert")
        );
        assert_eq!(
            before
                .compare_and_store(key, 1, b"advanced before the restart", expires_at)
                .await
                .expect("compare and store"),
            CasOutcome::Stored { version: 2 }
        );
    }

    // The record is in the backend, so the process that comes up next finds
    // the key held at exactly the version the last one left it at.
    let after = open().await;
    let stored = after
        .load(key)
        .await
        .expect("load")
        .expect("the record outlived the handle that wrote it");
    assert_eq!(stored.bytes, b"advanced before the restart".to_vec());
    assert_eq!(stored.version, 2);
    assert_eq!(after.count_instances().await.expect("count"), 1);
    assert!(
        !after
            .insert_if_absent(key, b"a restart is not a free key", expires_at)
            .await
            .expect("insert"),
        "a restart does not release the instances the previous process held"
    );
    assert_eq!(
        after
            .compare_and_store(key, 2, b"advanced after the restart", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Stored { version: 3 },
        "and the new process continues the record's versions"
    );
}

// --- The ledger kernel over a distributed record store ---

/// A record store whose view of store time a test can move forward.
///
/// Both distributed adapters carry the same doc-hidden seam, and
/// [`MirroredClockStore`] is written against this rather than against either
/// of them so the conformance suite runs over the database store and the
/// Redis store through one piece of glue.
trait OffsetRecordStore: InstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64);
}

impl OffsetRecordStore for SqlInstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method, not this trait method: an
        // inherent method wins over a trait method of the same name, so this
        // is a delegation rather than the infinite recursion it reads as.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

impl OffsetRecordStore for RedisInstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method; see the note on the database
        // store's copy of this delegation.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

/// Test glue that keeps a distributed store's clock in step with the node
/// clock the conformance suite advances.
///
/// The suite moves one [`ControlledClock`], and a provider that shares that
/// clock with its store - which is what the memory reference does - sees one
/// timeline. A distributed store's clock is the backend's, which no test may
/// move, so this wrapper mirrors every advance of the node clock onto the
/// store's own test offset before each operation. What it cannot mirror is
/// the real milliseconds that pass while the suite runs, so store time is
/// always the node's plus that drift; every deadline the suite depends on is
/// either sixty seconds away or already elapsed, so drift decides nothing.
struct MirroredClockStore<S: OffsetRecordStore> {
    inner: S,
    clock: Arc<ControlledClock>,
    base_ms: u64,
}

impl<S: OffsetRecordStore> MirroredClockStore<S> {
    fn new(inner: S, clock: Arc<ControlledClock>, base_ms: u64) -> Self {
        Self {
            inner,
            clock,
            base_ms,
        }
    }

    fn mirror(&self) {
        let node_ms = self
            .clock
            .now()
            .expect("the conformance clock is readable")
            .get();
        self.inner
            .set_time_offset_for_test(node_ms.saturating_sub(self.base_ms));
    }
}

#[async_trait::async_trait]
impl<S: OffsetRecordStore> InstanceRecordStore for MirroredClockStore<S> {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        self.mirror();
        self.inner.load(key).await
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.mirror();
        self.inner.insert_if_absent(key, bytes, expires_at).await
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        self.mirror();
        self.inner
            .compare_and_store(key, expected_version, bytes, expires_at)
            .await
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        self.mirror();
        self.inner.remove(key).await
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        self.mirror();
        self.inner.load_promotion(key).await
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.mirror();
        self.inner
            .insert_promotion_if_absent(key, bytes, expires_at)
            .await
    }

    async fn count_instances(&self) -> Result<usize, LedgerError> {
        self.mirror();
        self.inner.count_instances().await
    }
}

/// One ledger handle over the database record store, on a node clock the
/// conformance suite owns.
fn conformance_ledger<S: OffsetRecordStore + 'static>(
    store: S,
    clock: &Arc<ControlledClock>,
    base_ms: u64,
) -> Arc<dyn suprnova_live::ledger::LiveInstanceLedger> {
    Arc::new(DistributedInstanceLedger::new(
        Arc::new(MirroredClockStore::new(store, Arc::clone(clock), base_ms)),
        Arc::clone(clock) as Arc<dyn Clock>,
        ledger_conformance::conformance_limits(),
    ))
}

// --- The Live ledger driver tests: environment, mount, and second node ---

/// Sets `LIVE_LEDGER_DRIVER` (and, for the Redis driver, its endpoint and key
/// namespace) for the body of one test and unsets them however that test
/// ends, so a failed assertion never leaves the variable set for whatever
/// runs next in this process. Every caller holds the environment lock.
struct LedgerDriverEnv {
    names: Vec<&'static str>,
}

impl LedgerDriverEnv {
    fn set(pairs: &[(&'static str, String)]) -> Self {
        for (name, value) in pairs {
            // SAFETY: the environment lock each caller holds is what
            // serialises every environment mutation in this test binary.
            unsafe { std::env::set_var(name, value) };
        }
        Self {
            names: pairs.iter().map(|(name, _)| *name).collect(),
        }
    }
}

impl Drop for LedgerDriverEnv {
    fn drop(&mut self) {
        for name in &self.names {
            // SAFETY: as above.
            unsafe { std::env::remove_var(name) };
        }
    }
}

/// The revision carried by a Live snapshot body or an accepted action, which
/// the wire spells as a decimal string.
fn revision_of(value: &Value) -> Revision {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => panic!("no revision here: {other}"),
    };
    Revision::parse(&text).expect("a canonical decimal revision")
}

/// A ledger handle with nothing in common with the running runtime's but the
/// backend: its own record store, its own clock, and the limits the runtime
/// itself builds - read from the runtime's own assembly seam rather than
/// restated here, so a change to those numbers cannot leave this node
/// running a state machine the real one does not. This is the "second
/// process" of these tests.
fn a_second_nodes_ledger<S: InstanceRecordStore + 'static>(
    store: S,
) -> Arc<dyn LiveInstanceLedger> {
    Arc::new(DistributedInstanceLedger::new(
        Arc::new(store),
        Arc::new(SystemClock),
        production_ledger_limits().expect("the runtime's ledger limits"),
    ))
}

/// Mounts the identity-bound dogfood island, acts on it once, and answers
/// with the revision the mount carried and the revision the action committed.
///
/// Shared by the database and Redis ledger tests: what differs between them
/// is which driver the runtime bound, never what the browser does.
async fn mount_and_act_on_the_identity_bound_island() -> (Revision, Revision) {
    live_dogfood_support::fixture();
    let router = Arc::new(build_public_router());
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    let middleware = production_middleware();

    // Sign in on one request, as a login handler would, so the identity-bound
    // render on the next request binds the session that survives the
    // framework's fixation rotation.
    let mut login = live_get(DOCUMENT_PATH);
    login
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    let (status, headers, body) =
        dispatch_live(Arc::clone(&router), Arc::clone(&middleware), login).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let signed_in = session_cookie(&headers);

    let mut private = live_get(PRIVATE_DOCUMENT_PATH);
    private
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    private
        .headers_mut()
        .insert("cookie", signed_in.parse().expect("cookie"));
    let (status, headers, body) =
        dispatch_live(Arc::clone(&router), Arc::clone(&middleware), private).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let snapshot = live_dogfood_support::decoded_snapshot(&body);
    let mounted = revision_of(&snapshot["body"]["revision"]);

    let (status, _, body) = dispatch_live(
        Arc::clone(&router),
        Arc::clone(&middleware),
        private_action_request(ActionRequest {
            snapshot,
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: Some("user-7"),
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let accepted: Value = serde_json::from_slice(&body).expect("an accepted action");
    assert_eq!(
        accepted["outcome"],
        "accepted",
        "{}",
        String::from_utf8_lossy(&body)
    );
    // The action's own response carries the successor envelope, and the
    // revision inside it is the one the ledger committed.
    let committed = revision_of(&accepted["snapshot"]["body"]["revision"]);
    assert_eq!(
        committed.get(),
        mounted.get() + 1,
        "the action committed the mounted revision's successor"
    );
    (mounted, committed)
}
