# Authentication

Suprnova ships a Laravel-shaped authentication system: a static `Auth`
facade, named guards resolved through an `AuthManager`, pluggable user
providers, an `Authenticatable` trait on your User model, and middleware
to gate routes. A scaffolded project boots with a session guard (`web`)
and a token guard (`api`) already wired against your typed `User`, so
login, registration, and protected routes work the day you run
`suprnova new`.

## The pieces

| Type | Role |
|---|---|
| `Auth` | Framework facade for guards plus Magnetar-backed password, magic-link, passkey, and OAuth operations |
| `MagnetarConfig` / `init_magnetar` | Compose and atomically install the default password, session, lockout, passkey, and factor engines |
| `Authenticatable` | Trait your application model implements; surfaces `get_auth_identifier() -> String` and the password hash |
| `UserProvider` | Trait that fetches application users; `EloquentUserProvider<M>` and `DatabaseUserProvider` ship built in |
| `AuthManager` | Holds the `AuthConfig` and registered providers; resolves named guards on demand |
| `SessionGuard` / `TokenGuard` | Framework stateful and stateless guard contracts |
| `BearerTokenMiddleware` | Resolves Magnetar bearer sessions into framework request authentication state |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | Route guards |
| `Credentials` | JSON-shaped credential map, typically `{ "email", "password" }` |

Framework guard/provider code lives in `framework/src/auth/`. The Magnetar host
adapters and facades live in `framework/src/magnetar_integration/`; the engine
crate lives in `crates/suprnova-magnetar/`. Higher-level email verification,
password reset, lockout, and TOTP flows live in `framework/src/auth_flows/` and
are covered by [Auth flows](auth-flows.md). OAuth, Apple, and magic-link login
are covered by [OAuth and passwordless login](oauth.md).

## Identifier model

The authenticated user's id flows through Suprnova as a `String`
end-to-end - session storage, [`UserProvider::retrieve_by_id`], the
remember-me table, every auth event. The canonical surface is
`Authenticatable::get_auth_identifier() -> String` (Laravel's
`getAuthIdentifier`). Numeric primary keys stringify trivially; UUIDs,
ULIDs, and opaque OAuth provider ids flow through unchanged.

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` is what the built-in providers verify a plaintext
password against via `hashing::verify_async`. Return `None` for users
that authenticate by other means (OAuth, passkey, magic link). The
`auth_identifier_name() -> &'static str` method (default `"id"`) names
the column the id lives in. The convenience `auth_identifier() -> i64`
default-parses the string id and falls back to `0` for non-numeric ids -
Suprnova itself never calls it; override only for integer-keyed models
that want to skip the parse.

### Why Suprnova diverges

Laravel's `getAuthIdentifier()` returns `mixed`. PHP doesn't care
whether the id is an int, a UUID string, or a stringly-typed primary
key from a legacy table. Rust needs a single concrete type the session,
the provider, and the events all agree on. `String` is the only choice
that accommodates every id shape without forcing the framework to know
which one your app uses. The `auth_identifier()` integer convenience
exists for the common case where your column is a `BIGINT`, but the
framework never depends on it - switch your `User` to a ULID tomorrow
and nothing in the auth stack notices.

## Wiring auth at boot

