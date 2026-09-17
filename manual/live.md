# Live

Suprnova Live is the framework's server-driven interaction engine. A Live
component is a Rust struct whose state lives on the server, whose view is an
Askama template, and whose actions run over a signed protocol from a small
browser runtime that morphs the re-rendered HTML in place. There is no
client-side state model to keep in sync, no build tool to install to use the
shipped runtime, and no inline JavaScript in your documents.

This chapter covers the application-facing surface: authoring a component,
registering it, serving documents and islands, the security boundaries every
Live request crosses, uploads, asynchronous updates, assets, testing,
diagnostics, and recovery. Everything here uses only `suprnova::live` and
`suprnova::view`.

## Quick start

A project created by `suprnova new` is Live-ready: it ships `src/live/mod.rs`
with an empty component registry and a `routes()` function, its bootstrap
binds the registry, and `cmd/main.rs` installs the routes. Scaffold a
component, then check it:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` writes `src/live/counter.rs` and `templates/live/counter.html`,
registers the component in `src/live/mod.rs`, and prints the next steps.
`live:check` builds your application and proves every registered view against
the integrated checker.

## Authoring a component

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` is the registered component name. Use a dotted, kebab-case name
  such as `app.counter`; the CLI derives `<package>.<kebab>`.
- `view` is the template identity, relative to the template root.
- `#[public]` fields are rendered and carried in the signed snapshot. `#[model]`
  fields additionally accept browser proposals through `live:model`.
  A model field declares its timing on the attribute, such as
  `#[model(debounce = 250)]`; a debounce is 100, 250, or 500 milliseconds,
  the durations `live:model.debounce.<n>ms` accepts, and any other fails to
  compile.
  A form submitted with `live:submit` proposes every model control inside
  it in one request, which carries up to 127 fields beside the action;
  `live:check` refuses a larger form.
  A proposal the field cannot decode, such as an empty number for a `u64`
  field, is a validation error on that field: the action does not run, the
  field keeps its value, and `live:error` shows the error.
- `#[action]` methods are the only entry points the browser can invoke. They
  receive validated arguments and may return typed outcomes such as a
  redirect or a flash.

Every field type must implement `Default`; a fresh island starts from those
defaults unless a mount hook says otherwise.

## Views

Views are Askama templates. The template root is `templates/` unless an
`askama.toml` names other directories, so `live/counter.html` lives at
`templates/live/counter.html`:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

Directives use the closed `live:` grammar: `live:click`, `live:submit`,
`live:model`, `live:upload`, `live:key`, `live:loading`, and the rest of the
documented set. The checker proves every directive against the component:
an unknown action, an unknown model field, a raw `safe` filter, or an
accessibility violation fails `live:check` with the file, line, and column.
`live:key` names an element's stable identity across morphs, and it is the
one key attribute a template writes: the runtime reads it for morph
identity, for morph controls such as `live:preserve.self`, and for the
scopes that keep browser state; `data-suprnova-live-key` is the engine's
own spelling on the roots it renders. `live:key` values and element ids
inside an island use one alphabet, the one the runtime checks at every morph:
an ASCII letter or digit first, then letters, digits, `_`, `-`, `.`, and `:`,
at most 128 bytes, each unique in the island. `live:check` refuses a literal
key or id outside it, and an id a loop repeats.

Documents that place islands are ordinary views declared with
`#[suprnova::view]`; the only unescaped value they accept is `TrustedHtml`
through the `trusted_html` filter.

## Registration and bootstrap

`src/live/mod.rs` owns the registry and the routes:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

Bind it during bootstrap so the server, the workers, and the `suprnova
live:*` commands see the same components:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

The registry is immutable once the runtime assembles. A duplicate component
name or view, or a component whose actions need validation without a
validation port, fails registration with a typed `RegistryError`.

## Routes

`Router::try_live()` installs the reserved namespace exactly once:
`/__live/action`, `/__live/upload`, the `/__live/async/*` control
routes and WebSocket handshake, and the immutable `/__live/assets/*`
routes. Startup fails if an application route can claim `/__live`.

The reserved request routes carry a strict policy: every request needs
session, origin, CSRF, principal, tenant, and rate-limit facts. The framework
records the session and the CSRF proof; your application attaches the rest
with the route guard:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

Install the routes from the entry point so the runtime and the mount catalog
are ready before the first request:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## Documents and islands

