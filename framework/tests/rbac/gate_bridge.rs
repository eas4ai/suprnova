#![cfg(feature = "testing")]

//! The opt-in bridge that lets a held permission answer the gate:
//! `suprnova::rbac::register_gate_bridge`.
//!
//! Gate before-hooks are keyed by the user's type in a process-wide
//! registry, so the bridge is installed for `Member` only, a type no other
//! test in this binary uses, and `Outsider` is never bridged at all.

use std::any::Any;
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::queue::worker::register_job;
use suprnova::queue::{Job, Queue, SyncQueueDriver};
use suprnova::rbac::migrations::CreateRbacTables;
use suprnova::rbac::{GateBridgeMiddleware, register_gate_bridge};
use suprnova::testing::TestDatabase;
use suprnova::{
    App, Authenticatable, DB, FrameworkError, Gate, HasRoles, HttpResponse, Middleware, Next,
    Request, policy,
};

/// Held by the member directly.
const DIRECT: &str = "documents.review";
/// Held by the member through the `editor` role.
const THROUGH_ROLE: &str = "documents.publish";
/// Held by nobody and defined by no gate.
const NOT_HELD: &str = "documents.shred";
/// Held by nobody until a transaction grants it.
const LATE_GRANT: &str = "documents.approve";

/// The user type the bridge is installed for.
#[derive(Clone)]
struct Member {
    id: &'static str,
}

impl Authenticatable for Member {
    fn get_auth_identifier(&self) -> String {
        self.id.to_owned()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl HasRoles for Member {}

/// A user type the bridge is never installed for.
struct Outsider {
    id: &'static str,
}

impl Authenticatable for Outsider {
    fn get_auth_identifier(&self) -> String {
        self.id.to_owned()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl HasRoles for Outsider {}

/// What every check is about. The bridge never looks at it, because a
/// permission belongs to the user; the gate definitions and the policy do.
#[derive(Clone)]
struct Document {
    owner: &'static str,
}

struct DocumentPolicy;

#[policy(Member, Document)]
impl DocumentPolicy {
    /// Registers the `edit-document` ability: the owner may edit.
    fn edit(member: &Member, document: &Document) -> bool {
        document.owner == member.id
    }
}

struct BridgeMigrator;

impl sea_orm_migration::MigratorTrait for BridgeMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(CreateRbacTables)]
    }
}

/// A fresh database where `member` holds [`DIRECT`] itself and
/// [`THROUGH_ROLE`] through the `editor` role, with the bridge installed.
async fn setup(member: &Member) -> TestDatabase {
    let db = TestDatabase::fresh::<BridgeMigrator>().await.unwrap();
    suprnova::rbac::give_permission_to_role("editor", THROUGH_ROLE)
        .await
        .unwrap();
    member.assign_role("editor").await.unwrap();
    member.give_permission_to(DIRECT).await.unwrap();
    register_gate_bridge::<Member>();
    db
}

/// Ask the async gate about each ability, in order, from the handler of one
/// request behind [`GateBridgeMiddleware`], and return the answers.
async fn one_request(
    member: &Member,
    document: &Document,
    abilities: &[&'static str],
) -> Vec<bool> {
    let answers = Arc::new(Mutex::new(Vec::new()));
    let handler = asking(
        member.clone(),
        document.clone(),
        abilities.to_vec(),
        answers.clone(),
    );
    let response = GateBridgeMiddleware
        .handle(Request::for_test("GET", "/documents"), handler)
        .await;
    assert!(
        response.is_ok(),
        "the handler behind the middleware always answers"
    );
    let recorded = answers.lock().unwrap();
    recorded.clone()
}

/// The handler [`one_request`] runs: every check, recorded into `answers`.
fn asking(
    member: Member,
    document: Document,
    abilities: Vec<&'static str>,
    answers: Arc<Mutex<Vec<bool>>>,
) -> Next {
    Arc::new(move |_request| {
        let member = member.clone();
        let document = document.clone();
        let abilities = abilities.clone();
        let answers = answers.clone();
        Box::pin(async move {
            for ability in abilities {
                let allowed = Gate::allows_async(ability, &member, &document).await;
                answers.lock().unwrap().push(allowed);
            }
            Ok(HttpResponse::text("ok"))
        })
    })
}

/// Run `handling` as the handler of one request behind
/// [`GateBridgeMiddleware`], and return what it returned.
async fn within_one_request<T, Fut>(handling: Fut) -> T
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let pending = Arc::new(Mutex::new(Some(handling)));
    let finished = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&finished);
    let handler: Next = Arc::new(move |_request| {
        let handling = pending.lock().unwrap().take();
        let slot = Arc::clone(&slot);
        Box::pin(async move {
            if let Some(handling) = handling {
                let output = handling.await;
                *slot.lock().unwrap() = Some(output);
            }
            Ok(HttpResponse::text("ok"))
        })
    });
    let response = GateBridgeMiddleware
        .handle(Request::for_test("GET", "/documents"), handler)
        .await;
    assert!(
        response.is_ok(),
        "the handler behind the middleware always answers"
    );
    let output = finished.lock().unwrap().take();
    output.expect("the middleware ran the handler")
}

