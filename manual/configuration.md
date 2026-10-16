# Configuration

Suprnova reads configuration from environment variables (loaded from
`.env` in development, the process environment in production) and
exposes them to your code in two shapes:

1. **Direct env access** - `env::env`, `env_required`, `env_optional`
   for one-off lookups
2. **Typed config structs** - `Config::register` / `Config::get` for
   anything you read more than once, with strong typing

The framework reads a handful of env vars itself (`APP_KEY`,
`APP_ENV`, `DATABASE_URL`, etc.); the rest are yours.

## The `.env` file

`suprnova new` writes a starter `.env` with the values your app needs
to boot:

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detailed error pages + verbose logs
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Database - SQLite by default; swap to postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # set true in production (HTTPS only)
# SESSION_PATH=/             # unset: the public root of each request
SESSION_SAME_SITE=Lax

# Mail - defaults to `log` driver (writes outgoing mail to the
# tracing log, good for dev). Set MAIL_DRIVER to one of
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# for production.
MAIL_DRIVER=log
# SMTP credentials (only read when MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Left blank it derives from the credentials
# above - starttls with them, none without. Production refuses to boot
# unencrypted; see the Mail chapter.
MAIL_SMTP_ENCRYPTION=
```

A sibling `.env.example` ships the same keys with placeholder values -
commit it; do not commit `.env`. The default `.gitignore` excludes
`.env` already.

## How `.env` loading works

At boot, the framework:

1. Loads `.env` from the project root.
2. Detects the environment from `APP_ENV`, set in the process or in
   `.env` (case-insensitive, `prod`/`dev`/`stage`/`stg`/`test` are also
   recognised). **An unset `APP_ENV` is production**, as in Laravel.
3. Loads `.env.local` on top, if it exists.
4. If `APP_ENV` is set and a per-environment file exists
   (`.env.staging`, `.env.production`), loads it and then
   `.env.<environment>.local` on top - their values override `.env`. With
   `APP_ENV` unset no per-environment file loads, as Laravel's loader
   picks one only for a named environment.
5. Real process environment variables override all of them (this is what
   container orchestration relies on).

The order in one line: **process env > `.env.<environment>.local` >
`.env.<environment>` > `.env.local` > `.env`**.

```rust
use suprnova::Config;

// With the scaffold's APP_ENV=local:
let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

A process that sets no `APP_ENV` runs as production: debug is off, and
every production check runs, among them the `APP_KEY` refusal, the SQLite
fallback refusal, the Inertia manifest check, and the refusal of a mail
driver that delivers nothing, of the in-memory rate limiter, of an unknown
queue driver and of the mock payment provider. `db:seed` and
`migrate:fresh` ask for `--force`. The scaffold's `.env` sets
`APP_ENV=local`, so a new project runs as before; a deployment names its
environment or gets production. `Environment::detect_explicit()` answers
`None` when `APP_ENV` is unset, for code that needs to tell the two
apart.

In a CI run with `APP_ENV=testing`, the framework loads `.env.testing`
on top of `.env` so you can override DB URLs and disable mail drivers
without touching the dev `.env`.

`#[suprnova::main]` does this loading before it builds the Tokio
runtime, because writing the process environment is only sound while no
other thread can read it. `Config::init` and `config::load_dotenv`, the
functions it calls, refuse to run where they cannot be sound: they
return an error, and write nothing, when you call them from inside a
Tokio runtime, or again after `#[suprnova::main]` loaded the environment
and started its runtime threads. To load a changed `.env`, restart the
process. A file that fails to load leaves the process environment as it
was: the real process variables still win over what any file set before
the failure, and `Config::init` registers no config.

## Direct env access

