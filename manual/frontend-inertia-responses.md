# Inertia Responses

Inertia responses are how a Suprnova handler ships state to a Svelte / React /
Vue page component. Every handler that renders an Inertia page returns one,
built either through the [`inertia_response!`](#the-inertia_response-macro)
macro (for typed, compile-time-checked eager props) or the
[`InertiaResponse`](#the-inertiaresponse-builder) builder (for everything
else - lazy props, deferred props, merge, once, scroll, flash). This
chapter covers the response surface end-to-end: the macro, the builder, the
v3 protocol features (partial reloads, history encryption, version
detection), shared data via `App::inertia_share*`, and the flash bag carried
across redirects.

If you haven't picked a frontend yet, [Frontend Overview](frontend.md) and
[Page Components](frontend-pages.md) come first; this chapter assumes the
SPA bridge is wired and focuses on what your handler returns.

## The `inertia_response!` macro

The macro is the shortest path from a handler to a typed eager page. It
takes the current request, a component name, and a props expression:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Three things to know:

- **The leading `&req` is required.** The macro reads `X-Inertia` headers,
  the URL, and the partial-reload filtering headers off the request, so it
  needs the request value (or a reference). Without it, partial reloads
  would silently break.
- **Component existence is checked at compile time.** The macro looks for
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; if no file
  matches, the build fails with a "did you mean…?" suggestion sourced from
  the actual filenames on disk. Nested paths work the same way -
  `inertia_response!(&req, "Admin/Dashboard", …)` resolves
  `frontend/src/pages/Admin/Dashboard.svelte` (or your frontend's
  extension). A project with another page layout sets its own lookup in
  `Cargo.toml`; see [Another page layout](frontend-pages.md#another-page-layout).
- **The macro expands to an `await`ed `Result`.** Your handler must
  return [`Response`](error-model.md) (which is
  `Result<HttpResponse, HttpResponse>`) or another type that absorbs
  `FrameworkError` through `?` / `From`. Failures during prop
  serialization or response building are returned as `Err`, not panics.

For a page with no logic at all - about, terms, privacy - skip the
handler entirely and declare the route:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

See [Routing](routing.md#router-level-redirects-and-views). The
component there is a runtime string, so it doesn't get this macro's
compile-time existence check - that's the trade for not writing the
handler.

### JSON-style props

For prototyping and tiny pages you can skip the typed struct:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

The macro still validates the component file. The trade-off is that you
lose the typed-prop chain - no `#[derive(InertiaProps)]`, no automatic
TypeScript generation, no compile-time check that the frontend's
expected shape matches.

### Optional config override

The macro accepts an optional trailing `InertiaConfig` for per-response
overrides (different SSR settings, a custom default title for one page):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

Most apps register a single config at boot via [`Inertia::install`](#bootstrap-inertia-install)
and never touch this argument - the installed config is already what
every response starts from. Pass one here only to override the installed
config for a single page.

## `#[derive(InertiaProps)]`

`InertiaProps` emits a `Serialize` impl whose key names match your field
names. It exists so the typed-props path stays terse and so the
TypeScript generator (`suprnova generate-types`) has a marker to find:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Nested types compose normally - fields can be `Vec<T>`, `Option<T>`,
nested structs, anything `Serialize`-able. The nested types themselves
don't have to derive `InertiaProps`; they just need `Serialize`. Use
`#[derive(InertiaProps)]` on the *top-level* props struct and you get
the automatic TypeScript surface (see [TypeScript Types](frontend-typescript-types.md))
for the whole tree.

## The `InertiaResponse` builder

The macro covers eager typed props. Anything else - lazy, optional, deferred,
mergeable, cached-on-client, flash, history-encryption overrides - uses
the builder directly:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: closure runs only when the prop will actually be sent
        // (initial visit, or partial reload that requests this key).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: never sent on initial visits; the client must
        // explicitly ask for the key via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: skipped on the initial render; the client issues a
        // follow-up XHR and the closure runs then.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: append-into-existing on partial reloads ("load more").
        .merge("rows", next_page().await?)
        // Once: cached client-side across navigations; resolver skipped
        // on subsequent visits unless server forces refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: one-shot toast; appears under `page.flash`, not `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Method | Purpose | Maps to Laravel |
|---|---|---|
| `.with(k, v)` | Eager prop, honours partial-reload filtering | typed prop |
| `.always(k, v)` | Eager prop, ignores partial-reload filters | `Inertia::always(…)` |
| `.always_with(k, ‖)` | Async resolver, ignores partial-reload filters | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | Resolver runs only when prop will be sent | `fn () => …` closure |
| `.optional(k, ‖)` | Never on initial visit; must be requested explicitly | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Initial-visit-skipped; follow-up XHR triggers resolution | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combine with existing client state on partial reloads | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | Client caches across navigations | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.scroll_lazy` / `.scroll_lazy_with` / `.paginate` (via `Inertia::paginate`) | Infinite-scroll pagination | `Inertia::scroll(…)` |
| `.flash(k, v)` | One-shot value under `page.flash` (not `props`), kept in the session until a page shows it | `Inertia::render(…)->flash(…)` |
| `.title(…)` | Default `<title>` for the HTML shell | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Per-response history encryption | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Force history key rotation on **this** page | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Keep `#fragment` after Inertia visit | `Inertia::preserveFragment()` |

Eager builder methods have `try_*` siblings (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_scroll_wrapped`, `try_flash`) that return
`Result<Self, FrameworkError>` when a value's `Serialize` impl might
fail at runtime - the infallible methods convert the panic into a 500
via [the panic boundary](error-model.md), so reach for `try_*` when
you'd rather handle the failure explicitly.

`.clear_history()` marks the response you are building. A logout handler
redirects, and the browser discards the redirect's response - so the login
page, not the logout response, is the one that has to carry the flag.
`App::clear_history()` (also `Inertia::clear_history()`) is the fix for that
case - it's a free function, not a builder method, so it isn't in the table
above. It sets the session entry `inertia.clear_history`, which the next
Inertia page object turns into `clearHistory: true` and removes. It needs a
session scope, and it lasts until a page emits it, however many redirects
come first, so a logout that bounces through two redirects still clears.

Call it **after** `Auth::logout()` / `Auth::logout_and_invalidate()`, not
before - invalidation flushes the whole session, and the flag lives in
that session, so flashing it first only gets erased by the flush:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Composing flags on one prop

The methods above each set one flag. A prop can carry several, and some
combinations are how the Inertia protocol expects real pages to work: a
deferred list that appends into what the client already rendered, a
merge prop the client caches across navigations, an optional prop with
its own cache key. Build the prop with `Prop`, then attach it with
`.prop(key, prop)`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

That prop is skipped on the first render and announced under
`deferredProps`. The client issues its follow-up request, the resolver
runs, and the value arrives with a `mergeProps` instruction, so it
appends to the list already on screen instead of replacing it.

The flags fall into five groups:

| Group | Methods | Effect |
|---|---|---|
| Visibility | `.always()`, `.optional()`, `.defer()` | Mutually exclusive; the last call wins |
| Defer detail | `.group(name)`, `.rescue()` | Read only when the prop is deferred |
| Merge | `.merge()`, `.prepend()`, `.deep_merge()`, `.append_at(paths, match_on)`, `.prepend_at(paths, match_on)`, `.match_on(fields)`, `.merge_with_path(path)` | How the client folds the value in, and at which path |
| Client cache | `.once()`, `.once_with(options)`, `.as_key(key)`, `.until(moment or span)`, `.fresh(bool)` | Whether the client keeps the value across navigations - see [Once props](#once-props) |
| Scroll | `.scroll(metadata)`, `.scroll_wrap(key)`, `.scroll_at_root()` | Infinite-scroll `scrollProps` entry plus unconditional merge metadata under the wrapper (`data` by default); `.scroll_wrap` and `.scroll_at_root` read only when `.scroll` is set |

Sources are `Prop::eager(value)`, `Prop::lazy(closure)`,
`Prop::from_resolver(resolver)` for a resolver you built yourself, and
`Prop::absent()` for a prop that never reaches the response - what
`when_loaded!` returns for an unloaded relation.

Two rules are worth knowing before you compose:

- **Visibility is one setting, not three flags.** `.always().optional()`
  is an optional prop, and `.optional().always()` is an always prop.
  Neither is an error; the earlier call is erased.
- **Metadata follows the partial-reload lists, not the value.** A prop's
  `mergeProps` and `onceProps` entries are emitted whenever an
  `X-Inertia-Partial-Data` entry names the prop or an ancestor of it (or
  there is no such list) and no `X-Inertia-Partial-Except` entry does,
  even on a visit where the value itself is withheld. That is what
  carries the merge instruction across a deferred prop's two requests.
  Its `scrollProps` entry needs only that the key passes the lists. The
  consequences:
  - An entry deeper than the prop selects the prop but carries no
    instruction: `only: ['items.data']` against a merge prop `items`
    sends the whole value with no `mergeProps` entry, so the client
    replaces what it holds, as Laravel's `isIncludedInPartialMetadata`
    rules. A scroll prop keeps its cursor in that case.
  - A `.once()` prop the client already holds, and that is not deferred,
    sends its `onceProps` entry and nothing else - no `mergeProps` entry,
    since no new value arrives to merge. A held `.scroll().once()` prop
    keeps its cursor too.
  - An `.always().merge()` prop outside the requested set still sends its
    value and does not send its merge instruction, so the client replaces
    rather than appends.
  - `scrollProps` has one extra condition on top of the lists: a
    `.scroll().defer()` prop announces its merge instruction at the bare
    key on a non-partial visit but ships no cursor there, because nothing
    is on screen yet for a cursor to describe. Every matched partial
    reload gets the cursor and the instruction under the wrapper, whether
    or not that request also resolves the value.
  - `deferredProps` is the one block the lists never govern. It is
    dropped whole on any matched partial reload, no matter what the
    lists say - Laravel's `resolveDeferredProps` returns `[]` the
    moment the request is partial. A partial reload is the client
    working through announcements it already holds, so re-announcing
    the keys it left out of this round would send it back for them
    again. A partial reload aimed at a *different* component is a
    standard visit for every gate, announcements included.

`.group(name)` and `.rescue()` are stored on any prop but only read when
the prop is deferred, so `.rescue().defer()` and `.defer().rescue()`
mean the same thing. A scroll prop takes its merge direction from the
client's `X-Inertia-Infinite-Scroll-Merge-Intent` header, so `.merge()`
and `.prepend()` on a scroll prop are redundant and not read.
`.deep_merge()` is the exception: it routes the prop into
`deepMergeProps` instead of `mergeProps`, the same way Laravel's
`ScrollProp` does.

### Once props

A once prop is resolved the first time a page needs it and then kept by
the client across navigations: on later visits the client lists its key
in `X-Inertia-Except-Once-Props` and the server skips the resolver. The
options are Laravel's:

```rust
use suprnova::{InertiaResponse, OnceOptions, Prop};
use serde_json::json;

InertiaResponse::new("Billing/Index")
    // Inertia::once(fn () => ...)->as('plans')->until(3600)
    .once_with("planCatalog", OnceOptions::new().as_key("plans").until(3600), || async {
        Ok::<_, suprnova::FrameworkError>(load_plans().await?)
    })
    // The same options on a composed prop.
    .prop(
        "rates",
        Prop::lazy(|| async { json!({ "usd": 1 }) })
            .defer()
            .once()
            .until(suprnova::chrono::Duration::minutes(5))
            .fresh(false),
    )
```

| Option | Laravel | Effect |
|---|---|---|
| `.as_key(key)` | `as($key)` | The cache key the client dedupes on, the prop's name by default. A string, or an enum whose `Display` names the key |
| `.until(span or moment)` | `until($delay)` | When the client drops its copy. A whole number of seconds, a `std::time::Duration` or a `chrono::Duration` counts from the render; a `DateTime<Utc>` is that moment |
| `.fresh(bool)` | `fresh($value)` | `true` resolves even when the client claims a copy; `false` honours the claim again |
| `OnceOptions::once(bool)` | `once($value)` | `false` turns the flag off, leaving an ordinary prop |

`Prop::once_with(options)` and `InertiaResponse::once_with(key, options,
resolver)` take all of them at once, Laravel's `once($value, $as,
$until)`; a setting the options leave unset keeps what the prop already
has.

The page object carries the expiry as `onceProps.<key>.expiresAt`, in
milliseconds since the epoch, counted in whole seconds as Laravel counts
it: `.until(60)` on a page rendered at second `t` gives `(t + 60) * 1000`.
A moment already past gives the render moment.

The server also enforces a moment itself, which Laravel leaves to the
client: once `.until(deadline)` has passed, a client that still lists the
key in `X-Inertia-Except-Once-Props` gets a fresh value instead of nothing,
so a stale client cannot pin an old value. A span is counted from each
render, so it is the client's own expiry that ends it.

### Merge strategies and infinite scroll

`.merge` (append), `.merge_prepend`, and `.deep_merge` cover the common
"load more" cases. To diff-merge - update rows the client already holds
instead of duplicating them - reach for `.merge_with` with an explicit
`MergeStrategy` carrying a `match_on` key:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` names the field(s) the client dedupes on (emitted to the page
object as `matchPropsOn`) - one field or several, the same as
`Prop::match_on` (below) - so a refetch that overlaps the current window
replaces matching rows in place rather than appending copies. `Prepend`
and `Deep` take the same `match_on`.

`MergeStrategy` is the one-call form. `Prop::merge()` / `.prepend()` /
`.deep_merge()` / `.match_on(field)` are the same settings as separate
flags, for when the prop also needs a visibility or cache flag - see
[Composing flags on one prop](#composing-flags-on-one-prop).

`.match_on` takes one field or several in one call
(`.match_on(["id", "slug"])`). Each call replaces the list, as Laravel's
`matchOn` does, so `.match_on("id").match_on("slug")` dedupes on `slug`
alone.

To merge only part of a prop's value instead of the whole thing, name
the nested path with `.append_at` or `.prepend_at` - Laravel's
`append($path, $matchOn)` and `prepend($path, $matchOn)`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta })).append_at("data", "id"),
)
```

`mergeProps` now carries `"posts.data"` instead of `"posts"`, so only
`props.posts.data` folds into what the client already holds -
`props.posts.meta` is replaced outright, like any non-merge prop. The
second argument names the field to dedupe on at that path and adds
`"posts.data.id"` to `matchPropsOn`; pass `None` for no dedupe field.
The first argument takes one path or several:
`.append_at(["a.items", "b"], "id")` merges at both paths and adds
`"<key>.a.items.id"` and `"<key>.b.id"`. Calls accumulate, and one prop
can append at one path and prepend at another:

```rust
Prop::eager(json!({ "older": older, "newer": newer }))
    .append_at("older", None)
    .prepend_at("newer", "id")
```

That prop emits `mergeProps: ["activity.older"]`,
`prependProps: ["activity.newer"]` and
`matchPropsOn: ["activity.newer.id"]` under the key `activity`. Naming a
path turns off root-level merging for that prop entirely - a
path-merging prop never also merges its whole value. `.append_at` and
`.prepend_at` turn merging on by themselves, so the prop needs no
`.merge()`. A later `.match_on(...)` replaces the fields they added, as
it does in Laravel. `.deep_merge()` ignores the paths - a deep merge
already recurses into every nested field, so there's nothing a path
narrows.

`.merge_with_path(path)` is the older single-path form: it merges at the
path in the prop's own direction (`.merge()` or `.prepend()`) and adds no
dedupe field, so pair it with `.match_on("data.id")` yourself.

A merge prop's value can come from a resolver too, via `.merge_lazy` /
`.merge_lazy_with` - the resolver sibling of `.merge` / `.merge_with`:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

The resolver runs only when the merge prop will actually be sent -
skipped by partial-reload filtering and by `.defer()` like any other
resolver-backed prop.

Infinite scroll is the same machinery with pagination metadata attached.
`.scroll` / `.scroll_with` - or `.paginate`, which adapts a
`LengthAwarePaginator`, `Paginator` or `CursorPaginator` directly - emit
`scrollProps` next to the data, and the client's `<InfiniteScroll>`
component drives the next/previous fetches:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

A scroll prop merges under a wrapper, `data` by default, as Laravel's
`Inertia::scroll($value, $wrapper = 'data')` does: a paginator or resource
serializes its rows under `data`, and only the rows should fold into what
the client already holds. `.scroll("posts", metadata, value)` emits
`mergeProps: ["posts.data"]`. `.paginate` is the exception: it ships the
paginator's rows as a bare list, so it merges at the prop's root
(`mergeProps: ["posts"]`); under `posts.data` the client would find no
list to append to. Reach for `Prop::scroll_at_root()` for any other value
that is the list itself.

A scroll prop always carries merge metadata, not just on a follow-up
fetch: it defaults to append, and switches to prepend only when the
client's `X-Inertia-Infinite-Scroll-Merge-Intent` header says so (`append`
when scrolling down, `prepend` when scrolling up). `reset` is independent
of that header - it's `true` exactly when the client named the key in
`X-Inertia-Reset`, the same header a regular merge prop reads. A fresh,
unfiltered visit sends neither header, so it gets `reset: false` and an
append instruction, matching Laravel.

`.merge_with_path` has no effect on a scroll prop - the scroll block that
computes its merge instruction reads the prop's single wrapper, not
`.merge_with_path`'s accumulated path list. `.scroll_wrap` - reached
directly through `.prop(...)`, or through the `.scroll_wrapped` response
shortcut below - names another wrapper.

A scroll prop also honors `.match_on(...)`, the same as any other merge
prop - reach it through `.prop(...)`, since neither `.scroll` nor
`.match_on` has a combined response-level shortcut. The path is relative
to the prop, as in Laravel, with no wrapper prefix added, so name the
wrapper in it:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(serde_json::json!({ "data": rows }))
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("data.id"),
)
```

That emits `matchPropsOn: ["users.data.id"]`, which the client matches
against the `users.data` merge path. `.match_on("id")` would emit
`"users.id"`, which lines up with a scroll prop merging at its root
(`.paginate`, `.scroll_at_root()`) and with nothing under a wrapper.

When the value's list sits under another field - `{ items: [...],
meta: {...} }` - point the merge at that field with `.scroll_wrapped`:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "items",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "items": rows, "meta": { "total": total } }),
)
```

`mergeProps` then names `posts.items`, so the client folds new rows into
the nested array and leaves `meta` to be replaced wholesale each time.
`.scroll_with_wrapped` and `try_scroll_wrapped` are the resolver-based and
fallible siblings, matching `.scroll_with` / `try_scroll`.

A deferred scroll prop (`Prop::lazy(...).scroll(metadata).defer()`)
announces its bare key under `mergeProps` on the visit that withholds it,
and `posts.data` on the follow-up request that delivers the rows, as
Laravel does.

The metadata argument of `.scroll` takes a `ScrollMetadata` or anything
that implements `ProvidesScrollMetadata`, Laravel's interface of the same
name. `LengthAwarePaginator`, `Paginator` and `CursorPaginator` implement
it, so a paginator can be both the metadata and the value - Laravel's
`Inertia::scroll($paginator)`, its rows under `data`:

```rust
InertiaResponse::new("Feed/Index").scroll("posts", &page, &page)
```

The paginators report what Laravel's `ScrollMetadata::fromPaginator`
reports: the page parameter's name (`with_page_name` on
`LengthAwarePaginator` and `Paginator`, `with_cursor_name` on
`CursorPaginator`), and the previous, next and current page. A cursor
page's current page is `1` on the first page, else the cursor it was
fetched with - `Pagination::cursor` records it, `with_current_cursor` sets
it - else the request's cursor parameter.