A document route declares its islands once, renders them through
`LiveDocument`, and emits the bootstrap tags:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` declares an island any visitor may render; its
  state is a reusable seed promoted to an instance on the first action.
- `LiveMount::identity_bound` declares an island that belongs to the current
  session and principal; the document route must authenticate.
- Mount every island before `bootstrap`, and call `bootstrap` once. The
  bootstrap emits the inert configuration element and the script tags for
  the ESM or classic strategy, adding the upload and asynchronous roles when
  a mounted component needs them and the Stimulus bridge on request.
- The document template places `{{ bootstrap|trusted_html }}` in `<head>` and
  each island where it belongs.

## Security boundaries

Live never bypasses the framework's middleware. What each request needs:

| Fact | Recorded by |
|---|---|
| Session | `SessionMiddleware` |
| Origin and CSRF | `CsrfMiddleware` with origin verification enabled |
| Principal | `AuthMiddleware` on its authenticated branch |
| Tenant | `LiveTenantMiddleware` with your resolver |
| Rate limit | `RateLimitMiddleware` on its allowed branch |

The shipped runtime sends the Live media type and the browser's own
`Sec-Fetch-Site` header; it carries no session token. The CSRF middleware
verifies that proof for every Live request on its own, whatever origin policy
you configure, so a same-origin Live request passes with the stateless CSRF
disposition while a cross-site or header-less request falls back to token
validation and is refused. Ordinary routes keep token validation under the
default policy; using Live relaxes nothing else:

```rust
global_middleware!(CsrfMiddleware::new());
```

Anonymous visitors render public seeds, and they can act on them when the
guard uses `AuthMiddleware::optional()`: a signed-in principal is recorded, an
anonymous visitor continues, and the mount kind decides. A public seed then
promotes for the visitor's own session on the first action, while an
identity-bound island still refuses a request without principal evidence.
With `AuthMiddleware::new()` the guard answers `401` for every anonymous
request before any engine work. Identity-bound islands require a session and
a principal; the tenant is bound into the island's scope whenever your
resolver names one, and a resolver that cannot determine the tenant must
return an error rather than `None`. Every rejection is closed:
a `409` for a stale or tampered snapshot carries no body, and production
messages never include snapshots, tokens, cookies, or rendered HTML.

## Uploads

Declare an upload policy on a model field:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

The view binds the field with `<input type="file" live:upload="avatar">`. The
runtime creates, transfers, and completes the upload through
`/__live/upload`; the file waits in quarantine until the declared finalize
action runs, when the framework hands it to your `UploadFinalizer`. Bind the
finalizer, and any scanner or validator, before the runtime assembles:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

Uploads are authorized per field and control through the gate. Define the
abilities `live:<component>.upload.<field>.<Control>` for `Create`,
`Reacquire`, `Status`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`,
`Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`,
and `Fail`.

A browser that lost its transfer grant reacquires it through a route your
application owns outside the reserved namespace:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

The route requires the same facts as an action, answers only the session and
principal that created the upload, and returns a fresh grant with the current
transfer state.

## Asynchronous updates

A component declares the streams it listens to; the browser runtime
subscribes over SSE or WebSocket and falls back to polling:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

Define the ability `live:<component>.stream.<name>` for subscribers, then
publish from anywhere in the application:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

A refresh tells subscribed islands to fresh-render; an event is delivered to
the island's registered handlers. Polling is the ordinary fresh render: the
island's state catches up whenever a transport is unavailable, but event
payloads published in between are not replayed to their handlers, which the
runtime reports as a degraded stream rather than a current one. A component
that declares exactly one stream gets its island root subscribed for it; a
component with several streams subscribes each through the runtime's
registered calls.

A stream ends with the session that opened it. When a session is destroyed on
the node holding the stream, through a plain logout, invalidation, id
regeneration, or a "log out everywhere", every membership it opened there is
retired at once and no later event reaches it. A session destroyed on another
node is caught by delivery itself: each membership's session is re-checked
against the session store at most once per ten seconds, so events stop within
that interval. The stream's gate is asked again before every delivery
regardless, so a policy change ends delivery immediately on every node.

## Assets and no-build use

The framework serves the exact reviewed runtime artifacts at
`/__live/assets/<identity>/<file>` with immutable caching, strong
validators, and integrity attributes in the bootstrap tags. A strict
`script-src 'self'` policy holds because documents contain no inline script.
To publish the same bytes to a CDN or a static directory:

```bash
suprnova live:assets --out public/__live
```

The publication is atomic and refuses to replace a directory whose bytes
differ unless you pass `--replace`.

## Component library

Suprnova ships the foundations of a component library for Live: a token
stylesheet with a base layer, and a form family of presentational components
built on native controls and the `live:model`, `live:error`, and
`live:loading` vocabulary. The base is a runtime artifact. Opt a document in
and it arrives as one stylesheet link under the same identity, integrity, and
cache contract as the runtime scripts:

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