For one-off reads of strings, numbers, bools - anything implementing
`std::str::FromStr` - use the `env::*` family:

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // with default
let url: String = env_required("APP_URL");                   // panics if missing - boot-only
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // None if missing
```

- `env(key, default)` - type-coerced read with fallback
- `env_required(key)` - panics if the key is missing or fails to
  parse. Only use this at boot time (in `bootstrap()` or `config::register()`)
  where a missing required value should crash the process immediately
- `env_optional(key)` - returns `Option<T>`; `None` for missing or
  unparseable values

Each unique key is also logged once on first read, so you can audit
exactly which env vars your app touches.

## Typed config structs

For anything your app reads more than once, define a typed struct
and register it. The pattern is:

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

Then read it anywhere with one line:

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

The registry is keyed by `TypeId`, so each struct is stored once.
Calling `Config::register` again with the same type replaces the
previous entry - convenient for tests.

### Defaults a crate registers

A crate that ships its own configuration registers it without
overwriting what the application set, whichever runs first.
`Config::register_default` registers a value only when none of its type
is registered, and answers whether it did:

```rust
use suprnova::Config;

#[derive(Clone)]
pub struct BillingConfig {
    pub currency: String,
}

// The application's config::register runs first...
Config::register(BillingConfig { currency: "EUR".into() });
// ...and the billing crate's defaults change nothing.
let registered = Config::register_default(BillingConfig { currency: "USD".into() });
assert!(!registered);
```

`Config::merge` is Laravel's `mergeConfigFrom`: it merges a map of
defaults under the registered map of the same type, keeping every key the
application set and adding the keys only the defaults have. With nothing
registered it registers the defaults.

```rust
use std::collections::BTreeMap;
use suprnova::Config;

Config::register(BTreeMap::from([("disk".to_string(), "s3".to_string())]));
Config::merge(BTreeMap::from([
    ("disk".to_string(), "local".to_string()),
    ("root".to_string(), "storage/app".to_string()),
]));
// disk = s3, root = storage/app
```

`Config::merge` takes `HashMap<String, V>`, `BTreeMap<String, V>` and
`serde_json::Map<String, Value>`. For a struct of your own, implement
`MergeConfig`, whose `merge_defaults(&mut self, defaults)` decides which
fields the defaults fill. Both calls check and write under one lock of the
registry, so two crates registering at the same time cannot both win.

### Why Suprnova diverges

Laravel's configuration is one tree of arrays, so `mergeConfigFrom`
merges by key. Suprnova's is a registry of typed values, one per type: the
merge applies to map-shaped values, and to any type that says how it
merges through `MergeConfig`. Like Laravel's `array_merge`, the merge is
shallow: a key the application set keeps its whole value.

### Wiring registration into your app

The scaffold's `cmd/main.rs` includes a `.config(…)` step in the
fluent boot pipeline:

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← this calls your registration
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` typically delegates to each section module:

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### Deserialising whole structs from env

For larger configs, you can deserialise directly from env vars via
`serde`. Suprnova exposes two helpers:

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// Reads SERVER_HOST / SERVER_PORT from the environment
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - deserialise from all process env vars
- `Config::resolve_prefixed::<T>("PREFIX_")` - deserialise only
  vars with the given prefix (the prefix is stripped before
  deserialisation)

Both return `Result<T, FrameworkError>` so a missing required field
surfaces as a `FrameworkError::Internal` carrying the envy diagnostic
instead of a panic.

## Environment-specific config

The `Environment` enum covers the standard set:

| Variant | Recognised `APP_ENV` values |
|---|---|
| `Local` | `local` |
| `Development` | `development`, `dev` |
| `Staging` | `staging`, `stage`, `stg` |
| `Production` | `production`, `prod`, or `APP_ENV` unset |
| `Testing` | `testing`, `test` |
| `Custom(String)` | anything else (preserves your casing, used for `.env.<custom>` lookup) |

Common branches:

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // strict cookies, real mail driver, etc.
}

