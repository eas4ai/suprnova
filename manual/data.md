# Data Objects

Suprnova's `#[derive(Data)]` lets you describe an inbound request shape, an outbound response shape, and a TypeScript export in **one struct**.

## Quick start

```rust
use suprnova::Data;
use suprnova::data::Field;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserDto {
    pub id: i64,

    #[validate(email)]
    pub email: String,

    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub display_handle: String,

    pub bio: Field<String>,
}
```

`#[derive(Data)]` generates:
- `Serialize` (skipping `#[data(input_only)]` fields)
- `Deserialize` (rejecting `#[data(output_only)]` fields in the payload, defaulting them to `T::default()`)
- `FormRequest` with `authorize: true` by default - handlers can take the type directly as an extractor
- `IntoInertiaData` (the `Inertia::data(component, dto)` dispatch path)
- An `inventory::submit!` registration for any `#[data(allow_include)]` fields

Add `#[derive(Validate)]` separately so `#[validate(...)]` attributes stay visible at the field call site.

## Field attributes

| Attribute | Effect |
|---|---|
| `#[data(input_only)]` | Accepted on Deserialize, omitted from Serialize |
| `#[data(output_only)]` | Rejected on Deserialize (422), included in Serialize |
| `#[data(allow_include)]` | Field is `?include=`-eligible. **Default-deny**: any `?include=foo` request where `foo` isn't on the allowlist returns 400 |
| `#[data(lazy)]` | Field is a `Prop` resolved against the request's include-set; auto-registers as `allow_include` |
| `#[data(lazy(inertia))]` | Same as `lazy`, tagged for Inertia's partial-reload protocol |
| `#[data(lazy(deferred))]` | Tagged for Inertia's deferred-props protocol |
| `#[data(lazy(closure))]` | Always resolved on initial visit; lazy on partial reloads |
| `#[data(lazy(when_loaded))]` | Resolved only if the source entity has the relation preloaded |
| `#[data(from_route_param)]` | Field value comes from a path capture (e.g. `/users/{id}`). Default key = field name; pass `#[data(from_route_param("id"))]` to override |

## Struct attributes