/// [`within_one_request`] inside a container scope, as the server runs every
/// request: the scope is open before the global middleware runs.
async fn within_one_scoped_request<T, Fut>(handling: Fut) -> T
where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    App::run_scoped(within_one_request(handling)).await
}

/// What the checks of [`CheckingJob`] answered, in order.
static JOB_ANSWERS: Mutex<Vec<bool>> = Mutex::new(Vec::new());

/// A job that asks the async gate about each of its abilities for the member
/// `member-1`, and records the answers in [`JOB_ANSWERS`].
#[derive(Serialize, Deserialize, Clone)]
struct CheckingJob {
    abilities: Vec<String>,
}

#[async_trait]
impl Job for CheckingJob {
    fn job_name() -> &'static str {
        "rbac_gate_bridge::CheckingJob"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let member = Member { id: "member-1" };
        for ability in &self.abilities {
            let allowed = Gate::allows_async(ability, &member, &elsewhere()).await;
            JOB_ANSWERS.lock().unwrap().push(allowed);
        }
        Ok(())
    }
}

/// Make [`Queue::push`] run a job inline, in the task that pushes it.
fn use_the_sync_queue() {
    register_job::<CheckingJob>();
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));
}

/// Push a [`CheckingJob`] for `abilities` and return what its checks answered.
async fn checks_in_an_inline_job(abilities: &[&str]) -> Vec<bool> {
    JOB_ANSWERS.lock().unwrap().clear();
    let job = CheckingJob {
        abilities: abilities.iter().map(|a| (*a).to_owned()).collect(),
    };
    Queue::push(job).await.unwrap();
    let answers = JOB_ANSWERS.lock().unwrap();
    answers.clone()
}

/// A document someone else owns, so no owner check here allows on it.
fn elsewhere() -> Document {
    Document {
        owner: "someone-else",
    }
}

/// The query log, on for one test and off again when dropped, including
/// when an assertion fails first.
struct QueryLog;

impl QueryLog {
    fn start() -> Self {
        DB::flush_query_log().unwrap();
        DB::enable_query_log().unwrap();
        Self
    }

    /// How many times the bridge's permission read has run so far.
    fn permission_reads(&self) -> usize {
        Self::reads_so_far()
    }

    /// The same count, for code that runs where the guard is not at hand.
    fn reads_so_far() -> usize {
        DB::get_query_log()
            .unwrap()
            .iter()
            .filter(|query| query.sql.contains("SELECT permissions.name"))
            .count()
    }
}

impl Drop for QueryLog {
    fn drop(&mut self) {
        let _ = DB::disable_query_log();
        let _ = DB::flush_query_log();
    }
}

#[tokio::test]
#[serial]
async fn without_the_bridge_a_held_permission_does_not_answer_the_gate() {
    let _db = TestDatabase::fresh::<BridgeMigrator>().await.unwrap();
    let outsider = Outsider { id: "outsider-1" };
    outsider.give_permission_to(DIRECT).await.unwrap();
    assert!(outsider.has_permission_to(DIRECT).await.unwrap());

    let document = Document {
        owner: "someone-else",
    };
    assert!(
        !Gate::allows_async(DIRECT, &outsider, &document).await,
        "RBAC and the gate do not know each other until the application opts in"
    );
}

