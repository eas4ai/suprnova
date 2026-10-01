# Authorization

Authentication answers _"who are you?"_; authorization answers _"are you
allowed to do this?"_ Suprnova ships a Laravel-shaped `Gate` facade plus the
`#[policy]` macro for resource-oriented wiring, with sync and async variants
of every check so the same surface works whether your policy body needs a DB
hit or just a struct-field comparison.

## Quick start

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// Lets users opt into the `user.can(action, &resource)` ergonomics.
impl Authorizable for User {}

// Wire one ability:
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// Return 403 directly from a handler:
alice.authorize("update", &foreign_post)?;
```

## The `Gate` surface

### Defining abilities

```rust
// Sync closure - invoked directly, no boxed future.
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// Async closure - the future must be owned (no borrows past closure return).
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ...DB lookup, RPC call, etc.
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

Type-erased internally; the registry keys on `(action, TypeId<U>, TypeId<R>)`.
A `User` action gate and a `Comment` action gate of the same name live
independently - `Gate::has::<User, Post>("publish")` and
`Gate::has::<User, Comment>("publish")` answer separately.

### Checking abilities

| Method | Returns | Use |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | Quick branch |
| `Gate::denies(action, &user, &resource)` | `bool` | Inverse |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 403 on a bare deny; a rich denial carries its own status/message (see [Rich decisions](#rich-decisions-response-inspect-raw)) - short-circuits a handler with `?` |
| `Gate::inspect(action, &user, &resource)` | `Response` | Full decision: `allowed` + `message` + `code` + HTTP `status` |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | Like `inspect`, but `None` = no rule defined (vs an explicit deny) |
| `Gate::any(&[...], &user, &resource)` | `bool` | True if any allow |
| `Gate::none(&[...], &user, &resource)` | `bool` | True if none allow |
| `Gate::check(&[...], &user, &resource)` | `bool` | True if all allow |

Every method has an `_async` sibling that works for both sync- and
async-registered gates, so handlers don't need to know which kind of
closure backs the action.

### Introspection

```rust
// Is an ability defined?
Gate::has::<User, Post>("publish");  // bool

// What abilities exist? (sorted + deduped by action name)
let all: Vec<String> = Gate::abilities();
```

`abilities()` dedupes across resource types: registering `"view"` for
both `User`-on-`Post` and `User`-on-`Comment` yields a single `"view"`
entry. Useful for admin pickers and Inertia shared-data.

### Missing-gate semantics

Calling `allows` / `denies` / `authorize` on an action that was never
registered **defaults to deny**. Same for calling the sync API on an
async-registered gate (the sync path can't await - defaulting deny
surfaces the bug in logs via `tracing::warn!` rather than silently
passing). Async-registered gates respond correctly from the
`_async` paths.

## Policies with `#[policy]`

When a resource type has several abilities, group them into a policy struct
and let `#[policy]` register every method as a gate:

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // A `-> bool` method is a plain allow/deny gate.
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // anyone can list posts
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // A `-> Response` method can carry a message + HTTP status on denial.
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // hide the post from non-admins
        }
    }
}
```

Each method becomes one `inventory::submit!`. `Server::run` drains the
inventory via `init_policies()` at boot, so by the time the first request
arrives every action is registered (see [Bootstrap](bootstrap.md) for where
this slots into the boot sequence). `init_policies()` lives at
`suprnova::authorization::init_policies` and is idempotent - call it manually
in tests that exercise policy registration without standing up a server.

Policy methods are stateless associated functions taking `(user, resource)` -
the same shape as Laravel's `update(User $user, Post $post)`, where `$this` is
the stateless policy object. Every method takes both arguments for a uniform
gate signature; `view_any` / `create` simply ignore the resource (`_post`).
Methods you don't write aren't registered, and an unregistered action
default-denies.

### Method-name → action mapping

Method name is used directly as the action's verb segment, with the
resource kebab-cased and suffixed:

| Method | Action |
|---|---|
| `view` on `Post` | `"view-post"` |
| `view_any` on `Post` | `"view_any-post"` |
| `force_delete` on `UserProfile` | `"force_delete-user-profile"` |

This diverges from Laravel's camelCase action names (`viewAny`,
`forceDelete`) to keep the Rust surface idiomatic - every action
string mirrors the method identifier you'd autocomplete in your
editor.

### Return type: `bool` or `Response`

A policy method's return type selects how it registers - and what a denial
can carry:

| Return type | Registers via | Denial surfaces as |
|---|---|---|
| `bool` | `Gate::define` | bare `403` (`This action is unauthorized.`) |
| `Response` | `Gate::define_with` | the message, code, and HTTP status the `Response` carries |

Return `bool` for a simple yes/no. Return a `Response` (imported from
`suprnova::authorization::Response`) when a denial should carry a reason or a
non-403 status - `Response::deny_with("…")` for a message, or
`Response::deny_as_not_found()` to answer `404` and hide the resource's
existence. Both compile to the same type-erased gate (a `bool` is wrapped into
a bare allow/deny). Any other return type - or a missing one - is a compile
error.

## The `Authorizable` trait

Drop-in user-side sugar for the `Gate` calls:

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// Sync sugar
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 403 on deny

// Async sugar
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

Every method has a default body that delegates to the matching `Gate`
method, so `impl Authorizable for User {}` (no body) is enough.
Opt-in rather than blanket-impl: not every type that can be passed to
`Gate::allows` is meant to be the subject of `.can` - most often
it's your application's `User`.

## Composition patterns

### Gating route groups

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// Middleware checks the auth user; the handler authorizes the action.
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ... render edit form
}
```