if Config::is_debug() {
    // verbose error pages, query logging
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` returns `true` when `APP_DEBUG=true` is set explicitly,
or - when `APP_DEBUG` is unset - when the detected environment is
`Local`, `Development`, or `Testing`. Production (an unset `APP_ENV`
included), staging, and any unrecognised custom environment default to
`false`. Keep it off in
production; it controls error-page detail and a few internal defaults.

### `APP_KEY` is required in non-development

In production (any `APP_ENV` other than `local`/`development`/
`testing`, and an unset one), Suprnova requires `APP_KEY` to be set to a valid 32-byte
URL-safe base64 string. Booting without it fails closed with a
descriptive error message - there is no silent fallback.

If you don't have an `APP_KEY` yet:

```bash
suprnova key:generate          # prints the key with a hint reminding you to add it to .env
suprnova key:generate --show   # prints only the key, suitable for `APP_KEY=$(suprnova key:generate --show)`
```

Neither form edits `.env` for you - copy the printed key into your
`.env` (or your secrets manager) yourself.

For key rotation (where old encrypted data must still decrypt during
the migration window), see [Encryption](encryption.md#key-rotation).

## Configuration in tests

In tests, register config in the test setup rather than relying on
`.env`:

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … your test
}
```

The `#[suprnova_test]` attribute also sets up isolated container
state so concurrent tests don't see each other's bindings - see
[Testing](testing.md).

## Common env vars Suprnova reads

A non-exhaustive list - these are vars the framework itself looks at.
Your app reads more on top.

| Var | Default | What it does |
|---|---|---|
| `APP_NAME` | `"app"` | Logged at boot, used in some default error messages |
| `APP_ENV` | `production` | Drives `Environment::detect` and `.env.<suffix>` lookup; unset means production and loads no `.env.<suffix>` |
| `APP_DEBUG` | env-aware (`false` in production) | Verbose error pages + extra logging |
| `APP_URL` | `http://localhost:8765` | Base URL for absolute URL generation, signed URLs |
| `APP_KEY` | none (required in prod) | AES-256 key for `Crypt`, sessions, cursors |
| `APP_KEY_PREVIOUS` | none | Comma-separated previous keys for rotation (max 8) |
| `SERVER_HOST` | `127.0.0.1` | Bind address |
| `SERVER_PORT` | `8765` | Bind port |
| `DATABASE_URL` | none | Required if your app uses the database |
| `DB_MAX_CONNECTIONS` | `10` | sqlx pool max |
| `DB_MIN_CONNECTIONS` | `1` | sqlx pool min |
| `DB_CONNECT_TIMEOUT` | `30` (seconds) | sqlx pool connect timeout |
| `SESSION_LIFETIME` | `120` (minutes) | Session expiry |
| `SESSION_TOUCH_INTERVAL` | `300` (seconds) | Minimum sliding-expiry write cadence |
| `SESSION_GC_INTERVAL` | `3600` (seconds) | Supervised expired-session cleanup cadence |
| `SESSION_COOKIE` | `suprnova_session` | Cookie name |
| `SESSION_SECURE` | `true` | Set `Secure` cookie flag. Override to `false` for local-HTTP development. |
| `SESSION_SAME_SITE` | `Lax` | `Strict`, `Lax`, or `None` |
| `MAIL_DRIVER` | `log` | One of `smtp`, `ses`, `mailgun`, `postmark`, `sendgrid`, `resend`, `log`, `memory` |
| `CACHE_DRIVER` | `memory` | One of `memory`, `redis`, `database` |
| `QUEUE_DRIVER` | `memory` | One of `memory`, `sync`, `null`, `redis`, `database`, `sqs`, `failover`. An unknown value is a boot error in production; elsewhere it logs a warning and uses `memory` |
| `RATE_LIMIT_DRIVER` | `memory` | One of `memory`, `redis` |
| `LOG_FORMAT` | env-aware (`pretty` in dev/local, `json` in production) | `pretty` or `json` |
| `LOG_LEVEL` | `info` | A `tracing` filter directive; the eight PSR-3 names (`warning`, `notice`, `critical` and the rest) are accepted, and its bare level is the lowest level the file and stream channels keep |

The full audited list lives in [Environment Variables](env-vars.md).

## Next

- [Application Bootstrap](bootstrap.md) - where typed config registration
  is called from
- [Service Container](container.md) - how registered config is read
  alongside bound services
- [Environment Variables](env-vars.md) - the full reference list
- [Deployment](deployment.md) - production env setup