#[tokio::test]
#[serial]
async fn a_direct_permission_and_one_through_a_role_answer_the_async_gate() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    let document = Document {
        owner: "someone-else",
    };

    assert!(
        suprnova::middleware::has_global_middleware::<GateBridgeMiddleware>(),
        "the bridge installs the middleware that keeps its reads per request"
    );
    assert_eq!(
        one_request(&member, &document, &[DIRECT, THROUGH_ROLE]).await,
        vec![true, true]
    );
    assert!(
        !Gate::allows(DIRECT, &member, &document),
        "the synchronous forms cannot wait for the read and skip the bridge"
    );
}

#[tokio::test]
#[serial]
async fn an_ability_that_is_no_permission_goes_to_its_gate_definition() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    Gate::define::<Member, Document>("documents.archive", |member, document| {
        document.owner == member.id
    });
    let own = Document { owner: "member-1" };
    let other = Document {
        owner: "someone-else",
    };

    assert_eq!(
        one_request(&member, &own, &["documents.archive"]).await,
        vec![true]
    );
    assert_eq!(
        one_request(&member, &other, &["documents.archive", NOT_HELD]).await,
        vec![false, false],
        "the definition decides its ability, and an ability nothing defines is denied"
    );
}

#[tokio::test]
#[serial]
async fn a_user_without_the_permission_is_still_allowed_by_a_policy_that_allows() {
    suprnova::authorization::init_policies();
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    // The ability is a permission, held by another member and not this one.
    Member { id: "member-2" }
        .give_permission_to("edit-document")
        .await
        .unwrap();
    assert!(!member.has_permission_to("edit-document").await.unwrap());
    let own = Document { owner: "member-1" };
    let other = Document {
        owner: "someone-else",
    };

    assert_eq!(
        one_request(&member, &own, &["edit-document"]).await,
        vec![true],
        "the bridge answers nothing, so the policy decides, and it allows the owner"
    );
    assert_eq!(
        one_request(&member, &other, &["edit-document"]).await,
        vec![false]
    );
}

#[tokio::test]
#[serial]
async fn a_revoked_permission_does_not_answer_the_next_request() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    let document = Document {
        owner: "someone-else",
    };
    assert_eq!(
        one_request(&member, &document, &[DIRECT, THROUGH_ROLE]).await,
        vec![true, true]
    );

    member.remove_permission_to(DIRECT).await.unwrap();
    member.remove_role("editor").await.unwrap();

    assert_eq!(
        one_request(&member, &document, &[DIRECT, THROUGH_ROLE]).await,
        vec![false, false],
        "nothing the first request loaded is kept for the next one"
    );
}

#[tokio::test]
#[serial]
async fn the_permissions_are_read_once_per_request_however_many_checks_ask() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    // A second registration for the same type must add no second hook.
    register_gate_bridge::<Member>();
    let document = Document {
        owner: "someone-else",
    };
    let log = QueryLog::start();

    let answers = one_request(&member, &document, &[DIRECT, THROUGH_ROLE, NOT_HELD]).await;
    assert_eq!(answers, vec![true, true, false]);
    assert_eq!(
        log.permission_reads(),
        1,
        "three checks in one request share one read"
    );

    let answers = one_request(&member, &document, &[DIRECT]).await;
    assert_eq!(answers, vec![true]);
    assert_eq!(log.permission_reads(), 2, "the next request reads again");

    assert!(Gate::allows_async(DIRECT, &member, &document).await);
    assert!(!Gate::allows_async(NOT_HELD, &member, &document).await);
    assert_eq!(
        log.permission_reads(),
        4,
        "outside a request nothing is kept, so each check reads, and reads once: \
         two hooks would have read twice for the ability the member does not hold"
    );
}