### Many-action checks

A "list all the things this user can do on this resource" page:

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// Or short-circuit:
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### Multi-gate authorization

```rust
// Only allow if the user can do ALL of these actions on the resource.
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // Combine checks.
}
```

### Gating resource routes

When a `Router::resource` surface exists, `authorize_resource::<U, R>()`
wires the conventional ability check onto all seven routes at once, so you
do not depend on every controller method remembering to authorize:

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view, store→create, …
    .into();
```

A denied ability returns `403` before the handler runs; an unauthenticated
request fails closed. The check runs against the user of the route's guard,
the same user `#[authorize]` checks (see [Authorize a
handler](#authorize-a-handler)). The full action → ability table lives in
the [routing chapter](routing.md).

## Authorize a handler

A gate check inside the handler body works only if nobody forgets it.
`#[authorize]` on a `#[handler]` declares the check instead, the way
Laravel's `#[Authorize]` controller attribute does:

```rust
use suprnova::http::text;
use suprnova::{Response, RouteParam, authorize, handler};
use crate::models::Post;
use crate::requests::StorePost;

// Route: post!("/posts", controllers::post::store)
#[handler]
#[authorize("create-post", Post)]
pub async fn store(form: StorePost) -> Response {
    text(format!("stored {}", form.title))
}

// Route: put!("/posts/{post}", controllers::post::update)
#[handler]
#[authorize("update-post", post)]
pub async fn update(post: RouteParam<Post>) -> Response {
    text(format!("updated {}", post.id))
}
```

The second argument is either a parameter or a type:

- A single name that starts with a lowercase letter, such as `post`, is a
  parameter of the handler. The check runs against the value the route
  binds to it. For a `RouteParam<Post>` parameter, that is the `Post`
  inside. The name can also be the binding of a pattern, as in
  `RouteParam(post): RouteParam<Post>`. A name the handler does not take
  is a compile error.
- Anything else, such as `Post` or `post::Model`, is a type. The gate is
  keyed by type, so the check runs against `Post::default()`, the same
  stand-in [`authorize_resource`](#gating-resource-routes) uses. Every
  `#[suprnova::model]` struct implements `Default`.

The ability goes to the gate as written. A `#[policy(User, Post)]` method
`update` registers the ability `update-post`, so a handler that relies on
that policy names `"update-post"`. An ability registered with
`Gate::define` or `Gate::define_async` is named the way it was registered.

The check runs at a fixed point in the request:

1. The route parameters are bound. A model that does not exist answers
   `404 Not Found`, whatever the gate would decide.
2. Each `#[authorize]` runs, in the order written. The first one that
   fails ends the request.
3. The request body is read and validated, so a denied user never sees a
   validation error.
4. The handler body runs.

The check asks the async gate about the user of the route's guard.
Policies, async gates, async `before` hooks, and the
[permission bridge](#answering-the-gate-with-permissions) all answer it.

The route's guard is the guard that the last `AuthMiddleware` to pass the
request on checked. For `AuthMiddleware::new().for_guard("api")`, that is
the `api` guard, so the check runs against the user the `api` guard
authenticated. With no `for_guard`, or with no `AuthMiddleware` at all,
it is the default guard, and the user is the one `Auth::user()` resolves:

```rust
use suprnova::{AuthMiddleware, Router};

// `update` carries `#[authorize("update-post", post)]`. The check asks the
// `api` guard for the user, not the default guard.
pub fn routes() -> Router {
    Router::new()
        .put("/api/posts/{post}", controllers::post::update)
        .middleware(AuthMiddleware::new().for_guard("api"))
        .into()
}
```

When the route's guard has no user, the check answers `401`, even if
another guard has one. A user of the default guard never stands in for the
user of the route's guard. This also holds under
`AuthMiddleware::optional().for_guard("api")`, which lets a guest through.

When the check fails, it answers the request with one of these statuses:

| Situation | Status |
|---|---|
| No user on the route's guard | `401 Unauthorized`, `{"message": "Unauthenticated."}` |
| The gate denies | `403 Forbidden` |
| A rich denial with a status, such as `Response::deny_as_not_found()` | That status, `404 Not Found` here |

`#[authorize]` may sit above or below `#[handler]`, and a handler may carry
several. Without `#[handler]` it is a compile error. The handler must be an
`async fn`, and the parameter it names must come from the route: a
`RouteParam<M>`, a `...::Model`, or a path value such as `id: i64`. A form
request or a `Request` reads the body, and the check runs before the body
is read, so naming one is a compile error too.

### Why Suprnova diverges

A guest gets `401 Unauthorized`. Laravel's `can` middleware passes a guest
to the gate, which answers `403 Forbidden` unless a policy method accepts a
missing user. Suprnova answers the way `AuthMiddleware` does for a guest,
so a client can tell "log in" from "not allowed".

In Laravel, `auth:api` makes `api` the default guard for the rest of the
request, so `Auth::user()` and the `can` middleware both read the `api`
user. In Suprnova, `AuthMiddleware::for_guard("api")` doesn't change the
default guard: `Auth::user()` still reads the default guard, and only the
`#[authorize]` check follows the route's guard. To read the same user in
the handler body, call `Auth::guard("api")?.user()`.

Laravel tells a model class from a route parameter by the backslash in
`Post::class`. Rust has no class strings, so Suprnova uses Rust's naming
conventions instead: a single lowercase name is a parameter, and anything
else is a type. For the type form, Laravel calls a policy method that takes
no model. Suprnova's gate always passes a resource, so the type form passes
a default value of the type.

## Async semantics

`Gate::define_async`'s closure must return an **owned** future - the
type-erased registry cannot let `&user` or `&resource` references
outlive the closure return. Copy or clone any fields you need inside
the `async move {}` block before returning it:

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // copy primitive
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // No `user` / `post` references here - only the captured copies.
        admin || check_can_publish(user_id, post_id).await
    }
});
```

Sync gates work transparently from the async path (`Gate::allows_async`
dispatches them without an `.await`), so a codebase can register
sync gates today and migrate individual abilities to async later
without changing call sites.

## Lock-poison posture

The `Gate` registry uses an `RwLock` internally. If the lock is ever
poisoned (a thread panicked while holding the write guard), the
registry **safe-denies** - every subsequent `authorize` call returns
`Unauthorized` rather than panicking. Registration calls log to
`tracing::error!` and continue. This matches the broader framework
policy: a poisoned lock never aborts the process.

## Rich decisions: `Response`, `inspect`, `raw`

A bare `bool` gate answers only allow/deny. For a denial that carries a
*message*, a machine *code*, or a non-403 HTTP *status*, register the gate
with `define_with` (or `define_async_with`) and return a `Response`:

```rust
use suprnova::authorization::Response;  // re-exported at the crate root as `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// Hide a resource's existence rather than admit it exists:
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // a 404, not a 403
    }
});
```

Inspect the full decision with `Gate::inspect` (sync) / `Gate::inspect_async`:

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - None here; Some(404) after deny_as_not_found
```

