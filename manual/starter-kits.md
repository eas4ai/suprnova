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

### Agents in the Inertia kits

The React, Vue and Svelte kits serve [agents](agents.md) from the first
`cargo run`: the MCP endpoint at `/mcp`, the OAuth server an MCP client signs
its user in through, and agent accounts under auth.md. `src/agents.rs` turns
them on, and `cmd/main.rs` serves the routes through `agents::serve`. The
kit replaces the framework's three default pages with its own, rendered by
`src/controllers/agents.rs` in the kit's frontend:

The browser helpers use the community's `webmcp-types` 0.1.10 definitions,
pinned in `devDependencies`, to check their registrations and annotations.

| Page | Route | What the user does there |
|---|---|---|
| `Agents/Approve` | `GET /oauth/approve` | Allows or denies an MCP client. The page names the client, its host, every permission it asks for and the address the user returns to. |
| `Agents/Claim` | `GET /agent/claim` | Lets an agent act for them, with the code the agent gave them. A wrong code comes back as the field's error with the attempts left. |
| `Agents/Connected` | `GET /connected-agents` | Sees and revokes every app and agent that acts for them. The app layout links it as **Agents**. |

The paths of the first two are the framework's defaults, so a link a client
or an agent holds opens the same page with or without the kit. A guest who
opens either one signs in and comes back: the sign-in page keeps a
`return_to` that is a path on this application, and a registration in the
same session keeps it too. A `return_to` that a browser would read as
another site, such as `//evil.example`, is ignored, and the user goes to the
dashboard.

The pages read and record what the user decides through `AgentApprovals`,
`AgentClaims` and `ConnectedAgents`, which act for the signed-in user only.
Approving a client needs a signed-in user, as the dashboard does. Claiming
an agent needs the address the agent named, verified: the claim page tells
an unverified user to verify first. A few rules hold on every page:

- Each route runs `ConsentPageHeaders`, so another site can't frame the
  page, and no cache or referrer keeps the authorization id or the claim's
  attempt token. The rest of the security headers policy, its nonce among
  it, stays as `SecurityHeaders` sets it.
- The global CSRF middleware checks every form.
- A decision leaves for the client by a location visit
  (`Inertia::location`), which the browser follows itself. A redirect that
  answers the form would be blocked by the policy's `form-action 'self'`.
- No page offers a tool, and no consent route declares one, so an agent in
  the browser can never approve a client or claim itself. A form tool marks
  the form it submits with the `Suprnova-Form-Tool` header, and the pages
  refuse it with a 403, so even a form tool an application adds to one of
  them cannot approve or claim.
- Text a client, an agent or the user chose sits in `<bdi>`, and no page
  renders raw HTML or loads a URL the client chose.