Every rule in it sits inside the `suprnova-ui` cascade layer, so your own
unlayered styles win without a specificity fight. Every visual value is a
`--sn-` custom property for color, font, space, radius, shadow, motion,
density, and state, with light and dark values: override a token on `:root` to
retheme, or drop the layer and keep every behavior, name, and state attribute,
because a component styles its states from the attributes the checker proves
(`aria-invalid`, `aria-busy`, `aria-expanded`, `aria-pressed`, `aria-current`,
`aria-selected`, `:disabled`), never from a class. A Tailwind CSS 4 `@theme`
preset maps the tokens onto Tailwind's namespaces; Tailwind is never required.
Components install with `live:add`, one directory each under the reserved
`templates/suprnova-ui/` root: the Askama macro view, the stylesheet, the
JavaScript when the component has one, and the manifest that names them:

```bash
suprnova live:add field
suprnova live:add password-input
```

`live:add` records the digest of every file it writes, so a later run
replaces a file you never edited when the library changes it, keeps a file
you edited, and says so; `--force` replaces an edited file too. A third-party
component installs from its own manifest with `--manifest`, under its own
root, and each file it names must be a regular file inside the manifest's
directory, never a symbolic link. Call the macros from your views, serve the
vendored stylesheet and script with `try_live_ui_assets()`, and link them
from the document:

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

`try_live_ui_assets()` reads the files from `templates/suprnova-ui/` under the
application base path on each request, so ship that directory with the
binary and start the application from the directory that holds it, or set
`APP_BASE_PATH` to that directory. When the directory cannot be read, the
application refuses to start and names it.

The checker expands the macros, so `live:check` proves a library view like any
other. The form family today: field, label, input, textarea, number input,
slider, search input, password input with reveal, checkbox and checkbox group,
radio group, switch, select, button and link button, button group, fieldset,
form actions, validation summary, and file input. Library components are named
`suprnova.*` and the registry refuses that prefix from any other crate; custom
elements are light DOM and carry the `sn-` prefix.

Each value control takes the island's current value, so the page shows it and
a submit that changed nothing sends it back unchanged: `value=` for an input,
textarea, number input, slider, search input, and date picker, `checked=` for
a checkbox and a switch, and `selected=` for a select, a radio group, and a
checkbox group, which takes the list of checked values. A password and a
one-time code never render their value. Radio and checkbox group inputs are
keyed by value, so a choice the user has not sent survives a re-render. A
field that more than one checkbox binds is proposed as the list of checked
values, and a group of one checkbox as a boolean. A render that must replace
what the user has typed, such as the one answering a reset, passes a sequence
number as `authority=`, and the next render passes none:

```html
{% call input::input("email", kind="email", value=email, authority=authority) %}{% endcall %}
{% call checkbox::checkbox_group("topics", "Topics", topic_options, topics) %}{% endcall %}
```

The overlay family ships on the same foundations: tooltip, collapsible and
accordion, popover, a single-level dropdown menu, dialog, sheet, and drawer.
Each one owns its open state through the browser's own primitive before any
script runs: `details` for disclosures, the `popover` attribute for popovers
and menus, and `dialog` for the three modals, which the vendored `sn-dialog`,
`sn-sheet`, and `sn-drawer` elements open with `showModal()` and close with
focus returned to the trigger. Opening and closing never makes a Live request;
only an action you place inside an overlay does. Every overlay root carries a
stable key and `live:preserve.self`, so an open overlay survives a morph that
did not replace its region:

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

The `popover` attribute sets the supported baseline at Chrome and Edge 114,
Firefox 128, and Safari 17. Where CSS anchor positioning exists the popover and
menu sit under their trigger; elsewhere the browser centres them. The
accordion's single-open mode rests on `details name`, which older supported
releases treat as independent disclosures. The tooltip stays open while the
pointer moves from its trigger onto the bubble, so its text can be read or
selected.

