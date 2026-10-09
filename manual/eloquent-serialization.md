# Eloquent Serialization

You convert Eloquent models to arrays, JSON, and serde output with one
visibility policy. You declare hidden fields, visible fields, and default
appends on the model. You can change visibility or add an append on one
instance. You include eager-loaded relations through explicit accessors
or resources.

## Table of contents

- [The contract](#the-contract)
- [`to_array` and `to_json`](#to_array-and-to_json)
- [Hiding fields - `hidden = [...]`](#hiding-fields--hidden--)
- [Whitelisting fields - `visible = [...]`](#whitelisting-fields--visible--)
- [Appending accessors - `appends = [...]`](#appending-accessors--appends--)
- [The filter pipeline order](#the-filter-pipeline-order)
- [Per-call filtering - `to_array_except` / `to_array_only`](#per-call-filtering--to_array_except--to_array_only)
- [Conditional hiding by viewer](#conditional-hiding-by-viewer)
- [Serde serialization](#serde-serialization)
- [Serializing collections](#serializing-collections)
- [Eager-loaded relations and serialization](#eager-loaded-relations-and-serialization)
- [What about JSON:API?](#what-about-jsonapi)
- [Where each piece lives](#where-each-piece-lives)
- [Next](#next)

## The contract

Every `#[suprnova::model]` struct gets two serialization methods from
the `Model` trait:

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` produces a `serde_json::Value` for use in handler responses
and tests. `to_json` is a thin wrapper -
`serde_json::to_string(&self.to_array())` - so a single filter
pipeline owns both shapes.

The output is a JSON object keyed by struct field name (or whatever
serde rename you've applied), filtered through three optional knobs
declared on `#[model(...)]`:

- `hidden = [...]` - column denylist
- `visible = [...]` - column whitelist (mutually exclusive with `hidden`)
- `appends = [...]` - accessor methods to inject under named keys

You use the same policy through the model's `Serialize` implementation.
You exclude the framework's `__eager` and `__pivot` state on every path.
You keep persistence separate from this output policy, so hidden fields
still participate in saves.

## `to_array` and `to_json`

The minimum useful example - a row out the door as JSON:

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` accepts any `serde_json::Value`; `user.to_array()`
produces one. The string-shaped equivalent is `user.to_json()` -
identical body, identical filters, just one extra `to_string`.

You can also use `serde_json::to_value(&user)` or
`serde_json::to_string(&user)`. You get the same visibility and appends
policy, including changes you make on that instance.

## Hiding fields - `hidden = [...]`

The denylist form. Every column except the listed ones serialises:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

The user-facing JSON for this model never contains `password` or
`remember_token`:

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` is the right tool when **most fields go to the wire** and you
need to subtract a small set of secrets, internal flags, or auth-only
data.

## Whitelisting fields - `visible = [...]`

The allowlist form. Only the listed columns serialise:

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

Useful for a model that exists specifically to be a thin public
projection (think Laravel's "Profile" / "PublicUser" types). `visible`
is also the right tool when the table holds dozens of internal columns
and only a few belong on the wire - listing the keep-set is shorter
than listing the strip-set.

`hidden` and `visible` are **mutually exclusive at compile time**. The
macro emits an error if you set both:

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

The two are policy opposites - pick the one whose intent matches the
shape of your model, not both.

## Appending accessors - `appends = [...]`

`appends` injects computed values into the JSON output. Each entry
names an `#[accessor]`-tagged method on the model; the macro calls it
during `to_array()` and stores the return value under the same key.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

The serialised user now carries both computed keys:

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

The macro validates `appends` entries at compile time:

- Each name must parse as a Rust identifier (`"full-name"` fails - it's
  not a valid ident).
- If the named method doesn't exist on the model's `impl` block, the
  compiler points at the macro-generated dispatcher with a clear
  `no method named 'full_name' found` error.

Calling `user.full_name()` directly from Rust works exactly like any
other method - `appends` only controls the **JSON dispatch table**.
Accessors stay regular methods.

You select a runtime append with `append(name)?`. You register that name
in `accessors = [...]` when you do not want it in the default `appends`.
You add a name once, and an unknown name returns an error. You keep the
append on the instance for every subsequent output.

## The filter pipeline order

You get the same steps for serde, `to_array`, and `to_json`:

1. You serialize the model's runtime attributes, preserving serde field options.
2. You exclude `__eager` and `__pivot`.
3. You retain only visible names when the visible list is non-empty.
4. You remove hidden names.
5. You apply those lists to default and runtime appended names before calling
   their accessors. You insert only permitted accessor values.

You suppress an appended name when it is hidden or outside a non-empty
visible list. You do not evaluate a suppressed accessor. An allowed accessor
with the same name as a stored column replaces that column in the output.
Serde returns accessor serialization errors. The existing infallible
`to_array` and `to_json` helpers represent serialization failure as JSON null.

## Per-call filtering - `to_array_except` / `to_array_only`

For one-off cases where the column declaration doesn't fit, two
terminal helpers run the full `to_array` pipeline then trim the result
by name:

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // strip a few extra fields for an admin endpoint that needs most
    // of the row but not these:
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    )
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // public directory - only the columns we want to publish:
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    )
}
```

Both produce a `serde_json::Value` - they don't mutate `self` and they
don't change future serialisations of the same row. They run the
full `hidden` / `visible` / `appends` pipeline first, then apply their
own trim on top. `to_array_only` returns a *fresh* JSON object
containing only the named keys; `to_array_except` returns the full
object minus the named keys.

### Instance visibility

You change every later conversion of one instance with these methods:

```rust
user.make_hidden(["email", "phone"]);
user.make_hidden_if(!is_admin, "internal_notes");
user.make_visible("email");
user.make_visible_if(is_admin, ["internal_notes", "full_name"]);
```

You pass one name, an array, a vector, or a slice. Each method returns
`&mut Self` so you can chain changes. A false condition leaves the lists
unchanged. You remove a name from hidden with `make_visible`. When you
already have a non-empty visible list, you also add that name to visible.
You keep the changes on the instance and its clones. You do not change
other instances or the model's declared defaults.
You still cannot declare both `hidden` and `visible` on one model.

## Conditional hiding by viewer

The idiomatic pattern when visibility depends on the viewer is a
match at the call site, branching into the right per-call filter:

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

For more elaborate per-viewer shape - different attributes for admins,
trial users, paid users - the right tool is the **JSON:API resource
layer** with `Maybe<T>` / `MissingValue<T>` fields. See
[JSON:API resources](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)
for the declarative form.

## Serde serialization

You use direct serde serialization for model values and nested model values:

```rust
let value = serde_json::to_value(&user)?;
let body = serde_json::to_string(&user)?;
let nested = serde_json::to_value(vec![user])?;
```

You get the same filtered attributes and appends on each path. You retain
serde renames and field serialization options. You can still trim one output
with `to_array_except` or `to_array_only`, without changing the instance.

## Serializing collections

A `Collection<M>` - returned by `Builder::get()`, `Model::all()`, and
relation accessors - has its own `to_array()` and `to_json()` that
walk the underlying `Vec<M>` and call **per-row** `to_array()`. The
result is a JSON array of filtered objects:

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

You get the same per-model policy from `serde_json::to_value(&users)`.
You also get it when models appear in a paginator's data vector or another
serde container. You can use the [JSON:API paginated form](eloquent-resources.md#pagination)
when you need its resource envelope, links, and metadata.

## Eager-loaded relations and serialization

This is the second divergence to internalise.

When you call `.with(["posts"])` on a builder, the framework loads the
posts and stores them in a per-row `EagerLoadCache` (the auto-injected
`__eager` field). The accessor for reading them - `user.posts_loaded()` -
pulls from that cache.

**The cache is `#[serde(skip)]` and `to_array()` strips it
unconditionally.** Eager-loaded relations do not auto-fold into the
JSON output. A `to_array()` on a user with eagerly-loaded posts looks
identical to a `to_array()` on a user without.

### Why Suprnova diverges

Laravel's `toArray()` walks `$model->getRelations()` and folds every
loaded relation into the output. PHP's array-shaped model bag makes
this natural - a relation is just another keyed entry on the model.

Rust's typed Eloquent structs don't have that bag. A `User` struct has
typed columns, not a heterogeneous map of "whatever relations were
loaded". Folding `posts` in would require either runtime field
injection on a typed struct (a serde-bypass mechanism), or a parallel
serialisation path that consults the cache after running the column
serialiser. Both options would couple every model's JSON shape to
which relations a particular caller eager-loaded - a contract that's
load-bearing in PHP because clients learn to depend on it, and a
contract Suprnova explicitly refuses to ship because it makes JSON
shape depend on caller-side query construction.

### The two ways to ship relation data

**1. Explicit accessor + appends.** Define a method that pulls from
`<rel>_loaded()`, register it in `appends`. The relation shows up
under whatever key you name. This works when the relation is *always*
eager-loaded on the read path:

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // posts_loaded() PANICS if .with(["posts"]) wasn't called on
        // the read path. The accessor MUST run after eager-loading.
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// Read path MUST eager-load:
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // each user's "posts" key is populated
```

The contract is loud: forget the `.with(["posts"])`, and the accessor
panics on the first row's `posts_loaded()` call (the eager cache
panics on read when the relation wasn't loaded, by design - a silent
empty array would hide the bug). For optional eager-load, use the
HasOne form which returns `Option<&T>` and gives you a `match`:

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. The JSON:API resource layer.** When the relation shape and
inclusion policy belong on the wire format rather than the model, use
a `#[derive(Data)] #[json_resource]` struct with
`#[data(allow_include)]` on the relationship field. Clients opt in via
`?include=posts.comments`, the framework walks the include tree, and
populates `included` with deduplicated resource objects. This is the
right answer when:

- Relation shape is a wire-format concern (sparse fieldsets, conditional
  inclusion, cross-link metadata).
- Different endpoints want different default inclusions.
- The same model appears under different envelopes (one endpoint ships
  `posts`, another ships `subscriptions`).

See [JSON:API resources](eloquent-resources.md#compound-documents--include-chains)
for the full pattern.

## What about JSON:API?

The `to_array()` pipeline and the `Resource` / `JsonApi` facade are
two layers, and they serve different jobs:

| Concern | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **Shape** | Flat object - column names map directly to keys | JSON:API envelope (`data`, `included`, `meta`, `links`, `jsonapi`) |
| **Per-attribute control** | `hidden` / `visible` / `appends` on `#[model]` | `#[data(input_only)]`, `Maybe<T>`, sparse fieldsets via `?fields[type]=` |
| **Relations** | Manual (accessor + appends, see above) | First-class via `#[data(allow_include)]` + `?include=` |
| **Pagination** | Wrap a `Vec<Value>` by hand | `Resource::paginated(p)` handles links + meta |
| **Errors** | Render through `FrameworkError` | `into_json_api_response()` produces JSON:API `errors` envelope |
| **When to reach for it** | Simple endpoints, internal tools, ad-hoc shapes | Public APIs, third-party consumers, JSON:API-aware clients |

`to_array()` is the lower layer - it's what gets called for most
internal handlers, admin pages, Inertia props (via serde), and tests.
The JSON:API layer composes on top: it doesn't replace `to_array`, it
adds an envelope around per-resource attribute / relationship logic
that's too rich to live on the model itself.

You can pass models through serde into typed Inertia props with their
visibility policy intact. You use a resource or a dedicated DTO when you
need a different response shape.

## Where each piece lives

| Concern | File |
|---|---|
| `Model::to_array` / `to_json` trait defaults | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| `appends` accessor dispatch, trait default | `framework/src/eloquent/model.rs` |
| Macro-emitted `Serialize` and unfiltered attribute view | `suprnova-macros/src/model/serialization.rs` |
| `appends` accessor dispatch, macro-emitted | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache` (the `__eager` field) | `framework/src/eloquent/relations/eager_cache.rs` |
| `hidden` / `visible` / `appends` macro parsing | `suprnova-macros/src/model/parse.rs` |
| `#[accessor]` function-level macro | `suprnova-macros/src/lib.rs` |

## Next

- [Eloquent API](eloquent.md) - the full model surface, attribute
  reference, and where `#[accessor]` / `#[mutator]` are defined
- [JSON:API resources](eloquent-resources.md) - the declarative
  resource layer for richer per-viewer shapes, sparse fieldsets, and
  compound `?include=` documents
- [Validation](validation.md) - how request input becomes a typed
  struct before the model layer sees it
- [Responses](responses.md) - `HttpResponse` builders, headers, and
  cookies; the surface `json_response!` ultimately produces
- [Error Model](error-model.md) - how an error becomes a JSON body
  with the same `request_id` correlation as the success path
