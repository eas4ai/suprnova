# Routing

Routing is how Suprnova turns an inbound HTTP request into a handler call.
You declare your routes in `src/routes.rs` using the `routes!` macro (or
build a `Router` by hand), then `Server::from_config` takes that router
and runs it for the life of the process. Same shape as Laravel's
`routes/web.php`, with Rust types instead of facades.

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

The macro expands to `pub fn register() -> Router { ... }`. Call it from
your bootstrap and hand the result to the server.

## HTTP verbs

One macro per verb. All seven take a path-then-handler pair and return a
builder you can chain `.name(...)` and `.middleware(...)` onto.

| Macro | Method | Use for |
|---|---|---|
| `get!`     | GET     | Read endpoints, static pages |
| `post!`    | POST    | Create resources |
| `put!`     | PUT     | Full replacement updates |
| `patch!`   | PATCH   | Partial updates (RFC 5789) |
| `delete!`  | DELETE  | Destroy |
| `head!`    | HEAD    | Headers-only probes (HEAD falls back to the GET registry per RFC 9110 § 9.3.2 when not explicitly registered) |
| `options!` | OPTIONS | Capability discovery, `Accept-Patch`. CORS preflight is answered by `CorsMiddleware` before the router, so you usually don't need this one |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

Every verb macro checks at compile time that the path starts with `/` -
a missing leading slash fails the build, not a request.

### Multi-method and `any!`

`any!` registers one handler against all seven common verbs. Use it for
webhook receivers and other endpoints that need to accept whatever HTTP
sends.

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

When you only want a subset of verbs sharing one handler, reach for the
builder API and `Router::methods`:

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` and `.middleware(...)` fan across every verb the route was
registered against, so reverse-lookup yields the same URL whichever
method the caller looks up.

### WebSocket routes

`ws!` registers a long-lived upgrade handler. The macro is part of the
same `routes!` body - covered in detail by [WebSockets](websockets.md).

## Route parameters

Dynamic segments use curly braces (`{id}`). For familiarity Suprnova also
accepts Express/Rails-style colons (`:id`) and normalises them to braces
before handing the pattern to `matchit`.

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // matchit-native
    get!("/users/:id", controllers::users::show),        // Express/Rails - same thing
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

The colon is only treated as a parameter opener at the start of a path
segment, so literal colons mid-segment survive untouched
(`/files/note:draft` stays a literal route, not `/files/{draft}`).

Read parameters off the request inside a handler:

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

For typed extraction without the `unwrap_or` dance, see route model
binding below or `#[handler]` in [Controllers](controllers.md).

### Optional parameters

End a parameter name with `?` to make the segment optional. `/posts/{id?}`
matches `/posts` and `/posts/42`:

```rust
routes! {
    get!("/posts/{id?}", controllers::posts::show),
    get!("/archive/{year?}/{month?}", controllers::posts::archive),
}
```

On the short form the request has no `id`, and `req.param("id")` returns
`Err(ParamError)`. Read it with `req.param("id").ok()` when you want an
`Option`. The colon spelling takes the `?` too: `/posts/:id?`.

- **Optional parameters fill from the left.** In
  `/archive/{year?}/{month?}` the router matches `/archive`,
  `/archive/2026` and `/archive/2026/05`. It never matches a month
  without a year.
- **Only optional parameters may follow an optional one.**
  `/posts/{id?}/comments` is refused when you register it, because the
  router could not tell a request that leaves `id` out from one that fills
  it. So is an optional parameter that is only part of a segment
  (`/a/pre-{x?}`), and a pattern with an optional parameter and an empty
  segment, such as a trailing slash.
- **Middleware, name and constraints apply to every form.** The route's
  middleware runs on `/posts` as it does on `/posts/42`.
- **A second route for one form of an optional route is refused.**
  `/posts/{id?}` and a separate `/posts` route, or a separate
  `/posts/{id}` route, collide when you register them.
- **A WebSocket route takes optional parameters as well.**
- **`route(...)` leaves an optional segment out when it has no value.**
  `route("archive", &[])` returns `/archive`. A value for a later parameter
  with none for an earlier one is a missing parameter: `try_route` returns
  `RouteUrlError::MissingParams`.

### Parameter constraints