The feedback family and the navigation family follow. Feedback: alert,
skeleton, spinner, progress, empty state, and a toast region with a flash
region beside it. Each one presents a state the server or the runtime already
holds. An alert chooses its role from its variant and marks every variant with
a glyph and a hidden label, never color alone. A spinner or skeleton is bound
with `live:loading.show` to a registered action and authored hidden, so the
runtime reveals it after its own delay and keeps it past its minimum, and a
fast action never flashes it. Progress is the native `progress` element with a
label and a text readout, and carries a value only for determinate work. The
empty state takes its reason (empty, no results, no permission, disconnected)
from server-rendered state and offers a next action only where the caller
renders one. A toast announces once from a polite status region and never
takes focus; the vendored `sn-toast-region` element times toasts out, pauses
while the pointer is over any part of a toast or focus is inside it, bounds how many show at once, and answers the
dismiss button, with each toast keyed and preserved so a dismissed toast stays
dismissed across a morph. A critical error belongs in an alert as well; a
toast is never its only surface. Toasts render inside a loop, so their keys
pass through the `live_key` filter, and the island that mounts them exposes
it with `pub mod filters { pub use suprnova::view::filters::live_key; }`.
`live_key` fails the island's render for a value outside the key alphabet,
such as an email address; key such data with `live_key_digest`, which turns
any value into a stable key in the alphabet and is exported the same way from
`suprnova::view::filters`.
The flash region renders what the previous request left in the session, once:

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

Navigation: header bar, footer, sidebar with collapsible groups, breadcrumbs,
tabs, pagination, and load more. Every destination is an anchor with a real
route URL and every action is a button; the current item carries
`aria-current` from the value you bind, never from the browser's location.
The sidebar's groups are native `details`, keyed and preserved. Tabs require a
mode: `local` panels with tablist semantics, arrow keys from the vendored
`sn-tabs` element, and no request on a change, or `route` tabs as anchors.
Tabs nest: an inner tabs instance selects only its own tabs and panels.
Pagination requires a mode too: route pages are canonical links, and Live
pages are buttons on your actions whose result reflects the new query into the
current history entry through `url_intent`, with no history entry per page.
Load more is a button on a registered action that appends to a keyed list, so
the morph keeps every row already there, and the control leaves the view when
you render it exhausted. A URL reflection is a protocol 2 result, so an island
that paginates through `url_intent` declares `minimum_protocol_version = 2`;
its keyed rows pass through `live_key` like a toast does:

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

The data display family closes the built-in set. Presentational: separator,
scroll area, aspect image, card, badge, avatar and avatar group, list group,
description list and stat card. Each keeps document order and native
semantics: the separator is an `hr` or a labeled separator role, the scroll
area is a focusable labeled region that scrolls natively, the aspect image is
the `img` itself with a named ratio, the card is an article or section
labeled by its own heading with actions in a labeled group, and the
description list is a `dl`. A badge always carries its text, an avatar names
its person in `alt` or in the label of its initials, and a stat's trend says
"Up", "Down" or "Flat" in text before the delta, so no status rests on color
alone. The list group keys every item through `live_key`, so a reorder keeps
each node. The chart is server-rendered: the island calls `render_chart`
from `suprnova::live::charts`, which draws bar or line marks through
`charts-rs` from bounded typed series and returns trusted markup, and the
macro renders the SVG beside a text summary and a data table in a disclosure,
so the canonical document reads without the picture and no charting script
ever reaches the browser:

```rust
use suprnova::live::charts::{ChartKind, ChartSeries, render_chart};

pub fn chart_svg(&self) -> TrustedHtml {
    render_chart(
        ChartKind::Bar,
        &["Apr", "May", "Jun"],
        &[ChartSeries::new("Revenue", vec![42.0, 47.0, 51.0])],
    )
    .expect("a bounded fixed series renders")
}
```

`render_chart` returns an error rather than drawing a value whose magnitude
exceeds 1e9, beyond the range the renderer's axis arithmetic holds.

The datatable is the last component, and one island per table. It is a
native `table` with a caption naming the result count, column headers with
`scope`, and `aria-sort` on the sorted column. Sort and filter are Live
submits on the island's model fields, page changes are Live buttons, and the
island declares the applied sort, direction, filter and page as `#[url]`
fields and reflects them through `url_intent` after every action, so the
address bar always holds a shareable URL and the document mounts the same
view from it:

```rust
#[live(name = "app.invoices", view = "live/invoices.html", minimum_protocol_version = 2)]
pub struct Invoices {
    #[model]
    pub sort: String,
    #[url(key = "sort")]
    pub sorted_by: String,
    #[url(key = "dir")]
    pub direction: String,
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    #[url(key = "page")]
    pub page: u64,
    pub rows: Vec<Invoice>,
}
```

The live-native family is the last one, the components that only make sense
on the running runtime. The upload widget renders the shipped upload
protocol: its file input carries `live:upload` for the island's upload
field, its `progress` element is the runtime's progress root, and cancel,
retry and remove act on the temporary reference through `live:upload.cancel`
and its siblings. Every state the domain knows is rendered as text and shown
from the progress root's `data-live-upload-state`, and "ready" reads as
verified but not saved, because nothing is durable until the finalizing
action runs:

