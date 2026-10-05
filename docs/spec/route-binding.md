# Route binding

Status: Draft
Prefix: BIND

Drafted 2026-10-05 from issue #144 and the developer's ruling on it, "If
that is how Laravel works then we should support it period", which he
limited to this feature ("I am speaking about this specific feature"), and
his direction on the old surface: "we can replace all of it to be proper...
don't spend a ton of time on backward compatibility if it will add undo
complexity", with the limit he set on it: existing routes keep working, so
handlers written with `RouteParam<User>` or `user::Model` do not change.
The Observed section describes the code at framework `2bd4bd53d` (v3.2.1);
only the requirements below it are contract once Agreed. Laravel references
cite `reference/framework-13.27.0/src/Illuminate/` and its line numbers; the
binding behaviour is the same in 13.22.0 and 13.34.0, though some of those
files differ in other ways.

## Observed at 2bd4bd53d

Status: Observed

- Two binding paths exist. A handler argument whose type path ends in
  `Model` with at least two segments (`user::Model`), or any path ending in
  `RouteParam`, binds from the route; every other non-primitive type is
  treated as a form request (`suprnova-macros/src/handler.rs:349-396`). The
  parameter name is the argument's identifier, or the single field of a
  destructured `RouteParam(x)` (`handler.rs:326-346`). The generated code
  reads the parameter, answering 400 when it is absent, and calls
  `AutoRouteBinding::from_route_param` (`handler.rs:443-452`).
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
  `framework/src/error.rs:1313`); a missing row answers 404, naming the
  model as `User` on the `RouteParam` path and `user` on the bare path
  (`route_binding.rs:261,390`).
- The `RouteBinding` trait (`param_name`, `from_route_param`) and the
  `route_binding!` macro, which parses only `i32` keys, are exported from
  the crate root and never called or implemented anywhere
  (`route_binding.rs:168-187,306-326`; `framework/src/lib.rs:166-174`;
  `framework/src/database/mod.rs:141`). Stale rustdoc: the trait says
  `#[handler]` calls it (`route_binding.rs:178-179`), the module doc calls
  binding automatic for every model (`route_binding.rs:5-25`), and
  `handler.rs:50,62,384` name the removed trait and `M::find`.
- A `{post:slug}` segment registers a parameter literally named `post:slug`
  (matchit 0.9.2 accepts `:` in a name; `framework/src/routing/macros.rs:46-52`
  checks only the leading `/`), so a handler binding `post` answers 400 on
  every request, `where_*("post")` fails at boot, and `route()` leaves the
  placeholder in the URL (`framework/src/routing/router.rs:357-362,1020-1042`).
  The router supports optional parameters, `{id?}` (`framework/src/routing/params.rs:3-15`).
- Nothing scopes a child parameter to its parent, no binding takes a
  column other than the primary key, no route-level binder exists, and no
  enum binds from a route (an enum argument is a form request).
- `route()` takes `&[(&str, &str)]` pairs only (`router.rs:534`); `try_route`
  reports missing parameters (`router.rs:605`).
- Resource controllers are `Arc<dyn ResourceController>` whose actions take
  a raw `Request` (`framework/src/routing/resource.rs:234-275,298,315`), so
  `#[handler]` binding cannot reach them; resources have one `parameter`
  and no nesting (`resource.rs:301,389`), and the `parameters()` their
  docs advertise does not exist (`resource.rs:47-48`).
- Handlers are boxed as type-erased closures at registration
  (`router.rs:667-668,1346-1348`); the `#[handler]` macro emits only the
  rewritten function (`handler.rs:237-246`), so route registration sees no
  handler metadata. Relations are registered through `inventory` with
  their parent type, target type, name, kind and keys
  (`framework/src/eloquent/relations/mod.rs:383-542`), but relation
  queries are inherent methods (`framework/src/eloquent/relations/has_many.rs:43-45`)
  that nothing can call by name. `EloquentModel` carries no column list
  (`framework/src/eloquent/mod.rs:124-138`).
- `#[handler]` decides by spelling, before type checking, which arguments
  read the body (at most one: `handler.rs:146-170,188-201`), that an
  `#[authorize]` target is route-bound (`handler.rs:291-300`), and the
  extraction order (`handler.rs:230-234`); it accepts generic arguments
  (`handler.rs:122,239`).
