# Route binding

Status: Agreed 2026-10-05
Prefix: BIND

Drafted 2026-10-05 from issue #144 and the developer's ruling on it, "If
that is how Laravel works then we should support it period", which he
limited to this feature ("I am speaking about this specific feature"), and
his direction on the old surface: "we can replace all of it to be proper...
don't spend a ton of time on backward compatibility if it will add undo
complexity", with the limit he set on it: existing routes keep working, so
handlers written with `RouteParam<User>` or `user::Model` do not change.
The Observed section describes the code at framework `2bd4bd53d` (v3.2.1),
and every line it cites is unchanged at `c34ee7b15`; only the requirements
below it are contract once Agreed. Laravel references cite
`reference/framework-13.27.0/src/Illuminate/` and its line numbers; the
binding behaviour is the same in 13.22.0 and 13.34.0, though some of those
files differ in other ways.

## Observed at 2bd4bd53d

Status: Observed

- Two binding paths exist. A handler argument whose type path ends in
  `Model` with at least two segments (`user::Model`), or any path ending in
  `RouteParam`, binds from the route; a primitive, an integer type or
  `String`, is read through `FromParam`; every other type is treated as a
  form request (`suprnova-macros/src/handler.rs:349-396,405-422`). The
  parameter name is the argument's identifier, or the single field of a
  destructured `RouteParam(x)` (`handler.rs:326-346`). The generated code
  reads the parameter, answering 400 when it is absent
  (`handler.rs:437-438,448-449`; `framework/src/error.rs:1305`), and calls
  `FromParam::from_param` (`handler.rs:433-442`) or
  `AutoRouteBinding::from_route_param` (`handler.rs:443-452`). A route
  whose handler reads a parameter, primitive or bound, that its path does
  not declare answers 400 on every request.
- `RouteParam<M>` for a `#[model]` struct looks the row up through
  `M::query()` filtered on `M::primary_key_name()`, so global scopes, the
  soft-delete filter and the per-model connection apply
  (`framework/src/database/route_binding.rs:339-395`).
- The bare `x::Model` form is a blanket `AutoRouteBinding` impl over SeaORM
  models whose entity implements `EntityExt` (`route_binding.rs:240-265`).
  It calls `EntityExt::find_by_pk`, which applies no scope, no soft-delete
  filter and no per-model connection (`framework/src/database/model.rs:188-209`).
  `#[model]` never implements `EntityExt`; the `db:sync` entity template
  does, in the file sync never overwrites (`suprnova-cli/src/templates/mod.rs:1244,1261-1262`),
  and application code can add it to a `#[model]`'s inner entity, after
  which `x::Model` binds that inner row unscoped (as the tests do:
  `framework/tests/routing/route_binding_route_param_scoped.rs:35`,
  `framework/tests/schema/unsigned_keys.rs:50`). `RouteParam`'s rustdoc
  documents the bare form as the unscoped escape hatch for trashed rows
  (`route_binding.rs:98-111`).
- A key that does not parse as the primary key's type answers 400 with the
  raw value in the message (`route_binding.rs:250`,
  `framework/src/error.rs:977,1313`); a missing row answers 404, naming the
  model as `User` on the `RouteParam` path and `user` on the bare path
  (`route_binding.rs:261,390`). A `unique_id` model's key is a `String`
  (`framework/tests/eloquent/unique_id.rs:11-21`), which every value
  parses as, so a malformed value reaches the database;
  `UniqueIdKind::is_valid` checks the format
  (`framework/src/eloquent/unique_id.rs:77`) and the binding path never
  calls it.
- The `RouteBinding` trait (`param_name`, `from_route_param`) and the
  `route_binding!` macro, which parses only `i32` keys, are exported from
  the crate root and never called or implemented anywhere
  (`route_binding.rs:168-187,306-326`; `framework/src/lib.rs:166-174`;
  `framework/src/database/mod.rs:141`). Stale rustdoc: the trait says
  `#[handler]` calls it (`route_binding.rs:178-179`); the module doc
  presents the bare form as the only automatic binding
  (`route_binding.rs:5-25`), shows `show(post: Post)`, which `#[handler]`
  reads as a form request, at `:52-53`, and cites
  `framework/tests/authorization.rs`, now the folder
  `framework/tests/authorization/`, at `:64`; and `handler.rs:50,62,384`
  name the unused `RouteBinding` trait and `M::find`.