`suprnova new` writes a signing key for agent auth into `.env`, as
`AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`. The application doesn't
start without one. Every other environment needs its own key, so a deploy
that doesn't copy `.env`, such as the production image, sets both
variables. `cargo run -- agents:key` prints a new pair. For rotation, see
[Keys and operations](agents.md#keys-and-operations).

The kit ships two [tools](agents.md#tools) on the signed-in user's own
notes, so the MCP endpoint has something to list from the first run.
`src/controllers/note_tools.rs` defines them, and `src/agents.rs` declares
their scopes with [`tool_scopes!`](agents.md#declare-scopes), in
`agents::scope`:

| Tool | Route | Scope | What it does |
|---|---|---|---|
| `search_notes` | `GET /notes/search` | `notes:read`, "Read your notes" | Finds the user's notes by the words in their titles and text, best match first, or lists the newest. It takes `query`, up to 200 characters, and `limit`, 1 to 50 and 10 when left out. |
| `create_note` | `POST /notes/tools` | `notes:write`, "Write notes for you" | Writes a note for the user, with the notes form's rules: a `title` of 1 to 255 characters and an optional `body` of up to 10000. It answers the note. |

Both tools accept users and agents, and each call acts for one user: the
one signed in, the one who approved the client, or the one who claimed the
agent. A client's token holds the scopes the user allowed on the approval
page, which shows each scope by its description. An agent holds both scopes
once a user claims it, and none before: an agent no user has claimed gets a
403 with `insufficient_scope`. `search_notes` is marked `read_only` and
`untrusted_content`, because it hands back note text, which the user wrote
and an agent must not take as instructions.

The notes page offers both tools to an agent in the user's browser, through
`.with_tools(["search_notes", "create_note"])` on its route. It is the only
page that offers a tool, and a guest is never offered either. After
`create_note`,
the page reloads its props, so the list shows the new note.
`frontend/src/types/tool-schemas.ts` carries both inputs, as
`SearchNotesInput` and `CreateNoteInput`.

To call the tools from Claude Code, run the project with `suprnova serve`
and add its endpoint, over the Streamable HTTP transport, at the URL
`APP_URL` names in `.env`:

```bash
claude mcp add my-app --transport http http://localhost:8765/mcp
```

Claude Code connects without a token and lists both notes tools. Calling
one answers 401, and Claude Code reports that the server needs you to sign
in; run `/mcp` to sign in. The client gets its token from the kit's OAuth
server: it registers, sends you to the approval page to allow the scopes it
asks for, and from then on lists the tools those scopes reach. A token with
`notes:read` alone sees `search_notes` and not `create_note`. See
[Users who approve a client](agents.md#users-who-approve-a-client).

### Why Suprnova diverges

Laravel's starter kits ship a dashboard with placeholder content and no list
page. Suprnova's scaffold lists the signed-in user's own notes, so deferred
and optional props, cursor scrolling, a JSON form, and flashed toasts each
have a working server to talk to. No handler lists accounts: a directory of
users would show every member the name and email of every other member.

## The API kit

For API-only services, `suprnova new my-api --api` initializes Magnetar,
installs bearer-session middleware, and scaffolds password registration and
login against the canonical `app_users` table without a frontend.

### Agents in the API kit

`suprnova new my-api --api` serves agents from `src/agents.rs`: the MCP
endpoint at `/mcp`, the OAuth server that MCP clients sign users in through,
and agent accounts under auth.md. For what each of these does, see
[Agents](agents.md). The kit has no frontend, so it serves the framework's
own pages, which are plain HTML with no script:

| Page | Path | What the signed-in user does there |
|---|---|---|
| Approval | `/oauth/approve` | Allows or denies an MCP client |
| Claim | `/agent/claim` | Enters the code an agent gives them |
| Connected agents | `/oauth/connected` | Sees and revokes the clients and agents that act for them |

`src/agents.rs` names each path once, as a constant, and gives the same
constants to the OAuth server, agent auth, the sign-in page and the session.

#### The consent sign-in

A person approves a client or claims an agent in a browser, so these pages
act for the user of a browser session. The kit signs a browser in at
`/login` (`src/controllers/sign_in.rs`) for them:

1. A guest who opens the approval or claim page is sent to
   `/login?return_to=...`.
2. The guest signs in with their email address and password, through
   `Auth::password()`.
3. The page continues to the address in `return_to`. Only a path on this
   application is kept: an address such as `//evil.example` or
   `https://evil.example` sends the user to `/oauth/connected` instead.

The page is fixed markup with one value in it, the session's CSRF token. Its
routes carry `ConsentPageHeaders`, so it is never framed, cached, or sent as
a referrer, and its form posts through the route's `CsrfMiddleware`. An
account with a second factor is told that the page does not offer one, and
stays signed out. To let those users approve clients, add the factor
challenge to the page.

#### Email verification

A claim needs the user's email address verified, so the kit verifies it with
a link, through `Auth::email_verification()`
(`src/controllers/email_verification.rs`):

1. Registration mails a new account the link, once. Registering an address
   that already has an account mails nothing. `POST /api/auth/register`
   takes six requests a minute from one client address, and answers `429`
   past that.
2. A signed-in client asks for a new link with
   `POST /api/email/verification-notification`. It answers `202` when it
   mails one, and `204` when the address is already verified. The route
   takes six requests a minute, and one address gets at most three links
   an hour, the registration's included; past either it answers `429`.
3. The link opens `/email/verify?token=...` in a browser. A guest is sent
   to `/login` and comes back, and the page then verifies the address of
   the user signed in there.

The link works once and expires after 24 hours. It verifies only for the
account's own user: anyone can register another person's address with a
password of their own, and a link that verified whoever opened it would
verify that account. It also proves only the address it was mailed to: once
the account's address is another, the page answers `409` and verifies
nothing, and the user asks for a new link. Verifying with one link spends
the account's other links. Verifying changes no credential and starts no
session. The mail comes from `MAIL_FROM`, and names the application as
`APP_NAME`.

A new API project's `.env` mails as the other kits' does: through a local
SMTP catcher on port 1025 (maildev, mailpit or mailhog), from
`hello@example.com`. Set a real relay and sender for production, which
refuses an unencrypted connection. When the link cannot be mailed,
registration still answers `202` and logs the failure, and the resend
route answers `500`.

#### Sessions on the consent pages only

`bootstrap.rs` registers `agents::ConsentSession`, which runs the session
middleware on these paths and no others:

- `/login`, the sign-in page, and its form.
- `/logout`, where the sign-out form posts.
- `/oauth/authorize`, `/oauth/approve` and `/oauth/authorizations/{id}`,
  where a user approves a client.
- `/oauth/connected` and the revoke forms under it.
- `/agent/claim` and its form.
- `/email/verify`, the page the verification link opens.

Every other route serves a client of the JSON API, an MCP client, or an
agent, which sign in with a bearer token: `/api/*`, `/mcp`, the
`/.well-known/*` documents, the token, registration and revocation
endpoints, and agent auth's JSON routes. None of them starts a session or
sets a cookie, so no client or agent can create a session row per request,
and the JSON sign-in at `POST /api/auth/login` returns a token and nothing
else. The migrator creates the `sessions` table, and the session settings
are the `SESSION_*` variables in [Environment variables](env-vars.md), with
two exceptions. A consent session ends after 15 minutes without a request
(`agents::CONSENT_SESSION_IDLE`), and its cookie has no expiry, so the
browser drops it when it closes. The kit adds no global CSRF middleware, so
the JSON API still takes no token.

A signed-in user ends the session at once with the **Sign out** form, which
posts to `/logout` with the session's CSRF token. The form is on the email
verification page, the page a signed-in user sees: `/login` sends a user who
is already signed in straight on. Signing out ends the session, and the next
approval or claim asks for a sign-in again.

`bootstrap.rs` also registers a `DatabaseUserProvider` over `app_users`, so
`Auth::user()` finds the user a bearer token or a consent page's session
signed in.

#### Connected agents over JSON

A client of the JSON API lists and revokes the same connections as the
connected-agents page. The routes sit behind `AuthMiddleware` beside the
user directory, and act for the user of the bearer token from
`POST /api/auth/login`:

| Route | Answer |
|---|---|
| `GET /api/connected-agents` | `200` with `{"clients": [...], "agents": [...]}`, each newest first |
| `DELETE /api/connected-agents/clients/{grant}` | `204`, or `404` with `{"message": "No such connected app."}` |
| `DELETE /api/connected-agents/agents/{registration}` | `204`, or `404` with `{"message": "No such connected agent."}` |

```bash
curl -H "Authorization: Bearer $TOKEN" -H "Accept: application/json" \
  http://localhost:8765/api/connected-agents
```

An id that belongs to another user answers `404`. An MCP access token is
not a sign-in to this API, so it gets `401` here. Revoking takes `DELETE`,
never `POST`: a plain form on another site can send `POST`, and the JSON
API runs no CSRF check. For the fields of each entry, see
[Connected agents](agents.md#connected-agents).

#### The signing key in production

`suprnova new` writes a signing key for agent auth into `.env` as
`AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`. A deploy that does not
copy `.env` must set both variables, and the application does not start
without them. To print a new pair, run `cargo run -- agents:key`. Outside
the local and testing environments, also set `APP_URL`: it is the OAuth
server's issuer. For more about keys, see
[Keys and operations](agents.md#keys-and-operations).

#### Why the kit ships no browser helpers

The Inertia kits ship browser helpers that offer a page's tools to an agent
in the browser (see [Tools in the browser](agents.md#tools-in-the-browser)).
The API kit has no pages of its own to offer tools on, so it ships none. An
application that adds a frontend adds the helpers with it.

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

### Agents in the Live kit

The kit serves the MCP endpoint, the OAuth server that MCP clients sign
users in through, and agent accounts under auth.md, all from
`src/agents.rs` (see [Agents](agents.md)). Three pages of its own let the
person decide what acts for them:

| Page | Route | What it shows |
|---|---|---|
| Approve an app | `/oauth/approve` | The client, its host, each permission it asks for and where the user returns, with **Allow** and **Deny** |
| Claim an agent | `/agent/claim` | The agent that asks to act as the user, the account it would act for, and a form that takes the code the agent gave them |
| Connected agents | `/connected-agents` | Every MCP client and agent that acts for the signed-in user, each with a **Revoke** button |

The header and the account menu link the connected-agents page. The paths
are the framework's defaults, so a link a client or an agent holds opens the
same page with or without the kit.

These pages are plain server-rendered documents with native forms, not
Live documents. They mount no island and load no Live runtime, so no agent
in the browser is offered a tool that approves or claims: consent comes
from the person. Each form posts the session's CSRF token.
`src/controllers/agents.rs` reads and records each decision with
`AgentApprovals`, `AgentClaims` and `ConnectedAgents`, which act for the
signed-in user only. Another user's authorization reads as ended, and
their grants as unknown.

A guest who opens the approval or the claim page signs in and comes back
to it. A guest who registers instead comes back too, in the same browser
session. The address is not carried across email verification, and the
claim page asks an unverified address to verify first. A Live action can
only redirect to a named route, so the sign-in and registration forms
continue through `/login/continue` (`login.continue`), which follows the
address the session holds. A `return_to` that names another host is
ignored.

The routes run `ConsentPageHeaders`. No page or refusal here is framed,
cached or named in a referrer, since their addresses carry the
authorization id or the claim's attempt token. The kit's other security
headers stay, nonce policy included. That policy carries
`form-action 'self'`, under which a browser blocks a redirect that answers
a form post on another origin. So **Allow** and **Deny** answer with a page
that sends the browser on to the client's redirect URI by a meta refresh,
with a link beside it. Every name, host and address a client, an agent or
the user chose sits in `<bdi>`, and no page shows a logo or links a page
the client chose.

`suprnova new` writes a signing key for agent auth into `.env`:
`AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`. The application does not
start without them. `.dockerignore` keeps `.env` out of the image, so every
deployed environment sets both itself, each with a key of its own, and
names its public URL in `APP_URL`, which is the OAuth server's issuer. This
command prints a new pair and writes nothing:

```bash
cargo run -- agents:key
```

`tests/documents.rs` renders the connected-agents page and the approval
page's refusal. A test process loads no `.env`, so the test calls
`suprnova::testing::install_test_agent_auth_key()` for a key made for the
process, and registers an `AppConfig` with a URL of its own. See
[Agents](agents.md#keys-and-operations) for rotating the key.

Like the Inertia kits, the kit ships two [tools](agents.md#tools) on the
signed-in user's own notes, from the same `src/controllers/note_tools.rs`,
with their scopes declared by [`tool_scopes!`](agents.md#declare-scopes) in
`src/agents.rs`:

| Tool | Route | Scope | What it does |
|---|---|---|---|
| `search_notes` | `GET /notes/search` | `notes:read`, "Read your notes" | Finds the user's notes by the words in their titles and text, best match first, or lists the newest. It takes `query`, up to 200 characters, and `limit`, 1 to 50 and 10 when left out. |
| `create_note` | `POST /notes/tools` | `notes:write`, "Write notes for you" | Writes a note for the user, with the note form's rules: a `title` of 1 to 255 characters and an optional `body` of up to 10000. It answers the note. |

Each call acts for one user: the one signed in, the one who approved the
client, or the one who claimed the agent. A client's token holds the scopes
the user allowed on the approval page, and an agent holds both once a user
claims it and none before, so an agent no user has claimed gets a 403 with
`insufficient_scope`. `search_notes` is marked `read_only` and
`untrusted_content`, because the note text it hands back is the user's, not
instructions for an agent.

The notes page is the one page that offers tools: its route names both with
`.with_tools(["search_notes", "create_note"])`, and the document's head
carries them in its `suprnova-agent-tools` element, which the Live runtime
registers with WebMCP. A guest is never offered either. After
`create_note`, the runtime renders the page's islands again, so the list
shows the new note.

To call the tools from Claude Code, run the project with `suprnova serve`
and add its endpoint, over the Streamable HTTP transport, at the URL
`APP_URL` names in `.env`:

```bash
claude mcp add my-app --transport http http://localhost:8765/mcp
```

Claude Code connects without a token and lists both notes tools. Calling
one answers 401, and Claude Code reports that the server needs you to sign
in; run `/mcp` to sign in. The client gets its token from the kit's OAuth
server: it registers, sends you to the approval page to allow the scopes it
asks for, and from then on lists the tools those scopes reach. See
[Users who approve a client](agents.md#users-who-approve-a-client).

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

The React, Vue and Svelte scaffolds also ship `tests/page_budget.rs`. It audits
`/`, `/login`, `/register`, `/forgot-password` and `/reset-password` through the
application router and middleware. It creates an isolated test database and
sets the test environment in a child process before runtime threads start.
The test follows built assets when a Vite manifest exists. A fresh scaffold
can run it before the frontend build. See [Page Budget](page-budget.md).

```bash
cargo test --test page_budget
```

The repository checks the generated tests and origin asset policy in
`suprnova-cli/tests/kit_budget.rs`. The dogfood application has its own
`app/tests/page_budget.rs` test over its public entry pages.

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
