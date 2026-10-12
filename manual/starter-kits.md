# Starter Kits

Starter kits are ready-made Suprnova applications you fork and ship. Each one
wires the controllers, routes, migrations, frontend pages, and tests for a
complete product surface - so you start from a running app, not an empty
scaffold.

Three kits ship, modelled on Laravel's lineage. Pick the one closest to
what you're building and customise from there.

## Nebula - authentication (Breeze-tier)

**Repo: [github.com/eas4ai/Nebula](https://github.com/eas4ai/Nebula)**

The minimal full-auth kit - Suprnova's Breeze equivalent. Everything you need
for accounts and nothing you don't:

- Registration with email verification
- Login with remember-me
- Password reset with anti-enumeration responses
- Profile management - update email and password, delete account
- A branded Inertia 3 + Svelte 5 frontend (dark by default), with the
  logged-in user menu wired

Nebula ships two test suites: facade-level auth logic, and a wire-level HTTP
suite that drives real routes, sessions, CSRF round-trips, and the
guest / auth / verified gates over a loopback socket.

Reach for Nebula when you want a clean account-management foundation to build
your own product on top of.

## Pulsar - product site & community

**Repo: [github.com/eas4ai/Pulsar](https://github.com/eas4ai/Pulsar)**

A complete developer-tool / SaaS company site on Vue 3.5 + Vuetify. Everything
in Nebula's auth story, plus the surfaces a real product site needs:

- Marketing landing page and a user dashboard
- A Markdown documentation pipeline (`docs:build`) with search and a generated
  table of contents
- A blog / articles system with an RSS feed
- Public member profiles
- Taxonomy - topics, tags, and categories
- Role-based access control: roles, permissions, and gates
- Admin and moderation surfaces for content and members

Pulsar is the source kit for downstream products such as `suprnova.app`. Reach
for it when you're shipping a product site with docs, a blog, and a member
community - not just authentication.

## Directory starter - listings, moderation, and paid publication

**Repo: [github.com/eas4ai/suprnova-directory-starter](https://github.com/eas4ai/suprnova-directory-starter)**

A free, MIT-licensed directory on Vue 3.5: owners submit listings, moderators
review them, visitors search them, and publication is free or paid through
Stripe and Paddle. Everything in Nebula's auth story, plus:

- Directory discovery - searchable listings, category filters, pagination,
  and public detail pages
- Owner submissions and moderation - drafts, revision history, approval,
  rejection, resubmission, and suspension
- Free and paid publication - Stripe and Paddle, configurable plans, separate
  test and live settings, authenticated webhooks, and payment recovery
- Editorial publishing - article drafts, previews, categories, tags, and RSS
- Administration - permission-based roles, account suspension, audit
  history, and an admin overview
- SEO - site defaults and per-content overrides, server-rendered metadata,
  sitemaps, redirects, a 404 report, and Markdown versions of public pages
- Media and notifications, colour presets with a remembered light or dark
  preference, demo content, and documented backup and restore workflows

Reach for the directory starter when the product is a directory or a
marketplace of listings, with or without paid placement.

## Which kit?

| You want… | Start with |
|---|---|
| Accounts and a place to build | **Nebula** |
| A full product site - landing, docs, blog, community, RBAC | **Pulsar** |
| A directory or marketplace of listings, free or paid | **Directory starter** |
| Server-rendered pages with no Node toolchain | `suprnova new my-app --frontend live` |
| An API-only backend (token auth, no frontend) | `suprnova new my-api --api` |

All three kits track the framework as a git dependency and run on the same stack you
already know - see each repo's README for setup. More kits are planned; watch
the [releases](https://github.com/eas4ai/suprnova/releases) or open an
issue if there's one you want.

## What the default scaffold gives you

If neither kit fits, `suprnova new my-app --frontend svelte` (or `react`, or
`vue`) already ships a working authentication flow - login, register, logout,
email verification, password reset, session authentication with the
`authenticate` middleware, CSRF protection, and a protected `/dashboard`
route - on any of the three frontends (Svelte 5,
React 19, Vue 3.5) with Tailwind v4 and Inertia v3, or on the
[Live starter kit](#the-live-starter-kit), which needs no Node toolchain. See
[Installation](installation.md) for the scaffold output and
[Quickstart](quickstart.md) for the first-five-minutes walkthrough.

Behind the authentication group, the scaffold ships the server side of the
kit pages, all of it over the signed-in user's own data:

- A `notes` table and a `Note` model (`src/models/note.rs`) that belongs to
  a user, with `Note::owned_by(user_id)` as the one way the handlers read
  notes.
- `GET /dashboard`, which sends `stats` deferred (the user's note count
  and the notes written today) and `recent_notes` optional (the user's
  five newest notes).
- `GET /notes`, which lists the user's notes 10 at a time with
  `cursor_paginate` through `Inertia::paginate`, filtered by the `search`
  query parameter in title or body without regard to case, and sends
  `search` back to the page.
- `GET /notes/{id}`, which shows one of the user's notes and answers `404`
  for a note of another user or an id that doesn't exist.
- `POST /notes`, which validates `title` (1 to 255 characters) and `body`
  (at most 10000 characters), writes the note, flashes a toast, and returns
  to the page the form was posted from, preserving its search query, with
  `/notes` as the fallback.
- `POST /profile/name`, a JSON handler for the dashboard's name form that
  answers `200` with `{"user": {...}}` or `422` with the framework's
  validation body.

Every page receives the signed-in user, or `null` for a guest, under the
shared `auth` prop: `bootstrap.rs` shares it with `App::inertia_share_lazy`,
which reads the request's session for each response that sends it, and
`SharedData` in `src/props/shared.rs`, marked `#[inertia_props(shared)]`,
adds `auth` to the generated `SharedProps`. A layout reads
`usePage().props.auth.user`.

Sign-in, registration, a reset-link request, a password reset, email
verification, a verification resend, sign-out, and a saved note each flash a
`Toast` (`src/props/flash.rs`) under the key `toast`. The `Flash` struct
beside it holds that toast and is marked `#[inertia_props(flash)]`, so
`suprnova generate-types` names it as Inertia's `flashDataType` and
`page.flash.toast` is typed in every page.

The routes carry the names Laravel's kits give them: `dashboard`,
`notes.index`, `notes.show`, `notes.store`, `profile.name`, `login`,
`register`, `password.request`, `password.email`, `password.reset`,
`password.update`, `verification.notice`, `verification.send`,
`verification.verify`, and `logout`.

The dashboard handler shows how each prop travels:

```rust
#[handler]
pub async fn index(req: Request) -> Response {
    let user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let user_id = user.id;

    Ok(InertiaResponse::new("Dashboard")
        .defer("stats", move || note_stats(user_id))
        .optional("recent_notes", move || recent_notes(user_id))
        .resolve(&req)
        .await?)
}
```
The Svelte kit's pages show Inertia 3.8's client components at work:

- `layouts/AppLayout.svelte` and `layouts/GuestLayout.svelte` frame every
  page through `createInertiaApp`'s `layout` option and stay mounted across
  visits. They carry the `Link` navigation, the signed-in user's name and a
  sign-out `Link` (or sign-in and register links for a visitor), and the
  toast a handler flashes, which `components/FlashToast.svelte` reads from
  `page.flash`.
- `Dashboard` loads the note counts with `Deferred` and polls them with
  `usePoll`, loads the recent notes with `WhenVisible` once they scroll
  into view, and sets the layout's heading with `setLayoutProps`. Its
  display-name form posts JSON through `useHttp` and shows the new name
  before the server answers.
- `Notes/Index` creates a note with the `Form` component, keeps its search
  box with `useRemember`, loads more notes with `InfiniteScroll`, and opens
  a note with a prefetched, instant `Link` to `Notes/Show`.
- The pages under `auth/` submit through the `Form` component, and every
  page titles its tab with the kit's `components/Head.svelte`.

For the code, see [Page Components](frontend-pages.md).
The React starter (`--frontend react`) runs on Inertia 3.8 through the
`@inertiajs/vite` plugin and shows the client side of each feature on a
page you can copy. `Dashboard` renders `stats` with `Deferred`, polls it
with `usePoll`, loads `recent_notes` with `WhenVisible`, and changes your
display name through `useHttp` with an optimistic update. `Notes/Index`
creates a note with `Form`, remembers its search with `useRemember`,
scrolls your notes with `InfiniteScroll`, and opens one with an instant,
prefetched `Link`; `Notes/Show` renders it. `Login`, `Register`,
`ForgotPassword`, `ResetPassword`, and `VerifyEmail` submit through `Form`.
Every page sets its title with `Head`, and the pages render inside a
guest layout (the `auth/` pages) or an application layout (the rest). Both
show the flash toast and, from the shared `auth` prop, the signed-in
user's name and a sign-out link or the sign-in and register links. See [Page Components](frontend-pages.md) for the
code.
The Vue kit (`--frontend vue`) runs on Inertia 3.8 and its Vite plugin, and
each page shows part of the client at work:

- `Dashboard.vue` loads the note counts as a deferred prop with `Deferred`,
  polls them with `usePoll`, loads the recent notes with `WhenVisible`, and
  saves the display name through `useHttp` with an optimistic update.
- `Notes/Index.vue` creates a note with `Form`, remembers its search with
  `useRemember`, scrolls the signed-in user's notes with `InfiniteScroll`, and
  opens a note as an instant visit from a prefetching `Link`.
- `Notes/Show.vue` shows one of the signed-in user's notes.
- The five pages under `auth/` submit through `Form`.
- `layouts/AppLayout.vue` and `layouts/GuestLayout.vue`, applied through the
  `layout` option, carry the navigation, the flash toast from
  `components/FlashToast.vue`, and the account links from
  `components/AccountLinks.vue`: the signed-in user's name and a sign-out
  button, or the sign-in and register links for a guest. Every page titles
  its tab with `Head`.

For API-only services, `suprnova new my-api --api` initializes Magnetar,
installs bearer-session middleware, and scaffolds password registration and
login against the canonical `app_users` table without a frontend.

### Why Suprnova diverges

Laravel's starter kits ship a dashboard with placeholder content and no list
page. Suprnova's scaffold lists the signed-in user's own notes, so deferred
and optional props, cursor scrolling, a JSON form, and flashed toasts each
have a working server to talk to. No handler lists accounts: a directory of
users would show every member the name and email of every other member.

## The Live starter kit

`suprnova new my-app --frontend live` scaffolds the same application as the
Inertia kits without a frontend build: no `frontend/` directory, no
`package.json`, and no Node toolchain. Every page is a server-rendered
[Live](live.md) document. The application serves the Live browser runtime
under `/__live/assets/` and the library components' stylesheets and scripts
under `/suprnova-ui/`, so the backend is the only process you run.

The kit keeps the backend the Inertia kits share: the `users` and `notes`
tables and models, `Note::owned_by(user_id)` as the one way to read notes,
the account flows through `Auth`, `PasswordReset`, and
`EmailVerification`, and the route names. Pages render and act through Live
instead of Inertia:

| Page | Route | Layout | Live components |
|---|---|---|---|
| Welcome | `/` | application | none |
| Sign in | `/login` | guest | `LoginForm` |
| Register | `/register` | guest | `RegisterForm` |
| Forgot password | `/forgot-password` | guest | `ForgotPasswordForm` |
| Reset password | `/reset-password` | guest | `ResetPasswordForm` |
| Verify email | `/verify-email` | guest | `VerifyEmail` |
| Dashboard | `/dashboard` | application | `NoteStats`, `ProfileName` |
| Notes | `/notes` | application | `NoteForm`, `NoteList` |
| A note | `/notes/{id}` | application | none |
| Error | any failed request | application | none |

The forms on the guest pages are public seeds, which any visitor can use.
Every other island is identity-bound: it belongs to the signed-in user, and
its route sits behind the `auth` middleware.

### Files

The kit adds these files to the shared backend:

- `src/controllers/` - one controller per page. Each opens the request's
  `LiveDocument`, mounts the page's islands from `live::mounts()`, and
  renders the page's view.
- `src/views/mod.rs` - the view structs and `Frame`, the part both layouts
  read: the title, the Live bootstrap tags, the signed-in user, the CSRF
  token of the sign-out form, and the toast.
- `templates/live/layouts/` - `guest.html` for the account pages and
  `app.html` for the rest, with the header bar, the account menu, and the
  toast region. `head.html` holds the bootstrap tags and the stylesheet
  links both layouts include.
- `templates/live/pages/` - one template per page. Each starts with
  `{% extends "live/layouts/app.html" %}` or the guest layout.
- `src/live/` and `templates/live/*.html` - the Live components and their
  views, registered in `src/live/mod.rs`.
- `templates/suprnova-ui/` - the library components the views import,
  installed the way `suprnova live:add` installs them and recorded in
  `suprnova.toml`, so a later `live:add` keeps them current.
- `src/flash.rs` - the `Toast` a finished flow shows on the next page.
- `src/controllers/errors.rs` - the `ErrorPage` middleware, which renders
  every framework error response as the error page in the application
  layout, keeping the response's status, its other headers and its error
  report. A client that expects JSON keeps the JSON body, and in debug mode
  a `5xx` keeps the development error page.
- `src/controllers/profile.rs` - `POST /profile/name`, the JSON endpoint
  the Inertia kits ship for the display name; the dashboard's island saves
  it through Live.
- `public/css/app.css` - the page layout, written with the same `--sn-`
  tokens the library uses.

### What each component does

- `LoginForm`, `RegisterForm`, `ForgotPasswordForm`, and
  `ResetPasswordForm` check their fields with the same rules as the Inertia
  kits' form requests and show each message under its field. An action
  that succeeds signs the user in, mails a link, or rotates the password,
  then sends the browser to the next page with a toast.
- `VerifyEmail` names the address the link went to and mails a new link on
  request.
- `NoteStats` shows the user's note count, the notes written today, and the
  five newest notes. It subscribes to the stream `notes/:principal`, which
  Live fills with the user's own id. Saving a note publishes a refresh on
  that topic, so every dashboard the user has open renders again without a
  page load, over server-sent events or a WebSocket, and through polling
  when neither connects. Its **Refresh** button renders it again on
  request.
- `ProfileName` changes the display name and reloads the dashboard, so the
  navigation shows the new name.
- `NoteForm` checks the title (1 to 255 characters) and the text (at most
  10000 characters), saves the note, and reloads the list with a toast.
- `NoteList` filters the notes by title and text. The applied search is
  bound to the `?search=` query: each search puts it in the address bar
  without a history entry, and the page mounts the list from it, so a
  reloaded or shared URL shows the same list. **Load more** adds 10 notes
  at a time.

A finished flow keeps its `Toast` in the session, and the next document
takes it with `Toast::take()`, which removes it. The toast is an ordinary
session value rather than a session flash, because a flash ages on every
request of the session, the Live requests included. Saving a note makes
your open dashboards refresh, and that refresh would age a flashed toast
away before the redirect lands. The kit also turns session blocking on
(`SESSION_BLOCK=true` in `.env`), so the requests of one session run one
at a time and a refresh cannot write over the toast a save just kept.

### Add a page

To add a page, give it a route, a view, and a controller:

1. Add the route to `src/routes.rs`, in the authenticated group if the page
   needs a user.
2. Add the view struct to `src/views/mod.rs`, and its template to
   `templates/live/pages/`, starting with the layout it extends.
3. Write the controller. Open the request's `LiveDocument`, mount the
   page's islands, then build the `Frame`, which emits the bootstrap tags
   after the last mount, and render the view.
4. Declare the page in `live::routes()` in `src/live/mod.rs`: with
   `.try_live_document("/path")?` when it mounts no island, or with a
   `LiveMount` in `Mounts` and `.try_live_mount(&mounts.name)?` for each
   island.

An `/about` page with no island looks like this:

```rust
// src/views/mod.rs
#[suprnova::view(path = "live/pages/about.html")]
pub struct AboutPage {
    pub frame: Frame,
}

// src/controllers/about.rs
use suprnova::live::LiveDocument;
use suprnova::{FrameworkError, HttpResponse, Request, Response, handler};

use crate::views::{self, AboutPage, Frame, Nav};

#[handler]
pub async fn show(req: Request) -> Response {
    Ok(page(&req).await?)
}

async fn page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let frame = Frame::live(&mut document, "About", Nav::None).await?;
    views::render(document, "live/pages/about.html", &AboutPage { frame })
}
```

```html
{% extends "live/layouts/app.html" %}
{% block content %}
<h1 class="kit-page-title">About</h1>
{% endblock %}
```

### Add a component

1. Run `suprnova live:make Invoices`. It writes `src/live/invoices.rs` and
   `templates/live/invoices.html` and registers the component.
2. Add a `LiveMount` for it to `Mounts` in `src/live/mod.rs`:
   `LiveMount::identity_bound` when it belongs to the signed-in user, or
   `LiveMount::public_seed` when any visitor may use it. Seal it in
   `live::routes()` with `.try_live_mount(&mounts.invoices)?`, and mount it
   from the page's controller.
3. To use another library component in its view, run
   `suprnova live:add <name>` and link the component's stylesheet in
   `templates/live/layouts/head.html`.
4. Run `suprnova live:check` to prove every view against its component.

### Why Suprnova diverges

Laravel's Livewire starter kit renders Blade views with Livewire
components, and still builds its assets with Vite. The Live kit has no
build step: the framework serves the Live runtime, and the component
library ships as files the scaffold installs into your project, which you
own and edit like any other view.

## Contributing a starter kit

Built something reusable on top of Suprnova and want to upstream it as a
canonical kit? See [Contributions](contributions.md). We're happy to take a
real implementation and round it into a generic kit.

Every kit is held to the browser suite in `suprnova-cli/tests/kit_browser/`.
Run `cargo test -p suprnova-cli --test kit_browser -- --ignored --test-threads=1`
from the repository root. The harness scaffolds all three kits, builds one
backend against the in-tree framework, builds each frontend, and runs the
same Chromium assertions over each served application. It uses the existing
Playwright browser cache; it does not install browsers. Failed runs print
the kit name, the browser output, the server log, and the retained artifact
directory.

The Live kit is held to `suprnova-cli/tests/kit_live.rs` and to
`live_kit_compiles_and_renders` in `suprnova-cli/tests/scaffold_snapshot.rs`,
which builds a fresh Live scaffold against the in-tree framework, starts it,
and requests its pages:
`cargo test -p suprnova-cli --test scaffold_snapshot -- --ignored live_`.