- A `{post:slug}` segment registers a parameter literally named `post:slug`
  (matchit 0.9.2 accepts `:` in a name; `framework/src/routing/macros.rs:46-52`
  checks only the leading `/`), so a handler binding `post` answers 400 on
  every request, `where_*("post")` fails at boot, and `route()` leaves the
  placeholder in the URL (`framework/src/routing/router.rs:358-363,1020-1043`).
  The router supports optional parameters, `{id?}` (`framework/src/routing/params.rs:3-15`).
- Nothing scopes a child parameter to its parent, no binding takes a
  column other than the primary key, no route-level binder exists, and no
  enum binds from a route (an enum argument is a form request).
- `route()` takes `&[(&str, &str)]` pairs only (`router.rs:534`); `try_route`
  reports missing parameters (`router.rs:605`).
- Resource controllers are `Arc<dyn ResourceController>` whose actions take
  a raw `Request` (`framework/src/routing/resource.rs:234-276,298,315`), so
  `#[handler]` binding cannot reach them, and `Router::resources` and
  `api_resources` take lists of `Box<dyn ResourceController>`
  (`resource.rs:727-729,749-751`). Resources have one `parameter` and no
  nesting (`resource.rs:301,389`), and the `parameters()` their docs
  advertise does not exist (`resource.rs:48-49,285-288`).
- Handlers are boxed as type-erased closures (`router.rs:667-668`) at
  every registration site: the router
  (`router.rs:1349,1387,1421,1459,1497,1551,1595,1662`), the group
  builders (`framework/src/routing/group.rs:244,259,274,289,304,324,343,427`),
  the route macros (`macros.rs:985,1456,1499`, the last two before the
  router sees the handler) and resources (`resource.rs:629`). The
  `#[handler]` macro emits only the rewritten function
  (`handler.rs:237-246`), so route registration sees no handler metadata.
  `Server::from_config`, `try_from_config_with_routes` and
  `try_from_config_with_routes_async` return a `Result`, and `Server::run`
  returns one for a server built with `Server::new`
  (`framework/src/server.rs:104,145,158,188,375`).
- Relations are registered through `inventory` with their parent type,
  target type, name, kind and keys
  (`framework/src/eloquent/relations/mod.rs:383-542`), and eager loading
  and existence queries resolve them by name (`EagerLoadDispatch::eager_load`,
  `relations/mod.rs:659-666`; `Builder::with`, `has` and `where_has`,
  `framework/src/eloquent/builder.rs:1269,4460,4521`). The relation query
  methods are inherent (`framework/src/eloquent/relations/has_many.rs:43-45`),
  and nothing looks a child up through a parent's relation by name.
  `EloquentModel` carries no column list (`framework/src/eloquent/mod.rs:124-138`),
  but every SeaORM entity's `Column` enumerates its columns
  (`ColumnTrait: IdenStatic + Iterable + FromStr`,
  `reference/sea-orm-2.0.4/src/entity/column.rs:92`), and `#[model]`
  emits `Column::from_name` (`suprnova-macros/src/model/columns.rs:1-9,69`).
- `#[handler]` decides by spelling, before type checking, which arguments
  read the body (at most one, a compile error otherwise:
  `handler.rs:146-170,188-201`), which are route-bound, and the extraction
  order; it accepts generic arguments (`handler.rs:122,239`). Without
  `#[authorize]` every argument is extracted in declaration order; with
  it, the arguments read from the route are extracted in declaration
  order, then the checks run on the extracted values, then the
  body-reading argument is extracted (`handler.rs:203-246`).