The Rust analogue of `config/auth.php` is an `AuthConfig` registered as
an `AuthManager` singleton on the container, plus a `UserProvider`
registered under a name. `bootstrap.rs` typically does both in two
lines:

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ... DB::init, SessionMiddleware install, etc.

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` reads the default guard from `AUTH_GUARD`
(default `"web"`) and ships with two named guards out of the box: a
`web` session guard and an `api` token guard, both backed by the
`"users"` provider. Apps that need more guards (separate `admins`
provider, distinct stateful and stateless guards) build the config
explicitly:

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## Initialize the Magnetar engine

The API starter initializes Magnetar after the database and `APP_KEY` are
ready:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

The default engine shares the application SeaORM connection and creates its
schema unless `.apply_migrations(false)` is selected. It installs the
password/session and passkey adapters atomically. Reinitialization returns an
error rather than replacing one adapter while another request still uses the
old store.

`MagnetarConfig` also accepts session, lockout, and two-factor policy values:

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

The default host binding uses the canonical `app_users` table with `i64`
application IDs. Magnetar's public `UserId` remains opaque at the facade
boundary; the default binding parses the stored identifier only where it
crosses into the application table.

### Magnetar-backed facade methods

The installed engine powers these framework-owned methods:

- `Auth::password().register(...)`.
- `Auth::password().authenticate(...)`.
- `Auth::magic_link().send(...)` and `.consume(...)`.
- `Auth::passkey().begin_registration(...)` and `.finish_registration(...)`.
- `Auth::passkey().begin_authentication(...)` and
  `.finish_authentication(...)`.
- `Auth::oauth(provider)` when an OAuth delegate is installed.
- Remember-me issuance, rotation, and revocation.
- Bearer-session lookup through `BearerTokenMiddleware`.
- `list_sessions`, `revoke_session`, and `revoke_all_sessions` in
  `suprnova::magnetar_integration`.

Successful sign-in rotates the framework session ID and CSRF token, stores the
application user ID, and records an opaque Magnetar web binding. The framework
continues to own HTTP middleware, cookies, mail, events, and its guard/provider
contracts.

The framework's own login paths check credentials outside Magnetar:
`Auth::login_id`, `Auth::attempt`, `Auth::login`, `Auth::login_using_id`, and
`TwoFactor::complete_challenge`. With the engine installed, every web login
needs a Magnetar session, so revocation and auth epochs reach it. For these
paths, `SessionMiddleware` issues the user's Magnetar session at the end of the
request and records its binding before it stores the session. The issuance
keeps Magnetar's factor policy: an account with a confirmed Magnetar second
factor is refused, because the framework login did not prove it. When the
engine refuses or fails, the request ends with a `500` and nothing is stored.
The browser is never told it signed in when the next request would sign it
out.

### Password authentication

Use the Magnetar password facade when the application wants the integrated
credential, lockout, factor-gate, and session path:

```rust,ignore
use suprnova::{Auth, HttpResponse, Registration};

// Both outcomes get the same answer, so the endpoint does not reveal which
// addresses already have an account.
let _registration: Registration = Auth::password()
    .register("alice@example.com", password)
    .await?;
let response = HttpResponse::json(serde_json::json!({
    "message": "Registration received. Sign in with your email and password."
}))
.status(202);

// Later, on the sign-in endpoint:
let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`register` returns a `Registration`. `Registration::Created(user)` carries the
new account. An address that already has an account returns
`Registration::Accepted`, which carries nothing: that account is neither
changed nor returned, so registration can never sign the requester in as its
owner. Signing a `Created` account in at once is safe, because its password is
the one just submitted, but that answer differs from the one `Accepted` can
give, so it shows that the address was free. Answer both variants the same way
when registration must not reveal which addresses have accounts.

### Why Suprnova diverges

Laravel's starter kits validate registration with a `unique:users` rule, which
answers "The email has already been taken" and so tells any visitor which
addresses have accounts. `register` instead reports an existing address as
`Registration::Accepted` and returns nothing about its account, so the
application can give one answer to every registration.

`authenticate` returns HTTP 401 errors for invalid credentials, lockout, or a
required second factor. Storage and engine failures remain server errors. The
method never returns password material.

### Passkeys

Passkey begin and finish calls require `SessionMiddleware` because the
single-use ceremony selector is stored in the framework session:

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

Registration follows the corresponding `begin_registration` and
`finish_registration` pair. Existing-account enrollment requires a verified
request actor and recent reauthentication through the plugin path; a bare user
ID in a legacy session is not promoted into a credential actor.

### First email proof and auth epochs

Magnetar treats the first successful mailbox proof on an unverified account as
an atomic credential boundary. Password reset, magic-link consumption, and
OAuth verified-email completion can win this boundary.

The transaction advances the account's authentication epoch, revokes old
sessions and remember credentials, and removes provisional credentials that a
squatter could have registered before the mailbox owner arrived. Password,
passkey, linked-account, and two-factor writes carry an actor snapshot and fail
if the account epoch changed while the operation was in flight.