A type outside this crate's `pagination` module - a third-party
paginator, a hand-rolled cursor - can describe itself to `.scroll` the
same way:

```rust
use suprnova::ProvidesScrollMetadata;

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", &page, page.rows)
```

A list loaded lazily describes its own pages from the loaded value, so it
needs no second query for them. `.scroll_lazy` reads them from a value
that implements `ProvidesScrollMetadata`, Laravel's
`Inertia::scroll(fn () => User::paginate())`; `.scroll_lazy_with` builds
them with a function of the loaded value, Laravel's callable metadata:

```rust
use suprnova::{FrameworkError, InertiaResponse, Prop, ScrollMetadata};

InertiaResponse::new("Feed/Index")
    .scroll_lazy("posts", || async {
        Ok::<_, FrameworkError>(Post::query().paginate(20).await?)
    })
    .scroll_lazy_with(
        "events",
        || async { Ok::<_, FrameworkError>(load_events().await?) },
        |events: &EventPage| ScrollMetadata::new("after").next(events.next_token.clone()),
    )
```

`Prop::scroll_lazy(resolver, metadata)` is the same prop for composing
with other flags, such as `.defer()`. The function runs only when the
value loads, so the `scrollProps` entry ships with the value and not on a
visit that withholds it. The value ships whole on a partial reload, as
Laravel ships a closure's result.

### Dot-notation nesting

A key containing `.` nests into the response instead of shipping as a
literal string key - Laravel's `Arr::set`-backed dot notation
(`Inertia::share('user.name', …)`, `resolveArrayableProperties`):

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

ships as:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

not two literal `"user.name"` / `"user.locale"` keys. Two calls sharing a
prefix accumulate into one object; a key with no dot is unaffected. This
applies to every prop-attaching method - `.with`, `.always`, `.lazy`,
shared-registry keys - and to nothing else: it never recurses into a
prop's *value*, so a validation `errors` object keeps whatever dotted
field names it carries internally. There is no escape hatch for a key
that must keep a literal dot (`.with("config.json", …)` still nests) -
this matches Laravel, where `Arr::set` has no escaping mechanism either.

## Partial reloads

The Inertia 3 client can request a subset of a page's props (or a
superset by including an Optional or Defer key). The protocol uses
three request headers:

| Header | Meaning |
|---|---|
| `X-Inertia-Partial-Component` | The component being partial-reloaded - must match the response's component for filtering to apply. |
| `X-Inertia-Partial-Data` | Whitelist: comma-separated prop paths to include. |
| `X-Inertia-Partial-Except` | Blacklist: comma-separated prop paths to exclude. Applied after `Partial-Data`, so it wins on a path both name. |

Both lists are read as Laravel reads them: split on `,`, empty segments
dropped, and no trimming, so `a, b` names `a` and ` b`. A header that names
nothing - empty, or only commas - counts as absent rather than as a list
that matches no prop. The other list headers the client sends,
`X-Inertia-Reset` and `X-Inertia-Except-Once-Props`, are read by the same
rule.

Filtering reads one thing: the prop's visibility, set by `.always()`,
`.optional()`, or `.defer()`. A prop with none of those has the default
visibility.

- Default-visibility props follow whitelist / blacklist semantics.
- `.always()` props are sent regardless.
- `.optional()` and `.defer()` props never ship on a standard visit. On a
  matching partial reload they resolve whenever their key passes the
  whitelist and blacklist, as Laravel's do: a reload that sends only
  `X-Inertia-Partial-Except` (`router.reload({ except: ['stats'] })`)
  resolves every optional and deferred prop it does not exclude.

