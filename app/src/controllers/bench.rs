//! Benchmark-only routes, compiled in **only** under `--features bench`.
//!
//! These exist to make the benchmark measure things it otherwise could
//! not: concurrent I/O inside one request, a slow downstream, deep eager
//! loading, and bulk writes. They are deliberately not part of the
//! dogfood app's surface — several return large payloads or perform
//! unbounded-ish work by design, which is fine for a load generator
//! pointed at a throwaway database and is not fine anywhere else. The
//! feature gate is what keeps them out of a production binary; see
//! `routes::register`.
//!
//! Every route here is a **read** except `POST /bench/posts/bulk`, and
//! every one takes explicitly bounded parameters. "Bounded" matters more
//! here than it looks: the benchmark's seeded database holds 50M posts
//! and 200M comments, so a route that forgets a limit does not return
//! slowly, it returns never.
//!
//! Corresponding tiers are named per route; see `bench/PLAN.md`.

use std::time::Instant;

use suprnova::{
    DB, FrameworkError, Http, HttpResponse, Model, Request, Response, Router, attrs, get, group,
    post,
};

use crate::models::comments::Comment;
use crate::models::posts::Post;
use crate::models::role_user::RoleUser;
use crate::models::tags::Tag;
use crate::models::users::User;

/// Mount the benchmark routes onto the router.
///
/// Called from `routes::register` behind `#[cfg(feature = "bench")]`, so
/// a binary built without the feature never links any of this — the call
/// site does not exist rather than being a branch that is never taken.
pub fn register(router: Router) -> Router {
    group!("/bench", {
        get!("/dashboard", dashboard).name("bench.dashboard"),
        get!("/external", external).name("bench.external"),
        get!("/users/hydrate", users_hydrate).name("bench.users.hydrate"),
        get!("/posts/paginated", posts_paginated).name("bench.posts.paginated"),
        get!("/posts/{id}/deep", post_deep).name("bench.posts.deep"),
        post!("/posts/bulk", posts_bulk).name("bench.posts.bulk"),
    })
    .register(router)
}

/// Ceiling on any `?rows=` / `?per_page=` style parameter.
///
/// The load generator supplies these, so they are not attacker-controlled
/// in the usual sense — but an unbounded value is still the difference
/// between a benchmark and an OOM, and a typo in a targets file should
/// not cost a run.
const MAX_ROWS: u64 = 50_000;

/// Read a positive integer query parameter, falling back to `default`
/// and clamping to `MAX_ROWS`.
fn bounded_param(req: &Request, key: &str, default: u64) -> u64 {
    req.query_param(key)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default)
        .clamp(1, MAX_ROWS)
}

// ---------------------------------------------------------------------
// Tier 2.1 — concurrent database queries
// ---------------------------------------------------------------------

/// `GET /bench/dashboard?user_id=N`
///
/// Five independent queries issued concurrently with `tokio::try_join!`.
/// This is the shape Tier 2.1 compares against Laravel's sequential
/// Eloquent and against `Octane::concurrently()`.
///
/// All five are genuinely independent — none consumes another's result —
/// because a chain that has to await its predecessor measures latency
/// serialisation rather than concurrency. That constraint is why the
/// user's roles come from the `role_user` pivot directly instead of
/// `User::find(id)?.roles()`, which would have to sequence behind the
/// user lookup.
///
/// **Each in-flight request holds five pool connections, not one.** Tier
/// 2.2's small-pool runs have to be read with that in mind: at
/// `DB_MAX_CONNECTIONS=10`, two concurrent dashboards exhaust the pool.
pub async fn dashboard(req: Request) -> Response {
    dashboard_inner(req).await.map_err(HttpResponse::from)
}