A constraint holds a parameter to the values it may take. The router checks
it after the path has matched. A value the constraint refuses is a `404`, as
if the route had not matched, and neither the route's middleware nor its
handler runs. Chain a constraint onto a route:

```rust
use suprnova::{get, routes};

routes! {
    get!("/posts/{id}", controllers::posts::show).where_number("id"),
    get!("/reports/{year}", controllers::reports::show)
        .where_pattern("year", "[0-9]{4}"),
    get!("/posts/{id}/{status}", controllers::posts::by_status)
        .where_uuid("id")
        .where_in("status", ["draft", "published"]),
}
```

The same methods are on the `Router` builder, where they apply to the route
you just registered:

```rust
use suprnova::Router;

let router = Router::new()
    .get("/posts/{id}", show).where_number("id")
    .get("/archive/{year?}/{month?}", archive)
    .where_pattern("year", "[0-9]{4}")
    .where_in("month", ["01", "02", "03"]);
```

| Method | The parameter must be |
|---|---|
| `where_number(param)` | One or more ASCII digits. |
| `where_alpha(param)` | One or more ASCII letters. |
| `where_alpha_numeric(param)` | One or more ASCII letters and digits. |
| `where_uuid(param)` | A UUID in its hyphenated form, in either case. |
| `where_ulid(param)` | A ULID: 26 characters of Crockford base 32. |
| `where_in(param, values)` | One of `values`, compared exactly. |
| `where_pattern(param, expression)` | A value that the regular expression matches from its first character to its last. |

There is no `where!` macro. The constraints are methods on the route.

- **A pattern has to match the whole value.** `where_pattern("id", "[0-9]+")`
  refuses `12a`, as it does in Laravel.
- **`\d` and `\w` match every script.** They match the digits and letters
  of all scripts, where Laravel's match ASCII alone. Write `[0-9]` for the
  ASCII digits, or use `where_number`.
- **A constraint skips a parameter the request left out.** An optional
  parameter is checked only when it is there.
- **A constraint holds for the method you set it on.** A constraint on a
  `GET` route does not apply to a `POST` route with the same pattern. An
  `any!` route and a route with several methods hold every method they
  cover.
- **A group's constraint may name a parameter of the group's prefix.** In
  `group!("/teams/{team}", { get!("/members", h).where_number("team") })`,
  `team` is a parameter of the prefix.
- **A constraint on a parameter the route does not have stops the boot.**
  It would never be checked, and the route would look guarded while it is
  not. `where_pattern` also panics on an expression that is not a regular
  expression.

Every route builder also has `.constrain(param, ParamConstraint)`, which the
`where_*` methods call. A `Router` route has `.try_constrain(param,
constraint)`, which returns `Result<_, FrameworkError>` where `constrain`
panics. `ParamConstraint::pattern(expression)` returns a `Result` for an
expression that is not a regular expression, and `ParamConstraint::one_of`
builds the list form.

## Route model binding

A handler argument binds from the route parameter its name names whenever
its type implements `RouteBinding`. Every `#[suprnova::model]` struct does,
so naming the model is enough:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::Post;

// Route: GET /posts/{post}
#[handler]
pub async fn show(post: Post) -> Response {
    json_response!({ "title": post.title })
}
```

`#[handler]` reads `{post}`, and the router looks the row up through
`Post::query()` before the handler runs. Global scopes, the soft-delete
filter, and the model's `#[model(connection = "...")]` apply, as they do
to every query of the model. The value is parsed as the key's type, and a
`unique_id` key's value must also be a well-formed identifier. A value
that does not parse and a value that matches no row both answer a 404
whose body names the model, `{"message": "Post not found"}`, without the
value.

The argument's name must match the placeholder (`/posts/{post}`, not
`/posts/{id}`). Primitives (`id: i64`, `slug: String`) stay path values
read through `FromParam`, and any other type stays a form request. Mix
them freely; one argument at most reads the request body:

