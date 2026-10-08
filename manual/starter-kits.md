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
React 19, Vue 3.5) with Tailwind v4 and Inertia v3. See
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
  to the list.
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

For API-only services, `suprnova new my-api --api` initializes Magnetar,
installs bearer-session middleware, and scaffolds password registration and
login against the canonical `app_users` table without a frontend.

### Why Suprnova diverges

Laravel's starter kits ship a dashboard with placeholder content and no list
page. Suprnova's scaffold lists the signed-in user's own notes, so deferred
and optional props, cursor scrolling, a JSON form, and flashed toasts each
have a working server to talk to. No handler lists accounts: a directory of
users would show every member the name and email of every other member.

## Contributing a starter kit

Built something reusable on top of Suprnova and want to upstream it as a
canonical kit? See [Contributions](contributions.md). We're happy to take a
real implementation and round it into a generic kit.