async fn dashboard_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let user_id = bounded_param(&req, "user_id", 1) as i64;
    // A post id derived from the user id rather than looked up, so the
    // comment query does not have to wait for the post query.
    let post_id = user_id;

    let started = Instant::now();
    let (user, posts, comments, tags, roles) = tokio::try_join!(
        <User as Model>::find(user_id),
        async { Post::for_author(user_id).await },
        <Comment as Model>::query()
            .filter("commentable_id", post_id)
            .filter("commentable_type", "post")
            .limit(100)
            .get(),
        <Tag as Model>::query().limit(100).get(),
        <RoleUser as Model>::query()
            .filter("user_id", user_id)
            .limit(100)
            .get(),
    )?;
    let elapsed_us = started.elapsed().as_micros() as u64;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "user_id": user_id,
        "found": user.is_some(),
        "posts": posts.len(),
        "comments": comments.len(),
        "tags": tags.len(),
        "roles": roles.len(),
        // Reported so the harness can separate in-handler query time from
        // the request's total wall clock without a profiler attached.
        "query_us": elapsed_us,
    })))
}

// ---------------------------------------------------------------------
// Tier 2.3 — slow downstream
// ---------------------------------------------------------------------

/// `GET /bench/external?delay=N`
///
/// Issues one outbound HTTP request to the echo server named by
/// `BENCH_ECHO_URL`, asking it to hold the response for `delay`
/// milliseconds. A payment gateway, a mail API, an LLM endpoint.
///
/// The whole point is what the runtime does *while* waiting, so the
/// handler does nothing else — any local work would dilute the signal.
///
/// Returns 503 rather than 500 when `BENCH_ECHO_URL` is unset: a missing
/// downstream is a harness configuration problem, and it should be
/// distinguishable in the vegeta output from the server failing.
pub async fn external(req: Request) -> Response {
    external_inner(req).await.map_err(HttpResponse::from)
}

async fn external_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let delay_ms = req
        .query_param("delay")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .min(60_000);

    let base = std::env::var("BENCH_ECHO_URL").map_err(|_| {
        FrameworkError::domain(
            "BENCH_ECHO_URL is not set — /bench/external has no downstream to call",
            503,
        )
    })?;

    let started = Instant::now();
    let response = Http::get(format!("{base}?delay={delay_ms}"))
        .send()
        .await
        .map_err(|e| FrameworkError::domain(format!("downstream call failed: {e}"), 502))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "requested_delay_ms": delay_ms,
        "observed_ms": elapsed_ms,
        "downstream_status": response.status(),
    })))
}

// ---------------------------------------------------------------------
// Tier 3.1 — large result hydration
// ---------------------------------------------------------------------

/// `GET /bench/users/hydrate?rows=N`
///
/// Hydrates `N` users into models and serialises them. Stresses per-row
/// allocation and serialisation cost at a volume where any per-row
/// overhead becomes visible.
///
/// `rows` is explicit and capped rather than "all". The seeded table
/// holds a million users; hydrating all of them measures how a process
/// dies, not what a row costs.
pub async fn users_hydrate(req: Request) -> Response {
    users_hydrate_inner(req).await.map_err(HttpResponse::from)
}

async fn users_hydrate_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let rows = bounded_param(&req, "rows", 10_000);

    let users = <User as Model>::query()
        .order_by_asc("id")
        .limit(rows)
        .get()
        .await?
        .into_vec();

    // Serialised explicitly rather than through the model's own
    // `Serialize`: `User` hides `password` and `remember_token`, and
    // "the bench route accidentally emitted the hash column" is not a
    // sentence worth risking for the sake of three fewer lines.
    let payload: Vec<suprnova::serde_json::Value> = users
        .into_iter()
        .map(|u| {
            suprnova::serde_json::json!({
                "id": u.id,
                "name": u.name,
                "created_at": u.created_at,
            })
        })
        .collect();

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "count": payload.len(),
        "users": payload,
    })))
}

// ---------------------------------------------------------------------
// Tier 3.2 — deep eager loading
// ---------------------------------------------------------------------