```rust
// Route: PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(form: UpdateComment, post: Post, comment: Comment) -> Response {
    // post and comment are bound; form is validated.
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

The router binds every bound argument, in path order, after the route's
middleware and before the handler. A missing row is a 404 before the form
is read, even with the form declared first. An `Option<Post>` argument
binds `None` when its optional parameter, `{post?}`, is absent.

### Binding by another column

A `{name:column}` segment registers the parameter `name` and binds it by
`column`. `req.param("post")`, `where_alpha("post")` and `route()` all use
the name:

```rust
routes! {
    get!("/posts/{post:slug}", controllers::posts::show).name("posts.show"),
}
```

`{post:slug?}` is the optional form. To bind a model by another column on
every route, set its route key:

```rust
#[model(table = "posts", route_key = "slug")]
pub struct Post {
    pub id: i64,
    pub slug: String,
    pub title: String,
}
```

The build fails when `route_key` names no column of the model.

### Scoped bindings

When two bound parameters follow each other, the first is the parent of
the second. A child whose segment names a column is looked up through the
parent's relation named by the child in the plural, as `Str::plural` forms
it, so a row the parent does not own answers 404:

```rust
#[model(table = "users", relations = { posts: HasMany<Post> })]
pub struct User {
    pub id: i64,
    pub name: String,
}

// GET /users/{user}/posts/{post:slug}: only the user's own posts bind.
#[handler]
pub async fn show(user: User, post: Post) -> Response {
    json_response!({ "author": user.name, "title": post.title })
}
```

`scope_bindings()` on a route or a group scopes children without a field
too, and `without_scoped_bindings()` turns scoping off. A child bound by
`bind()` is never scoped. Every relation kind with one child type works,
`BelongsToMany` and `HasManyThrough` included; a `MorphTo` cannot scope a
child.

### Custom resolution

A model can replace its binding. `#[model(custom_route_binding)]` leaves
`RouteBinding` to you, and the default lookup stays callable:

```rust
#[model(table = "tags", custom_route_binding)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

#[suprnova::async_trait]
impl suprnova::RouteBinding for Tag {
    fn route_key_name() -> &'static str {
        "name"
    }
    fn route_key(&self) -> String {
        self.name.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, suprnova::FrameworkError> {
        suprnova::database::resolve_model_route_binding::<Self>(
            &value.to_lowercase(),
            Some("name"),
            false,
        )
        .await
    }
}
```

Any other type that implements `RouteBinding` binds the same way. To pass
it to `route()`, implement `RouteValue` with `suprnova::bound_route_value`.
A type that is a scoped parent finds its children in
`resolve_child_route_binding` and says so from `route_binding_info()`
with `.with_children(ChildBindings::Custom)`; without that, a route that
scopes a child under it is refused at startup.

The router binds a parameter name on every route, those registered before
the call and those after. `bind` takes the raw value and the matched route
and wins over the type's own binding; `model` binds by the route key and
calls its fallback when no row matches. A `-` in the name reads as `_`:

```rust
let router = Router::new()
    .bind("user", |value: String, _route| async move {
        User::query().filter("name", value).first().await
    })
    .model::<Team, _, _>("team", |_value| async { Ok(Team::default()) })
    .get("/users/{user}/teams/{team}", controllers::teams::show);
```

### Soft-deleted rows

A soft-deleted row binds only on a route that calls `with_trashed()`.
There, every binding goes through the soft-deletable lookups, scoped
children and the bare SeaORM form included:

```rust
let router = Router::new()
    .get("/admin/posts/{post}", controllers::admin::posts::show)
    .with_trashed();
```

### Missing rows

`missing(handler)` on a route, a group or a resource answers instead of
the 404 when a binding finds no row, finds no row its parent owns, or gets
a value that does not parse. The handler receives the request:

```rust
let router = Router::new()
    .get("/posts/{post}", controllers::posts::show)
    .missing(|_req| async { suprnova::redirect_to("/posts").into() });
```

### Enums

A unit-only enum derives `RouteBinding`. Each variant binds from its
`#[route(value = "...")]`, or else from its name in snake case, matched
exactly. Any other value answers 404, and never reaches `missing()`:

```rust
#[derive(suprnova::RouteBinding)]
pub enum Category {
    Fruits,                // /categories/fruits
    PantryStaples,         // /categories/pantry_staples
    #[route(value = "veg")]
    Vegetables,            // /categories/veg
}

#[handler]
pub async fn show(category: Category) -> Response {
    suprnova::http::text(suprnova::RouteBinding::route_key(&category))
}
```

### Startup checks

