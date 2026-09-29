# Pagination

Suprnova ships three paginators that match Laravel's surface line-for-line:
length-aware (knows the total), simple (one query per page), and cursor
(opaque keyset). All three serialize into the Laravel-shaped
JSON Inertia and JSON:API consumers already understand - you fetch a
page and return it; nothing else is required.

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

That one call runs the `COUNT(*)` and the `LIMIT/OFFSET` page fetch,
parses `?page=N` from the active request, and returns a
`LengthAwarePaginator<User>` ready to ship. The two siblings -
`simple_paginate(20)` and `cursor_paginate(20)` - return the same shape
of value with different trade-offs. The rest of this chapter is which
one to reach for, what each one costs, and how the JSON arrives.

## Picking a paginator

The fastest way to choose is the trade-off table:

| Method | Type | Queries / page | Knows total? | Use when |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2 (`COUNT(*)` + page) | yes | UI shows numeric pages or "page 3 of 17" |
| `simple_paginate(n)` | `Paginator<M>` | 1 (`LIMIT n+1`) | no | Large tables; a "Next" button is enough |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1 (`LIMIT n+1`) | no | Infinite scroll; deep pages on hot tables |

The cost difference matters once your table is large. `COUNT(*)` over
a hundred million rows is the most expensive query in your request
budget. `simple_paginate` saves the count. `cursor_paginate` saves the
count *and* avoids the `OFFSET N` linear scan that bites every
deep-page request on a big table - a cursor seek is `O(1)`-ish with the
right index, regardless of where in the result set the user is.

### Why Suprnova diverges

The JSON of the three paginators has the fields of Laravel's
`toArray()`, so a front end that was written for Laravel's paginator
reads it. That includes the pagination components of the Inertia starter
kits, which draw the `links` array as it comes. Four differences matter
to a reader who knows Laravel:

- A paginator does not know the base URL of the request. You give it
  one with `with_path(...)`. With no `path`, the page URLs are relative
  (`?page=2`) and `path` is left out of the JSON, where Laravel writes
  `/`.
- Give a `path` and a query string, never a URL with a host. The `path`
  is written into every URL as you give it, and the host of a request is
  whatever its `Host` header says, which the client chooses.
- A paginator with no rows has a `last_page` of `0` and a `links` array
  of two entries, the link before and the link behind. Laravel has a
  `last_page` of `1` and a link to page 1. `last_page_url` is the URL of
  page 1 in both.
- `simple_paginate` keeps `has_more` beside Laravel's fields.