- An `#[authorize("ability", x)]` parameter target may be a route-bound
  value or a primitive path value such as `id: i64`, never a body reader
  (`handler.rs:276-300`); `suprnova-macros/src/lib.rs:471-477` and
  `manual/authorization.md:385-390` document both forms, and
  `framework/tests/prelude/macro_hygiene_qualified_paths.rs:73-78` uses a
  primitive target. A type target (`#[authorize("create", Post)]`) checks
  a default value of the type (`handler.rs:269-273`).
- Tests that pin today's behaviour: the 400 for a malformed key
  (`framework/tests/schema/unsigned_keys.rs:180-195`,
  `route_binding_route_param_scoped.rs:126-138`), trashed rows through the
  bare form (`route_binding_route_param_scoped.rs:84-111`), the
  compile-time refusal of two body readers (`handler.rs:485-521`),
  declaration order without `#[authorize]` (`handler.rs:869-877`), and the
  existing handler forms BIND-003 keeps working (`unsigned_keys.rs:50,59-71`,
  `framework/tests/authorization/handler_authorize.rs:154,241`,
  `framework/tests/routing/route_param_destructured.rs:32-43`,
  `framework/tests/schema/unsigned_reads.rs:52`,
  `macro_hygiene_qualified_paths.rs:73-78`, and the macro tests at
  `handler.rs:561-770`).
- The manual describes binding wrongly in `manual/routing.md:234-325`
  (the bare form only, and a `User::find_by_pk` that does not exist),
  `manual/controllers.md:69,101-127` (any `#[model]` binds through
  `user::Model`), `manual/macros.md:384-398` (`route_binding!`),
  `manual/http-tests.md:750-754`, `manual/tutorial-inertia-crud.md:176-212`
  (`todo::Model` without `EntityExt` does not compile, and lines 117-120),
  `manual/parity.md:60`, `manual/from-laravel.md:163-174` (`post::Model`
  as the signal for every model) and `manual/from-rust-web.md:80,155`
  (binding from `{id}`, where the parameter must carry the argument's
  name). These pages describe behaviour the requirements below change:
  `manual/controllers.md:72-75,194-197` (extraction order),
  `manual/authorization.md:385-390` (`#[authorize]` targets),
  `manual/http-tests.md:765-774`, `manual/urls.md:7-64` (`route()`),
  `manual/routing.md:556-660` (resources) and `manual/glossary.md:733-736`.

## The binding contract

