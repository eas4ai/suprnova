//! The opt-in bridge that lets a held permission answer the gate.
//!
//! RBAC layers on the gate the way spatie/laravel-permission layers on
//! Laravel's: one before-hook, so every permission a user holds is also a
//! gate ability. The hook only ever allows. An ability that is no permission
//! of the user is left to the gate's definitions and policies, so a user
//! without the permission can still be allowed by one of them.
//!
//! Reading a user's permissions is a database read, so the hook is an async
//! one ([`Gate::before_async`]) and answers the gate's async forms only.
//! The read runs once per request per user: the loaded set lives in a store
//! that [`GateBridgeMiddleware`] opens around the request and drops with it.
//! Nothing is cached across requests, so a permission revoked during one
//! request no longer answers the next. A check inside
//! `DB::transaction` reads for itself and keeps nothing, because what it
//! reads may still be rolled back.
//!
//! The set belongs to the unit of work that opened it: the store remembers
//! the identity of the container scope the request runs in, and only a check
//! in that same scope uses or fills the set. A unit of work that opens a
//! scope of its own inside the request, such as a job the sync queue driver
//! runs inline, reads for itself, so it never sees a permission the request
//! loaded earlier and never leaves one for the request.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::authorization::Gate;
use crate::container::scope::ContainerScope;
use crate::database::after_commit::in_transaction;
use crate::http::{Request, Response};
use crate::middleware::{Middleware, Next};

use super::HasRoles;
use super::has_roles::{observe_permission_names_read, permission_names_for_model};

/// One load of a user's permission names: the set, or `None` when the read
/// failed.
type Loaded = Option<Arc<HashSet<String>>>;

/// The loads of one request, keyed by the user's RBAC model type and model
/// id - the pair the permission rows themselves are keyed by. The cell makes
/// two checks that race inside one request share a single read.
type LoadCells = HashMap<(String, String), Arc<OnceCell<Loaded>>>;

/// The store of one request: its loads, and the unit of work they belong to.
struct RequestLoads {
    /// The identity of the container scope the request runs in, or `None`
    /// when the middleware runs with no scope. Only a check whose own scope
    /// id equals it may use `loads`.
    owner: Option<u64>,
    /// The std mutex is only held to fetch a cell, never across an `.await`.
    loads: Mutex<LoadCells>,
}

tokio::task_local! {
    // Opened by `GateBridgeMiddleware` around one request and dropped when
    // the request ends, so no load outlives the request that made it.
    static REQUEST_LOADS: RequestLoads;
}

/// The user types the bridge is installed for, so a second call for the
/// same type adds no second hook.
static BRIDGED: Mutex<Vec<TypeId>> = Mutex::new(Vec::new());

/// Let every permission a user of type `U` holds answer the [`Gate`], as an
/// ability of the same name.
///
/// Call it once, in the application's bootstrap. It installs a before-hook
/// on the gate for `U`: when the ability is the name of a permission the
/// user holds on the default `"web"` guard - directly or through a role, as
/// [`HasRoles::has_permission_to`] resolves it - the gate allows. For every
/// other ability the hook answers nothing and the gate goes on to its
/// definitions and policies. It never denies, so the bridge can only add
/// abilities, never take one a gate or policy grants. Like every
/// before-hook, its allow comes first: a held permission grants the ability
/// of the same name even where a definition or policy would deny it. That is
/// why it is opt-in - an application whose ability names and permission
/// names overlap by accident would otherwise find a permission granting an
/// ability it meant a policy to decide.
///
/// ```rust,no_run
/// # use std::any::Any;
/// # use std::sync::Arc;
/// # use suprnova::{Authenticatable, HasRoles};
/// # struct User { id: i64 }
/// # impl Authenticatable for User {
/// #     fn get_auth_identifier(&self) -> String { self.id.to_string() }
/// #     fn as_any(&self) -> &dyn Any { self }
/// #     fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> { self }
/// # }
/// # impl HasRoles for User {}
/// # struct Post;
/// # async fn ex(user: User, post: Post) {
/// suprnova::rbac::register_gate_bridge::<User>();
///
/// // Allowed when the user holds the "posts.publish" permission.
/// let allowed = suprnova::Gate::allows_async("posts.publish", &user, &post).await;
/// # }
/// ```
///
/// # Async forms only
///
/// Reading the permissions is a database read, and the gate's synchronous
/// forms cannot wait for one, so a permission answers
/// [`Gate::allows_async`], [`Gate::authorize_async`],
/// [`Gate::inspect_async`], and the other async forms, and never
/// [`Gate::allows`], [`Gate::authorize`], or [`Gate::inspect`]. Blocking
/// a thread on the read inside a synchronous hook would stall a runtime
/// worker for every check, so the bridge does not offer that.
///
/// # One read per request
///
/// The hook loads the user's permission names once per request, with one
/// statement, and answers every later ability in the same request from that
/// set. The set is kept only for the request, by [`GateBridgeMiddleware`],
/// which this function installs as the first global middleware. A grant or
/// revocation made before the first check for a user in a request is seen
/// by that check; after the first check, one is seen from the next request
/// on.
///
/// Inside a [`DB::transaction`](crate::DB::transaction) a check neither uses
/// the set nor adds to it. It reads the permissions for itself, on the
/// transaction, so it sees the grants and revocations the transaction made,
/// and nothing it read outlives a rollback.
///
/// The set belongs to the unit of work that opened it, which is the request
/// as the middleware sees it: the container scope the request runs in. A
/// check uses or fills the set only when it runs in that same scope. Work
/// that opens a scope of its own inside the request - a job or a queued
/// listener attempt that the sync queue driver runs inline, a future run
/// through [`App::run_scoped`](crate::App::run_scoped) - neither uses the
/// request's set nor adds to it; each of its checks reads the permissions
/// again, so it sees the database as it is and leaves nothing behind for the
/// request. Work under [`App::in_current_scope`](crate::App::in_current_scope)
/// and [`App::spawn_scoped`](crate::App::spawn_scoped) carries the request's
/// own scope, so it is the request's work. The set reaches it only where the
/// request's task-local store reaches it: awaited in the request's task it
/// shares the set, and a spawned task does not inherit a task-local, so it
/// reads for itself. Outside a request that the middleware wraps - a job on
/// a queue worker, a console command, a task spawned off the request -
/// nothing is kept and every check reads the permissions again.
///
/// # When the read fails
///
/// A failed read logs one error and answers nothing, so the gate goes on to
/// its definitions and policies and denies unless one of them allows. The
/// failure never allows, and the log line carries no user id. Outside a
/// transaction the failure is kept for the request like a loaded set, so it
/// is logged once and the rest of the request does not retry. Inside one it
/// is not kept, like every read there, so the next check reads again.
pub fn register_gate_bridge<U>()
where
    U: HasRoles + 'static,
{
    {
        let mut bridged = BRIDGED.lock().unwrap_or_else(|e| e.into_inner());
        if bridged.contains(&TypeId::of::<U>()) {
            return;
        }
        bridged.push(TypeId::of::<U>());
    }
    crate::middleware::prepend_global_middleware(GateBridgeMiddleware);
    Gate::before_async::<U, _, _>(|user: &U, ability: &str| {
        let model_type = user.rbac_model_type();
        let model_id = user.rbac_model_id();
        answer(model_type, model_id, ability.to_owned())
    });
}