- Tests that pin today's behaviour: the 400 for a malformed key
  (`framework/tests/schema/unsigned_keys.rs:180-195`,
  `framework/tests/routing/route_binding_route_param_scoped.rs:126-137`),
  trashed rows through the bare form (`route_binding_route_param_scoped.rs:84-110`),
  and the existing handler forms BIND-003 keeps working (`unsigned_keys.rs:50,59-71`,
  `framework/tests/authorization/handler_authorize.rs:154,241`,
  `framework/tests/routing/route_param_destructured.rs:32-43`,
  `framework/tests/schema/unsigned_reads.rs:52`, and the macro tests at
  `suprnova-macros/src/handler.rs:561-770`).
- `#[authorize("ability", x)]` runs after route-bound extraction and before
  body extractors, on the bound value (`handler.rs:230-246`).
- The manual describes binding wrongly in `manual/routing.md:234-325`
  (the bare form only, and a `User::find_by_pk` that does not exist),
  `manual/controllers.md:69,101-127` (any `#[model]` binds through
  `user::Model`), `manual/macros.md:384-397` (`route_binding!`),
  `manual/http-tests.md:750-754`, `manual/tutorial-inertia-crud.md:176-212`
  (`todo::Model` without `EntityExt` does not compile, and lines 117-120)
  and `manual/parity.md:60`.

## The binding contract