`Response` constructors mirror Laravel: `allow()`, `deny()`,
`deny_with(msg)`, `deny_with_status(status, msg)`, `deny_as_not_found()`,
plus `with_message` / `with_code` / `with_status` / `as_not_found` builders.

### How a denial becomes an error

`Gate::authorize` collapses the decision through `Response::authorize()`:

| Decision | `authorize` result |
|---|---|
| allowed | `Ok(())` |
| bare `deny()` (no message/code/status) - what an unconfigured default denial response falls back to | `FrameworkError::Unauthorized` (403, `"This action is unauthorized."`) |
| rich denial (message and/or status set) - including a configured default denial response that carries one | `FrameworkError::Domain { message, status_code }` |

So `deny_as_not_found()` surfaces as a 404, `deny_with_status(422, "…")` as a
422, and `deny_with("…")` as a 403 carrying your message. The `code` is
readable on the inspected `Response` but does **not** travel through
`authorize` - `FrameworkError` has no code field; read it from `inspect()` if
you need it.

Whichever status a denial lands on, it reaches the client as the
framework's JSON error body. An Inertia app should also name an
[error page](frontend-inertia-responses.md#error-pages) - without one,
the Inertia client treats that body as a non-Inertia response and shows
its full-screen error modal instead of rendering anything, so a user
with the wrong role sees a crash rather than "you cannot do that".

### `raw`: "denied" vs "undefined"

`Gate::raw` (and `raw_async`) returns `Option<Response>`: `None` means *no
rule applied* - no `before` hook fired, no gate is registered, no `after`
hook filled in - as distinct from an explicit `Some(deny)`. `inspect`
normalizes that `None` to the configured default denial response (a bare
deny unless `Gate::default_denial_response` has set something else); `raw`
preserves the `None` for diagnostics ("is this action governed at all?").

### Default denial response

Laravel's `Gate::defaultDenialResponse($response)` reshapes what an
*undecided* denial looks like - not every denial, only the ones that would
otherwise fall back to the bare `Response::deny()`. Set it once, typically
in `bootstrap::register()`:

```rust
use suprnova::authorization::Response;
use suprnova::Gate;

Gate::default_denial_response(Response::deny_as_not_found());
```

After that call, two kinds of outcome pick up the new shape: a bare
`false` - from a bool gate (`define`/`define_async`, including a `#[policy]`
method returning `bool`), or from a `before`/`after` hook that decided
`false` -
and an evaluation nothing else decided at all: an undefined ability with no
hook opinion either. All of those used to surface as a bare
`Response::deny()` (a 403); now they surface as whatever
`default_denial_response` was given - a 404 in the example above. That is
the standard "hide the resource's existence from a user who may not view
it" move (see the `Secret` example earlier in this chapter), applied once
for the whole application instead of gate by gate.

The default applies to **bare `false` only**. A gate registered with
`define_with` (or `define_async_with`) already returned the `Response` it
wanted - `Response::deny_with("…")`, `Response::deny_as_not_found()`, even
an explicit bare `Response::deny()` - and every one of those passes through
`inspect` untouched. This mirrors Laravel's own rule: `Gate::inspect` only
substitutes the default for a truly falsy callback result, never for a
`Response` object the callback built itself.

## `before` / `after` hooks

`Gate::before` registers a check that runs *before* any gate; the first hook
to return `Some(decision)` short-circuits everything. The canonical use is a
global override:

```rust
// Administrators may do anything.
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` runs *after* the gate. Following Laravel's `??=` semantic, an
after hook can only **fill in** an undecided result (no gate matched and no
before hook fired) - it can never override an allow/deny already produced.
Every after hook still runs, so it doubles as the audit-logging seam:

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // observe every evaluation
    None                                    // record-only; don't change the result
});
```

Hooks are keyed by the **user type** `U`, not by resource - a hook fires for
every `(action, U, R)`. Put resource-specific logic in the gate. A `before`
hook is a synchronous predicate and applies to the async evaluation path too;
for async authorization logic in a gate, use `define_async` /
`define_async_with`.

### Async `before` hooks

A hook that has to wait on I/O, such as a database read, cannot be a
synchronous closure. Register it with `Gate::before_async`. The closure must
return an owned future, as `define_async` requires:

```rust
// Staff, as the directory service lists them, may do anything.
Gate::before_async::<User, _, _>(|user, _action| {
    let id = user.id;
    async move { is_staff(id).await.then_some(true) }
});
```

Sync and async `before` hooks share one list per user type. Evaluation walks
the list in the order you registered the hooks, and the first `Some(decision)`
wins. Only the async forms of the gate (`allows_async`, `denies_async`,
`authorize_async`, `inspect_async`, `raw_async` and the async multi-action
methods) wait on an async hook. Never block a thread on I/O inside a
synchronous hook to get around this: it stalls a runtime worker on every check.

#### A denial here is not enforced everywhere

A hook that answers `Some(false)` denies only on the async forms. The forms
that cannot wait skip the hook and go on as if it had answered nothing, so a
gate that allows still allows there. These forms are `Gate::allows`,
`denies`, `authorize`, `inspect`, `raw`, `any`, `none` and `check`, and `can`,
`cannot` and `authorize` on [`Authorizable`](#the-authorizable-trait). A hook
that must deny belongs in `Gate::before`. Otherwise every check your
application makes has to use an async form.

### Why Suprnova diverges

Laravel's `Gate::forUser($user)->allows(...)` rebinds the gate's *implicit*
current-user resolver so the next check evaluates as that user. Suprnova's
gate takes the user **explicitly** on every call, so "check as a different
user" is just `Gate::allows(action, &other_user, &resource)`. There is no
implicit resolver to rebind - the explicit API is strictly more general,
which makes `forUser` redundant rather than missing.

The same reasoning applies to Laravel's policy auto-discovery by class name.
Suprnova ties policy methods to the type-erased `(action, U, R)` key at
registration time, so a `Post` policy and a `Comment` policy with the same
method name register two distinct gates without a naming convention or a
discovery scan.

`Gate::default_denial_response` also diverges from Laravel in one respect:
passing it an allow-shaped `Response::allow()` is logged and ignored rather
than accepted. Laravel's `defaultDenialResponse` has no such guard, but this
is a *denial* default - accepting an allow-shaped one would silently invert
every bare `false` gate result to allowed, the one fail-open direction on
this surface.

## Roles and permissions

Suprnova ships a small role and permission system in `suprnova::rbac`. It is
optional. You add the tables, implement one trait on your user model, and
assign roles and permissions to users.

- A **permission** is a name, such as `"posts.publish"`.
- A **role** is a named set of permissions, such as `"editor"`.
- A user holds a permission **directly**, or **through a role** that carries it.

Add `suprnova::rbac::migrations::CreateRbacTables` to your migrator. It
creates the `roles`, `permissions`, `role_permissions`, `model_roles` and
`model_permissions` tables. Every role and permission has a guard name. The
helpers that take no guard use `"web"`, and so does every check on this
page.

Implement `HasRoles` on the user model. It has no required methods:

```rust
use suprnova::HasRoles;
use suprnova::rbac::{create_role, give_permission_to_role};