/// Middleware that keeps the permission names the gate bridge loads for the
/// length of one request, and drops them when the request ends.
///
/// [`register_gate_bridge`] installs it as the first global middleware, so
/// an application on the global middleware stack never adds it by hand. An
/// embedder that builds its own
/// [`MiddlewareRegistry`](crate::middleware::MiddlewareRegistry) without the
/// globals adds it there. Without it the bridge still answers correctly; it
/// reads the permissions once per check instead of once per request.
pub struct GateBridgeMiddleware;

#[async_trait]
impl Middleware for GateBridgeMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // The request's container scope is open before the global middleware
        // runs, so its id is the owner of the store.
        let loads = RequestLoads {
            owner: ContainerScope::current_id(),
            loads: Mutex::new(HashMap::new()),
        };
        REQUEST_LOADS.scope(loads, next(request)).await
    }
}

/// The bridge's answer for one ability: `Some(true)` when the user holds a
/// permission of that name, `None` otherwise. Never `Some(false)`, so the
/// gate goes on to its definitions and policies.
async fn answer(model_type: String, model_id: String, ability: String) -> Option<bool> {
    // Every answer records the tables the load reads, the one served from
    // this request's set as well as the one that ran the statement.
    observe_permission_names_read();
    // Inside `DB::transaction` the read runs on the transaction: it sees
    // grants and revocations the transaction has not committed, which a
    // rollback may still take back, and the request's set, if it was loaded
    // before the transaction began, sees none of them. So such a check reads
    // for itself and keeps nothing, in either direction, a failed read
    // included. `in_transaction` is the condition `ExecutorChoice::resolve_read`
    // uses to put this read on the transaction.
    let names = if in_transaction() {
        load(&model_type, &model_id).await
    } else {
        match request_load(&model_type, &model_id) {
            Some(cell) => {
                let loaded = cell.get_or_init(|| load(&model_type, &model_id)).await;
                loaded.clone()
            }
            None => load(&model_type, &model_id).await,
        }
    };
    match names {
        Some(names) if names.contains(ability.as_str()) => Some(true),
        _ => None,
    }
}

/// This request's load cell for one user, created on first use. `None`
/// outside a request that [`GateBridgeMiddleware`] wraps, and in a unit of
/// work whose container scope is not the one the request runs in.
fn request_load(model_type: &str, model_id: &str) -> Option<Arc<OnceCell<Loaded>>> {
    REQUEST_LOADS
        .try_with(|store| {
            if ContainerScope::current_id() != store.owner {
                return None;
            }
            let cell = store
                .loads
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry((model_type.to_owned(), model_id.to_owned()))
                .or_default()
                .clone();
            Some(cell)
        })
        .ok()
        .flatten()
}

/// Read one user's permission names.
///
/// This is where a failure is made fail-closed: it is logged once, here,
/// and comes back as `None`, which [`answer`] turns into no answer at all.
/// The gate then denies unless a definition or policy allows. Outside a
/// transaction the request's cell keeps the `None`, so the rest of the
/// request neither retries the read nor logs the failure again; inside one
/// [`answer`] keeps nothing. The model id stays out of the log; the model
/// type says which kind of user it was.
async fn load(model_type: &str, model_id: &str) -> Loaded {
    match permission_names_for_model(model_type, model_id).await {
        Ok(names) => Some(Arc::new(names)),
        Err(error) => {
            tracing::error!(
                model_type = %model_type,
                error = %error,
                "rbac gate bridge could not read a user's permissions; none of them \
                 answers the gate, which denies unless a gate or policy allows"
            );
            None
        }
    }
}