[`Inertia::paginate`](frontend-inertia-responses.md) attaches Inertia
scroll metadata (page identifiers, not absolute URLs), and
[`Resource::paginated`](eloquent-resources.md) attaches JSON:API
`links.{self,first,last,prev,next}` per the JSON:API recommendation. The
page URLs and helpers of the bare paginator are covered in the
[URL generation](#url-generation-and-paths) section.

## `paginate` - length-aware

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

The struct's public fields:

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // rows on this page
    pub current_page: u64,       // 1-based
    pub last_page: u64,          // 1-based; 0 when total == 0
    pub per_page: u64,
    pub total: u64,              // every row across all pages
    pub from: Option<u64>,       // 1-based first row index on this page
    pub to: Option<u64>,         // 1-based last row index on this page
    pub path: Option<String>,    // base URL for url_for_page (optional)
    pub page_name: Option<String>, // query key of the page; "page" when None
}
```

The JSON of a paginator with `with_path("/api/users")`, on page 2 of 3:

```json
{
  "current_page": 2,
  "data": [...],
  "first_page_url": "/api/users?page=1",
  "from": 11,
  "last_page": 3,
  "last_page_url": "/api/users?page=3",
  "links": [
    {"url": "/api/users?page=1", "label": "&laquo; Previous", "page": 1, "active": false},
    {"url": "/api/users?page=1", "label": "1", "page": 1, "active": false},
    {"url": "/api/users?page=2", "label": "2", "page": 2, "active": true},
    {"url": "/api/users?page=3", "label": "3", "page": 3, "active": false},
    {"url": "/api/users?page=3", "label": "Next &raquo;", "page": 3, "active": false}
  ],
  "next_page_url": "/api/users?page=3",
  "path": "/api/users",
  "per_page": 10,
  "prev_page_url": "/api/users?page=1",
  "to": 20,
  "total": 25
}
```

`path` is omitted from the JSON when unset. `from` and `to` are `null`
when the page is empty (no rows on this page, or the requested page is
past the last page). `next_page_url` is `null` on the last page, and
`prev_page_url` is `null` on the first page.

### The `links` array

`links` is the row of links that a pagination component draws. Each entry
has `url`, `label`, `page`, and `active`:

- The first entry is the link to the previous page. Its label is
  `&laquo; Previous`, with the entity written out, because a front end
  puts the label into the page as markup. Its `url` and `page` are `null`
  on the first page.
- One entry follows for each page in the window, labelled with the page
  number. `active` is `true` for the current page only.
- Where pages are left out, an entry with the label `...` stands in. It
  has a `null` `url`, and it has no `page` key.
- The last entry is the link to the next page. Its label is
  `Next &raquo;`. Its `url` and `page` are `null` on the last page.

The window is Laravel's. Up to 13 pages, every page is listed. With more
pages, the row lists the first two pages, the last two pages, and seven
pages around the current one, with `...` between the groups. Near either
end, the pages of that end are listed in one run of ten instead. For 50
pages with page 25 current, the labels are `1 2 ... 22 23 24 25 26 27 28
... 49 50`.

A paginator with no rows has `last_page` of `0`, both page URLs of page 1
(`?page=1` with no path), and a `links` array of two entries with no page
between them.

In Rust, `links()` returns the same entries as `Vec<PageLink>`. A
`PageLink` has the public fields `url: Option<String>`, `label: String`,
`page: Option<u64>`, and `active: bool`.

### Reading `?page=N` automatically

`paginate(n)` reads the current page from `?page=N` on the active
request via `Context::query_param`. Missing, empty, non-numeric, and
zero values clamp to `1`. There's nothing to wire up - if a request is
in scope, the parameter is read.

### Multiple paginators on one page

When a page renders more than one paginated list, give each its own
query-string key with `paginate_using`:

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` also sets `page_name` on the returned paginator so
`url_for_page` builds URLs with the same key:

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"  (when path is set)
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### Page-position predicates

The full Laravel `AbstractPaginator` predicate set is implemented:

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // we're not on page 1 OR more pages exist
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - page slice, not total
```

`count()` is the slice size, not the total - Laravel's `Countable`
shape; for the total use the `total` field directly.

## `simple_paginate` - one query, no count

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // was there an extra row past per_page?
    pub path: Option<String>,
}
```

JSON, with `with_path("/api/users")`, on page 2:

```json
{
  "current_page": 2,
  "current_page_url": "/api/users?page=2",
  "data": [...],
  "first_page_url": "/api/users?page=1",
  "from": 11,
  "has_more": true,
  "next_page_url": "/api/users?page=3",
  "path": "/api/users",
  "per_page": 10,
  "prev_page_url": "/api/users?page=1",
  "to": 20
}
```

`next_page_url` is `null` when `has_more` is `false`, and `prev_page_url`
is `null` on the first page. `from` and `to` are `null` for a page with no
rows. `path` is omitted when unset. The page key is always `page`.

The trick is in the SQL. `simple_paginate(20)` issues `LIMIT 21`, looks
at whether the 21st row came back, sets `has_more` from that, and
truncates `data` back to 20. One query per page; no `COUNT(*)`.

You give up `total`, `last_page`, `last_page_url`, and `links`. In exchange you can
paginate tables where `COUNT(*)` is too expensive to run on every page
load. The UI surface is "Next" / "Previous" buttons, not "page 7 of
142".

The same predicate set as the length-aware paginator is implemented:
`has_more_pages()`, `on_first_page()`, `on_last_page()`, `has_pages()`,
`is_empty()`, `is_not_empty()`, `count()`.