/// `GET /bench/posts/{id}/deep`
///
/// One post with five relations eager-loaded, spanning every relation
/// kind the framework has: `BelongsTo`, a nested `HasOne` and
/// `BelongsToMany` behind it, a `MorphMany`, and a `MorphToMany`.
///
/// A correct eager loader fires a fixed number of queries regardless of
/// result size; an incorrect one fires N+1. The count is measured by the
/// harness (via the database's own statistics), not asserted here —
/// a route that counted its own queries would be reporting the thing
/// under test.
pub async fn post_deep(req: Request) -> Response {
    post_deep_inner(req).await.map_err(HttpResponse::from)
}

async fn post_deep_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let id: i64 = req
        .param("id")?
        .parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let post = <Post as Model>::query()
        .with(["user", "user.profile", "user.roles", "comments", "tags"])
        .filter("id", id)
        .first()
        .await?
        .ok_or_else(|| FrameworkError::not_found("post"))?;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "id": post.id,
        "title": post.title,
        "author": post.user_loaded().map(|u| suprnova::serde_json::json!({
            "id": u.id,
            "name": u.name,
        })),
        "comments": post.comments_loaded().len(),
        "tags": post.tags_loaded().len(),
    })))
}

// ---------------------------------------------------------------------
// Tier 3.3 — paginated feed with relations
// ---------------------------------------------------------------------

/// `GET /bench/posts/paginated?page=N&per_page=M`
///
/// Offset pagination with eager-loaded relations on the result page —
/// the social feed query.
///
/// Deliberately offset-based, not keyset. Deep offsets are the expensive
/// case and the one worth measuring: `page=50` makes Postgres walk and
/// discard 1,000 rows before returning 20. `/api/users` covers the
/// keyset alternative, and the contrast between the two is a result.
pub async fn posts_paginated(req: Request) -> Response {
    posts_paginated_inner(req).await.map_err(HttpResponse::from)
}

async fn posts_paginated_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let per_page = bounded_param(&req, "per_page", 20).min(100);

    // `simple_paginate` reads `?page=N` from the request itself.
    let page = <Post as Model>::query()
        .with(["user", "tags"])
        .order_by_desc("id")
        .simple_paginate(per_page)
        .await?;

    let posts: Vec<suprnova::serde_json::Value> = page
        .data
        .iter()
        .map(|p| {
            suprnova::serde_json::json!({
                "id": p.id,
                "title": p.title,
                "author": p.user_loaded().map(|u| u.name.clone()),
                "tags": p.tags_loaded().len(),
            })
        })
        .collect();

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "current_page": page.current_page,
        "per_page": page.per_page,
        "has_more": page.has_more,
        "posts": posts,
    })))
}

// ---------------------------------------------------------------------
// Tier 3.4 — concurrent writes
// ---------------------------------------------------------------------

/// Rows inserted per `POST /bench/posts/bulk` call.
const BULK_ROWS: usize = 10;

/// `POST /bench/posts/bulk?author_id=N`
///
/// Inserts [`BULK_ROWS`] posts inside one transaction. Driven at high
/// concurrency this is Tier 3.4: throughput, p99, transaction failure
/// rate, and deadlock count under sustained contention.
///
/// One transaction rather than ten autocommits on purpose — the tier is
/// about how the two stacks behave when many transactions contend, and
/// a transaction that holds its rows for the length of ten inserts is
/// what produces contention worth measuring.
pub async fn posts_bulk(req: Request) -> Response {
    posts_bulk_inner(req).await.map_err(HttpResponse::from)
}

async fn posts_bulk_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let author_id = bounded_param(&req, "author_id", 1) as i64;

    let ids = DB::transaction(move |_tx| {
        Box::pin(async move {
            let mut ids = Vec::with_capacity(BULK_ROWS);
            for i in 0..BULK_ROWS {
                let post = Post::create(attrs! {
                    author_id: author_id,
                    title: format!("bulk post {i}"),
                    body: "bulk insert body",
                    is_public: true,
                })
                .await?;
                ids.push(post.id);
            }
            Ok(ids)
        })
    })
    .await?;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "inserted": ids.len(),
        "ids": ids,
    }))
    .status(201))
}