For an already verified account, password reset preserves legitimate passkeys,
linked accounts, and two-factor enrollment while still rotating the password
and invalidating sessions. OAuth never auto-links an unverified existing
account by email alone; it requires verified-email completion or explicit
linking according to host policy.

### Direct Magnetar crate surface

Most applications stay on the framework facades. Applications building a
custom identity host can depend directly on `suprnova-magnetar` for:

- Framework-neutral plugin routes and effect handlers.
- Password and password-management plugins.
- Passkey and two-factor engines.
- OAuth authorization, grants, provider plugins, device authorization, and
  token-broker services.
- Opaque, JWT, remember, and grant session engines.
- Custom storage bindings and the default SeaORM schema.
- Shape-aware auth-data migration.

Direct use does not transfer HTTP or application-user ownership to Magnetar.
The host still maps wire requests, mail effects, application IDs, rate-limit
drivers, and session bindings into its own framework.

## The `Auth` facade

The static `Auth` facade is the Laravel-shaped surface you call from
controllers and middleware. The credential- and user-based methods
delegate to the **default guard** (whatever `AuthConfig::default_guard`
points at, default `"web"`); the synchronous `check`/`guest`/`id` reads
are the session-backed fast path and need no manager.

```rust
use suprnova::{Auth, Credentials};

// Validate credentials and log the user in. Fires Attempting → (Login +
// Authenticated), honours remember-me. Returns the resolved user, or
// None on bad credentials.
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// Log a known user in directly.
Auth::login(user, remember).await?;

// Log in by id without re-checking credentials (e.g. just-finished registration).
Auth::login_using_id(&id, remember).await?;

// Validate credentials without persisting a session (password-confirmation dialogs).
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// Authenticate for this request only - no session write. Laravel's `once`.
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// Session-backed fast path (no AuthManager required).
if Auth::check()    { /* authenticated */ }
if Auth::guest()    { /* not authenticated */ }
if let Some(id) = Auth::id() { /* string id */ }

// Whether the current user was authenticated by remember-me cookie this
// request. Laravel's `viaRemember()`.
if Auth::via_remember() { /* … */ }

// Resolve the current user (via the registered provider).
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// Tear down auth + revoke remember-me + rotate CSRF + fire Logout.
Auth::logout().await?;

// Full session destruction (regenerate id + flush + revoke remember-me + fire Logout).
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` returns the resolved user on success rather than a bare
`bool` - richer than Laravel's API, and saves the follow-up `Auth::user()`
call. `Ok(None)` means the credentials did not resolve a user; `Err`
means a database / hashing / configuration failure that needs to bubble.

If you have already verified a user's identity yourself and only want
to establish the session - say after an OAuth callback completes -
reach for the synchronous primitive:

```rust
// Sync, no provider, no AuthManager, no events. Returns Err when called
// outside a request scope (no SessionMiddleware installed) so a
// silently-dropped login can never look like success.
Auth::login_id(user.id.to_string())?;
```

`login_id` regenerates the session id (preventing session fixation) and
rotates the CSRF token, then writes the id into the session. It's
deliberately failure-loud: previous versions silently no-op'd outside a
session scope, and the audit fixed that - a "successful login" that
never landed is the kind of bug nothing else catches.

## `Auth::user()` and `user_as<T>`

`Auth::user()` returns the user behind the trait:

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

That trait object covers anyone who implements `Authenticatable`. To get
your concrete `User` back, downcast through `user_as::<T>()`:

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // Field access on the model directly.
    println!("Welcome, {}!", user.name);
}
```

`user_as` returns `Ok(None)` both when no user is authenticated *and*
when the resolved user isn't a `T` (e.g. an `Auth::set_user(...)` of
a different type elsewhere in the stack). Inside a request the user is
cached per-request, so calling `Auth::user()` repeatedly only hits the
provider once.

## Named guards

The bare `Auth::*` methods talk to the default guard. To act against a
specific guard, resolve it by name:

```rust
use suprnova::Auth;

