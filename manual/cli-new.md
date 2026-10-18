# suprnova new

`suprnova new` scaffolds a Suprnova project - a fresh Cargo crate with
controllers, routes, migrations, a frontend, and a working auth flow
already wired together. The frontend is an Inertia SPA (Svelte, React, or
Vue) or the Live kit's server-rendered pages. Run it once per app, then
live in `suprnova serve` from there.

## Usage

```bash
suprnova new [name] [options]
```

If `name` is omitted, the interactive wizard prompts for it. The
name becomes the project directory, the Cargo package name (after
snake-casing), and the default `APP_NAME` in `.env`. Names must be
ASCII letters/digits/`-`/`_`, start with a letter, contain no path
separators or `..`, and be 64 characters or fewer.

## Options

| Option | Description |
|---|---|
| `--frontend <svelte\|react\|vue\|live>` | Pick the frontend non-interactively: an Inertia SPA (`svelte`, `react`, `vue`) or the [Live kit](#live-starter-kit) (`live`). Conflicts with `--api`. |
| `--api` | Scaffold a JSON:API-only project (no Inertia, no SPA, token auth instead of sessions). |
| `--no-interaction` | Skip all prompts and use defaults (name `my-suprnova-app`, frontend `svelte`, empty author/description). |
| `--no-git` | Skip `git init` in the new project. |
| `--with-portless` | Emit a `portless.json` so [`suprnova dev:tls`](dev-tls.md) can serve the app at `https://<name>.localhost`. Opt-in; changes nothing else. |

## Interactive mode

```bash
suprnova new my-app
```

The wizard asks four questions, in this order:

1. **Project name** - defaults to the directory argument (`my-app`)
2. **Description** - used as the Cargo package description
3. **Author** - used as the Cargo package author; defaults to your
   `git config user.name <name@email>` if set
4. **Frontend framework** - `Svelte (recommended)`, `React`, `Vue`, or
   `Live (server-rendered, no Node)`

After confirming, the scaffolder writes the project, runs `git init`
(unless `--no-git`), and prints the next steps:

```
Backend  http://localhost:8765
Frontend http://localhost:5765
```

A Live kit project runs one process, so it prints one address:

```
App http://localhost:8765
```

## Non-interactive mode

For CI, dotfiles, or scripted setup, pass `--no-interaction` plus the
flags you want to override:

```bash
suprnova new my-app --frontend svelte --no-interaction
```

Defaults under `--no-interaction`:

- Frontend: `svelte`
- Description: `"A web application built with Suprnova"`
- Author: empty
- Git: initialized

There are no `--description` or `--author` flags; those values are
only set via the interactive prompts or accept their defaults.

## Live starter kit

The Live kit is the frontend for an application without a Node toolchain:

```bash
suprnova new my-app --frontend live
```

It writes no `frontend/` directory, no `package.json`, and no Vite
config. Every page is a server-rendered Live document: a controller renders
an Askama view in one of two layouts, and the pages that act mount
[Live](live.md) components, such as the sign-in form, the dashboard's note
counts, and the notes list. The application serves the Live browser runtime
and the stylesheets and scripts of the shipped component library itself, so
`suprnova serve` starts the backend alone and there is nothing to build
before you deploy.

The kit keeps the backend the Inertia kits share: the `users` and `notes`
tables and models, the account flows, and the route names. The scaffold
installs the library components its views import under
`templates/suprnova-ui/`, the way `suprnova live:add` installs them, and
records them in `suprnova.toml`. `.env` sets `SUPRNOVA_FRONTEND=live`, so
`suprnova make:inertia` refuses in the project, and `suprnova docker:init`
writes a Dockerfile with no Node stage. For the pages, the components, and
how to add your own, see [the Live starter kit](starter-kits.md#the-live-starter-kit).

## API-only project

For service backends with no SPA, use `--api`:

```bash
suprnova new my-api --api
```

The API starter is significantly smaller: no `frontend/` directory, no
Inertia, no auth views, and a single-crate `src/main.rs` layout. It initializes
Magnetar against the shared SeaORM connection, creates the canonical
`app_users` model, installs `TimeoutMiddleware` and
`BearerTokenMiddleware`, and uses `Auth::password()` for registration and
login. `PASSKEY_RP_ID` and `PASSKEY_RP_ORIGIN` are read by the generated
bootstrap with local defaults.
The starter also includes a sample users controller and `UserResource` JSON
serializer and binds to port 8765 in `.env`.

The `app_users` migration creates the time columns as `timestamp with time
zone` on Postgres and `DATETIME` on MySQL and MariaDB: the `User` model and
Magnetar both read them as `DateTime<Utc>`, which Postgres decodes only from
`timestamp with time zone`. An API application generated before this change
has `timestamp` columns on Postgres, which neither can read. Convert them
once:

```sql
ALTER TABLE app_users
    ALTER COLUMN email_verified_at TYPE timestamptz USING email_verified_at AT TIME ZONE 'UTC',
    ALTER COLUMN locked_at TYPE timestamptz USING locked_at AT TIME ZONE 'UTC',
    ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC',
    ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';
```

`--api` is mutually exclusive with `--frontend`; passing both errors.
Under `--api`, only the project name is prompted - the
description/author/frontend prompts are skipped.

## What gets scaffolded

A full directory tour lives in [Directory Structure](structure.md);
the short version is:

- `cmd/main.rs` - binary entry; calls `Application::new()…run()`
- `src/` - controllers, actions, commands, config, middleware,
  models, migrations, plus `bootstrap.rs` and `routes.rs`. The
  generated `bootstrap.rs` wires the global middleware chain -
  logging, session, locale, the request timeout, CSRF, include
  parsing - and calls
  [`Inertia::install`](frontend-inertia-responses.md), which adds the
  Inertia protocol middlewares (asset-version `409`, `302 → 303` on
  `PUT`, `PATCH` and `DELETE` redirects). The asset version it advertises defaults to a
  hash of the Vite build manifest, so shipping a frontend build changes
  it automatically - see
  [Version detection](frontend-inertia-responses.md). The same call
  pins the frontend you scaffolded
  with, so the HTML shell loads that framework's Vite entry point;
  `.env` carries the matching `SUPRNOVA_FRONTEND` for the CLI's own
  generators
- `src/bin/console.rs` - the per-project `php artisan` analogue
- `frontend/` - Vite 8 + Tailwind v4 + your chosen framework, with
  Home / Dashboard / Login / Register / ForgotPassword / ResetPassword /
  VerifyEmail pages already wired through Inertia. The Live kit has no
  `frontend/`: its pages are views under `templates/live/` and its
  components live in `src/live/`
- the account flows: registration mails a verification link and
  continues to `/verify-email`, which resends and consumes it;
  `/forgot-password` mails a reset link to a verified address and
  `/reset-password` takes the new password. Mail leaves through the
  `MAIL_*` settings in `.env`, which point at a local catcher on port
  1025 (the Mailpit that `suprnova docker:compose --with-mailpit` adds);
  set `MAIL_DRIVER=log` to print each message, link included, to the
  server log instead
- `src/routes.rs` - the auth and account routes, and a static-file
  fallback that serves `public/` (the built frontend) in production
- `src/migrations/` - `users`, `sessions`, `remember_tokens`, and
  `auth_flow_tokens` tables ready to go
- `.env` - SQLite database by default, with a freshly-generated
  `APP_KEY` so the app boots without operator intervention
- `.gitignore`, `Cargo.toml`

### Why Suprnova diverges

Laravel ships with Blade and pulls a frontend in via Breeze/Jetstream
after the fact. Suprnova goes the other way: `suprnova new` always
scaffolds a complete frontend - a real SPA (Svelte/React/Vue on Inertia),
the server-rendered Live kit - or a real JSON:API project. Laravel's
Livewire starter kit still builds its assets with Vite; the Live kit needs
no build step, because the framework serves the Live runtime and the
component library's files.

The default frontend is **Svelte 5** (runes-on), not React. We picked
it because it's the lightest of the three at runtime and the closest
to the framework's "compile-time wins over runtime cleverness"
philosophy. React and Vue are equally first-class - pick what your
team knows.

## Distribution

The CLI itself ships via git, not crates.io (pre-launch):

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v4.3.0 suprnova-cli
```

`--force` on the same command updates an existing install. Scaffolded
projects depend on the framework crate the same way - a git
dependency in their `Cargo.toml`, pinned to the current release tag.
See [Installation](installation.md) for the full
toolchain prerequisites.

## Next

- [Installation](installation.md) - Rust/Node/DB prerequisites and
  toolchain setup
- [Directory Structure](structure.md) - what each scaffolded file
  does
- [Quickstart](quickstart.md) - first 5 minutes after `suprnova new`
- [suprnova serve](cli-serve.md) - the dev runner you'll use next
- [Console](console.md) - `cargo run --bin console` and the
  `#[command]` system