## `cursor_paginate` - opaque keyset

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // None on the last page
    pub prev_cursor: Option<String>,  // None on the first page
    pub path: Option<String>,
}
```

JSON, with `with_path("/api/users")`:

```json
{
  "data": [...],
  "path": "/api/users",
  "per_page": 10,
  "next_cursor": "...",
  "next_page_url": "/api/users?cursor=...",
  "prev_cursor": null,
  "prev_page_url": null
}
```

`next_cursor`, `next_page_url`, `prev_cursor`, and `prev_page_url` are
always present as JSON keys (`null` when absent) so client schemas can
rely on field presence. A URL is the `path` with the cursor in the
`cursor` key, and it is `null` when its cursor is. `path` is omitted when
unset.

### How cursors work on the wire

The client passes the previous page's cursor through `?cursor=<opaque>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` decodes the cursor, walks the keyset filter
(`pk > boundary ASC` for `next`; `pk < boundary DESC` for `prev`,
reversed back to ASC), fetches `LIMIT n+1` rows, and re-emits
`next_cursor` / `prev_cursor` as the page's neighbours exist. It's
bidirectional - the client can walk forward and back without losing
its position.

Cursor pagination **replaces** any existing `ORDER BY` on the builder.
A stable total order over the primary key is required for the keyset
filter to slice the table deterministically; an arbitrary `ORDER BY
random_score()` cursor would skip and duplicate rows. If you need a
non-PK sort, switch to `paginate` / `simple_paginate`.

### Cursors are encrypted and authenticated

Suprnova cursors are **not** Laravel's base64-JSON plaintext. The wire
cursor is the keyset boundary (a typed `sea_orm::Value` - `Int`,
`BigInt`, `Uuid`, datetimes, decimals, strings, bytes) plus a direction
tag, JSON-encoded and then sealed with AES-256-GCM via the framework
`Crypt` keyring (bound to `CryptPurpose::Cursor`, so a cursor
ciphertext can never be replayed into any other surface - cookie, 2FA
secret, cast).

This means three things in practice:

1. **No tampering.** A client that flips bits in `?cursor=` gets a 400
   `Invalid pagination cursor`, not a different page of data.
2. **No information leak.** The boundary value (often a primary key,
   sometimes a timestamp) is sealed inside the cursor - clients can't
   enumerate ranges by editing it.
3. **Typed boundaries round-trip losslessly.** The wire envelope tags
   the SeaORM variant (`"BigInt"`, `"Uuid"`, etc.), so on decode the
   value re-binds with the same SQL type the original column emitted.
   No string-coercion bugs across Postgres / MySQL / SQLite.

There is no plaintext fallback. If `Crypt` is not initialised - which
should be impossible after `Server::from_config` - encoding errors
rather than emitting a forgeable cursor.

### Why Suprnova diverges

Laravel's cursor paginator is forward-only by default and the wire
cursor is a base64-encoded JSON blob - readable, editable, replayable.
Suprnova's cursor is bidirectional (matching the `cursorPaginate()`
surface Laravel added later) and is authenticated end-to-end so the
client can't construct or alter one. The Rust ecosystem already has
AES-GCM as a primitive; using it costs the framework one extra trait
impl and gives every cursor a security property a plaintext base64
payload can't offer.

## The facade - `Pagination::length_aware` / `Pagination::cursor`

Most chapters of this manual show pagination through the Eloquent
builder, because that's the common path. If you're building a SeaORM
`Select<E>` directly - say, joining onto a non-model query for a
report - the `Pagination` facade is the equivalent surface:

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // or any SeaORM Select<E>
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

The facade also offers `length_aware_on(conn, ...)` and
`cursor_on(conn, ...)` for routing to a specific named connection, and
a typed `cursor(query, cursor, per_page, order_col)` form that takes
the keyset column explicitly - used when the cursor sorts on something
other than the primary key.

Routing rules match the Eloquent builder. An ambient
`DB::transaction` is honoured (both the COUNT and the page query run on
the transaction's connection), and a registered `__read_replica__`
connection is used automatically for reads. The `__primary__` sentinel
selects the default pool when you want to bypass the replica.

## Validation - `per_page == 0`

All three methods reject `per_page == 0`:

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

The error renders as HTTP 400 with the standard error body. There is no
silent "empty page" - a zero page size is always wrong and is rejected
at the call site, matching the Eloquent builder and the `Pagination`
facade. The same validation lives on `cursor_paginate`, `simple_paginate`,
`Pagination::length_aware`, `Pagination::length_aware_on`,
`Pagination::cursor`, and `Pagination::cursor_on` - one rule, six
entry points.

The `current_page` value is **clamped**, not validated: `0` becomes `1`,
negative numbers from a defensive frontend cannot happen (the parser is
`u64`), and any `?page=N` greater than `last_page` returns a paginator
with empty `data` plus `from`/`to` of `None`. Walking past the end is
the client's mistake, not an error.

## Error shape

| Condition | Variant | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Tampered / invalid cursor | `FrameworkError::Domain` (`"Invalid pagination cursor"`) | 400 |
| `Crypt` not initialised at cursor decode | `FrameworkError::Internal` | 500 |
| Cursor variant mismatch on `decode_cursor` | `FrameworkError::Internal` | 500 |
| Underlying DB failure | `FrameworkError::Database` | 500 |

The tampered-cursor case is the one to remember. Cursors are read
directly off the wire - the `?cursor=…` query string is attacker input
by definition, and bit-flipped base64 and replayed ciphertext are
expected failure modes, not server bugs. The decryption step downgrades
to a 400 `Invalid pagination cursor` so client-triggerable failures
don't pollute the 500 telemetry channel. The static message gives the
client nothing to probe with.

Post-decrypt failures (JSON parse, variant-tag dispatch, direction
parse) stay 500 - any byte sequence that survived AEAD authentication
was produced by *us*, so a malformed payload past that point is a
framework bug worth flagging.

## URL generation and paths

The paginator carries an optional `path` field. When set,
`url_for_page(n)` and the cursor link emission use it to build query
strings:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

When the base path already carries a query string, the separator
switches to `&` so the URL stays well-formed:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

If `path` is unset, `url_for_page` falls back to a bare relative
query: `?page=2`. The page-parameter name comes from
`with_page_name(...)` (defaulting to `"page"`); `paginate_using(name, n)`
sets it automatically so the generated URLs use the same key the
paginator was driven from. The parameter name is form-urlencoded, so
even a name with reserved characters can't corrupt the URL.

A page parameter that the `path` already has is replaced, so you can give
the path and query string of the current request as the `path`. A
fragment of the `path` stays at the end of the URL:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?role=admin&page=2#list");

page.url_for_page(3);    // "/users?role=admin&page=3#list"
```