// Read-only operations work on every driver.
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attempt need a stateful guard. Token guards fail loud here.
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` returns `Arc<dyn Guard>` (the read contract) and
`Auth::stateful_guard("name")` returns `Arc<dyn StatefulGuard>` (adds
`attempt`/`login`/`logout`). Asking for the stateful contract on a token
guard returns an error with a remediation message rather than silently
limiting the API.

## Guards of your application

The framework ships two guard drivers, `session` and `token`. For a
guard it does not ship (an API key, a client certificate, a single
sign-on cookie), you register a driver of your own and declare a guard
that uses it. `GuardDriver` has three variants: `Session`, `Token`, and
`Custom(String)`. It is not `Copy`, so borrow a driver or call
`.clone()` where you copied it, and give a `match` that names every
variant a `GuardDriver::Custom(_)` arm.

The name of a guard of your application cannot contain `:`. The manager
returns an error when you resolve such a guard, and `via_request` refuses
to register a resolver for it, before either changes anything. The rule
keeps two guards from attesting the same principal (see [Guards and
Live](#guards-and-live)). A session or token guard may have a `:` in its
name.

### `Auth::extend`

`Auth::extend(driver, factory)` registers the factory of a driver.
`GuardConfig::custom(driver, provider)` declares a guard of that driver:

```rust
use std::sync::Arc;
use suprnova::{Auth, AuthConfig, Guard, GuardConfig, TokenGuard};

let config = AuthConfig::from_env()
    .guard("partner", GuardConfig::custom("api_key", "partners"));

// The factory gets the name of the guard and its provider. It returns
// any `Guard`; `TokenGuard` stands in for your own type here.
Auth::extend("api_key", |_name, provider| {
    Ok(Arc::new(TokenGuard::new(provider)) as Arc<dyn Guard>)
})?;
```

The factory has the type
`Fn(&str, Arc<dyn UserProvider>) -> Result<Arc<dyn Guard>, FrameworkError>`.
The manager builds a guard each time you resolve it, so the factory runs
on every `Auth::guard("partner")`. Keep the state of a guard in
request-scoped state, not in the guard value.

If you resolve a guard whose driver has no factory, you get an error
that names the guard and the driver and tells you to call
`Auth::extend`. It never falls back to a built-in driver. Registering a
driver twice keeps the last factory, and `Auth::extend` never replaces
`session` or `token`. It refuses a driver name that starts with
`via_request:`, because that prefix belongs to `Auth::via_request`.

### `Auth::via_request`

Most custom guards read one thing from the request and look up a user.
`Auth::via_request(name, resolver)` is the short form for that. The
resolver gets the `&Request` and answers with the user (`Ok(Some(user))`),
with no user (`Ok(None)`), or with an error. You declare the guard with
the driver that `AuthManager::via_request_driver(name)` derives from its
name.

This example authenticates a partner from the `X-Api-Key` header, from
the config to the route:

```rust
use std::sync::Arc;
use suprnova::http::text;
use suprnova::{
    handler, App, Auth, AuthConfig, AuthManager, AuthMiddleware, GuardConfig,
    Request, Response, Router, UserProvider,
};

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // `PartnerProvider` is your `UserProvider`. Its
    // `retrieve_by_credentials` looks the partner up by a hash of the
    // key, so the code never compares a secret with `==`.
    let partners: Arc<dyn UserProvider> = Arc::new(PartnerProvider);

    // Declare the guard under the driver that `via_request` derives.
    let driver = AuthManager::via_request_driver("partner");
    let entry = GuardConfig::custom(driver, "partners");
    let config = AuthConfig::from_env().guard("partner", entry);
    App::singleton(AuthManager::new(config));
    Auth::register_provider("partners", partners.clone())?;

    // Read what the future needs from the request first, so the future
    // owns its data.
    Auth::via_request("partner", move |request| {
        let key = request.header("x-api-key").map(str::to_owned);
        let partners = partners.clone();
        Box::pin(async move {
            let Some(key) = key else { return Ok(None) };
            let credentials = serde_json::json!({ "api_key": key });
            partners.retrieve_by_credentials(&credentials).await
        })
    })?;
    Ok(())
}

#[handler]
pub async fn me(_req: Request) -> Response {
    match Auth::guard("partner")?.user().await? {
        Some(user) => text(format!("partner {}", user.get_auth_identifier())),
        None => text("no partner"),
    }
}