impl HasRoles for User {}

// Setup, for example in a seeder:
create_role("editor").await?;
give_permission_to_role("editor", "posts.publish").await?;

user.assign_role("editor").await?;
user.give_permission_to("posts.delete").await?;

user.has_role("editor").await?;                  // true
user.has_permission_to("posts.publish").await?;  // true, through the role
user.has_permission_to("posts.delete").await?;   // true, held directly
```

For a route, `RoleMiddleware::<User>::new("editor")` and
`PermissionMiddleware::<User>::new("posts.publish")` require the role or the
permission. Put them after `AuthMiddleware`. A user who lacks it gets a `403`,
or a redirect if you build the middleware with `redirect_to`. A failed
database read also refuses the request.

### Answering the gate with permissions

Without more setup, the gate does not know about permissions:
`Gate::allows_async("posts.publish", &user, &post)` asks only the gate
definitions and the policies. Call `register_gate_bridge` once, in
`bootstrap::register()`, to connect the two:

```rust
suprnova::rbac::register_gate_bridge::<User>();

// Allowed when the user holds "posts.publish", directly or through a role.
if Gate::allows_async("posts.publish", &user, &post).await {
    // ...
}
```

The bridge is a `before` hook of the gate, built on `Gate::before_async`. It
follows these rules:

- **It allows and never denies.** If the ability is a permission the user
  holds, the gate allows. For any other ability the bridge has no opinion, and
  the gate goes on to its definitions and policies. A user who does not hold
  the permission can still be allowed by a gate or a policy.
- **A `before` hook answers first.** A held permission allows the ability of
  the same name even where a gate definition or a policy would deny it. This
  is why the bridge is opt-in: keep permission names and policy ability names
  apart unless you want them to overlap.
- **Only the async forms see permissions.** The bridge reads the database,
  so `allows_async`, `authorize_async`, `inspect_async` and the other async
  forms use it. `allows`, `authorize` and `inspect` skip it, and a permission
  never answers them.
- **One read per request.** The bridge reads all the permissions of a user
  with one query the first time a check in a request needs them, and answers
  the rest of the request from that set. It keeps nothing after the request.
  A grant or a revocation made before the first check of a user in a request
  is seen by that check. After that check, it is seen from the next request.
- **A check in a transaction reads for itself.** Inside `DB::transaction` a
  check neither uses the set nor adds to it. It reads on the transaction, so it
  sees what the transaction changed, and it keeps nothing.
- **A unit of work with its own scope reads for itself.** A job that the sync
  queue driver runs inline, or a future you run with `App::run_scoped`, opens
  its own container scope inside the request. Its checks read the database
  again and leave nothing for the request.
- **Outside a request, every check reads the database.** A queued job, a
  console command or a task you spawn from a handler is not inside the request.
- **A failed read allows nothing.** The bridge logs the error, without the id
  of the user, and answers nothing. The gate then denies unless a definition
  or a policy allows.
- **The guard is `"web"`.** Permissions on another guard do not answer the
  gate.

`register_gate_bridge` also adds `GateBridgeMiddleware` to the front of the
global middleware, which holds the per-request set. Calling it twice for the
same user type changes nothing. If you never call it, nothing changes: the gate
answers as before.

### Why Suprnova diverges

The gate takes the user explicitly and has a sync surface and an async
surface. A permission is a database read, so it can answer only the async
surface. Use `allows_async` and `authorize_async` wherever a permission
decides.

## Next

- [Authentication](authentication.md) - the user-side half: guards,
  `Auth::user()`, `Auth::user_as::<T>()`
- [Bootstrap](bootstrap.md) - where `init_policies()` runs in the boot
  sequence, plus how to register before/after hooks
- [Middleware](middleware.md) - pairing `AuthMiddleware` with route-level
  authorization
- [Error Model](error-model.md) - how a gate denial collapses into a 403, a
  404, or a custom-status `FrameworkError::Domain`
- [Events](events.md) - listening on policy outcomes via `Gate::after` for
  audit logging