The JSON uses these URLs for `first_page_url`, `last_page_url`,
`next_page_url`, `prev_page_url`, and each `url` in `links`.

In Rust, each paginator returns the same values as methods:

| Paginator | Methods |
|---|---|
| `LengthAwarePaginator` | `url_for_page(n)`, `next_page_url()`, `previous_page_url()`, `links()` |
| `Paginator` | `url_for_page(n)`, `next_page_url()`, `previous_page_url()`, `first_item()`, `last_item()` |
| `CursorPaginator` | `next_page_url()`, `previous_page_url()` |

`next_page_url()` and `previous_page_url()` return `None` where the JSON
has `null`.

Cursor paginators have the same shape: `with_path(...)` sets the base,
`with_cursor_name(...)` overrides the query key (defaults to `"cursor"`),
and the JSON:API link builder picks them up automatically.

Most apps hand the paginator straight to a response. The JSON already
carries the URLs, and the two integration surfaces below build the URLs
the right way for their protocol.

## Inertia integration - infinite scroll props

For Inertia front-ends, the `Inertia::paginate(component, key, paginator)`
helper attaches the paginator as a scroll prop:

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

All three paginators work here - `LengthAwarePaginator`, `Paginator`, and
`CursorPaginator`. The metadata page-name comes from the paginator
itself: `"page"` for the two offset paginators, `"cursor"` for
`CursorPaginator`. The client receives the rows under the chosen prop key
plus a `ScrollMetadata` descriptor with `current_page`, `next_page`,
`previous_page` (page identifiers for the offset paginators; cursor
strings for cursor paginators) - which the `useInfiniteScroll` /
`WhenVisible` Inertia helpers consume for infinite scroll.