pub fn routes() -> Router {
    Router::new()
        .get("/partner/me", me)
        .middleware(AuthMiddleware::new().for_guard("partner"))
}
```

`GuestMiddleware::for_guard("partner")` runs the resolver the same way.

These rules hold for a guard of `Auth::via_request`:

- The resolver runs once for a request, inside the middleware for that
  guard. `AuthMiddleware::for_guard` and `GuestMiddleware::for_guard`
  run it, and the guard reads the answer.
- Outside that middleware, `Auth::guard("partner")` reports no user,
  as the token guard does without `BearerTokenMiddleware`. A handler
  that no such middleware protects sees a guest.
- An error of a factory or of a resolver fails the request with `500`.
  It is never a guest, and `AuthMiddleware::optional()` does not change
  that.
- The guard is read-only through the manager. `Auth::stateful_guard`
  returns an error for it, and so does `BasicAuthMiddleware::for_guard`.
- Each guard sees its own user. A guard of your application never reads
  the user of the default guard or of another guard, and
  `AuthMiddleware::for_guard` takes the principal of the request from
  the user of that guard.

`AuthManager::via_request(name, resolver)` is the method of the manager
behind `Auth::via_request`. It returns a `Result`, so end the call with
`?`.

The same rules hold for a guard that `Auth::extend` registers, except
that no resolver runs. A middleware earlier in the chain validates the
credential and records what it proved in request-scoped state, and your
guard reads that state, as the token guard reads the bearer id.

### A guard of your application as the default guard

A guard of your application is read-only through the manager. When you
declare one as the default guard, the functions of `Auth` that take no
guard name act as follows:

- `Auth::user` and the other functions that load the user ask that guard.
- `Auth::logout`, `Auth::login_id` and `Auth::login_remember` return an
  error and change nothing. So do the stateful functions that go through
  `Auth::stateful_guard`, such as `Auth::login` and `Auth::attempt`.
- `Auth::set_user` behaves as it does with the token guard: `Auth::id` and
  `Auth::check` see the user, and `Auth::user` still answers from the guard
  itself.
- `AuthMiddleware::new()`, with no `for_guard`, asks that guard, runs its
  resolver, and attests the user it returns.

A logout forgets the users that the guards of your application resolved in
the request, so the request does not sign itself back in through a later
middleware.

### Guards and Live

The principal that `AuthMiddleware` attests for a Live component is
`<guard>:<id>` for a guard of your application, and the bare id for a
session user. The same id under two guards of your application is two
principals, and web user `7` differs from partner `7`.

Because a session user attests its bare id, a session user id that has the
form `<guard>:<id>` attests the same principal as that guard's user. If the
session ids of your application can contain `:`, name your guards so that no
id starts with `<guard>:`.

Live's gated actions read the session identity, not the guard. See
[Live](live.md#security-boundaries).

### Why Suprnova diverges

In Laravel, the closure of `Auth::viaRequest` runs when the guard
resolves its user, and it gets the current request. A Rust guard cannot
reach the request from `user()`. So the middleware for the guard runs
the resolver with the request, and the guard reads the answer. That is
why the guard reports no user outside that middleware.

## User providers

A `UserProvider` tells the auth stack how to fetch and validate users.
Two providers ship built in, so the common case needs no custom
implementation:

- **`EloquentUserProvider<M>`** - resolves through a typed
  `#[suprnova::model]` `User` that is also `Authenticatable`. Looks up
  by primary key for ids, by `email` (default) for credentials.
- **`DatabaseUserProvider`** - resolves a raw table by name into a
  `GenericUser` (id + attribute map). Use it when you don't have or
  want a typed model.

Both filter credential lookups against an allowlist (default
`["email"]`) - a hostile credential map cannot inject extra `WHERE`
predicates. Customise the allowlist with `.credential_columns([...])`,
the lookup column with `.identifier_column("uuid")`, or the id-binding
strategy with `.with_id_parser(...)`.