`#[handler]` records every argument of a handler: the parameter it reads,
its type, and whether it binds, reads a path value, or reads the body.
Before the first request, the router checks every route against its
record and refuses to start, naming the route and the parameter, when:

- An argument reads a parameter the path does not declare, such as
  `/users/{user}` with `id: i64`.
- A binding field names no column of the model, or a column whose type
  cannot be parsed from a path segment.
- A scoped child needs a relation the parent does not declare, one of
  another type, or a `MorphTo`.
- A binder returns another type than the argument it binds, or covers a
  parameter the handler reads without binding.
- A handler reads the request body twice.

The fallback and every `missing()` handler are checked too, each against
the path of the route it answers. `Server::from_config` returns the
refusal as an error. A router driven through `handle_request` in a test
runs the same checks before its first request and answers every request
with a 500 when they fail, a request the fallback would answer included;
`router.prepare_bindings()` returns the error itself. A closure handler
and a generic `#[handler]` function carry no record and are not checked
at startup. A closure binds nothing. A generic handler's arguments of a
concrete type that implements `RouteBinding` bind as any handler's do: in
path order, by the route's binding fields, scoped, through the router's
binders, with `with_trashed()` and `missing()`. The route plans them at
the handler's first request and keeps the plan; a binding field, a scoped
child or a binder the checks above would refuse answers that request with
a 500 instead of binding. Its generic arguments read the body.