#[tokio::test]
#[serial]
#[tracing_test::traced_test]
async fn a_failing_read_denies_and_is_logged_once_without_the_user_id() {
    let member = Member {
        id: "member-behind-a-failing-read",
    };
    let db = setup(&member).await;
    let fallback = "documents.fallback";
    Gate::define::<Member, Document>(fallback, |_member, _document| true);
    let document = Document {
        owner: "someone-else",
    };
    assert_eq!(
        one_request(&member, &document, &[DIRECT]).await,
        vec![true],
        "the permission answers while the read works"
    );

    db.execute_unprepared("DROP TABLE model_permissions")
        .await
        .unwrap();

    let answers = one_request(&member, &document, &[DIRECT, THROUGH_ROLE, fallback]).await;
    assert_eq!(
        answers,
        vec![false, false, true],
        "a failed read allows nothing, and the gate still decides what it defines"
    );
    // One failed read for the request, so one line, however many checks.
    logs_assert(|lines: &[&str]| {
        let failures = lines
            .iter()
            .filter(|line| line.contains("could not read a user's permissions"))
            .count();
        match failures {
            1 => Ok(()),
            n => Err(format!("logged {n} times, not once")),
        }
    });
    assert!(
        !logs_contain("member-behind-a-failing-read"),
        "the user id stays out of the log"
    );
}

#[tokio::test]
#[serial]
async fn a_grant_rolled_back_with_its_transaction_does_not_answer_after_it() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;

    let (inside, after) = within_one_request(async move {
        let seen = Arc::new(Mutex::new(None));
        let seen_inside = Arc::clone(&seen);
        let granted = member.clone();
        let outcome: Result<(), FrameworkError> = DB::transaction(move |_tx| {
            Box::pin(async move {
                granted.give_permission_to(LATE_GRANT).await?;
                let allowed = Gate::allows_async(LATE_GRANT, &granted, &elsewhere()).await;
                *seen_inside.lock().unwrap() = Some(allowed);
                Err(FrameworkError::internal("roll the grant back"))
            })
        })
        .await;
        assert!(outcome.is_err(), "the transaction rolled back");
        let inside = seen.lock().unwrap().take();
        let after = Gate::allows_async(LATE_GRANT, &member, &elsewhere()).await;
        (inside, after)
    })
    .await;

    assert_eq!(
        inside,
        Some(true),
        "inside the transaction the check sees the grant the transaction made"
    );
    assert!(
        !after,
        "the grant was rolled back, and the check inside the transaction kept nothing"
    );
}

#[tokio::test]
#[serial]
async fn a_revocation_inside_a_transaction_is_seen_by_a_check_inside_it() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    let checking = member.clone();

    let (before, inside) = within_one_request(async move {
        // The first check loads the request's set, the permission in it.
        let before = Gate::allows_async(DIRECT, &checking, &elsewhere()).await;
        let revoking = checking.clone();
        let inside = DB::transaction(move |_tx| {
            Box::pin(async move {
                revoking.remove_permission_to(DIRECT).await?;
                let allowed = Gate::allows_async(DIRECT, &revoking, &elsewhere()).await;
                Ok::<bool, FrameworkError>(allowed)
            })
        })
        .await
        .unwrap();
        (before, inside)
    })
    .await;

    assert!(
        before,
        "the member holds the permission before the transaction"
    );
    assert!(
        !inside,
        "the check reads the transaction's revocation, not the set loaded before it began"
    );
    assert_eq!(
        one_request(&member, &elsewhere(), &[DIRECT]).await,
        vec![false],
        "the revocation committed, so the next request does not see the permission"
    );
}

#[tokio::test]
#[serial]
async fn a_read_that_failed_inside_a_rolled_back_transaction_is_not_kept() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;

    let (inside, after) = within_one_request(async move {
        let seen = Arc::new(Mutex::new(None));
        let seen_inside = Arc::clone(&seen);
        let checking = member.clone();
        let outcome: Result<(), FrameworkError> = DB::transaction(move |_tx| {
            Box::pin(async move {
                // Every permission read fails until the rollback restores it.
                DB::statement("DROP TABLE model_permissions", Vec::new()).await?;
                let allowed = Gate::allows_async(DIRECT, &checking, &elsewhere()).await;
                *seen_inside.lock().unwrap() = Some(allowed);
                Err(FrameworkError::internal("roll the drop back"))
            })
        })
        .await;
        assert!(outcome.is_err(), "the transaction rolled back");
        let inside = seen.lock().unwrap().take();
        let after = Gate::allows_async(DIRECT, &member, &elsewhere()).await;
        (inside, after)
    })
    .await;

    assert_eq!(
        inside,
        Some(false),
        "the read failed, so the permission answered nothing"
    );
    assert!(
        after,
        "after the rollback the check reads again, instead of keeping the failure"
    );
}