[BIND-001] The framework MUST define one public trait, `RouteBinding`, the
counterpart of Laravel's `UrlRoutable`: the route key's column name
(`route_key_name`, the primary key's by default), the key of a value
(`route_key`) and the value of any binding column (`route_field`), both
for URL generation (BIND-012), the lookup by a value and an optional
column (`resolve_route_binding`), and the lookup of a child through a
parent (`resolve_child_route_binding`). `#[model]` MUST implement it for
every model it defines, one implementation per model on the struct the
application declares (not its inner SeaORM row), so a model can replace it
(BIND-007). `AutoRouteBinding` MUST remain as a second name for
`RouteBinding`, with `from_route_param(value)` resolving by the route key.
The unused `RouteBinding` trait with `param_name` and the `route_binding!`
macro MUST be removed.
Falsifier: a `#[model]` struct does not implement `RouteBinding`; code naming `AutoRouteBinding` stops compiling; or the `param_name` trait or `route_binding!` still exists.
Mechanism: `route-binding`.
Rationale: Laravel's binding is one contract every routable type implements (`Contracts/Routing/UrlRoutable.php:12-38`); today's blanket impl cannot be replaced per model, which BIND-007 needs. A blanket impl for SeaORM rows beside per-struct impls from `#[model]` is coherent; the framework already does it for `Persistable` (`framework/src/eloquent/factory/persist.rs:114-121`, `suprnova-macros/src/model/derive_eloquent.rs:1532-1558`).
Status: Draft

[BIND-002] A bound lookup MUST go through the model's query, so global
scopes, the soft-delete filter and the model's connection apply, and MUST
match the column the route names (BIND-004), else the model's route key.
The value MUST be parsed as that column's type; a value that does not
parse and a value that matches no row MUST both answer 404 with the same
body, which names the model's type and never echoes the value.
Falsifier: a bound lookup returns a row a global scope excludes, or a soft-deleted row without `with_trashed()` (BIND-008), which lifts only the soft-delete filter; a malformed value answers anything but 404; the two 404 bodies differ; or a body contains the requested value.
Mechanism: `route-binding`.
Rationale: Divergence from Laravel, which passes the value to the query unchecked (`Database/Eloquent/Model.php:2575-2578`): MySQL coerces `7abc` to `7` and binds row 7, and Postgres raises an error, a 500; only unique-string ids answer 404 for a malformed value (`Database/Eloquent/Concerns/HasUniqueStringIds.php:54-65,104-107`). Laravel's 404 message carries the value (`Routing/ImplicitRouteBinding.php:58,61`); Suprnova's does not. Today a malformed key is a 400 that echoes the value.
Status: Draft

[BIND-003] Existing routes MUST keep working: a handler that binds with
`RouteParam<T>` or with the bare `x::Model` form binds as it does today,
the bare form keeping its lookup by primary key for SeaORM entities that
implement `EntityExt`. The only change existing routes see is the 404 for
a malformed key (BIND-002). `with_trashed()` on the route (BIND-008) is
the documented way to reach soft-deleted rows.
Falsifier: a handler written with `RouteParam<User>` or `user::Model` today stops compiling or binds differently, apart from the malformed-key 404.
Mechanism: `route-binding`.
Rationale: The developer's limit on the replacement: "existing routes keep working".
Status: Draft

## Keys and columns

[BIND-004] A route path segment `{name:column}` MUST register the
parameter `name` and record `column` as its binding field, so
`req.param("name")`, `where_*("name")` and `route()` all use `name`, and
the binding of `name` matches `column`; `{name:column?}` is the optional
form. The router MUST refuse at startup a binding field that is not a
column of the model bound to that parameter, naming the route, the
parameter and the field. To make this and the other startup checks
(BIND-006, BIND-013, BIND-015) possible, `#[handler]` MUST record each
handler's bound parameters, their types and their kinds where the router
can find them when the route is registered (an `inventory` entry keyed by
the handler function's type, recorded by the router before it boxes the
handler), and `#[model]` MUST record each model's columns. A route whose
handler carries no record (a closure) is exempt from the checks and binds
nothing.
Falsifier: `{post:slug}` registers a parameter named `post:slug`; the binding ignores the field; a field naming no column starts the application; or a `#[handler]` route escapes the startup checks.
Mechanism: `route-binding`.
Rationale: `Routing/RouteUri.php:39-60` records binding fields the same way. Today the router sees no handler metadata and models carry no column list.
Status: Draft

[BIND-005] `#[model(route_key = "column")]` MUST set a model's route key,
and the build MUST fail when the column is not a field of the model.
Falsifier: a model with `route_key = "slug"` binds by its primary key, or one naming an unknown column compiles.
Mechanism: `route-binding`.
Rationale: The counterpart of `getRouteKeyName()` and Laravel 13's `#[RouteKey]` (`Model.php:2477-2480`, `Database/Eloquent/Attributes/RouteKey.php`).
Status: Draft

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
returning the child's type; `#[model]` MUST emit a lookup of a child
through a relation by name, with qualified columns for many-to-many and
through relations. A route that needs a relationship the parent does not
declare MUST be refused at startup, naming the route, the parent and the
relation, unless the parent model replaces its child resolution
(BIND-007).
Falsifier: `/users/1/posts/{post:slug}` binds another user's post; a route with `scope_bindings()` does not scope; a route with `without_scoped_bindings()` scopes; or a missing relation is found at request time instead of at startup.
Mechanism: `route-binding`.
Rationale: `Routing/Route.php:597-606`, `Routing/ImplicitRouteBinding.php:48-59`, `Model.php:2546-2549,2562-2565`. Laravel finds a missing relationship at request time as a 500 (`Model.php:2871`); this design finds it before the first request. Relations are registered with their names and types today, but no relation can be queried by name.
Status: Draft

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
from the handler argument's MUST be refused at startup.
Falsifier: a model's own `RouteBinding` is not used; a non-model type that implements `RouteBinding` does not bind; a `bind` resolver loses to the type's binding; or `model` with a fallback answers 404 for a missing row.
Mechanism: `route-binding`.
Rationale: `resolveRouteBinding` overrides, `Route::bind` and `Route::model` (`Routing/Router.php:958-967,1170-1188`; `Routing/RouteBinding.php:34-87`); an explicitly bound child is never scoped (`Routing/ImplicitRouteBinding.php:36-38`).
Status: Draft

## Soft deletes and missing rows

[BIND-008] A route's `with_trashed()` MUST let its bindings, scoped
children included, match soft-deleted rows; without it they MUST NOT. A
resource registration's `with_trashed(actions)` MUST apply it to those
actions, `show`, `edit` and `update` when none are named.
Falsifier: a soft-deleted row binds on a route without `with_trashed()`, or fails to bind on one with it, as a parent or as a scoped child.
Mechanism: `route-binding`.
Rationale: `Routing/Route.php:614-629`, `Routing/ResourceRegistrar.php:127-130`; Laravel's trashed rows are excluded by the soft-delete scope (`Database/Eloquent/SoftDeletes.php:31`).
Status: Draft

[BIND-009] A route, group or resource MUST accept `missing(handler)`,
called with the request when any binding of the route, explicit or
implicit, finds no row or receives a value that does not parse; its
response replaces the 404.
Falsifier: a route with `missing()` answers the plain 404 for a missing or malformed binding.
Mechanism: `route-binding`.
Rationale: `Routing/Middleware/SubstituteBindings.php:44-47`, `Routing/RouteRegistrar.php:78`, `Routing/PendingResourceRegistration.php:279-284`.
Status: Draft

## Enums, resources and URLs

[BIND-010] A unit-only enum MUST be able to derive `RouteBinding`; each
variant binds from exactly one string, its declared value or else its
name in snake case, matched exactly and case-sensitively, and a value
that matches no variant MUST answer 404 without calling `missing()`.
Falsifier: a derived enum binds a value differing only in case, or a non-matching value answers anything but 404.
Mechanism: `route-binding`.
Rationale: Laravel binds string-backed enums by `tryFrom` (`Routing/ImplicitRouteBinding.php:86-97`); its missing hook does not catch enum misses. Rust enums carry no backing value, so the snake-case fallback is a divergence.
Status: Draft

[BIND-011] Resource actions MUST be able to take bound arguments as
handlers do. Resource registration MUST support nested resources
(`users.posts`), `parameters()` renaming, `scoped(fields)`, which gives
each nested parameter a binding field and scopes it (BIND-006),
`with_trashed(actions)` (BIND-008) and `missing()` (BIND-009).
Falsifier: a resource action cannot receive a bound model; a nested resource cannot be registered; or `scoped()` leaves a nested child unscoped.
Mechanism: `route-binding`.
Rationale: `Routing/ResourceRegistrar.php:123-125,560-569,599-607`. Today resource actions are trait methods taking a raw `Request` behind `dyn ResourceController`, with one parameter and no nesting, so this changes the resource controller's shape.
Status: Draft

[BIND-012] `route()` and `try_route()` MUST accept a bound value for a
parameter and fill it with the value's binding field (`route_field`) when
the route names one, else its route key, percent-encoded as other values
are; the named-route registry MUST keep each route's binding fields.
Falsifier: `route("posts.show", post)` on `/posts/{post:slug}` produces anything but the post's slug in the path.
Mechanism: `route-binding`.
Rationale: `Routing/RouteUrlGenerator.php:296-300`, `Routing/UrlGenerator.php:606-617`.
Status: Draft

## Startup checks and documentation

[BIND-013] A handler that binds a parameter its route path does not
declare MUST be refused at startup, naming the route and the parameter.
Falsifier: such a route starts and answers 400 at request time.
Mechanism: `route-binding`.
Rationale: Laravel silently injects an empty model in this case (`Routing/ResolvesRouteDependencies.php:87-92`); refusing at startup is the divergence.
Status: Draft

[BIND-014] The manual MUST describe binding as this contract specifies,
with a "Why Suprnova diverges" section for strict value parsing and the
404 that never echoes the value (BIND-002), the enum name fallback
(BIND-010) and the startup checks (BIND-004, BIND-006, BIND-013,
BIND-015); every page and rustdoc comment the Observed section lists MUST
be corrected, the tutorial's code compiling.
Falsifier: a manual page or rustdoc comment still documents the removed trait or macro, presents the bare form as the general one, or a manual example does not compile.
Mechanism: `manual-check`.
Rationale: Seven pages are wrong today.
Status: Draft

## The handler form

[BIND-015] A handler argument MUST bind from the route parameter its name
names whenever its type implements `RouteBinding`, written as the type
alone (`post: Post`), the choice made by the generated code at compile
time; a type that does not implement it stays a form request, and so does
a generic argument, which the generated code cannot choose for. This form
is added beside `RouteParam<T>` and the bare `x::Model` form (BIND-003). Route
binding MUST run first, then `#[authorize]`, then body extraction. An
`#[authorize]` target MUST be required, at compile time, to implement
`RouteBinding`. A handler with more than one body-reading argument MUST be
refused at startup (BIND-004's handler record), since the macro can no
longer tell body readers apart by spelling. An `Option<T>` argument binds
`None` when its optional parameter is absent.
Falsifier: `post: Post` for a type implementing `RouteBinding` is read as a form request; a type that does not implement it binds from the route; an `#[authorize]` target that is not bound compiles; or a handler with two body readers starts.
Mechanism: `route-binding`.
Rationale: Laravel binds by the type hint alone. Rust macros cannot see trait implementations, so the generated code chooses by trait at compile time. The cost: today's compile-time error for two body readers becomes a startup error, and generic arguments cannot bind. The developer accepted this form on 2026-10-05, and accepted these costs once explained ("ok on BIND-015").
Status: Draft