Each paginator builds that descriptor through `ProvidesScrollMetadata` -
the same interface Laravel's paginator adapter satisfies
(`ProvidesScrollMetadata::getPageName` / `getPreviousPage` / `getNextPage`
/ `getCurrentPage`). A paginator this crate doesn't know about - a
third-party crate's cursor type, a hand-rolled repository result - can
implement the four methods and hand the framework a `ScrollMetadata` the
same way: see [Inertia
Responses](frontend-inertia-responses.md#merge-strategies-and-infinite-scroll).

`simple_paginate` is worth calling out, because a listing over a table
big enough to make `COUNT(*)` the dominant cost of the request is exactly
where an Inertia collection page hurts:

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // no COUNT, one query
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

Its `next_page` comes from the `LIMIT n+1` overflow probe rather than
from a computed last page, since there is no total to compute one from.
The client gets "there is another page" instead of "there are 4,812
pages" - which is all an infinite-scroll UI ever reads.

### Projecting rows before they go out

Paginators have no `map` / `through` (Laravel's do). Rebuild from the
public fields instead - the counters and cursors describe the *query*, so
they carry across a change of row type unchanged:

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

Worth doing rather than serialising the model directly whenever the route
is unauthenticated and the model carries anything the caller should not
see. A cursor over a user table hands out one page at a time, but it
hands out every page eventually.

The same helper exists as a chainable method on
`InertiaResponse::paginate(key, paginator)` if you want to mix a
paginator with other props:

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

See [Inertia Responses](frontend-inertia-responses.md) for the broader
prop model.

## JSON:API integration - `Resource::paginated`

For JSON:API consumers, `Resource::paginated(paginator)` builds the
full envelope:

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

The response carries:

- `data` - every row rendered through the model's `IntoJsonResource`.
- `meta.pagination` - `{ total, per_page, current_page, last_page }`
  for length-aware; `{ next_cursor, prev_cursor }` for cursor.
- `links.{self,first,last,prev,next}` - absolute hrefs for the
  length-aware paginator (built from `path`); `links.{prev,next}` for
  the cursor paginator.

Both paginator types implement the `Paginated<T>` trait that
`Resource::paginated` consumes - there is no separate code path for
length-aware vs cursor. If you build a custom paginator-like type that
implements `Paginated<T>`, it composes the same way.

See [JSON:API resources](eloquent-resources.md) for the resource
model.

## Custom JSON envelopes

If neither Inertia nor JSON:API matches your client, ship the
paginator directly through `json_response!`:

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

Or just hand the whole paginator across - its `Serialize` impl
emits the shape documented above:

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

The fields are public; reshape as your contract requires.

## Routing across connections

Pagination respects the same multi-connection routing the Eloquent
builder uses. Inside a `DB::transaction(...)` the COUNT and the page
query both run on the transaction's connection - they never split
across connections, so the count never disagrees with the page it
described. A registered `__read_replica__` is used automatically for
reads outside a transaction. To pin a paginator to a specific named
connection use the `_on(connection, ...)` variants on the `Pagination`
facade, or `Builder::on("replica_b").paginate(20)` from the Eloquent
side.

See [Eloquent - multi-connection routing](eloquent.md) for the routing
contract.

## When to reach for which

A rough decision tree:

- **Numeric page UI is part of the design** → `paginate`. You need
  `last_page` to render "Page 3 of 17", and the COUNT cost is OK on
  your table size.
- **"Next" / "Previous" buttons only, large table** → `simple_paginate`.
  One query per page; you give up `total` and `last_page` but the page
  load halves.
- **Infinite scroll** → `cursor_paginate`. Bidirectional cursors mean
  the client can keep scrolling past page 1000 without the OFFSET
  scanning thousands of rows first.
- **Tail of a hot append-only feed** → `cursor_paginate`. Keyset
  ordering by primary key is concurrent-safe: new rows land beyond the
  cursor, never inside it. OFFSET-based pagination skips rows under
  inserts.
- **Building a `Select<E>` outside an Eloquent model** →
  `Pagination::length_aware` / `Pagination::cursor`. Same trade-offs;
  the facade is the model-less equivalent.

When in doubt, start with `paginate`. Move to `simple_paginate` when
the `COUNT(*)` shows up in your slow query log. Move to
`cursor_paginate` when deep pages start dominating request time, or
when the UI is infinite scroll.

## Where each piece lives

| Piece | File |
|---|---|
| `Pagination` facade, `Paginated<T>` trait | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>` (simple) | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`, `CursorDirection`, `encode_value`, `decode_value` | `framework/src/pagination/cursor.rs` |
| `IntoInertiaScroll` bridge | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`, `InertiaResponse::paginate` | `framework/src/inertia/facade.rs`, `framework/src/inertia/response.rs` |
| `Resource::paginated`, `JsonApi::paginated` | `framework/src/resources/response.rs` |

## Next

- [Eloquent API](eloquent.md) - the model layer that drives every
  paginator returned from `Builder::paginate*`
- [Query Builder](queries.md) - the model-less queries that compose
  with `Pagination::length_aware` and `Pagination::cursor`
- [Inertia Responses](frontend-inertia-responses.md) - how scroll
  props attach paginators to Inertia pages
- [JSON:API resources](eloquent-resources.md) - `Resource::paginated`,
  links, meta, and the `Paginated<T>` trait
- [Error Model](error-model.md) - the `FrameworkError::param`
  validation rule and the cursor-tampering downgrade