A handler inside an `impl` block names its type,
`#[handler(Self = Posts)]`, and is checked as a free handler is; see
[Handlers inside an `impl` block](controllers.md#handlers-inside-an-impl-block).

### Older binding forms

Two older forms keep working. `RouteParam<Post>` binds as `Post` does. A
SeaORM row whose entity implements `EntityExt` binds by its primary key
through `EntityExt::find_by_pk`, with no global scope and on the default
connection; its 404 names the entity's module:

```rust
impl suprnova::database::EntityExt for post::Entity {}

#[handler]
pub async fn show(post: post::Model) -> Response {
    json_response!({ "title": post.title })
}
```

Bind the `#[model]` struct in new code: its lookup applies the model's
scopes and connection.

### Binding is identity, not authorization

Route model binding answers "does this row exist?" - it does **not**
answer "is the current user allowed to see this row?". A bare bound
handler lets any authenticated user view any post by guessing
`/posts/N`. Authorize against the bound model with
`#[authorize("view", post)]` or `Gate::authorize` - see
[Authorization](authorization.md).

### Opting out

Take the key as a path value and query yourself:

```rust
use suprnova::{handler, json_response, FrameworkError, Model, Response};
use crate::models::User;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = User::find(id)
        .await?
        .ok_or(FrameworkError::model_not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

### Why Suprnova diverges

- **A value is parsed before the query.** Laravel passes the raw value to
  the query, so MySQL reads `7abc` as `7` and binds row 7, and Postgres
  raises an error. Here a value that does not parse as the column's type,
  or breaks a `unique_id` key's format, answers the same 404 a missing row
  does.
- **The 404 never repeats the value.** Laravel's message carries the
  requested value; Suprnova's names the model alone.
- **An enum binds by name.** Rust enums carry no backing value, so a
  variant without `#[route(value = "...")]` binds from its name in snake
  case.
- **Mistakes stop the server at startup.** Laravel finds a missing
  relationship at request time, as a 500, and injects an empty model for
  an argument the route does not declare. Suprnova refuses those routes,
  and the other mistakes the startup checks list, before the first
  request.

## Named routes

Names give you stable identifiers for URL generation. Attach one with
`.name(...)`:

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Names follow the Laravel convention `<resource>.<action>` -
`users.show`, `posts.destroy`, `admin.dashboard`. Look them up with the
top-level `route(name, &[...])` helper:

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")

// A bound value fills its parameter with its route key, or with the
// column a `{post:slug}` parameter names.
let post_url = route("posts.show", &post);
//   Some("/posts/hello-world")
```

`route` returns `Option<String>` and percent-encodes parameter values
into path-safe form (so `("slug", "a/b")` becomes `/posts/a%2Fb` -
matchit-safe and round-trips through `req.param("slug")`). A catch-all
segment such as `/files/{*rest}` takes its value under the name the
handler reads it by, `rest`, and keeps its slashes:
`route("files.show", &[("rest", "docs/a b.txt")])` returns
`/files/docs/a%20b.txt`. For redirect
targets and email links use the strict sibling `suprnova::routing::try_route`,
which returns `Result<String, RouteUrlError>` and refuses to emit a URL
containing an unfilled `{placeholder}` segment. See
[URL Generation](urls.md) for the full URL surface (signed URLs,
absolute URLs, `Redirect::route`).

Route names are globally unique and process-global. Registering the same
name to two different paths panics at boot - silent shadowing was a
security-shaped bug because redirects would route to whichever
registration happened to win. Use `RouteBuilder::try_name` (or
`suprnova::routing::try_register_route_name`) for the fallible variant.

## Per-route middleware

Chain `.middleware(M)` on any route builder:

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // Public
    get!("/", controllers::home::index).name("home"),

    // Protected
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // Multiple middleware compose left-to-right (outermost first)
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

`.middleware_named("auth")` adds the middleware that a registered alias
stands for. `.middleware_named("throttle:60,1")` gives the alias its
arguments, and a name can stand for a group of middleware. See
[Middleware](middleware.md#named-aliases-and-groups).

Route-local middleware runs after any global middleware
(`Server::with_middleware`) and any group middleware that wraps the
route. The middleware map is keyed by `(method, path)`, so attaching
auth to `POST /api/posts` never bleeds onto a public `GET /api/posts`
on the same path. For the middleware contract and writing your own, see
[Middleware](middleware.md).

## Route groups

`group!` factors out a shared path prefix and/or shared middleware:

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Shared /api prefix + middleware
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // Admin area
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

A group prefix is concatenated with each route path. A route at `/`
inside a group resolves to the group prefix exactly
(`group!("/users", { get!("/", index) })` → `GET /users`).

### Group name prefix

`.name(prefix)` puts a prefix in front of the name of every route in the
group. Mirrors Laravel's `Route::name('admin.')->group(...)`. Each route
then names itself by the part the group leaves out:

```rust
routes! {
    group!("/admin/users", {
        get!("/", controllers::admin::users::index).name("index"),   // admin.users.index
        get!("/{id}", controllers::admin::users::show).name("show"), // admin.users.show
        get!("/export", controllers::admin::users::export),          // no name
    }).name("admin.users."),
}
```

The route is known by its full name, `route("admin.users.index", &[])`, and
not by `index`. Three rules:

- **The prefix is used as you write it.** End it with the separator your
  names use, here the dot.
- **A group inside adds its own prefix after this one.** With
  `.name("admin.")` outside and `.name("users.")` inside, a route named
  `index` is `admin.users.index`. A group with no prefix of its own passes
  the outer one on.
- **A route without a name stays without one.** The prefix names nothing by
  itself.

The name prefix is on `group!`. The `Router::group(...)` builder has no
name prefix.

### Group controller

A group can name the module its handlers live in. Write `controller =` after
the path prefix, and a route names its handler by function alone:

```rust
routes! {
    group!("/admin/users", controller = controllers::admin::users, {
        get!("/", index).name("index"),
        post!("/", store).name("store"),
        get!("/{id}", show).name("show"),
    }).name("admin.users."),
}
```

This is the same as writing `controllers::admin::users::index` in each
route. The path prefix stays the first argument of the macro. Two rules:

- **The controller applies to a handler written as one bare name.** A
  handler written as a path, `get!("/health", controllers::health::check)`,
  is used as written. That lets a route reach a handler in another module.
- **A group inside names its own controller.** A nested `group!` is added as
  written, so it takes its own `controller =`, or none.

`.name(...)` and `.middleware(...)` chain onto the group in the same way
with or without `controller =`.

### Nested groups

Groups nest to any depth. Prefixes concatenate; middleware inherits from
parent to child:

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| Route | Effective path | Middleware chain |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

For a single route inside a nested group, the execution order is
**outermost middleware first**: parent group → child group → route-local.
Per-route `.middleware(...)` runs innermost.

## Fallback route

`fallback!` registers a handler that runs when no other route matches.
Use it for custom 404 pages.

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

Fallback supports its own middleware chain (`fallback!(handler).middleware(M)`).
If no fallback is registered, the framework returns a plain-text
`404 Not Found`.

## Resource routing

For a standard 7-action REST surface, implement `ResourceController` and
register the resource through the `Router` builder, or name a module of
`#[handler]` functions with `resource!` (see [Actions that bind](#actions-that-bind)).
Laravel parity for `Route::resource()` and `Route::apiResource()`.

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit default to 404.
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

Methods you don't override return 404. Use `api_resource` to drop
`create` and `edit` - the two routes that exist only to render forms.

### Default routes and names

| Verb | Path | Trait method | Name |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

The path parameter defaults to the singular of the resource name -
`posts` → `{post}`, `categories` → `{category}`. Irregular plurals get
the literal last segment; override with `.parameter(...)`.

### Restricting and renaming

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // pin to two verbs
    .names([("index", "posts.list")])                          // rename a default
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Rust-side aliases that read better in some call sites: `.keep(...)` for
`.only(...)`, `.drop(...)` for `.except(...)`, `.rename(...)` for
`.names(...)`.

### Actions that bind

A `ResourceController` action takes the request. For actions that take
bound arguments, name a module of `#[handler]` functions with `resource!`
instead; each action is the function of its name:

```rust
use suprnova::{routes, resource, api_resource};

// controllers::posts::{index, create, store, show, edit, update, destroy}
routes! {
    resource!("posts", controllers::posts),
    resource!("photos", controllers::photos, only = [index, show]),
    api_resource!("tags", controllers::tags, except = [destroy]),
}
```

```rust
// src/controllers/posts.rs
#[handler]
pub async fn show(post: Post) -> Response {
    json_response!({ "title": post.title })
}
```

`only = [...]` and `except = [...]` go inside the macro: an action the
selection keeps whose function the module does not define fails to
compile, and an action it leaves out needs no function. The builder takes
`.names(...)`, `.parameter(...)`, `.parameters(...)`, `.unnamed()` and
`.authorize_resource::<U, R>()` as the controller form does.

### Nested resources

A dotted name nests one resource in another. `users.posts` registers
`/users/{user}/posts` and `/users/{user}/posts/{post}`, named
`users.posts.index` through `users.posts.destroy`:

```rust
routes! {
    resource!("users.posts", controllers::user_posts)
        .parameters([("users", "author")])   // /users/{author}/posts/{post}
        .scoped([("post", "slug")])          // /users/{author}/posts/{post:slug}
        .with_trashed(&[])                   // show, edit and update bind trashed rows
        .missing(|_req| async { suprnova::redirect_to("/posts").into() }),
}
```

`parameters` renames a segment's parameter. `scoped` gives parameters a
binding field and scopes every nested parameter to its parent; an empty
list scopes without fields. `with_trashed` binds soft-deleted rows on the
actions it names, `show`, `edit` and `update` when it names none.
`missing` answers for every route of the resource when a binding finds
nothing. The controller form takes the same four.

### Bulk registration

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### Authorizing the whole resource

`authorize_resource::<U, R>()` attaches the conventional ability check to
every generated route as per-route middleware - Laravel's
`authorizeResource` parity. Without it, a resource surface is ungated
unless every controller body remembers to call `Gate::authorize`; a single
forgotten `destroy` ships an ungated delete.

```rust
use suprnova::{Router, Gate};

// Abilities are keyed on (ability, user type, resource marker type).
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

The action → ability mapping mirrors Laravel:

| Action(s) | Ability |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` shares the `update` action, so it is gated identically to `PUT`. A
denied ability short-circuits with `403` before the handler runs, and an
unauthenticated request fails closed. The resource marker `R` only needs
`Default` - the gate discriminates on its *type*, the way Laravel
discriminates on the model class. See the [authorization chapter](authorization.md)
for defining the abilities themselves.

## Router-level redirects and views

Three sugar methods on `Router` cover route declarations that don't need
a handler function:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // Static redirect: GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // 301 sibling
    .permanent_redirect("/legacy", "/new")
    // Inertia static page: GET /about renders the About component
    .inertia("/about", "About", json!({ "team_size": 4 }))
    .name("about");
```

`Router::inertia` is Suprnova's `Route::inertia($uri, $component,
$props)`. It registers `GET`; a `HEAD` request falls through to it and
has its body stripped at the server boundary, so there's nothing extra
to register. It returns a `RouteBuilder`, so `.name(...)` and
`.middleware(...)` chain off it like any other route.

Props must be a JSON object, or `null` for none. Anything else -
an array, a string - is a registration error, not a silently empty prop
bag. `try_inertia` is the fallible form.

`Router::view` is the same method under its older name; it returns
`Router` rather than `RouteBuilder`, so a route declared with it can't
be named. Prefer `inertia`.

### Why Suprnova diverges

Laravel's `Route::view` renders a Blade template; Suprnova renders an
Inertia component, because the framework's templating system is Inertia,
not Blade. One consequence: the component name is a runtime string here,
so it doesn't get the compile-time page-component check that the
`inertia_response!` macro performs. Write the handler out with
`inertia_response!` when you want a typo in a component name to fail the
build rather than the request.

For redirect *responses* (not route declarations) - `Redirect::route`,
`Redirect::back`, `Redirect::intended`, signed redirects - see
[URL Generation](urls.md) and [Responses](responses.md).

## Signed URLs

HMAC-signed routes are routing-adjacent (you mint a URL against a named
route, then verify the signature on the inbound request). They're
covered in full by [URL Generation](urls.md); the short version:

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

Verify inside a handler with `url::has_valid_signature(&request)` (boolean)
or `url::signature_verdict(&request)` (the three-way
`Valid`/`Expired`/`Invalid` split, so you can render a "request a fresh
link" page instead of a generic 403).

## Fallible registration

Route registration runs once at boot, so a duplicate or malformed route
is treated as a programmer error: the plain helpers (`Router::get`,
`post`, `put`, `delete`, `ws`, `RouteBuilder::name`, the
`GroupBuilder` → `Router` `From` conversion) **panic** to fail loudly at
startup. That's the right default for routes declared in source.

When patterns or names come from a fallible source - dynamic config, a
plugin system, a test that deliberately registers conflicting routes -
use the `try_*` siblings. They return `Result<_, FrameworkError>`
(naming the offending method, path, or conflicting name) instead of
panicking:

| Panicking | Fallible sibling | Returns |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws` (and every `ws_*` variant) | `try_ws` (and every `try_ws_*`) | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router` via `.into()` | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` comes from dynamic config; a malformed or duplicate pattern
// is recoverable, not a startup panic.
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

A duplicate group route is recoverable the same way - because `From`
cannot be fallible, the fallible counterpart of `.into()` is the
inherent `try_finalize` method:

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

The panicking helpers stay as ergonomic escape hatches; the `try_*`
siblings are purely additive.

## Why Suprnova diverges

**Dual path-parameter syntax.** Laravel uses `{param}`; Express uses
`:param`. Suprnova accepts both and normalises `:param` to `{param}`
before the path reaches `matchit`. Both styles compose with everything
else - groups, model binding, signed URLs. The reason isn't
indecisiveness; it's that we can't predict which background you bring,
and routing syntax is too high-frequency a friction point to make people
relearn.

**Two co-equal APIs: macro and builder.** Laravel ships one DSL
(`Route::get(...)`). Suprnova ships the declarative `routes! { ... }`
macro AND the chainable `Router::new().get(...).name(...)` builder.
They produce identical registrations. The macro reads better for
top-level route tables; the builder reads better when you're composing
routers dynamically (plugins, generated routes, tests). Pick whichever
fits the call site - there's no canonical answer because both shapes
are first-class.

**Boot-time panics, not silent shadowing.** A duplicate route name or
pattern collision panics at startup. Laravel's array-keyed registries
silently let the later registration win, which is fine when your routes
file is the only registrar but unsafe once plugins or generated routes
enter the picture. `try_*` siblings are the escape hatch when fallibility
is what you actually want.

## Next

- [Controllers](controllers.md) - `#[handler]`, form requests, returning JSON/Inertia
- [Middleware](middleware.md) - the `Middleware` trait, ordering, building your own
- [URL Generation](urls.md) - named-route URLs, signed URLs, redirects, `RouteUrlError`
- [Authorization](authorization.md) - gates and policies for bound models
- [WebSockets](websockets.md) - `ws!`, the `WebSocketHandler` trait, per-route config