#[tokio::test]
#[serial]
async fn an_inline_job_neither_uses_the_requests_set_nor_fills_it() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    use_the_sync_queue();
    let checking = member.clone();

    let (first, from_job, after_job) = within_one_scoped_request(async move {
        // The first check fills the request's set, without the permission.
        let first = Gate::allows_async(LATE_GRANT, &checking, &elsewhere()).await;
        checking.give_permission_to(LATE_GRANT).await.unwrap();
        let from_job = checks_in_an_inline_job(&[LATE_GRANT]).await;
        let after_job = Gate::allows_async(LATE_GRANT, &checking, &elsewhere()).await;
        (first, from_job, after_job)
    })
    .await;

    assert!(!first, "the permission is not held yet");
    assert_eq!(
        from_job,
        vec![true],
        "the inline job reads for itself, so it sees the grant the request's set lacks"
    );
    assert!(
        !after_job,
        "the job left nothing in the request's set, which still lacks the grant"
    );
    assert_eq!(
        one_request(&member, &elsewhere(), &[LATE_GRANT]).await,
        vec![true],
        "the next request reads the grant"
    );
}

#[tokio::test]
#[serial]
async fn an_inline_job_does_not_see_a_permission_the_requests_set_still_holds() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    use_the_sync_queue();
    let checking = member.clone();

    let (before, from_job, after_job) = within_one_scoped_request(async move {
        // The first check fills the request's set with the permission held.
        let before = Gate::allows_async(DIRECT, &checking, &elsewhere()).await;
        checking.remove_permission_to(DIRECT).await.unwrap();
        let from_job = checks_in_an_inline_job(&[DIRECT]).await;
        let after_job = Gate::allows_async(DIRECT, &checking, &elsewhere()).await;
        (before, from_job, after_job)
    })
    .await;

    assert!(before, "the member holds the permission at first");
    assert_eq!(
        from_job,
        vec![false],
        "a revoked permission never answers for the job, though the request's set holds it"
    );
    assert!(
        after_job,
        "the request keeps its own set for the rest of the request"
    );
}

#[tokio::test]
#[serial]
async fn an_inline_job_reads_for_every_check_and_the_request_reads_once() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    use_the_sync_queue();
    let log = QueryLog::start();
    let checking = member.clone();

    let (request_reads, job_answers, total_reads) = within_one_scoped_request(async move {
        Gate::allows_async(DIRECT, &checking, &elsewhere()).await;
        Gate::allows_async(THROUGH_ROLE, &checking, &elsewhere()).await;
        let request_reads = QueryLog::reads_so_far();
        let job_answers = checks_in_an_inline_job(&[DIRECT, THROUGH_ROLE]).await;
        (request_reads, job_answers, QueryLog::reads_so_far())
    })
    .await;

    assert_eq!(request_reads, 1, "two checks of the request share one read");
    assert_eq!(job_answers, vec![true, true]);
    assert_eq!(
        total_reads - request_reads,
        2,
        "nothing is kept for the inline job, so each of its two checks reads"
    );
    assert_eq!(log.permission_reads(), total_reads);
}

#[tokio::test]
#[serial]
async fn a_middleware_with_no_container_scope_gives_a_nested_scope_a_read_of_its_own() {
    let member = Member { id: "member-1" };
    let _db = setup(&member).await;
    let checking = member.clone();

    let (first, inside_nested) = within_one_request(async move {
        let first = Gate::allows_async(LATE_GRANT, &checking, &elsewhere()).await;
        checking.give_permission_to(LATE_GRANT).await.unwrap();
        let nested = checking.clone();
        let inside_nested =
            App::run_scoped(
                async move { Gate::allows_async(LATE_GRANT, &nested, &elsewhere()).await },
            )
            .await;
        (first, inside_nested)
    })
    .await;

    assert!(!first, "the permission is not held yet");
    assert!(
        inside_nested,
        "the store has no owner scope, so a scope opened inside is another unit of work"
    );
}