[BIND-001] The framework MUST define one public trait, `RouteBinding`, the
counterpart of Laravel's `UrlRoutable`: the route key's column name
(`route_key_name`, the primary key's by default), the key of a value
(`route_key`) and the value of any binding column (`route_field`), both
for URL generation (BIND-012), the lookup by a value and an optional
column (`resolve_route_binding`), the lookup of a child through a parent
(`resolve_child_route_binding`), and the soft-deletable form of each
lookup, which a route's `with_trashed()` selects
(`resolve_soft_deletable_route_binding`,
`resolve_soft_deletable_child_route_binding`, BIND-008). `#[model]` MUST
implement it for every model it defines, one implementation per model on
the struct the application declares (not its inner SeaORM row), so a
model can replace it (BIND-007). `RouteParam<T>` MUST implement
`RouteBinding` whenever `T` does, delegating to `T`. `AutoRouteBinding`
MUST remain as a second name for `RouteBinding`, reachable as
`suprnova::AutoRouteBinding` and `suprnova::database::AutoRouteBinding`,
with `from_route_param(value)` resolving by the route key. The unused `RouteBinding` trait with
`param_name` and the `route_binding!` macro MUST be removed.
Falsifier: a `#[model]` struct does not implement `RouteBinding`, or `RouteParam<T>` does not for a `T` that does, or resolves otherwise than `T`; `#[model(custom_route_binding)]` still emits one; `<M as AutoRouteBinding>::from_route_param` on a model with `route_key = "slug"` looks up the primary key; code naming `AutoRouteBinding` through `suprnova::` or `suprnova::database::` stops compiling; `RouteBinding` lacks either soft-deletable lookup; or the `param_name` trait or `route_binding!` still exists.
Mechanism: `route-binding`.
Rationale: Laravel's binding is one contract every routable type implements (`Contracts/Routing/UrlRoutable.php:12-38`), with soft-deletable lookups a route's `withTrashed()` selects (`Database/Eloquent/Model.php:2501-2504,2527-2530`, `Routing/ImplicitRouteBinding.php:44-46,51-53`); today's blanket impl cannot be replaced per model, which BIND-007 needs. A blanket impl for SeaORM rows beside per-struct impls from `#[model]` is coherent; the framework already does it for `Persistable` (`framework/src/factory/persist.rs:114-121`, `suprnova-macros/src/model/derive_eloquent.rs:1532-1558`).
Status: Agreed 2026-10-05

[BIND-002] The lookup `#[model]` generates MUST go through the model's
query, so global scopes, the soft-delete filter and the model's
connection apply, and MUST match the column the route names (BIND-004),
else the model's route key. The value MUST be parsed as that column's
type, and checked against the key's format for a `unique_id` key; a value
that does not parse and a value that matches no row MUST both answer 404
with the same body, which names the model's type and never echoes the
value. The bare `x::Model` form (BIND-003) keeps its primary-key lookup
and its body naming the entity's module, MUST answer 404 for a value that
does not parse, and follows BIND-008 for soft-deleted rows.
Falsifier: a generated lookup returns a row a global scope excludes, or a soft-deleted row without `with_trashed()` (BIND-008), which lifts only the soft-delete filter; a model with `#[model(connection = "x")]` binds from the default connection; a malformed value answers anything but 404 in any binding form; a malformed value for a `unique_id` key answers 500 on Postgres; the two 404 bodies differ; or a body contains the requested value.
Mechanism: `route-binding`.
Rationale: Divergence from Laravel, which passes the value to the query unchecked (`Database/Eloquent/Model.php:2575-2578`): MySQL coerces `7abc` to `7` and binds row 7, and Postgres raises an error, a 500; only unique-string ids answer 404 for a malformed value (`Database/Eloquent/Concerns/HasUniqueStringIds.php:54-65,104-107`). Laravel's 404 message carries the value (`Routing/ImplicitRouteBinding.php:58,61`); Suprnova's does not. Today a malformed key is a 400 that echoes the value, and a malformed `unique_id` key reaches the database.
Status: Agreed 2026-10-05

[BIND-003] Existing routes MUST keep working: a handler that binds with
`RouteParam<T>` or with the bare `x::Model` form binds as it does today,
the bare form keeping its lookup by primary key for SeaORM entities that
implement `EntityExt`, and a handler with a primitive `#[authorize]`
target (`#[authorize("show", id)]` with `id: i64`) compiles and checks
the same value (BIND-015). Existing routes see these binding changes
only (the root that PFX-003 and PFX-004 add to URLs is PFX's): a
malformed key answers 404 (BIND-002); a bound argument is extracted
before every other argument, so a missing row answers 404 before the body
is read (BIND-015); the bare form, like every other form, binds a
soft-deleted row only on a route that calls `with_trashed()` (BIND-008),
the documented way to reach soft-deleted rows; and a route whose handler
reads a parameter, bound or primitive, that its path does not declare is
refused at startup (BIND-013), where today it answers 400 on every
request.
Falsifier: a handler written today with `RouteParam<User>`, `user::Model` or `#[authorize("show", id)]` over `id: i64` stops compiling or binds differently, apart from the four changes listed.
Mechanism: `route-binding`.
Rationale: The developer's limit on the replacement: "existing routes keep working". Ruled 2026-10-05: a primitive `#[authorize]` target keeps working, and a route whose handler reads an undeclared primitive parameter is refused at startup.
Status: Agreed 2026-10-05

## Keys and columns

[BIND-004] A route path segment `{name:column}` MUST register the
parameter `name` and record `column` as its binding field, so
`req.param("name")`, `where_*("name")` and `route()` all use `name`, and
the binding of `name` matches `column`; `{name:column?}` is the optional
form. The router MUST refuse at startup a binding field that is not a
column of the model bound to that parameter, or that names a column whose
type cannot be parsed from a path segment, naming the route, the
parameter and the field; a binding field on a parameter whose type
records no columns (BIND-007, BIND-010) is passed to its resolution
unchecked. To make this and the other startup checks (BIND-006,
BIND-007, BIND-013, BIND-015) possible, `#[handler]` MUST record each
handler's arguments, with the parameter each reads, its type and its kind
(bound, primitive path value or body reader), where the router can find
them when the route is registered (an `inventory` entry keyed by the
handler function's type, read wherever the handler is boxed: the router,
the group builders, the route macros and resource registration), and
`#[model]` MUST record each
model's columns. A route whose handler carries no record (a closure, or a
generic `#[handler]` function) is exempt from the checks; a closure binds
nothing. A startup refusal MUST surface as an error returned from the
server's boot path, never as a panic. The checks MUST run when the router
is built for serving, so a router a test builds and drives through
`handle_request` passes the same checks the server's does.
Falsifier: `{post:slug}` registers a parameter named `post:slug`, or `{post:slug?}` anything but an optional `post`; the binding ignores the field; a field naming no column, or a column whose type cannot be parsed from a path segment, starts the application; a route whose handler is a non-generic `#[handler]` function escapes the startup checks, including in a router a test drives through `handle_request`; a startup refusal panics instead of returning an error; or the refusal of a binding field does not name the route, the parameter and the field.
Mechanism: `route-binding`.
Rationale: `Routing/RouteUri.php:39-60` records binding fields the same way, and Laravel passes a field to a custom `resolveRouteBinding($value, $field)` unchecked. Today the router sees no handler metadata, handlers are boxed at 20 sites, and `EloquentModel` carries no column list, though every SeaORM entity's `Column` enumerates its columns.
Status: Agreed 2026-10-05

[BIND-005] `#[model(route_key = "column")]` MUST set a model's route key,
and the build MUST fail when the column is not a column of the model.
Falsifier: a model with `route_key = "slug"` binds by its primary key, or one naming an unknown column or an injected field such as `__eager` compiles.
Mechanism: `route-binding`.
Rationale: The counterpart of `getRouteKeyName()` and Laravel 13's `#[RouteKey]` (`Model.php:2477-2480`, `Database/Eloquent/Attributes/RouteKey.php`).
Status: Agreed 2026-10-05

## Scoped bindings

[BIND-006] Parameters MUST bind in path order. A bound parameter's parent
is the parameter immediately before it in the path, when that one is
bound. The child MUST be looked up through the parent's relationship
whenever it names a column (BIND-004) or the route or its group calls
`scope_bindings()`, and MUST NOT be when the route or group calls
`without_scoped_bindings()` or the child is bound by an explicit binder
(BIND-007); a row the parent does not own MUST answer 404. The
relationship MUST be the parent model's relation named by the child
parameter in the plural as `Str::plural` forms it (`posts` for `post`),
returning the child's type; a relation that cannot return one child
type, such as a `MorphTo`, MUST be refused at startup as a scoped
child's relationship. `#[model]` MUST emit a lookup of a child
through a relation by name, with qualified columns for many-to-many and
through relations. A route that needs a relationship the parent does not
declare MUST be refused at startup, naming the route, the parent and the
relation, unless the parent model replaces its child resolution
(BIND-007).
Falsifier: with a handler taking `user` and `post`, `/users/1/posts/{post:slug}` binds another user's post, or `/users/1/posts/{post}` under `scope_bindings()` does; a route with `without_scoped_bindings()` scopes; a child bound by `bind()` is scoped; a scoped child through a belongs-to-many or has-many-through relation fails to bind or binds a row the parent does not own; a missing, wrongly typed or `MorphTo` relation is found at request time instead of at startup; or that refusal does not name the route, the parent and the relation.
Mechanism: `route-binding`.
Rationale: `Routing/Route.php:597-606`, `Routing/ImplicitRouteBinding.php:48-59`, `Model.php:2546-2549,2562-2565`. Laravel finds a missing relationship at request time as a 500 (`Model.php:2871`); this design finds it before the first request. Relations are registered with their names and types, and eager loading and `has`/`where_has` resolve them by name today, but nothing looks a child up through a parent's relation.
Status: Agreed 2026-10-05

## Custom resolution

[BIND-007] A model MUST be able to replace its binding: `#[model(custom_route_binding)]`
omits the generated implementation, the model implements `RouteBinding`
itself, and the default lookup stays callable as a function. Any other
type that implements `RouteBinding` MUST bind the same way (BIND-015). The
router MUST offer `bind(name, resolver)` and `model::<M>(name, fallback)`,
router-wide and keyed by the parameter name, `-` read as `_`, covering
every route registered before or after them. `bind` receives the value
and the route, ignores binding fields and takes precedence over the
type's own binding; `model` binds `name` to `M` by its route key and
calls the fallback when no row matches. A resolver whose type differs
from the type of the handler argument it binds, or whose parameter the
handler names with an argument that does not implement `RouteBinding`,
MUST be refused at startup.
Falsifier: a model's own `RouteBinding` is not used; a non-model type that implements `RouteBinding` does not bind; a `bind` resolver loses to the type's binding; a binder registered after a route does not cover it; `bind("user-id", ...)` does not cover `{user_id}`; `model` with a fallback answers 404 for a missing row; or a resolver returning another type than the handler argument, or one for a parameter the handler reads as a primitive path value or names with a form request, starts.
Mechanism: `route-binding`.
Rationale: `resolveRouteBinding` overrides, `Route::bind` and `Route::model` (`Routing/Router.php:958-967,1170-1188`; `Routing/RouteBinding.php:34-87`); an explicitly bound child is never scoped (`Routing/ImplicitRouteBinding.php:36-38`). `#[handler]` picks each argument's extraction at compile time (BIND-015), so a binder for a parameter no argument binds could never run.
Status: Agreed 2026-10-05

## Soft deletes and missing rows

[BIND-008] A route's `with_trashed()` MUST make its bindings, scoped
children included, resolve through the soft-deletable lookups of
`RouteBinding` (BIND-001), and every other route's bindings through the
plain ones. The lookups `#[model]` generates and the bare `x::Model` form
(BIND-003) MUST match soft-deleted rows through the soft-deletable
lookups only. For the bare form, a row is soft-deleted when the entity's
table is the table of a `#[model]` that declares soft deletes, by that
model's deleted-at column; an entity no such model covers has no
soft-deleted rows. A resource registration's `with_trashed(actions)` MUST
apply it to those actions, `show`, `edit` and `update` when none are
named.
Falsifier: a soft-deleted row binds on a route without `with_trashed()`, or fails to bind on one with it, as a parent, as a scoped child or through the bare `x::Model` form; a model that replaces its binding receives the plain lookup on a route with `with_trashed()`; or a resource's `with_trashed()` naming no actions leaves `show`, `edit` or `update` unable to bind a trashed row, or applies to another action.
Mechanism: `route-binding`.
Rationale: `Routing/Route.php:614-629`, `Routing/ResourceRegistrar.php:127-130`, `Routing/ImplicitRouteBinding.php:44-46,51-53`; Laravel's trashed rows are excluded by the soft-delete scope (`Database/Eloquent/SoftDeletes.php:31`). Today the bare form binds trashed rows on every route.
Status: Agreed 2026-10-05

[BIND-009] A route, group or resource MUST accept `missing(handler)`,
called with the request when any binding of the route, explicit or
implicit, other than an enum's (BIND-010), finds no row, finds no row the
parent owns (BIND-006), or receives a value that does not parse; its
response replaces the 404.
Falsifier: a route with `missing()`, or a route in a group or resource with it, answers the plain 404 for a missing, malformed or unowned (BIND-006) binding.
Mechanism: `route-binding`.
Rationale: `Routing/Middleware/SubstituteBindings.php:44-47`, `Routing/RouteRegistrar.php:78`, `Routing/PendingResourceRegistration.php:279-284`.
Status: Agreed 2026-10-05

## Enums, resources and URLs

[BIND-010] A unit-only enum MUST be able to derive `RouteBinding`; each
variant binds from exactly one string, its `#[route(value = "...")]`
value or else its name in snake case, matched exactly and
case-sensitively, and a value that matches no variant MUST answer 404
without calling `missing()`. That 404's body MUST name the enum's type
and never echo the value, as BIND-002's does.
Falsifier: a variant's `#[route(value)]`, or a variant without one by its snake-case name, does not bind that variant; a derived enum binds a value differing only in case; a non-matching value answers anything but 404 or runs `missing()`; or that 404's body does not name the enum's type or contains the value.
Mechanism: `route-binding`.
Rationale: Laravel binds string-backed enums by `tryFrom` (`Routing/ImplicitRouteBinding.php:86-97`); its missing hook does not catch enum misses. Rust enums carry no backing value, so the snake-case fallback is a divergence.
Status: Agreed 2026-10-05

[BIND-011] Resource registration MUST accept two forms of controller. A
`ResourceController` implementation keeps its actions that take the
request, so every resource written today compiles and routes unchanged.
Beside it, a function form names a module whose `#[handler]` functions
are the actions, each found by its action name (`index`, `create`,
`store`, `show`, `edit`, `update`, `destroy`) the way
`group!(controller = ...)` finds a handler by its function name; these
actions take bound arguments as handlers do (BIND-015). The function form
registers all seven actions, or those `only()` or `except()` select, and
a selected action whose function the module does not define MUST fail to
compile. Resource
registration MUST support nested resources (`users.posts`), `parameters()`
renaming, `scoped(fields)`, which gives each nested parameter a binding
field and scopes it (BIND-006), `with_trashed(actions)` (BIND-008) and
`missing()` (BIND-009); the last three act on the bindings the function
form's actions make.
Falsifier: a function-form resource action cannot receive a bound model, or a selected action whose function the module lacks compiles; an existing `ResourceController` implementation stops compiling or routes differently; `users.posts` does not register `/users/{user}/posts/{post}` under `users.posts.*`; `parameters()` does not rename; `scoped()` leaves a nested child unscoped or without its field; or `with_trashed()` or `missing()` on a resource does not reach its routes.
Mechanism: `route-binding`.
Rationale: `Routing/ResourceRegistrar.php:123-125,560-569,599-607`. Ruled 2026-10-05: the trait stays for actions that take the request and the function form is added beside it, since typed bound arguments cannot pass through the `Box<dyn ResourceController>` lists `resources()` takes (`framework/src/routing/resource.rs:727-729`).
Status: Agreed 2026-10-05

[BIND-012] `route()` and `try_route()` MUST accept, for each parameter, a
string or a bound value, named or, for a single value, positional, and
fill a bound value with its binding field (`route_field`) when the route
names one, else its route key, percent-encoded as other values are; the
named-route registry MUST keep each route's binding fields. Every call
written today with string values MUST keep compiling and produce the same
URL, apart from the root PFX-003 adds.
Falsifier: `route("posts.show", post)` on `/posts/{post:slug}` produces anything but the post's slug in the path, or on `/posts/{post}` anything but its route key; `try_route` produces a different path for a bound value or rejects one; or a call written today with string values stops compiling or produces a different URL other than by the root PFX-003 adds.
Mechanism: `route-binding`.
Rationale: `Routing/RouteUrlGenerator.php:296-300`, `Routing/UrlGenerator.php:606-617`.
Status: Agreed 2026-10-05

## Startup checks and documentation

[BIND-013] A handler argument that reads a route parameter its route path
does not declare MUST be refused at startup, naming the route and the
parameter, whether it binds through `RouteBinding`, `RouteParam<T>` or
the bare `x::Model` form or is a primitive path value read through
`FromParam` (`id: i64`).
Falsifier: a route starts whose handler binds a parameter its path does not declare, or reads one as a primitive path value, such as `/users/{user}` with `id: i64`; or the refusal does not name the route and the parameter.
Mechanism: `route-binding`.
Rationale: Laravel silently injects an empty model in this case (`Routing/ResolvesRouteDependencies.php:87-92`); refusing at startup is the divergence. Ruled 2026-10-05: primitive path arguments are included, since such routes answer 400 on every request today (`suprnova-macros/src/handler.rs:433-442`).
Status: Agreed 2026-10-05

[BIND-014] The manual MUST describe binding as this contract specifies,
with a "Why Suprnova diverges" section for strict value parsing and the
404 that never echoes the value (BIND-002), the enum name fallback
(BIND-010) and the startup checks (BIND-004, BIND-006, BIND-013,
BIND-015); every manual page and rustdoc comment that describes binding,
those the Observed section lists and `from-laravel.md`,
`from-rust-web.md`, `authorization.md`, `urls.md` and the resources
section of `routing.md` included, MUST be corrected, the tutorial's code
compiling.
Falsifier: a manual page or rustdoc comment still names `route_binding!` or the `param_name` trait, or shows the bare `x::Model` form as the way to bind a `#[model]`; the manual lacks a "Why Suprnova diverges" entry this requirement names; or the tutorial's model, controller and routes, built from `manual/tutorial-inertia-crud.md` by the mechanism, do not compile.
Mechanism: `route-binding`.
Rationale: The manual teaches the bare form as the general one, a removed macro and a `User::find_by_pk` that does not exist.
Status: Agreed 2026-10-05

## The handler form

[BIND-015] A handler argument MUST bind from the route parameter its name
names whenever its type implements `RouteBinding`, written as the type
alone (`post: Post`), the choice made by the generated code at compile
time. A primitive path type (an integer type or `String`, read through
`FromParam`) stays a path value; any other type that does not implement
`RouteBinding` stays a form request, and so does a generic argument,
which the generated code cannot choose for. This form is added beside
`RouteParam<T>` and the bare `x::Model` form (BIND-003). The generated
code MUST extract the arguments bound through `RouteBinding` first, in
path order (BIND-006), then the other arguments in declaration order,
except that on a handler with `#[authorize]` the checks MUST run after
every path value is extracted and before the body-reading argument is.
An `#[authorize]` parameter target MUST be required, at compile time, to
implement `RouteBinding` or to be a primitive path value read through
`FromParam` (`#[authorize("show", id)]` with `id: i64`); the type form
(`#[authorize("create", Post)]`) is unchanged. A handler with more than
one body-reading argument MUST be refused at startup (BIND-004's handler
record), since the macro can no longer tell body readers apart by
spelling. A generic `#[handler]` function is exempt from the startup
checks (BIND-004). An `Option<T>` argument binds `None` when its optional
parameter is absent. The TypeScript route generator (`suprnova
generate-types`) MUST read `{name:column}` as the parameter `name` and a
bound argument as a route parameter, never as the form request.
Falsifier: `post: Post` for a type implementing `RouteBinding` is read as a form request; a type that does not implement it binds from the route; `id: i64` stops being read from the route; an `#[authorize]` parameter target that is neither bound nor a primitive path value compiles, or `#[authorize("show", id)]` over `id: i64` stops compiling; a handler declaring a form request before a bound argument answers 422 for an invalid body where the row is missing; a handler with two body readers starts; an `Option<T>` argument answers 400 when its optional parameter is absent; or the TypeScript route generator rejects `{post:slug}` or types a leading `post: Post` argument as the form request.
Mechanism: `route-binding`.
Rationale: Laravel binds by the type hint alone. Rust macros cannot see trait implementations, so the generated code chooses by trait at compile time. The cost: today's compile-time error for two body readers becomes a startup error, and generic arguments cannot bind. The developer accepted this form on 2026-10-05, and accepted these costs once explained ("ok on BIND-015"). Ruled 2026-10-05: a primitive `#[authorize]` target keeps working; binding by type applies to model-typed targets only.
Status: Agreed 2026-10-05