To plug in a custom source (LDAP, an external API), implement
`UserProvider` directly. `retrieve_by_id` takes the identifier as
a `&str`:

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // … fetch from LDAP, return as Arc<dyn Authenticatable>
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials have trait defaults
    // that return None / false. Override them to support `Auth::attempt`
    // and `Auth::validate` against your source.
}
```

Register it on the manager:

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## Protecting routes

### `AuthMiddleware`

Gate authenticated-only routes. Unauthenticated requests are redirected
to a login page or receive `401`:

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` returns `401 Unauthorized` instead - best for
JSON APIs. `AuthMiddleware::redirect_to("/login")` issues a `302` for
regular requests and a `409 X-Inertia-Location` for Inertia requests
(which the Inertia client turns into a full-page visit). To gate on a
specific guard, chain `for_guard`:

```rust
// 401 unless the api guard is authenticated.
.middleware(AuthMiddleware::new().for_guard("api"))
```

A token guard (`for_guard("api")`) relies on whatever bearer-token
middleware runs earlier in the chain to populate the request's auth id;
without it the guard always reports unauthenticated.

A guard of your application (see [Guards of your
application](#guards-of-your-application)) decides alone. The user it
resolves is the principal of the request, and the middleware attests it as
`<guard>:<id>`. With no `for_guard`, `AuthMiddleware::new()` does the same
when the default guard is a guard of your application.

### `GuestMiddleware`

The inverse - for login and registration pages that authenticated users
shouldn't see:

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` works the same way as
`AuthMiddleware::for_guard`.

### `BasicAuthMiddleware`

HTTP Basic auth from the `Authorization: Basic` header against a
guard's provider:

```rust
use suprnova::BasicAuthMiddleware;

// Stateful - logs the user into the session on success (Laravel's `basic`).
.middleware(BasicAuthMiddleware::new())

// Stateless - authenticates for this request only (Laravel's `onceBasic`).
.middleware(BasicAuthMiddleware::once())
```

The decoded username is matched against the `field` credential (default
`"email"`); a missing, malformed, or invalid header returns `401` with
a `WWW-Authenticate: Basic realm="..."` challenge. Configure with
`.field(...)`, `.realm(...)`, and `.for_guard(...)`.

## Lifecycle events

The guards dispatch five lifecycle events. Listen for them via the
[`EventFacade`](events.md):

| Event | When |
|---|---|
| `Attempting` | a credential attempt begins (`attempt`/`once`) |
| `Authenticated` | a user is actively authenticated this request (`login`/`once`/`once_using_id`) |
| `Login` | a user is persisted to the session (`login`/successful `attempt`) |
| `Logout` | a user is logged out |
| `Failed` | a credential attempt fails (bad password or unknown id) |

Every event carries the guard name and a string user id - never the
plaintext password and never the raw credential map. `Authenticated`
fires only when a user is actively established, not on a passive
`Auth::user()` resolution off an existing session, so listeners don't
get a stream of duplicates on every authenticated request.

## The scaffolded login flow

`suprnova new` generates an authentication controller that uses
`Auth::attempt` against the registered provider. `FormRequest` and `Validate`
produce the `{ message, errors }` validation envelope. For an Inertia request,
the installed validation-redirect middleware turns that failure into an HTTP
`303 See Other` redirect back and flashes the errors for the originating page.
A non-Inertia client receives the HTTP `422 Unprocessable Entity` JSON
envelope:

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, Auth, Credentials, FormRequest,
    InertiaProps, Request, Response, Validate, ValidationErrors,
};

// No `errors` prop: the framework seeds `errors` on every Inertia page
// from the session-flashed validation bag, and an explicit prop of the
// same name would replace it.
#[derive(InertiaProps)]
pub struct LoginProps {}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps {})
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

Registration follows the same shape: validate the form, create the
user, mail a verification link with `EmailVerification::send_link`, then
`Auth::login(Arc::new(user), false).await?` logs the freshly created user
into the session, fires the `Login` event, and continues to
`/verify-email`. The generated `email_verification` and `password_reset`
controllers complete the account flows; see [Auth flows](auth-flows.md).

## The scaffolded `User` model

The generated `User` is a `#[suprnova::model]` that implements
`Authenticatable`. It also contains
`email_verified_at: Option<DateTime<Utc>>` and implements `MustVerifyEmail` and
`CanResetPassword`. Those bridges let `EloquentUserProvider<User>` mark email
verification and supply password-reset identity data. The excerpt below shows
only the guard-login fields and helpers; use the generated model template for
the complete auth-flow implementation. Its password helpers use the
[`hashing`](hashing.md) module:

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
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

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

The `hidden = ["password", "remember_token"]` attribute makes the model
skip those columns when serialising to JSON for the wire - they exist
on the struct but never leak through an Inertia response.

## Remember-me

When a Magnetar engine is installed, `Auth::attempt(credentials, true)` and
`Auth::issue_remember_cookie` issue purpose-bound Magnetar remember
credentials. The browser still receives the framework's encrypted
`remember_me` cookie, while Magnetar owns verifier storage, auth-epoch checks,
single-use rotation, anomaly handling, and revocation.

On a request without an active framework login, `SessionMiddleware` consumes
the cookie through the installed engine, rotates the remember credential,
issues a fresh Magnetar session, and binds both session layers. A stale auth
epoch, revoked account session, malformed credential, or replay does not
authenticate the request.

`Auth::revoke_remember_tokens()` invalidates every remember credential for the
current user. The clear cookie is queued before backend revocation, so the
browser drops its credential even when the storage operation fails.

When no Magnetar engine is installed, the framework retains the legacy
`remember_tokens` fallback for compatibility. New applications should
initialize Magnetar rather than relying on that fallback.

## Security guarantees

A short list of invariants the auth stack establishes:

- **`Auth::login_id` fails loud outside a request scope.** Previous
  versions silently dropped the session write; a "successful login"
  that never landed is the kind of bug nothing else catches.
- **Session id and CSRF token regenerate on every login.** Both
  `login_id` and the guard-backed `login`/`attempt` rotate them to
  prevent session fixation.
- **Logout clears auth state before revoking remember-me.** If the DB
  revoke fails, the session is already in a logged-out state, so a
  stale auth slot cannot survive a partial logout. The remember-me
  clear cookie is queued *before* the DB delete, so the browser drops
  the cookie even when the row delete fails (the prune sweep cleans up
  later).
- **Credential allowlists block injection.** Both built-in providers
  filter `retrieve_by_credentials` against `credential_columns`, so
  extra keys in an attacker-influenced credential map cannot become
  extra `WHERE` predicates.
- **Credential writes are actor-fenced.** Password, passkey, linked-account,
  two-factor, session, and remember mutations carry the user ID and auth epoch
  established by verified authentication. Revocation or a first-proof epoch
  change makes an in-flight stale write fail.
- **The first mailbox proof is atomic.** On an unverified account, password
  reset, magic-link consumption, or OAuth verified-email completion advances
  the auth epoch and removes provisional credentials in the same transaction.
  A concurrent squatter write cannot restore access after commit.
- **Email verification is actor-bound.** The framework verification facade
  requires an authenticated user whose ID matches the token owner. A token for
  another account is rejected without being consumed.
- **OAuth email is not account ownership.** An unverified existing account is
  never auto-linked from a provider email alone. Verified accounts require
  explicit linking; unverified accounts require the first-email-proof
  completion path.
- **Auth events never carry plaintext.** Guard name + string user id,
  nothing else. Failed-attempt tracking (email-keyed lockouts) belongs
  to `BruteForce` in [Auth Flows](auth-flows.md), not the lifecycle
  events.

The [Session](session.md) chapter covers the cookie configuration
(`SESSION_LIFETIME`, `SESSION_COOKIE`, `SESSION_SECURE`,
`SESSION_SAME_SITE`, and `SESSION_COOKIE_PREFIX`) that session-backed guards
inherit.

## Next

- [Auth flows](auth-flows.md) - email verification, password reset,
  Magnetar-backed account lockout, framework TOTP 2FA, and auth-flow events
- [OAuth and passwordless login](oauth.md) - Magnetar OAuth, Apple, magic
  links, provider policy, and auth-data migration
- [Authorization](authorization.md) - `Gate`, policies, and `Authorizable`
- [Session](session.md) - the browser session and cookie layer
- [CSRF protection](csrf.md) - state-changing request protection
- [Hashing](hashing.md) - bcrypt and Argon2 helpers