| Attribute | Effect |
|---|---|
| `#[data(auto_lazy)]` | Every `Prop`-typed field is implicitly `#[data(lazy)]` |
| `#[data(authorize = "path::to::fn")]` | Route the generated `FormRequest::authorize` to a free function with signature `fn(req: &Request) -> bool`. The body parser, validator, Precognition support, and route-param injection still come from the derive |
| `#[data(after_validation = "path::to::fn")]` | Route the generated `FormRequest::after_validation` to `fn(dto: &Self) -> Result<(), ValidationErrors>`, where `validate!` rules run. See [Validation rules and database checks](#validation-rules-and-database-checks) |
| `#[data(after_validation_async = "path::to::fn")]` | Route `FormRequest::after_validation_async` to `async fn(dto: &Self) -> Result<(), ValidationErrors>`, where `Exists`, `Unique` and other async rules run |
| `#[data(allow_unknown_fields)]` | Accept payload keys that don't match any struct field. The default is **strict**: an unrecognised key fails the deserialize with `serde::de::Error::unknown_field(..)` and surfaces as a 422 through `FormRequest`. Opt into permissive only for response DTOs that read forward-compatible third-party payloads |

The earlier `#[data(custom_authorize)]` flag - which suppressed the whole `FormRequest` impl and forced you to reimplement body parsing, validation, and Precognition by hand - is gone. The macro emits a migration error if you try to use it. Use `#[data(authorize = "fn")]` instead.

## Validation rules and database checks

`#[validate(...)]` attributes cover the per-field checks. Everything else
in [Validation](validation.md) - `validate!` rows, cross-field rules like
`After::new(DateBound::Field(..))` or `ExcludeIf`, and database rules like
`Exists` - runs in the `FormRequest` hooks. The derive writes a Data
Object's `FormRequest` impl, so you name the functions it calls:

```rust
use suprnova::rules::{Accepted, After, DateBound, DateFormat};
use suprnova::{Data, Exists, FormContext, ValidationErrors, validate};
use validator::Validate;

#[derive(Data, Validate)]
#[data(after_validation = "booking_rules", after_validation_async = "booking_rows")]
pub struct BookingDto {
    pub room_id: i64,
    pub starts_on: String,
    pub ends_on: String,
    pub terms: bool,
}

fn booking_rules(dto: &BookingDto) -> Result<(), ValidationErrors> {
    let ctx: FormContext = [("starts_on".to_string(), dto.starts_on.clone())].into();
    validate! { dto =>
        starts_on => DateFormat(&["%Y-%m-%d"]);
        ends_on => DateFormat(&["%Y-%m-%d"]), After::new(DateBound::Field("starts_on")) => with ctx;
        terms => Accepted;
    }
}

async fn booking_rows(dto: &BookingDto) -> Result<(), ValidationErrors> {
    let mut errs = ValidationErrors::new();
    Exists::new("rooms", "id")
        .check_value(dto.room_id, &mut errs, "room_id")
        .await;
    errs.into_result()
}
```

The stages run in order and stop at the first that fails: the
`#[validate(...)]` attributes, then `after_validation`, then
`after_validation_async`. A malformed date therefore never reaches the
database. Precognition runs the same stages and reports the fields the
client asked about - asking about an array keeps the errors of its elements
(`tag_ids` keeps `tag_ids.3`). A Data Object with a `from_route_param` field
runs both hooks too.

A generic struct, or one with reference or lazy fields, gets no
`FormRequest` impl, so it would never run a hook; the derive refuses both
attributes on it.

## `Field<T>` - Absent / Null / Value

For PATCH endpoints where "absent from payload" must be distinguished from "explicit null":

```rust
use suprnova::data::Field;

match dto.bio {
    Field::Absent  => { /* don't touch this column */ },
    Field::Null    => { /* clear the column */ },
    Field::Value(text) => { /* set to text */ },
}
```

`Field::Absent` (default) round-trips to omitted-from-JSON when paired with `#[serde(default, skip_serializing_if = "Field::is_absent")]` at the call site. Without `skip_serializing_if`, `Absent` serializes to JSON `null`.

For three-way DB upserts: `dto.bio.into_option_or_null() -> Option<Option<T>>` maps `Absent → None`, `Null → Some(None)`, `Value(v) → Some(Some(v))`. Use this when "don't touch" and "set to NULL" need to be distinct downstream.

> **Caveat:** `Field<Option<T>>` is lossy - `Value(None)` and `Null` both serialize as JSON `null` and deserialize back to `Null`. For nullable inner types, prefer a flat `Field<T>` and let `Null` carry the "clear it" signal.

## `?include=` query string

The `IncludeMiddleware` parses the request's query string into a per-request `RequestIncludeSet`:

- `?include=foo,bar` - resolve lazy fields `foo` and `bar`.
- `?include[]=foo&include[]=bar` - array form, same result.
- `?exclude=`, `?only=`, `?except=` - Laravel-Data API parity.

Composition with `X-Inertia-Partial-Data` (Inertia's partial-reload header): the include-set + per-DTO allowlist runs **first** for owner-tagged lazy fields, so a request for a disallowed field returns 400 even if partial-data would have filtered it out. Partial-data is applied **after** as a final "only" filter on the resolved props.

Register `IncludeMiddleware` globally - typically between session and authorization in the middleware stack:

```text
SessionMiddleware → IncludeMiddleware → AuthMiddleware → handlers
```

### Programmatic include/exclude/only/except

`RequestIncludeSet` mirrors Laravel-Data's `IncludeableData` contract with chainable builders. Handlers, tests, and middleware can construct or override a set without poking the public fields directly:

```rust
use suprnova::data::RequestIncludeSet;

let set = RequestIncludeSet::default()
    .include(["author", "comments"])
    .exclude(["password"])
    .only(["id", "name"])
    .except(["secret"]);

assert!(set.is_visible("name"));   // on `only`, not in `except`
assert!(!set.is_visible("secret"));// `except` always wins
assert!(set.includes("author"));   // request for the `author` relation
```

| Method | Effect | Laravel equivalent |
|---|---|---|
| `.include(fields)` | append to the include list (lazy fields to resolve) | `Data::include(...$fields)` |
| `.exclude(fields)` | append to the exclude list (fields to drop) | `Data::exclude(...$fields)` |
| `.only(fields)` | initialise or extend the `only` allowlist | `Data::only(...$fields)` |
| `.except(fields)` | append to the except list (always-drop) | `Data::except(...$fields)` |
| `.include_when(cond, fields)` | append only when `cond == true` | `Data::includeWhen($field, $condition)` |
| `.exclude_when(cond, fields)` | append only when `cond == true` | `Data::excludeWhen($field, $condition)` |
| `.only_when(cond, fields)` | extend `only` only when `cond == true` | `Data::onlyWhen($field, $condition)` |
| `.except_when(cond, fields)` | append only when `cond == true` | `Data::exceptWhen($field, $condition)` |
| `.merge(other)` | union two sets (in-place layered overrides) | manual `array_merge` in PHP |
| `.includes(field)` | `field` (or `field.path`) in include list? | `relationLoaded()` analogue |
| `.is_excluded(field)` | `field` in exclude list? | reads exclude partial |
| `.is_excepted(field)` | `field` in except list? | reads except partial |
| `.is_only_listed(field)` | `field` allowed by `only` (or `only` unset)? | reads only partial |
| `.is_visible(field)` | full Laravel resolution order: except → exclude → only | `resolveResource` decision |

Builders take any `IntoIterator<Item = impl Into<String>>`, so arrays, vecs, and slices of `&str`/`String` all work. Strings are trimmed; empty entries are dropped (matching `from_query`).

Dot-paths in any list match the root segment when probed by bare name - `include=["author.posts"]` reports `set.includes("author") == true`, matching Laravel-Data's path resolution. The nested `posts` segment is consumed by `IncludeTree::from_include_set` for JSON:API compound documents.

### Handler-side override: `with_include_overrides`

To layer programmatic overrides on top of what the request's query string already declared (without losing the request's set), use `with_include_overrides`:

```rust
use suprnova::data::with_include_overrides;

async fn show_album(req: Request, user: User) -> Response {
    with_include_overrides(
        |set| set
            .include_when(user.is_admin(), ["audit_log"])
            .exclude_when(!user.is_admin(), ["price_cost"]),
        async move {
            // Inside this scope, the lazy-prop resolver and JSON:API
            // include resolver see the merged set.
            Inertia::data("Album/Show", album_dto).into_response()
        },
    ).await
}
```

The closure runs against a clone of the currently-bound set (or the empty default if no middleware has bound one). After the future completes, the original set is restored - this is a scoped override, not a mutation.

For tests, prefer `scope_include_set(set, future)` to install a fresh set without inheriting any ambient state.

## Generic structs

```rust
use serde::{Serialize, Deserialize};

#[derive(suprnova::Data)]
pub struct Paginated<T>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    pub items: Vec<T>,
    pub total: usize,

    #[data(allow_include)]
    pub meta: Option<serde_json::Value>,
}
```

The TypeScript extractor emits `export interface Paginated<T>` so frontend code can reuse the generic across instantiations.

The `?include=` allowlist is keyed on the fully-qualified type path (`concat!(module_path!(), "::", stringify!(Paginated))`), not on type-parameter instantiations. `Paginated<UserDto>` and `Paginated<ArticleDto>` declared in the same module share one allowlist - `allow_include` names a field, and field names don't depend on type parameters. Two different DTOs named `Paginated` in different modules each get their own allowlist; their keys don't collide.

Note: `FormRequest` is suppressed for generic structs because its trait bounds (`DeserializeOwned + Validate + Send`) can't be verified without knowing concrete type params. Provide your own impl if you need to extract a generic Data struct from a request.

## Route-parameter field injection

```rust
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UpdateUser {
    #[data(from_route_param("id"))]
    pub id: i64,

    #[validate(length(min = 1))]
    pub name: String,
}
```

For `PATCH /users/{id}` with body `{"name": "Ada"}`, the route-captured `id` is merged into the validated payload. **The path always wins over a body-supplied value** (prevents IDOR via body-tampering).

Bare `#[data(from_route_param)]` defaults to the field name. The macro classifies the field's last path segment at compile time and dispatches to a matching parser. Only the exact names listed below are recognised; everything else (including `i8`/`i16`/`isize`, `Uuid`, `DateTime`, custom newtypes) falls through to `pass_string` and lets the field's own `Deserialize` do the work.

| Field type | Parser |
|---|---|
| `i64` | `parse_i64` |
| `u64` | `parse_u64` |
| `i32` | `parse_i32` |
| `u32` | `parse_u32` |
| `i128` | `parse_i128` (validates then passes raw string through; field's `Deserialize` parses) |
| `u128` | `parse_u128` (same string-passthrough pattern) |
| `f64` | `parse_f64` (rejects non-finite values) |
| `f32` | `parse_f32` (rejects non-finite values) |
| `bool` | `parse_bool` (accepts only `"true"` / `"false"`) |
| Anything else | `pass_string` - raw string handed to the field's own `Deserialize` |
| `Option<T>` or `Field<T>` of any of the above | Same parser as `T`; missing route param leaves the field absent |

## Lazy props

```rust
use suprnova::Data;
use suprnova::inertia::Prop;

#[derive(Data)]
#[data(auto_lazy)]
pub struct AlbumDto {
    pub id: i64,
    pub songs: Prop,    // auto-registered as ?include=songs
    pub artist: Prop,   // auto-registered as ?include=artist
}
```

Explicit per-field flavor:

```rust
#[derive(Data)]
pub struct AlbumDto {
    pub id: i64,

    #[data(lazy(inertia))]
    pub songs: Prop,

    #[data(lazy(deferred))]
    pub lyrics: Prop,

    #[data(lazy(closure))]
    pub artist: Prop,
}
```

Every lazy flavor is gated the same way, `lazy(deferred)` included. A deferred field is opt-in twice over: `?include=lyrics` puts it in scope for the request, and Inertia's deferred-props protocol decides which round trip carries it. A field the request never included is dropped whole - no value, no `deferredProps` announcement - so the client is never sent after something this request has no claim on. A field named by `?include=` but missing from the allowlist returns 400 on the first visit, before `X-Inertia-Partial-Data` can quietly absorb the error.

Use `Inertia::data(component, dto)` to render - the derive generates an `IntoInertiaData` impl that consults the include-set and allowlist:

```rust
return Inertia::data("Album/Show", album_dto);
```

Note: lazy-bearing structs suppress `Serialize`, `Deserialize`, and `FormRequest` because `Prop` doesn't implement them. If a single endpoint needs both inbound parsing and lazy outbound, use two DTOs: one inbound (`#[derive(Data, Validate)]` plain) and one outbound (`#[derive(Data)]` with lazy fields).

## `when_loaded!` - relation-loaded conditional lazy

Mirrors Laravel-Data's `#[AutoWhenLoadedLazy]`. The user's `From<Entity>` impl decides whether the relation was preloaded:

```rust
use suprnova::{when_loaded, IsRelationLoaded};

impl From<&AlbumEntity> for AlbumDto {
    fn from(album: &AlbumEntity) -> Self {
        Self {
            id: album.id,
            songs: when_loaded!(album, "songs", || async {
                serde_json::json!(album.songs_relation()
                    .iter()
                    .map(SongDto::from)
                    .collect::<Vec<_>>())
            }),
            artist: Prop::eager(serde_json::json!(album.artist_name())),
            lyrics: Prop::lazy(|| async { /* ... */ }),
        }
    }
}
```

If the entity hasn't preloaded the named relation (per `IsRelationLoaded::is_relation_loaded`), `when_loaded!` returns `Prop::absent()` and the field is absent from the response.

SeaORM entities need a custom `IsRelationLoaded` impl that consults their loaded-relations state - there's no framework-supplied blanket impl because SeaORM's `ModelTrait` doesn't carry per-instance relation-loaded state (loaded relations live on query results, not the model struct itself).

## TypeScript export

`suprnova generate-types` emits TypeScript definitions for every `#[derive(Data)]` (and legacy `#[derive(InertiaProps)]`) struct. Behavior:

- `Field<T>` → `field?: T | null`
- chrono's `DateTime`, `NaiveDate`, `NaiveDateTime` and `NaiveTime` → `string`, the ISO 8601 text they serialize as
- `Prop` → `field?: T` (the lazy may-be-absent semantic; the `?` carries it, the type itself is plain)
- `#[data(input_only)]` → excluded from output type
- `#[data(output_only)]` → excluded from input type
- Generic struct → TypeScript generic interface (`export interface Paginated<T>`)
- When ANY field has `input_only` / `output_only` / `lazy`, two interfaces are emitted: `<Name>` (output) and `<Name>Input` (input)
- A plain struct a prop reaches, one that derives serde's `Serialize`, follows serde's attributes: `#[serde(skip)]` and `skip_serializing` leave the field out, `skip_serializing_if` makes it optional (`field?: T`), and `rename` and `rename_all` (or their `serialize = ...` forms) name the key. Other serde attributes, such as `flatten` and `transparent`, are not read. A key that is not an identifier, such as `display-name`, is quoted
- `#[derive(Data)]` and `#[derive(InertiaProps)]` structs write their own `Serialize`, which names every key after its Rust field, so `#[serde(...)]` attributes do not change their keys. Use `#[data(input_only)]` or `#[data(output_only)]` to leave a field out of one side

Generated types never leak Rust-only types (`Prop<...>` won't appear in the output `.d.ts`).

## Scaffolding

```bash
suprnova make:inertia UserDto --data
```

Emits a `#[derive(Data, Validate)]` skeleton instead of the legacy `#[derive(InertiaProps)]` template.

## Next

- [Validation](validation.md) - `#[derive(Validate)]`, async validators, and how `FormRequest` calls into them
- [Requests](requests.md) - the request extractor surface that `FormRequest` plugs into
- [Inertia Responses](frontend-inertia-responses.md) - the `Inertia::data` path and how lazy props become partial-reload-eligible
- [Eloquent Resources](eloquent-resources.md) - `#[derive(Data)]` with `#[json_resource]` for JSON:API outputs (sibling of `Data` for serialization-only payloads)
- [Error Model](error-model.md) - how `unknown_field` rejection becomes a 422 and how `FormRequest` failures travel back as `ValidationErrors`