```html
{% call upload::upload("attachment", "Attachment", accept="image/png") %}{% endcall %}
<button type="submit" live:loading.disabled="save_attachment">Save attachment</button>
```

The live feed and the notification bell sit on a stream-backed island. The
runtime writes `data-live-stream-state` on the island root and announces
every change into the `[data-live-stream-status]` element the macros render
(Updates disconnected, Connecting to updates, Updates current, Updates
degraded, Reconnecting to updates, Updates closed), so a degraded,
reconnecting or closed stream says so and only the current state reads
current. Feed items pass through `live_key`. The account menu is a `details`
disclosure of anchors and a sign-out form that posts with the session's
CSRF token; it is a stitch slot under RenderCache, so an application mounts
it as its own identity-bound island and the shared shell never holds the
principal's name.

The custom-element tier enhances native controls it never replaces. Each
element is a light-DOM `HTMLElement` subclass defined only by its own
vendored file, carries the `sn-` prefix, and holds no form value, because
the native input inside it is the control: block the script and the form
still submits the same value. The input OTP is one native input
(`inputmode="numeric"`, `autocomplete="one-time-code"`, a length pattern) on
a transient model, and `sn-input-otp` mirrors the typed characters into
`aria-hidden` cells. The date picker is a `type="date"` input, and its year,
month and day strips are fieldsets of native radios inside CSS scroll-snap
containers, so tap, click and arrow keys select with no script;
`sn-date-picker` composes a complete selection into the input. The combobox
is the accessible combobox pattern (`role="combobox"`, `aria-expanded`,
`aria-activedescendant`, a `role="listbox"` of options) over a native input
with a `datalist` for the script-free case, and `sn-combobox` moves the
active option and selects. By default the options are your server's answer
to the query: render them for the model field on every render, and the
element shows all of them while the query they answer is the input's text,
whatever your search matched, and keeps an answer to older text hidden, so a
stale result never replaces results for a newer query. Pass `remote=false`
for a fixed list, which the element filters by the typed text:

```html
{% call otp::input_otp("code", "One-time code") %}{% for index in cells %}{% call otp::otp_cell(index) %}{% endcall %}{% endfor %}{% endcall %}
{% call date::date_picker("when", "Renewal date", years, months, days, min="2026-01-01", max="2028-12-31") %}{% endcall %}
{% call combo::combobox("country", "Country", countries, query=country, placeholder="Type a country") %}{% endcall %}
```

### Why Suprnova diverges

Laravel ships Blade components and a starter kit's markup; Suprnova ships the
library through the framework itself, on Live's own vocabulary, with no client
application owning the page. The skin is on by default and removable with
nothing breaking, which is what headless means here.

## Testing

`suprnova::live::testing` prepares a router's runtime and mount catalog for
in-process tests. The application tests in `app/tests/live_*.rs` show the
complete pattern: an in-memory database, a seeded session cookie, the real
global middleware stack, and requests through `handle_request`:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

Decode an island's snapshot from its `data-suprnova-live-snapshot` attribute,
post an action with the session cookie and `Sec-Fetch-Site: same-origin`, and
assert on the accepted render. A stale snapshot answers `409` with an empty
body; a missing principal answers `401`.

## Diagnostics and operations

- `suprnova live:check` proves every registered view; `--allow-unproved`
  accepts dynamic structures the checker deliberately makes no claim about.
- `suprnova live:inspect` reports the bound registry, configuration limits,
  installed upload capabilities, assembled runtime services, and the asset
  identity without exposing state or secrets.
- `LiveConfig` bounds request and response bytes and the trusted context
  lifetime; bind a custom one before the runtime assembles.
- Errors carry closed kinds such as `live_document_context_rejected` and
  `invalid_live_bootstrap`; telemetry labels are closed enumerations.

## Recovery

- A `409` tells the runtime to fresh-render the island; the operation is not
  replayed.
- A closed asynchronous transport is retired and the runtime reconnects with
  a new transport generation; a stale generation is refused.
- A session that expires or rotates invalidates identity-bound work; the
  application exposes its sign-in path and the visitor continues from a fresh
  document.

Live runs complete without RenderCache. Caching Live documents is
RenderCache's job; see [RenderCache](render-cache.md).

## CLI reference

| Command | Purpose |
|---|---|
| `suprnova live:make <name>` | Scaffold a component and its view and register it |
| `suprnova live:check` | Prove every registered view with the integrated checker |
| `suprnova live:inspect` | Report safe runtime, registry, provider, and artifact state |
| `suprnova live:assets --out <dir>` | Publish the reviewed runtime artifacts atomically |