The merge and scroll flags do not enter into it: they decide how the
client folds a value it receives, not whether it receives one, so a
`.defer().merge()` prop filters exactly like a plain `.defer()` one.
`.once()` doesn't enter into it either, though it isn't purely a folding
instruction - on a full visit where the client reports the value already
cached, the server skips the resolver and sends no value, as the note
below describes. What all three change is which metadata blocks ride
along - see [Composing flags on one prop](#composing-flags-on-one-prop).
On a partial reload the `merge` and `once` instructions are stricter than
the value: they ship only when an `only` entry names the prop or an
ancestor of it, so an entry deeper than the prop (`items.data`) sends the
prop whole with no instruction.

The handler doesn't have to do anything special - register every prop
through the builder, and the framework consults the headers when
serializing the page object.

A `once` prop's client-side cache is honoured only on a **full** Inertia
visit. On a partial reload that names the key
(`router.reload({ only: ['stats'] })`), the resolver runs and the value is
sent - the client asked precisely because it wants a fresh one, and
honouring its stale-cache claim there would return nothing at all for the
key it asked for.

### Nested only/except (dot notation)

`X-Inertia-Partial-Data` and `X-Inertia-Partial-Except` entries can name a
path inside a prop's value, not just the prop's own key. A client calling
`router.reload({ only: ['user.name'] })` sends
`X-Inertia-Partial-Data: user.name`, and the response narrows a literal
`user` prop down to just that field:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` prunes the same way instead of narrowing - `router.reload({
except: ['user.email'] })` leaves every other field of `user` in place.

The rule is Laravel's: dotted entries narrow **literal values** only - a
value passed to `.with(...)`, a value shared with `App::inertia_share`, an
eager Data field. The walk keeps each nested path that is, descends from,
or leads to an `only` entry, and that neither is nor descends from an
`except` entry.

Rules:

- A value that came from a resolver or a prop object ships whole when its
  key, an ancestor, or a path inside it is selected. `.lazy(...)`,
  `.optional(...)`, `.defer(...)`, `.merge(...)`, `.once(...)`,
  `.scroll(...)` and `.always(...)` props are not walked, so
  `only: ['users.name']` against `.lazy("users", …)` sends every field of
  `users`. One exception follows Laravel too: a flag-free resolver under a
  dotted key (`.lazy("auth.user", …)`) narrows like a literal, because
  Laravel calls a dotted key's closure before its walk.
- A bare entry (`user`) still means the whole prop. If `only` names both
  `user` and `user.name`, the whole value ships - the bare entry wins.
- An entry can also name an *ancestor* of a dotted prop key. A prop
  registered under `auth.user` - by `.with("auth.user", …)` or
  `App::inertia_share("auth.user", …)` - participates in
  `only: ['auth']`, and ships whole, because the caller asked for the
  whole `auth` root. A bare `except: ['auth']` drops it for the same
  reason. The prefix has to end on a segment boundary, so an unrelated
  `authAgent.user` prop is untouched by either.
- `except` wins on a path both headers name, the same way it wins at the
  top level.
- A path that resolves to nothing - an unknown field - contributes nothing,
  without dropping the sibling fields requested alongside it. A value that
  keeps none of its children is `[]`, PHP's empty array: `only:
  ['user.missing']` sends `"user": []`.
- The walk goes into lists by index (`rows.0.id`). A list that keeps a
  prefix of its items stays a list; one that keeps other items becomes an
  object keyed by their indexes (`{"1": …}`), as PHP encodes an array whose
  keys no longer start at 0. A scalar a deeper path runs into
  (`config.level` under `only: ['config.level.nested']`) ships as it is.
- `Optional` and `Defer` props resolve on a partial reload whenever their
  key passes the lists. A dotted entry (`permissions.read`) selects the
  top-level key, and the resolved value ships whole.
- A dotted `except` doesn't delete the field on the client - it stops the
  field from refreshing on this response, and the client's merge restores
  it from whatever it already had cached. `deepMergeObjects` builds the
  merged object by cloning the cached value first and then only
  overwriting the keys the server actually sent; a key the server pruned
  is never touched, so it survives with its old value. On a
  client's first-ever load of that prop (nothing cached yet) the pruned
  field is genuinely absent, since there's no cache to fall back to - the
  "restores from cache" behavior only applies to a page the client has
  already seen.

## Shared data via `App::inertia_share*`

Some props are the same on every Inertia page - auth state, the CSRF
token, the current locale, app-wide flags. Register them once at
bootstrap and they merge into every response:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materialized once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolved per response (skipped by partial reloads that
    // exclude the key).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cached on the client across navigations - `share_once` runs on
    // the first page that needs it, then the client skips re-resolution
    // via `X-Inertia-Except-Once-Props` until the cache key changes.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    })
    .until(3600);
}
```

`inertia_share_once` returns the registered prop, as Laravel's
`Inertia::shareOnce` returns its `OnceProp`, and it takes the options any
once prop takes - `.as_key(key)`, `.until(span or moment)`,
`.fresh(bool)` and `.once_with(options)`, described under
[Once props](#once-props). Each call changes the registered prop at once;
a later share under the same key replaces it, and the earlier handle then
changes nothing.

Shared keys nest on dots the same way `.with` does - two static shares
under `"user.name"` / `"user.age"` land in one `user` object on the wire.
Read a shared value back, or clear the static registry entirely, with
`App::inertia_shared` / `App::flush_inertia_shared` - Laravel's
`Inertia::getShared` / `Inertia::flushShared`:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

A numeric segment reads into a shared list, as `Arr::get` does:
`App::inertia_shared("users.0.name")` reads the first user's name.

`inertia_shared` reads the static registry only - it returns `None` for a
key registered via `inertia_share_lazy` / `inertia_share_once` (there's no
request to resolve one against, mirroring Laravel's `getShared`, which
returns the raw closure rather than invoking it) and for a per-request
trait-provider share. `flush_inertia_shared` clears only the static
registry too; a provider registered via `register_inertia_shared` has no
per-request state to flush.

For per-request shared data (the authenticated user, request-scoped
flags), implement [`InertiaSharedData`](#per-request-shared-data) and
register the singleton - the framework calls `share(&req, component)` on
every Inertia response and merges the result. `component` is the page
being rendered, so a provider can vary its output by page - see below.

### Precedence on key collision

When the same key appears in more than one layer, later writes win:

1. Static registry (`App::inertia_share` / `App::inertia_share_lazy`),
   then the shared [providers](#prop-providers) in the order they were
   shared
2. Per-request trait provider (`InertiaSharedData::share`)
3. The page's [providers](#prop-providers), in the order they were given
4. Per-response builder methods (`.with`, `.lazy`, etc.)

This lets a handler override a globally-shared default for one page
without having to unregister anything.

### The `sharedProps` list

The page object lists the top-level key of every shared prop under
`sharedProps`, as Laravel's does: the static shares, the shared
providers' keys and the per-request provider's keys, each by its root
segment (`"auth"` for a share under `"auth.user"`), with `errors` first,
since every response shares the validation errors. The client reads the
list during an instant visit, to carry the shared values into the page it
renders before the server answers. A page that overrides a shared key
keeps it on the list; the client reads the value from `props`.

```json
{ "component": "Signup", "props": { "errors": { "email": "Taken" }, "auth": { "user": "Ada" } }, "sharedProps": ["errors", "auth"] }
```

`InertiaConfig::expose_shared_props(false)` leaves the list out of every
page object - Laravel's `inertia.expose_shared_prop_keys` setting. It is on
by default; the shared values still ship as props when it is off.

### Per-request shared data

The trait runs once per Inertia response with access to the request
**and** the page component name - Laravel's `RenderContext` (`component`,
`request`), passed as a plain parameter rather than a wrapper struct
since the request already covers the other half. Implementations need
`async_trait` (re-exported as `suprnova::async_trait`) and `IndexMap`
(re-exported as `suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignore `component` (`_component`) if your provider doesn't need to vary by page.

### Prop providers

A value that stands in for several props implements
`ProvidesInertiaProperties` - Laravel's interface of the same name. A page
takes any number of them with `.provide(...)`, the shared props any number
with `App::inertia_registry().share_provider(...)`, and each one expands
once per render with a `RenderContext` of the page component and the
request:

```rust
use suprnova::{
    FrameworkError, InertiaResponse, Prop, ProvidesInertiaProperties, RenderContext,
    indexmap::IndexMap,
};

pub struct TeamProps {
    team_id: i64,
}

impl ProvidesInertiaProperties for TeamProps {
    fn to_inertia_properties(
        &self,
        context: &RenderContext<'_>,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let team_id = self.team_id;
        let mut props = IndexMap::new();
        props.insert("teamId".into(), Prop::eager(team_id.into()));
        // Async work goes in a lazy prop, which runs only when it is sent.
        props.insert(
            "members".into(),
            Prop::lazy(move || async move { load_members(team_id).await }),
        );
        if context.component() == "Teams/Settings" {
            props.insert("canDelete".into(), Prop::eager(true.into()));
        }
        Ok(props)
    }
}

InertiaResponse::new("Teams/Show")
    .provide(TeamProps { team_id: 7 })
    .provide(BillingProps::for_team(7))
    .with("title", "Team")
```

Expansion is synchronous, as Laravel's is; a provider returns lazy props
for work that has to wait. Providers merge in the order they were given,
a later one winning over an earlier one, and the page's own props win
over its providers' whatever the call order. Shared providers expand
after the keyed shares, and their keys are shared keys. A `#[derive(Data)]`
object works the same way for its fields: `.with_data(dto)` adds one, any
number of times, with its lazy fields still behind the `?include=`
allowlist (`try_with_data` is the fallible sibling).

`App::flush_inertia_shared()` clears the shared providers with the keyed
shares, as Laravel's `flushShared` does.

One prop's value can convert itself when it is sent: implement
`ProvidesInertiaProperty` (Laravel's interface of the same name) and
attach the value with `.with_property(key, value)` or `Prop::property`.
The conversion gets a `PropertyContext` of the prop's key path, its
sibling props and the request:

```rust
use suprnova::{FrameworkError, PropertyContext, ProvidesInertiaProperty};

pub struct Money(i64);

impl ProvidesInertiaProperty for Money {
    fn to_inertia_property(
        &self,
        context: &PropertyContext<'_>,
    ) -> Result<serde_json::Value, FrameworkError> {
        let currency = context
            .props()
            .get("currency")
            .and_then(|prop| prop.as_value())
            .and_then(|value| value.as_str())
            .unwrap_or("USD");
        Ok(format!("{}.{:02} {currency}", self.0 / 100, self.0 % 100).into())
    }
}

InertiaResponse::new("Shop/Show")
    .with("currency", "EUR")
    .with_property("price", Money(1250))   // "12.50 EUR"
```

The conversion runs only when the prop is sent, so a prop a partial
reload leaves out is never converted. The siblings are the page's props
before resolution, shared ones included; a sibling given as a value can
be read with `Prop::as_value`, while a resolver has not run yet. The
converted value ships whole, as Laravel ships an object's conversion: a
dotted `only` entry does not narrow it.

## Flash and redirects

Flash data is one-shot state that should appear on the next render and
disappear after - toast messages, "just created" IDs, validation summaries.
Suprnova surfaces it under `page.flash`, outside `props`, so it never enters
the browser's history state. Laravel's `Inertia::flash` is the main writer:

```rust
use suprnova::{FlashKey, Inertia, InertiaResponse, Redirect, Response};

pub async fn store() -> Response {
    // A key and a value, or several at once.
    Inertia::flash("toast", "Saved")?;
    Inertia::flash_many([("created_id", 42), ("count", 3)])?;
    Redirect::to("/posts").into()
}

// An enum key, as a Laravel app flashes with an enum case.
enum Toast {
    Success,
}

impl FlashKey for Toast {
    fn flash_key(&self) -> String {
        match self {
            Toast::Success => "success".to_string(),
        }
    }
}
```

`Inertia::flash`, `App::flash` (the same call with a string key) and
`InertiaResponse::new("Posts/Show").flash("toast", "Saved")` all write the
session entry `inertia.flash_data`, Laravel's key. The next Inertia page
response emits it under `page.flash` and removes it, whatever the request that
set it answered: a handler that flashes and returns plain JSON still gets its
toast on the next page. A redirect keeps it for one more request, so it
reaches the page at the end of any number of redirects. Without a session in
scope the value rides on the current response only.

Read or take the pending data with `Inertia::get_flashed(&req)` and
`Inertia::pull_flashed(&req)` - Laravel's `getFlashed` and `pullFlashed`.
`get_flashed` returns exactly what `pull_flashed` would remove, and a page
rendered after a pull shows none of it.

`Redirect::to("/posts").with("toast", "Created")` is a plain session flash:
it lands under `_flash.new.*`, the next request's
[`SessionMiddleware`](csrf.md) ages it into `_flash.old.*`, and that page
surfaces it under `page.flash` too, below the Inertia flash data. It lasts one
request, so a second redirect drops it.

Same-request flash wins over inherited session flash on key collision, so a
destination handler can override an inbound value just by re-flashing the key.

Internal session keys (anything prefixed `_`) are filtered out of
`page.flash` - `_old_input` for form repopulation doesn't leak to the client.

The two history flags work the same way: `App::clear_history()` (or
`Inertia::clear_history()`), `Inertia::preserve_fragment()` and
`Redirect::preserve_fragment()` set the session entries
`inertia.clear_history` and `inertia.preserve_fragment`, which last until a
page response emits them as `clearHistory: true` and `preserveFragment: true`,
however many redirects come first.

### Redirect helpers

`Redirect` is the full Laravel surface:

```rust
Redirect::to("/dashboard")                       // 302 to a path
Redirect::route("posts.show").with("id", "42")   // named route, route params
Redirect::back("/")                              // session-recorded previous URL
Inertia::back(302, Some("/"))                    // Referer, previous URL, fallback
Redirect::refresh()                              // same URL, fresh GET
Redirect::guest(&req, "/login")                  // stashes intended URL
Redirect::intended("/dashboard")                 // pops the stashed URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signed URL
Redirect::to("/posts/42").preserve_fragment()    // keep #frag across visit
```

All `Redirect` variants accept `.with(k, v)`, `.with_input(map)`,
`.with_errors(map)`, `.with_errors_bag(name, map)`, `.cookie(c)`,
`.header(k, v)`, `.permanent()`, `.status(303)`, etc. The full chain
mirrors Laravel's `RedirectResponse`.

`Inertia::back(status, fallback)` is Laravel's `back()`: it tries the
request's `Referer` first, when that is a path on this host under the public
root (the check the validation redirect applies, so a foreign `Referer` is
never followed), then the session's previous URL, then `fallback`, then `/`.
It reads the `Referer` from the request `InertiaHeadersMiddleware` is
handling, which `Inertia::install` puts on every route; `Redirect::back`
reads the previous URL only.

The previous URL is recorded twice over: `SessionMiddleware` records every
successful page load that is not an Inertia visit, a prefetch or a JSON call,
and the Inertia middleware records an Inertia `GET` that matched a route,
unless it is a prefetch (`X-Moz`, `Purpose` or `Sec-Purpose` set to
`prefetch`), a Precognition request or a partial reload of the component it
rendered - deferred props, polling and infinite scroll reload the page the
visitor is on, which is no page they came from. Turn the Inertia half off
with `InertiaConfig::store_previous_url(false)`.

A redirect with a `#fragment` on an Inertia visit becomes `409` with
`X-Inertia-Redirect` holding the target, which the client visits itself so
the fragment survives; the fragment of a `Location` is lost inside the XHR
that follows it. A prefetch keeps its plain redirect, since it is never
shown. `InertiaResponse::redirect(url)` builds the same `409` by hand.

For non-GET Inertia visits, the framework auto-converts the response to
`303 See Other` when [`Inertia303Middleware`](#bootstrap-inertia-install)
is installed, so the browser issues a clean follow-up GET instead of
re-submitting the original PUT/PATCH/DELETE to the redirect target.

### Validation failures

When a handler fails validation on an Inertia visit, the framework
answers `303 See Other` back to the form page with the errors flashed,
instead of the `422` JSON a REST client gets. That is not cosmetic: the
Inertia client treats any response without an `X-Inertia` header as
non-Inertia and renders it in the full-screen error modal, so a `422`
never reaches `form.errors`. Nothing in the handler changes - the bridge
is one of the middlewares `Inertia::install` registers.

The destination is the request's `Referer` when it is same-origin, then
the session's recorded previous URL, then the failing request's own URL.
A cross-origin `Referer` is ignored rather than followed, and so is one
that only looks same-origin: a leading `//` or `/\` (a browser reads
either as protocol-relative once it folds a backslash into a slash) and
any ASCII control byte anywhere in the value (the URL parser strips tab
and newline from the whole string before it compares origins, so a
control byte can turn what looks like a safe path into a different
origin by the time a browser navigates it) both fall back the same way.
The same check applies to the final URL fallback too, so even an
unusual request path can't become an off-origin redirect.

A field's value is its **first** message, a plain string - the shape
Inertia's own `ErrorValue` type describes and what
`$page.props.errors.email` binds to. Set
`InertiaConfig::with_all_errors(true)` to get every message as an array
instead; the client-side type then needs the matching augmentation:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Multiple forms on one page stay isolated: send
`X-Inertia-Error-Bag: <name>` with the visit and the errors are flashed
under that bag and read back under it, arriving as `errors.<name>.<field>`.
The `errors` prop takes Laravel's shape: with the header, a session `default`
bag (from `Redirect::with_errors`) arrives as `{<name>: {...}}`, the named
bags arrive as they are when there is no `default` bag, and no errors at all
arrive as `{}`; without the header the `default` bag arrives flat.

The `errors` prop is always-visible by default, so a partial reload
never filters or narrows it. `only: ['users']` still ships the bag, and
so does `except: ['errors']`; `only: ['errors.email']` ships the whole
bag rather than just that field. This is Laravel's shape - its
middleware shares the bag as `Inertia::always(...)`, and `resolveAlways`
re-injects the raw value after the `only`/`except` rebuild. It matters
because the client folds a partial response in with
`{...current.props, ...response.props}`: an empty `errors` object would
wipe the messages already on screen, where an unfiltered one leaves them
correct. The rule covers both sources - the session-flashed bag and a
handler's own `.with("errors", …)`. An explicit visibility flag still
wins, so `.prop("errors", Prop::eager(…).optional())` behaves optionally.

Two things this does not do. It does not re-flash old input - the request
body is already consumed by the time the bridge runs, and an Inertia
`useForm` keeps its own state across a failed submit, so there is nothing
to repopulate. And it never touches a Precognition response: a dry-run
`422` is exactly what the client asked for.

To send the visitor **out** of the Inertia app - a payment provider, an
OAuth authorize endpoint, a hosted billing portal - use `location`:

```rust
use suprnova::{Inertia, Redirect, Response};

pub async fn checkout() -> Response {
    Ok(Inertia::location("https://billing.example/checkout"))
}

pub async fn portal() -> Response {
    // A redirect works too: a hard navigation gets it as it is.
    Ok(Inertia::location(Redirect::away("https://billing.example/portal").status(303)))
}
```

An Inertia XHR gets `409` + `X-Inertia-Location` (the client runs
`window.location = url`); a hard navigation gets a plain `302` + `Location`,
or the redirect you passed, status, flash and cookies included - Laravel's
`Inertia::location`. `InertiaResponse::location` is the same call. It reads
which kind of request it is answering from the facts the server scopes for
every request it dispatches, so a route outside the Inertia middleware
answers the same way; `InertiaResponse::location_for(&req, url)` decides
from the request you pass, for code that runs outside a dispatched request.

## Version detection

Inertia versions the asset manifest so a long-lived client doesn't try
to mount a page from yesterday's bundle against today's server. When
the client's `X-Inertia-Version` header on a `GET` doesn't match the
server's current version, [`InertiaVersionMiddleware`](#bootstrap-inertia-install)
responds with `409 Conflict` before the handler runs. `X-Inertia-Location`
holds the request's absolute URL (scheme, host, path and query, as
Laravel's `fullUrl()` gives it), and `X-Inertia-Version` holds the current
version. The Inertia client does a full page reload at that URL, picking up
the new bundle; for a poll or a background prop load it reads the version
header instead, so an asynchronous request does not force the reload. A
visit by any other method passes through: the `GET` its redirect leads to
gets the 409.

The bounce re-flashes the session first. The client answers a 409 with a
full-page GET, and that GET is a fresh request - without the re-flash, a
validation error or success message flashed by the previous request is aged
away before the destination page can read it, and the user loses their error
message purely because a deploy landed mid-submit. This needs
`SessionMiddleware` registered ahead of the version middleware.

By default you set nothing. The version resolves in the order Laravel's
`Middleware::version` uses:

1. When the config names an `asset_url` - the URL the built assets are
   published under when it changes with each deploy, such as a CDN path
   that carries a build id - the version is a hash of that URL. It
   defaults to the `ASSET_URL` environment variable, the one Laravel's
   `app.asset_url` reads; `.asset_url(...)` on the config wins over it.
2. Otherwise `InertiaConfig` hashes your Vite build manifest
   (`manifest_path`, default `public/assets/.vite/manifest.json`). The
   manifest is the one file that changes on every build and on no other
   occasion, so the version bumps itself.
3. When there is no manifest to read - local development, where Vite
   serves from memory - the version is the empty string, as Laravel's is,
   and a `debug` line is logged.

Both hashes are the first 16 bytes of a SHA-256, hex-encoded.

Override it when you want something else:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// Assets published under a per-deploy URL: the version follows the URL.
let cfg = InertiaConfig::new().asset_url("https://cdn.example.com/build-42");

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

The manifest is read on every version check, which is what Laravel's
`hash_file` does too - a few KB out of the page cache, and a rebuild is
picked up immediately. If you have measured that and want it gone,
resolve once at boot:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

For async or fallible version resolution (e.g. read a manifest hash
from S3), do the read once at boot and pass the cached `String` to
`.version(...)`.

### Setting the version at run time

`Inertia::version` sets the version while the app runs, as Laravel's
`Inertia::version` does, and `Inertia::get_version` reads it:

```rust
use suprnova::Inertia;

Inertia::version(deployment_id());     // a string
Inertia::version(|| read_build_id());  // a function, called on every read
Inertia::version(None::<String>);      // the empty version, Laravel's null

let current = Inertia::get_version();
```

The value replaces the version of the installed config, so every page
built after the call advertises it, and the version middleware that
`Inertia::install` registers compares the client's `X-Inertia-Version`
against it. A later `Inertia::install` replaces it again, and a response
given its own config with `with_config(...)` keeps that config's version.

## Bootstrap: `Inertia::install`

Most apps install the protocol middlewares in one call, from
`register_http_stack` - the HTTP-only bootstrap hook, which the server
path runs and the queue, schedule, workflow, and console binaries skip
(see [Bootstrap](bootstrap.md)):

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …the rest of your global middleware, in the order you want it to run
}
```

Anything the Inertia layer depends on - `SessionMiddleware` - and
anything an error page needs to read - `LocaleMiddleware` - goes *above*
this call. See [the ordering rules below](#bootstrap-inertia-install).

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Keep it out of `bootstrap::register`. `Inertia::install` fails closed in
production when the built frontend manifest is missing, which is exactly
the state of a worker or console image that ships no `public/assets` -
so installing it from the process-wide hook takes those binaries down
with it.

`Inertia::install` returns `Result` and, in order:

1. Fails closed if `cfg` resolves to production mode (`development ==
   false` - the default whenever `APP_ENV=production`) but no Vite
   manifest can be loaded from `cfg.manifest_path`. This is the CFG-01
   guard: a production boot with an unbuilt frontend errors loudly
   instead of silently falling back to a legacy hardcoded asset path.
2. Registers `InertiaHeadersMiddleware` - sets `Vary: X-Inertia` on every
   response. On an Inertia visit it turns an empty `200` into a redirect
   back (to the same-origin `Referer`, else the previous URL, else `/`; `302`,
   or `303` for `PUT`, `PATCH` and `DELETE`), turns a redirect with a
   `#fragment` into `409` + `X-Inertia-Redirect`, and records an Inertia
   `GET` as the session's previous URL - see
   [Redirect helpers](#redirect-helpers).
3. Registers `InertiaVersionMiddleware` - emits the `409` + `X-Inertia-Location`
   when client and server disagree on the asset version.
4. Registers `Inertia303Middleware` - upgrades `302` to `303` on non-GET
   Inertia redirects.
5. Registers `InertiaValidationRedirectMiddleware` - turns a `422` on an
   Inertia visit into a `303` back to the form page with the errors
   flashed. See [Validation failures](#validation-failures).
6. Registers `InertiaErrorPageMiddleware` - hands the framework's own
   error responses to your error callback, or turns them into the page
   `cfg` names with `.error_page(...)`. With neither, it changes nothing.
   See [Error pages](#error-pages). If you registered one yourself,
   further out, yours keeps its position and the component it names, and
   this step is skipped - see
   [Where the page is rendered](#where-the-page-is-rendered).

Order matters: the headers middleware is registered first, so it is the
outermost and sees every response - including the `409` the version
middleware returns before the handler ever runs. The validation-redirect
middleware is registered last, so it is innermost - closest to the
handler - and sees a `422` before the other three middlewares get a
chance to touch it.

Two render-time settings Laravel apps reach for at boot:

```rust
use suprnova::{Inertia, InertiaConfig};

// Rename components before they render; `None` keeps the name.
Inertia::transform_component_using(|component| {
    component.strip_prefix("Old/").map(|rest| format!("New/{rest}"))
});

// Make a component with no page file an error instead of a blank page.
let cfg = InertiaConfig::new()
    .ensure_pages_exist(true)
    .pages_dir("frontend/src/pages")              // the default
    .page_extensions(["svelte", "tsx", "jsx", "vue"]); // the default
```

The transformer runs for every response, whatever built it. With
`ensure_pages_exist` on, a render looks for `<pages_dir>/<Component>.<ext>`
and answers an error naming the component and the directory when there is no
such file - Laravel's `inertia.pages.ensure_pages_exist`. `inertia_response!`
already checks its component at compile time; this catches a name given as a
string to `InertiaResponse::new` or `Router::inertia`.

`install` also **retains the config**. Every `InertiaResponse` built
afterwards starts from it, so `.frontend(...)`, `.version(...)`,
`.default_title(...)`, `.ssr(...)` and `.encrypt_history(...)` set here
reach every page without a handler passing anything. A handler that wants
different settings for one page still overrides with `.with_config(...)`;
an app that never calls `Inertia::install` gets `InertiaConfig::default()`;
and calling `install` again replaces the retained config.

`.with_config(...)` replaces the config wholesale, `version` included.
`InertiaVersionMiddleware` still resolves the version `Inertia::install`
was given, so a config here that doesn't carry the same `.version(...)`
makes the page object advertise a version the middleware will bounce - the
client takes one extra full page load after visiting that page. Set
`.version(...)` on the override to match.

Register `SessionMiddleware` **ahead of** `Inertia::install` if you use
flash data. The version middleware re-flashes the session before bouncing
the client, so a flashed error survives the follow-up full-page GET; it
can only do that inside a session scope.

Register [`LocaleMiddleware`](localization.md) **ahead of it too**, if you
use an [error page](#error-pages). A middleware's post-`next` code runs
after everything inside it has already returned, so the error-page
middleware renders once any scope opened inside it has been popped -
which for the locale middleware means the page would get the app's
default locale instead of the visitor's. The Inertia layer reads nothing
from localization, so putting locale outside it costs nothing. The
scaffolded `bootstrap.rs` already does this. The same reasoning applies
to any middleware of yours whose request scope the error page needs to
read.

Everything you register **after** this call is covered by the error page;
anything above it is not, because a middleware that answers without
calling `next` hands its response to nothing inside it. If your
`CsrfMiddleware`, rate limiter, or auth guard has to sit above the
install, register the error-page middleware yourself between them - see
[Where the page is rendered](#where-the-page-is-rendered).

### Middleware hooks

A Laravel app changes what its `HandleInertiaRequests` middleware decides by
overriding its methods. Here those decisions are the methods of
`InertiaMiddlewareHooks`, each defaulting to the framework's behaviour, so an
implementation overrides only what it changes and installs with
`InertiaConfig::hooks`:

```rust
use suprnova::{
    DefaultInertiaHooks, HttpResponse, Inertia, InertiaConfig, InertiaMiddlewareHooks,
    InertiaRequestExt, InertiaVisit, Prop, indexmap::IndexMap,
};

struct Hooks;

impl InertiaMiddlewareHooks for Hooks {
    // Shared with every page this middleware serves, per request.
    fn share(&self, request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut props = IndexMap::new();
        props.insert("path".into(), Prop::eager(serde_json::json!(request.path())));
        props
    }

    // A handler that answers nothing means "done": 204, not a redirect back.
    fn on_empty_response(&self, _visit: &InertiaVisit, _response: HttpResponse) -> HttpResponse {
        HttpResponse::new().status(204)
    }

    // Start from the framework's 409 and add to it.
    fn on_version_change(&self, visit: &InertiaVisit, response: HttpResponse) -> HttpResponse {
        DefaultInertiaHooks
            .on_version_change(visit, response)
            .header("X-Deploy", "2026-10")
    }
}

Inertia::install(&InertiaConfig::new().hooks(Hooks))?;
```

| Hook | Laravel | Default |
|---|---|---|
| `version(request)` | `version()` | `None`: the configured version; an answer is what the client is compared against and what the page carries |
| `share(request)` | `share()` | nothing beyond the framework's `errors` |
| `share_once(request)` | `shareOnce()` | nothing; each value becomes a once prop |
| `root_view(request, config)` | `rootView()` | the config as it is; return a changed one to change this request's first-visit document |
| `url_resolver()` | `urlResolver()` | `None`: `InertiaConfig::url_resolver` |
| `on_empty_response(visit, response)` | `onEmptyResponse()` | the redirect back; a `302` it returns becomes `303` for `PUT`, `PATCH` and `DELETE` |
| `on_version_change(visit, response)` | `onVersionChange()` | the `409` with `X-Inertia-Location` |
| `on_redirect_with_fragment(visit, response)` | `onRedirectWithFragment()` | the `409` with `X-Inertia-Redirect` |

The request hooks run once per request, before the handler; the `on_*`
hooks run on an Inertia visit only and receive the response the framework
would send. `InertiaVisit` carries the request's method, path, URL and
headers, and `back_target(fallback)`, where a redirect back would go.

### The stack on a route group

`Inertia::install` puts the stack on every route, so an API route answers
with `Vary: X-Inertia` and has its `302` turned into `303` for an Inertia
`PUT`. To keep it to the groups that serve pages - Laravel's
`HandleInertiaRequests` on the `web` group only - install with
`register_globally(false)` and put the stack on those groups:

```rust
use suprnova::{Inertia, InertiaConfig, Router};

let cfg = InertiaConfig::new()
    .version(env!("CARGO_PKG_VERSION"))
    .register_globally(false);
Inertia::install(&cfg)?;           // retains the config, registers "inertia"

let router: Router = Router::new()
    .group("/", |r| r.get("/dashboard", dashboard))
    .middleware(Inertia::middleware(&cfg))  // or .middleware_named("inertia")
    .into();
let router: Router = router
    .group("/api", |r| r.get("/users", users)) // no Inertia stack here
    .into();
```

`Inertia::middleware(&cfg)` is the whole stack as one middleware, in the
order `install` registers it globally, with the error page when `cfg` names
one and with `cfg`'s hooks. With `register_globally(false)`, `install`
registers it as the named middleware `inertia` instead of globally, so a group
can name it. `SessionMiddleware` stays global, outside the group's stack, as
the Inertia layer reads the session it opens.

Skip the call only if you genuinely don't want one of these middlewares
(rare; each of them closes a real failure mode - cache poisoning across
the two representations of a URL, silent stale-bundle,
form-replay-on-redirect,
and a validation `422` dead-ending in the client's error modal instead of
reaching `form.errors`).

## Error pages

An Inertia visit that gets back a non-2xx from the framework does not
show an error page - it shows a crash screen:

```
All Inertia requests must receive a valid Inertia response, however a
plain JSON response was received.
```

The client checks one thing before it will render anything: an
`X-Inertia: true` header on the response. A `403` from an
[authorization](authorization.md) check or an RBAC permission
middleware, a `404` for an unrouted path, a `429` from the
[rate limiter](rate-limiting.md), a `500` from a
[failing handler](errors.md) - all of them carry the framework's JSON
error body and no such header, so the client hands them to its modal. A user with the wrong role clicks a nav
link and the app appears to break.

Name a page component and the framework renders those responses through
it instead, keeping the status code:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .version(env!("CARGO_PKG_VERSION"))
            .error_page("Error"),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

`"Error"` is resolved exactly like any other page name, so
`frontend/src/pages/Error.svelte` (or `.tsx`, or `.vue`) is all it takes.
**The three starters ship one and set `.error_page("Error")` already** -
a new project is covered without doing anything.

`.error_page(...)` is the default error callback: a fixed rule, described
below, that decides each error response for you. To decide each error
yourself, see [Deciding each error yourself](#deciding-each-error-yourself).

### Where the page is rendered

`Inertia::install` registers `InertiaErrorPageMiddleware` **innermost** of
the Inertia layer, so it sees the response the handler and the route
middleware actually produced. Everything you register *after* that call is
covered too - which is why the scaffold puts `CsrfMiddleware` below it.

Anything registered **above** the call is not covered. A middleware that
answers without calling `next` hands its response to nothing registered
inside it, so its rejection never reaches the Inertia layer at all. The
case that bites is a lapsed session posting a form: `CsrfMiddleware`
answers `419` with `{"message":"CSRF token mismatch."}`, and if it sits
above `Inertia::install` the user gets the crash modal on the one flow
they are most likely to hit. An outer rate limiter's `429` and an auth
guard's `401` behave the same way.

Register the middleware yourself when that is your shape, outside the
middleware whose rejections it should cover. This worked in 1.3.6 as a
side effect - the type was public and global registration is idempotent
per type, so an earlier registration kept its place - but nothing said so.
It is a documented contract from 1.3.7: `install` checks for your
registration, logs at `debug`, and skips its own.

```rust
use suprnova::{
    global_middleware, CsrfMiddleware, Inertia, InertiaConfig,
    InertiaErrorPageMiddleware, LocaleMiddleware, SessionConfig, SessionMiddleware,
};

pub fn register_http_stack() -> Result<(), suprnova::FrameworkError> {
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
    global_middleware!(LocaleMiddleware::from_env()?);

    // Outside CSRF, so it sees the 419 that never reaches the layer below.
    global_middleware!(InertiaErrorPageMiddleware::new("Error"));
    global_middleware!(CsrfMiddleware::new());

    Inertia::install(&InertiaConfig::new().error_page("Error"))
}
```

`Inertia::install` sees the registration, skips its own, and says so at
`debug`. The position you chose is the one that stands, and so is the
component you named - that instance is the one in the chain. You name the
page **once**, at your own registration, which makes `.error_page(...)` on
the config optional here: keep it or drop it, nothing else reads it. It is
still what makes `install` register a middleware for an app that does not
place one itself.

Two ordering rules come with placing it yourself.

**After `SessionMiddleware` and [`LocaleMiddleware`](localization.md).**
The page carries your shared props - `auth.user`, flash, the locale share -
and it is built on the way *out*, after every middleware registered inside
it has returned and popped whatever request scope it opened. Registered
above those two, every error page loses the visitor's session and renders
in the app's default locale rather than theirs. The same holds for any
request-scoped middleware of your own whose state the error page's shared
props read.

**Before the middleware whose rejections it should cover**, and no
further out than that. Every response that passes through it is one more
body it has to classify, and a middleware outside it can still answer
before it runs.

If you register nothing yourself, `Inertia::install` does all of this for
you - and the scaffolded `bootstrap.rs` already has `SessionMiddleware`
and `LocaleMiddleware` above the call and `CsrfMiddleware` below it.

### What the page receives

| Prop | Type | Always present | What it is |
|---|---|---|---|
| `status` | `number` | yes | The original HTTP status - `403`, `404`, `500`. |
| `message` | `string` | yes | The error body's `message`, or the status's reason phrase when it carried none. Already sanitized: a `5xx` reads `"Internal Server Error"`, never the underlying error - and that holds under `APP_DEBUG=true` too. The dev-only `debug_message` field the JSON path adds there is deliberately not read, so the raw error stays in the log and the JSON response and never renders into a page. |
| `request_id` | `string` | no | Present only when the error body carried one. The same id the structured log records, so the page can show a reference the operator can search. |

```svelte
<script lang="ts">
  interface ErrorProps {
    status: number
    message: string
    request_id?: string
  }

  let { status, message, request_id }: ErrorProps = $props()
</script>

<h1>{status}</h1>
<p>{message}</p>
{#if request_id}<p>Reference: {request_id}</p>{/if}
```

Declare the props in the component rather than importing them from
`types/inertia-props.ts`: [`suprnova generate-types`](frontend-typescript-types.md) rewrites
that file from your own `#[derive(InertiaProps)]` structs, and these
props come from the framework.

### What survives the swap

The status code is kept, and so is every header the original response
set, **except** two groups.

**What described the body being replaced.** Every `Content-*` field
(`Content-Length` on a page four times the size of the JSON it replaced
is a framing bug) and `Transfer-Encoding`.
`Content-Security-Policy` is carved out of that rule by name - it shares
the prefix by historical accident and is response policy, not
representation metadata.

**What governed how that body could be stored.** `Cache-Control`,
`Expires`, `Age`, `ETag`, `Last-Modified`. The page carries your shared
props - `auth.user`, flash, the locale share - where the error body it
replaced was the same for everyone, so it must never inherit permission
to be stored by a shared cache and handed to a different visitor, nor
validators that belong to an entity it is not. The page sets
`Cache-Control: no-cache, private` for itself instead, the same default
Laravel gives a session-bearing response.

Everything else carries: `Retry-After` on a `429` still tells the client
when to come back, `WWW-Authenticate` on a `401` still carries the
challenge, and `Vary`, `Set-Cookie`, and your request-id header all
arrive intact. The rule is stated as what gets dropped rather than what
gets kept, so a header the framework has never heard of survives instead
of silently disappearing.

Both audiences are covered. An Inertia XHR visit gets the JSON page
object with `X-Inertia: true`; a hard navigation - someone pasting
`/admin/articles` into the address bar - gets the full HTML shell, the
same one a first load of any page gets. So the error page works whether
the user arrived through the SPA or not.

With debug mode on, a `5xx` the framework built from an error reaches
both audiences as the development error page instead: the error, the
stack frames, and the request, as one HTML document. See
[Error Model](error-model.md#the-development-error-page). A `4xx`, and
every response with debug off, still renders your component.

### What it never touches

These are the default callback's choices. A callback of your own decides
for itself - see [Deciding each error yourself](#deciding-each-error-yourself).
The default callback only stands in where nobody else has an answer. It
leaves alone:

- **Validation `422`s.** `InertiaValidationRedirectMiddleware` owns
  those - see [Validation failures](#validation-failures). A `422` that
  survives that middleware (no `errors` object, or a Precognition
  dry-run) keeps its body too.
- **Anything carrying `X-Inertia-Location` or `X-Inertia-Redirect`.**
  The `409` version bounce, the `redirect_to` form of the RBAC
  middlewares, `Inertia::location`, and a redirect with a `#fragment`.
  The client acts on the header, not the body. No callback is handed
  these.
- **Redirects.** Only `400`-`599` is in scope.
- **API clients.** A request whose `Accept` prefers `application/json`
  over `text/html` keeps the JSON contract it has always had. `curl`'s
  `*/*` counts as no preference, so it keeps JSON too. Only an Inertia
  visit or a browser navigation gets a page.
- **Responses that already are Inertia pages.** A handler that rendered
  its own page and gave it a `410` keeps its own component.
- **Bodies that are not the framework's error shape.** Your own HTML
  error page, plain text that is not the router's own `404 Not Found`, or
  a JSON envelope keyed differently - none of those is overruled.
- **Everything, when neither `error_page` nor a callback is set.** The
  middleware `Inertia::install` registers hands every request on, so an
  app that has not opted in gets exactly the responses it got before.

### Which bodies get rewritten

The gate is the **shape of the body**, not who wrote it. At a `400`-`599`
status, exactly three shapes are replaced:

- an empty body;
- a JSON object whose `message` is a string - the framework's own error
  envelope, and anything else shaped like it;
- the router's fixed `404 Not Found` plain-text body.

Everything else passes through. That means a `401` a middleware of yours
answers with `HttpResponse::json(json!({ "message": "Unauthenticated." }))`
**does** become the error page - which is the point, since that is exactly
the response the client would otherwise modal - and it means only
`message` and `request_id` survive into the props. An envelope carrying
`errors`, `code`, or anything else loses those fields when it becomes a
page.

If a middleware of yours must keep its own JSON body on an error status,
give it a shape the gate does not match - key the human-readable text as
something other than `message` - or set `X-Inertia: true` on the response
yourself, which marks it as already being an Inertia response and takes
it out of scope. Both are one line at the point that builds the response.

A handler that **panics** is covered too. The middleware runs the rest
of the chain inside it under the panic boundary's rule, so the panic
becomes the same sanitized `500` the boundary sends, with the panic's
error report, and the page renders for it. A panic in a middleware
registered *outside* it still reaches the boundary around the whole
chain and its JSON `500`.

If the page itself fails to render - the component cannot be resolved,
SSR is down, a shared prop errors - the framework logs a `warn` with the
request id and returns the original error response. A broken error page
never masks the error it was rendering.

### Deciding each error yourself

`Inertia::handle_exceptions_using` installs a callback that decides every
error response the framework renders, the way Laravel's
`Inertia::handleExceptionsUsing` does. The callback receives an
`InertiaErrorResponse` and returns it with a decision, or `None` to send
the response the framework built:

```rust
use serde_json::json;
use suprnova::Inertia;

pub fn register_error_pages() {
    Inertia::handle_exceptions_using(|error| match error.status() {
        403 | 404 | 500 | 503 => {
            let status = error.status();
            Some(
                error
                    .render("Error", json!({ "status": status }))
                    .with_shared_data(),
            )
        }
        _ => None,
    });
}
```

`InertiaErrorResponse` shows what went wrong and who asked:

- `status()` - the status of the response the framework would send.
- `error()` - the [error report](error-model.md): the error and its
  source chain, or a panic's message and location. A response the
  framework built without an error behind it, such as the router's `404`
  for a path no route matches, reports its message or the status's reason
  phrase.
- `request()` and `method()` - the request as it arrived: its method,
  path, query, and headers.
- `response()` - the response the framework would send.

It offers three decisions:

- `render(component, props)` renders an Inertia page in place of the
  response and keeps its status. `props` is anything that serializes to a
  JSON object, such as `json!({ ... })`. The page gets the same treatment
  as the default callback's page: the root template the request's chooser
  picks, `Cache-Control: no-cache, private`, and the original headers
  minus the ones described in [What survives the swap](#what-survives-the-swap).
- `with_shared_data()` adds the shared props to that page: everything
  `Inertia::share` and `App::inertia_share` registered, the shared
  providers, the `InertiaSharedData` provider, and the `share` and
  `share_once` [middleware hooks](#middleware-hooks).
  Without it, the page carries only the props you gave it.
- `respond_with(response)` sends any other response instead.

The callback sees every error response the framework renders, for every
request type - an API client's included, so check
`error.request().header("Accept")` or `error.request().is_inertia()`
before you render a page for one. That covers a handler's or a
middleware's `Err(...)`, a handler that panics, the router's `404`, and
a middleware's own `{"message": ...}` answer. A response a handler built
itself with an error status, such as its own HTML `404` page, is that
handler's answer, and the callback doesn't see it. Neither are Inertia
protocol responses: pages, and anything carrying `X-Inertia-Location` or
`X-Inertia-Redirect`.

Responses from outside the Inertia stack reach the callback too, at the
server. A middleware registered before `Inertia::install` that answers
without calling `next` - `CsrfMiddleware`'s `419`, `TimeoutMiddleware`'s
`503`, an outer rate limiter's `429` - never hands its response to the
error-response middleware, and neither does a panic that only the
server's boundary catches. The server runs the same decision after the
whole stack for every error response that nothing inside the stack
decided, so your callback, or the default callback, sees those as well,
once each. The callback gets the same request, error, and response there.
`with_shared_data()` still adds the shared registry and the shared
providers, but every request scope a middleware opened has closed by
then: the page has no session data, renders in the default locale, and
gets none of the `share` and `share_once` hooks. To keep those on such a
page, register `InertiaErrorPageMiddleware` yourself, outside the
middleware whose answers it should cover - see
[Where the page is rendered](#where-the-page-is-rendered).

Validation failures never reach the callback either. A `422` whose body
carries an `errors` object is a validation result, and
`InertiaValidationRedirectMiddleware` owns it: an Inertia visit gets the
redirect back to the form with the errors flashed, and an API client or a
Precognition dry run gets the `422` with its errors - see
[Validation failures](#validation-failures).

With debug mode on, `response()` for a `5xx` that carries an error
report, sent to a browser or an Inertia visit, is the
[development error page](error-model.md#the-development-error-page).
Return `None` to keep it, as the default callback does. A page you render
instead is what the client gets, so check `Config::is_debug()` first if
you want the development page while you work.

`.error_page("Error")` is the default callback. An app that sets both
gets its own callback, and `None` from it keeps the framework's response
rather than falling back to the error page. A later call to
`handle_exceptions_using` replaces the callback. The callback lives on
the active container, so one installed under `TestContainer::fake()`
decides only that test's requests. `Inertia::install` registers the
middleware whatever the config says, so you can install the callback
before or after it.

### Why Suprnova diverges

Laravel puts this in the exception handler: you edit
`bootstrap/app.php`, match on the status yourself, and call
`Inertia::render('Error', ['status' => $response->getStatusCode()])`
with `$response->setStatusCode(...)` to put the code back. That is
flexible, and it is also a piece of framework plumbing every project
rewrites by hand, usually after seeing the modal in production first.

Here the common case is one config line, because the decision is the
same for almost every app: an Inertia visit or a browser navigation gets
a page, an API client gets JSON, and everything another contract owns is
left alone. That line installs the default callback. When the fixed rule
doesn't fit, `Inertia::handle_exceptions_using` gives you Laravel's
per-error decision - see
[Deciding each error yourself](#deciding-each-error-yourself).

## Server-driven `<head>` elements

Inertia 3.5 added a client option for letting the server decide what goes in
`<head>` - useful when meta tags depend on the record you just loaded, and you
don't want the title and OG tags to live in two places.

This needs no framework support. The client reads the elements from an
**ordinary prop**, so any handler can supply them:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
}
```

Opt in on the client:

```js
createInertiaApp({
  serverHead: true,        // reads the `head` prop
  // serverHead: 'meta',   // or read a differently-named prop
  // serverHead: (page) => [...],  // or compute from the whole page
})
```

Each string is an HTML element. The client stamps a `data-inertia` attribute on
anything that lacks one so it can diff head elements across navigations; supply
your own `data-inertia="og-title"` when you want stable identity rather than
positional matching.

Escape anything interpolated from user data - these strings are injected as
HTML, so the usual rules apply.

## The root template

A first visit is a whole HTML document: the page data, the mount element, the
Vite tags, and whatever else your application wants in the first load - a
favicon, fonts, meta tags a link preview reads, a `<noscript>`, attributes on
`<html>` and `<body>`. In Laravel that document is `app.blade.php`. In
Suprnova it is an Askama template under `templates/`, which you declare with
`#[inertia_root]` and install on the config:

```rust
use suprnova::{Frontend, Inertia, InertiaConfig, InertiaRootTemplate};

/// The document every Inertia first visit renders into: `templates/app.html`.
#[suprnova::inertia_root(path = "app.html")]
pub struct AppDocument;

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .frontend(Frontend::Svelte)
            .default_title("My App")
            .root_template(InertiaRootTemplate::of::<AppDocument>()),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

```html
<!DOCTYPE html>
<html lang="{{ lang }}" class="h-full">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <link rel="icon" href="/favicon.ico" />
    {{ title }}
    {{ head }}
    {% if !ssr %}
    <meta name="description" content="My App" />
    {% endif %}
  </head>
  <body class="antialiased">
    <noscript>My App needs JavaScript.</noscript>
    {{ body }}
  </body>
</html>
```

`suprnova new` writes this file as `templates/app.html` and declares it as
`AppDocument` in `src/bootstrap.rs`. Without a root template, the first visit
is the document the framework writes itself, byte for byte what it has always
been.

The framework hands the template these parts:

- `title`: the `<title>` element, from `.title(...)` on the response or
  `.default_title(...)` on the config. It is empty when the
  [SSR](#ssr) head carries a `<title>` of its own, because a document shows
  only its first title. A template that writes its own `<title>` leaves this
  part out.
- `head`: the `csrf-token` meta tag, the SSR head when the SSR server rendered
  the page, and the Vite tags, in that order.
- `body`: the page data element and the mount element, or the SSR body, which
  carries both.
- `lang`: the document's language, the locale in effect for the request.
- `csrf_token`: the session's CSRF token, for a template that places it
  itself. It is empty outside a session.
- `nonce`: the request's CSP nonce, for the template's own inline scripts. It
  is `None`: no nonce policy supplies one.
- `ssr`: whether the SSR server rendered this response. A template places
  fallback head content under `{% if !ssr %}`, for the pages whose own `Head`
  did not run on the server.
- `view`: the response's view data, described below.

`title`, `head`, and `body` are markup: placed bare, they are written as they
are, not escaped as text. `lang`, `csrf_token`, `nonce`, and view data values
are plain values, and Askama escapes them like any other. Askama checks the
template at compile time, so a template that names anything other than these
parts fails the build. A template that fails while it renders, such as one
whose custom filter returns an error, makes the response an error that names
the template; no part of the document is sent.

The parts write themselves into the template's output as it renders. The page
JSON is serialized straight into that output, so a first visit through a root
template allocates what the framework's own document does, and a root
template adds no size limit of its own.

### Choose the template per request

To choose a different document for some requests, give the config a function
of the request instead of one template. It reads the path, the query, and the
headers through `InertiaRequestExt`, like Laravel's `rootView(Request)`:

```rust
use suprnova::{InertiaConfig, InertiaRootTemplate};

#[suprnova::inertia_root(path = "admin.html")]
pub struct AdminDocument;

let cfg = InertiaConfig::new().root_template_with(|req| {
    if req.path().starts_with("/admin") {
        InertiaRootTemplate::of::<AdminDocument>()
    } else {
        InertiaRootTemplate::of::<AppDocument>()
    }
});
```

`InertiaRootTemplate::framework()` chooses the framework's own document. The
[error page](#error-pages) goes through the same function, with the request
as it arrived, so a `404` or a `403` under `/admin` keeps the admin document.

### View data

`.with_view_data(key, value)` hands the root template a value for one response
and keeps it out of the page props: an Inertia visit's JSON and the first
visit's page data never include it. It is Laravel's `withViewData`, for what
the first-load HTML must carry without running JavaScript, such as the meta
tags a link preview reads:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    InertiaResponse::new("Posts/Show")
        .with_view_data("description", post.title.clone())
        .with("post", post)
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}
```

```html
{% if let Some(description) = view.get("description") %}
<meta property="og:description" content="{{ description }}" />
{% endif %}
```

A value is anything serializable. Placed with `{{ value }}`, a string
displays as itself, and any other value, such as a number or an array,
displays as its JSON; the template escapes either like any other value.
`.try_with_view_data(key, value)` returns an error naming the key when the
value fails to serialize, where `.with_view_data` panics as `.with` does.

An error page carries no view data: the handler that would have set it failed
or never ran. The framework's own document places none.

Blade's view data becomes template variables. Here the template reads it
through `view.get`, so the template's names stay the fixed set of parts that
Askama checks at compile time.

### The mount id

The page data element's `data-page` attribute and the mount element's `id`
are `app` unless `.mount_id(...)` on the config names another. Set it to the
`id` your frontend passes to `createInertiaApp`, and under SSR to
`createServer`, since the worker writes the SSR body's elements itself:

```rust
let cfg = InertiaConfig::new().mount_id("root");
```

## SSR

Suprnova talks to an out-of-process SSR worker - typically the
`@inertiajs/{svelte,react,vue}/server` `createServer()` bundle run
under Node, Bun or Deno - over HTTP. SSR is on by default, as in Laravel,
and gated by bundle detection: a first visit goes to the worker only when
an SSR bundle exists, so an application without one renders on the client
and never contacts the worker. Configure it on the config you hand to
[`Inertia::install`](#bootstrap-inertia-install) - that config is what
every response starts from, so there is nothing to plumb through your
handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL, the default
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR is a property of the config: on for every response built from the
installed config, and off for a response that overrides with a
`.with_config(...)` that calls `.ssr_disabled()`. When it runs, the
framework posts the page object to `<url>/render` and inlines
`{ head, body }` in the HTML shell. A worker head that carries its own
`<title>` - which is every page using Inertia's `Head` component -
**replaces** the shell's title rather than joining it, and that means both
`.default_title(...)` on the config and a per-response `.title(...)`: a
document with two titles shows the first one, so the shell's would win over
the page's real one in the tab, in search results, and in every link
preview. With SSR on, set the title in `Head` rather than on the response. A
head with no title leaves the shell's title exactly where it was. A worker
answer with nothing to inline - empty JSON, `null`, `false`, anything but an
object, an object without a `body`, or a body that is not JSON at all -
renders on the client, and is not a failure.

### Bundle detection

While `.ssr_ensure_bundle_exists(true)` is set, which is the default, a first
visit is dispatched only when a bundle exists at the path
`.ssr_bundle_path(...)` names or, failing that, at one of the conventional
paths under the working directory, the list Laravel's `BundleDetector`
checks (`CONVENTIONAL_BUNDLE_PATHS`):

- `frontend/bootstrap/ssr/ssr.js`, where a scaffolded project's
  `vite build --ssr` writes it
- `frontend/bootstrap/ssr/app.js`
- `frontend/bootstrap/ssr/ssr.mjs`
- `frontend/bootstrap/ssr/app.mjs`
- `public/js/ssr.js`
- `public/js/app.js`

With no bundle the visit renders on the client at once, without a request to
the worker, an error or a log line, so it never pays `ssr_timeout` on a
worker that was never started. `detect_ssr_bundle(&config.ssr)` runs the
same search. Turn the check off when the worker's bundle lives where this
process cannot see it, such as a separate container, or in a test that uses a
stand-in worker:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://ssr.internal:13714")
        .ssr_ensure_bundle_exists(false),
)?;
```

To keep every first visit on the client, call `.ssr_disabled()`.

### Hot mode in development

While the Vite dev server runs, a first visit in development goes to it at
`/__inertia_ssr` instead of the worker, and the bundle check is skipped:
the dev server renders the page from source, so you need neither a bundle
nor a worker process while you work.

The hot file says the dev server runs. `suprnova serve` writes the dev
server's URL to `public/hot` when it starts Vite, and removes the file when
Vite stops or `serve` exits, the file Laravel's Vite plugin writes. A visit
goes hot while that file exists, or whenever you set `.ssr_hot_url(...)`.
The address is the `.ssr_hot_url(...)` URL, else the file's content, else
the `.vite_dev_server(...)` URL when the file is empty.
`.ssr_hot_file(...)` names another file. Without the file the visit takes
the worker path with its bundle check, whatever listens at the dev server's
port. Production never goes hot.

A dev server without the Inertia Vite plugin answers `/__inertia_ssr` with
a `404`. That visit renders on the client quietly: no `SsrRenderFailed`, no
`on_ssr_error` call, and no error under `ssr_throw_on_error`. Any other
error status from the dev server is a failure like the worker's.

### Worker failures and `SsrRenderFailed`

On a worker error, a timeout or an unreachable worker the response falls
back to the client (an empty `<div id="app">` the client mounts on) and the
`on_ssr_error(...)` hook fires. Every such failure also dispatches the
`SsrRenderFailed` event first. An Inertia 3 worker answers a failed render
with an error status and a JSON body (`error`, `type`, `hint`, `browserApi`,
`stack`, `sourceLocation`), and the event carries those fields with the
page's `component` and `url`. `error_type` is an `SsrErrorType`:
`BrowserApi`, `ComponentResolution`, `Render`, `Connection` for a worker that
could not be reached, or `Unknown`.

```rust
use std::sync::Arc;
use suprnova::{EventFacade, FrameworkError, Listener, SsrRenderFailed, async_trait};

struct ReportSsrFailure;

#[async_trait]
impl Listener<SsrRenderFailed> for ReportSsrFailure {
    async fn handle(&self, failed: &SsrRenderFailed) -> Result<(), FrameworkError> {
        tracing::warn!(
            component = %failed.component,
            url = %failed.url,
            kind = %failed.error_type,
            location = ?failed.source_location,
            hint = ?failed.hint,
            "SSR failed: {}",
            failed.error,
        );
        Ok(())
    }
}

// In `bootstrap::register`:
EventFacade::listen::<SsrRenderFailed, _>(Arc::new(ReportSsrFailure)).await;
```

Set `ssr_throw_on_error(true)` in CI to make those failures hard 500s
instead. The error names the component and, when the worker gave one, the
source location, as Laravel's `SsrException` does:
`SSR render failed for component [Dashboard]: window is not defined at
resources/js/Pages/Dashboard.vue:12:5`.

### Workers over HTTPS

A worker URL may be `https`. The client speaks TLS through rustls and
verifies the worker's certificate against the platform's trust store, so a
worker on another host or behind a TLS proxy needs no other setting.

### Run-time controls

`.ssr_exclude(pattern)` follows Laravel's `ExcludesPaths` rules, so a
pattern copied from a Laravel app excludes the same requests: slashes at
either end are ignored, `*` matches any characters including `/`, and each
pattern is tried against the path and the full URL. `admin/*` keeps
`/admin/users` and `/admin/users/edit` on the client, not `/adminx`.

The `Inertia` facade changes SSR at run time, as Laravel's does:

```rust
use suprnova::{Inertia, InertiaRequestExt};

// A switch for every request; `false` turns SSR on even where the
// configuration has it off (the worker URL still comes from the config).
Inertia::disable_ssr(true);

// Or decide per request. The answer replaces the configuration's switch.
Inertia::disable_ssr_if(|request: &dyn InertiaRequestExt| {
    request.path().starts_with("/admin")
});

// More exclusions, with the same rules as `ssr_exclude`.
Inertia::without_ssr(["admin/*", "https://app.test/reports*"]);

// Adjust the request sent to the worker: headers, a token, the timeout.
Inertia::configure_ssr_request_using(|request| {
    request
        .bearer_token(std::env::var("SSR_TOKEN").unwrap_or_default())
        .header("X-Tenant", "acme")
});
```

`App::disable_ssr_for_request()` still turns SSR off for the one request it
runs in, whatever the switch says.

### Your own gateway

The SSR call is a driver: the `SsrGateway` trait, with `HttpGateway` as the
default. Bind your own in the container and every first visit dispatches
through it:

```rust
use std::sync::Arc;
use serde_json::Value;
use suprnova::{
    App, FrameworkError, InertiaRequestExt, SsrConfig, SsrGateway, SsrResponse, async_trait,
};

struct EdgeGateway;

#[async_trait]
impl SsrGateway for EdgeGateway {
    async fn dispatch(
        &self,
        config: &SsrConfig,
        request: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        // Render `page` somewhere else; `Ok(None)` renders on the client.
        let _ = (config, request, page);
        Ok(None)
    }
}

App::bind::<dyn SsrGateway>(Arc::new(EdgeGateway));
```

`is_healthy`, `disable`, `except` and `configure_request_using` have
defaults a gateway overrides when it supports them. The facade calls in
[Run-time controls](#run-time-controls) act on the bound gateway; one that
lacks the capability logs a warning and ignores the call.

`Inertia::ssr_is_healthy().await` returns the gateway's health check. The HTTP
gateway sends `GET {url}/health`, through the request configurator and within
`ssr_timeout`, and answers `Some(true)` for a 2xx status and `Some(false)`
for any other status or no answer. `None` means the gateway has no health
check.

### Build and run the worker

`suprnova new` scaffolds `frontend/src/ssr.{ts,tsx}` and a `build:ssr`
npm script for every starter. Build it, then boot the worker:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` asks the SSR gateway's health check whether the
worker is answering: the HTTP gateway sends `GET /health`, which every
`createServer()` bundle answers without any extra code. `suprnova ssr:stop`
stops the worker. The application binary has the same three commands,
reading the configuration you installed, as in
`cargo run --bin <app> -- ssr:start`; see
[Console](console.md#ssr-commands).

### Why Suprnova diverges

Laravel's Vite plugin writes `public/hot` while the dev server runs. A
Suprnova project's Vite configuration carries no such plugin, so
`suprnova serve`, which starts Vite, writes and removes the file instead.
Run Vite on its own (`npm run dev`) and the backend does not go hot unless
you set `.ssr_hot_url(...)`. A `404` from the hot endpoint renders on the
client quietly, where Laravel dispatches `SsrRenderFailed`: the starter
kits do not ship the Inertia Vite plugin, so every first visit would
report a failure while you develop.

Laravel sets no SSR timeout of its own and inherits its HTTP client's
30 seconds. Suprnova keeps 5 seconds (`ssr_timeout`): a hung worker would
otherwise hold every first visit for 30 seconds before the client-rendered
fallback.

## Configuration

Inertia behaviour is configured programmatically via `InertiaConfig`, and
the config you hand to [`Inertia::install`](#bootstrap-inertia-install) is
the one every response starts from. `SUPRNOVA_FRONTEND` (`svelte` /
`react` / `vue`) only supplies the default entry-point filename and
page-component extensions when the config doesn't say - an explicit
`.frontend(Frontend::React)` on the installed config wins, and is what
`suprnova new --frontend react` scaffolds. `ASSET_URL` only supplies the
default `asset_url` the asset version hashes, and `.asset_url(...)` wins
over it (see [Version detection](#version-detection)). Everything else is
builder-shaped:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .with_all_errors(false)                   // one message per field, or all
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
```

A root-relative `assets_base_url` such as `/assets` gets the public root in
front of it in the Vite tags, so behind a proxy that serves the application
under `/billing` they load `/billing/assets/...`; an absolute or `//host`
base, such as a CDN's, is left as it is. The page `url` carries the root the
same way. See [Serving under a path prefix](deployment.md#serving-under-a-path-prefix).

Frontend-specific defaults:

| Frontend | Default entry point | Page extensions |
|---|---|---|
| Svelte (default) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

Two attributes of the HTML shell are worth calling out.

`<title>` comes from `.title(...)` on the response, or from
`.default_title(...)` when the response set none. Under [SSR](#ssr) the
page's own head wins over **both**: a worker head carrying a `<title>` is
the document's only one, and the shell leaves its title out entirely.

`<html lang="...">` is the one attribute the framework's own document does
not let you set, because the right value is already known - it is the
locale in effect for the request, what `LocaleMiddleware` detected or the
configured `APP_LOCALE` when nothing did. A [root template](#the-root-template)
places the same value as its `lang` part. See [Localization](localization.md); a screen reader takes its voice
from that attribute and a search engine reads it as the page's language,
so an app serving more than one language no longer has to rewrite the
finished document to correct it.

### The `url` field

`page.url` is the path **and** query string of the request
(`/users?page=2&sort=name`). The client writes it into `history.state`, so
it is what back/forward navigation and `router.reload()` replay - drop the
query and every paginated or filtered page silently resets to page one.

The query is normalised the way Laravel's `fullUrl()` normalises it
through Symfony, so the client compares the same page URLs it would get
from Laravel: the pairs are parsed as PHP parses a query string, sorted by
key, and re-encoded per RFC 3986. `/s?b=2&a=1%20x` becomes
`/s?a=1%20x&b=2`, a `+` becomes `%20`, reserved characters stay
percent-encoded (`a=%2Fx` is not turned into `a=/x`), a key without a
value gains an `=` (`?flag` becomes `?flag=`), a repeated key keeps its
last value, and bracketed keys are written with their indexes
(`tags[]=a&tags[]=b` becomes `tags%5B0%5D=a&tags%5B1%5D=b`). A query
already in that form is left as it is.
`InertiaVersionMiddleware` derives its `X-Inertia-Location` from the
request's path and query too, made absolute with the request's scheme and
host, so by default a 409 asset-version bounce lands the browser on
exactly the URL the page object named.

Override the derivation with `url_resolver` when the URL the client should
record differs from the one that arrived - a locale prefix the SPA doesn't
route on, or a path a reverse proxy rewrote:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

The resolver reads the request through `InertiaRequestExt`, and applies to
every response built from the config you pass to
[`Inertia::install`](#bootstrap-inertia-install) - the usual place for a
resolver that should apply app-wide. Override it for a single response
with `InertiaResponse::with_config(cfg)`. A resolver changes `page.url`
only. The 409 bounce keeps naming the URL that actually arrived - that is
the URL the browser has to fetch - so with a resolver in place the two
deliberately differ.

The Vite manifest at `manifest_path` is loaded lazily on first request
and cached for the process lifetime - every response built from the
installed config shares that one cache, so the file is read and parsed
once. When it's missing, production asset tags fall back to a hardcoded
legacy path and a `tracing::warn!` fires so the gap surfaces in logs.

### Big integers

JavaScript represents integers exactly only up to 9007199254740991
(2^53 - 1), so a 64-bit database id beyond it reaches the browser rounded.
Turn on `preserve_big_integers` to send every integer outside plus or minus
that bound, in props and flash and at any depth, as a marker the Inertia
client restores as a `BigInt`:

```rust
use suprnova::{InertiaConfig, InertiaResponse};

// Every response built from this config.
let cfg = InertiaConfig::new().preserve_big_integers(true);

// One response, whatever the config says.
let page = InertiaResponse::new("Orders/Show")
    .with("id", 9_007_199_254_740_993_u64)
    .preserve_big_integers(true);
```

That `id` arrives as `{"$bigint": "9007199254740993"}`, and the page object
carries `preserveBigIntegers: true` so the client knows to restore it.
Integers inside the safe range, floats and object keys are left alone.
With the setting off, the default, nothing is wrapped and the flag is
absent. This is Laravel's `inertia.preserve_big_integers` setting and
`Response::preserveBigIntegers`.

### Why Suprnova diverges

Laravel's Inertia adapter has a single global "shared data"
registry plus a per-request `Inertia::share($k, $v)` call. PHP's
request-per-process model makes this safe: a fresh process per request
means no leakage between concurrent visitors.

Rust's process model is the opposite - one process serves many
concurrent requests across many threads. So the registry lives on
the [container](container.md) (task-local → thread-local → global),
not in process-global statics. `App::inertia_share*` writes to the
active container's `InertiaRegistry`, which gives tests using
`TestContainer::fake()` clean isolation without having to unregister
anything. Same surface as Laravel; different machinery underneath
because the runtime is different.

Other Rust-shaped choices worth flagging:

- **Lazy-prop resolvers run concurrently**, capped by
  `max_concurrent_resolvers` (default 16). A page with twelve lazy
  props issues twelve parallel queries inside one Tokio task - that's
  what we built the framework on top of Tokio for. Tune the cap if a
  page has many lazy props each hitting an external service.
- **The compile-time component check** isn't a Laravel feature at all,
  because PHP can't see your frontend files at compile time. Suprnova
  does, so a typo in `inertia_response!("Dashbaord", …)` fails the
  build with a "did you mean Dashboard?" suggestion instead of
  surfacing as a runtime "component not found" later.
- **`Inertia::clearHistory()` has a response-local form too.**
  `App::clear_history()` and `Inertia::clear_history()` are Laravel's call:
  the flag lives in the session until a page emits it. `.clear_history()` on
  the builder marks a single response with no session dependency, for the
  case where the response you are returning is the page that should clear.
- **`.lazy()` isn't Laravel's `Inertia::lazy()`.** Laravel's method is
  deprecated and behaves like `optional()` - `LazyProp` is a straight
  alias for `OptionalProp`, skipped entirely on the initial visit
  (`ResponseFactory.php:174-181`). Suprnova's `.lazy()` is the
  plain-closure convention Laravel itself uses for a callable prop with
  no wrapper at all - included whenever partial-reload filtering lets the
  key through, standard visits included. Reach for `.optional()` for the
  initial-visit-skipped behavior the name "lazy" suggests if you're
  coming from Laravel.

## Next

- [Page Components](frontend-pages.md) - how the frontend resolves a
  component name to a Svelte / React / Vue module
- [TypeScript Types](frontend-typescript-types.md) - `suprnova generate-types`
  emits TS definitions from your `#[derive(InertiaProps)]` structs
- [Data Objects](data.md) - `#[derive(Data)]` for DTOs with per-field
  include/allowlist gating that composes with partial reloads
- [Error Model](error-model.md) - how `Response`, the panic boundary,
  and `FrameworkError` thread through Inertia responses
- [Container](container.md) - the lookup model behind
  `App::inertia_share*` and `InertiaSharedData`
