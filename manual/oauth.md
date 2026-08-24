# OAuth, Apple, and magic-link login

Suprnova exposes OAuth, Sign in with Apple, and passwordless magic links through
the framework-owned `Auth` facade. Magnetar supplies the credential, ceremony,
identity, factor-gate, and session engines behind that facade.

The public entry points are:

- `Auth::oauth(provider)` for OAuth and Apple.
- `Auth::magic_link()` for passwordless email login.

Suprnova does not install routes for these flows. Applications provide small
start and callback handlers and decide how to deliver magic-link email.

## Initialize Magnetar

Initialize the default password, passkey, session, lockout, and two-factor
engines after `DB::init` and after `APP_KEY` has initialized `Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`MagnetarConfig` uses the application's SeaORM connection. The default engine
creates its schema when `apply_migrations` is enabled, which is the default.
Set `.apply_migrations(false)` only when deployment runs the same schema setup
separately.

`init_magnetar` installs password/session and passkey adapters atomically. A
second installation returns an error instead of replacing the engine and
splitting authentication state.

## OAuth engine installation

OAuth support is compiled in by the framework's default `magnetar-oauth`
feature, but provider registration is always an explicit runtime step. In a
`--no-default-features` build, enable `magnetar-oauth` explicitly.
`init_magnetar` does not return or expose its internal concrete host engine, so
the example below applies only to an application that constructs and retains
its own `MagnetarHostEngine`; it cannot be appended to the preceding
default-initialization example. The current public API has no convenience
method for adding an OAuth registry to an engine already installed through
`MagnetarConfig`.

```rust,ignore
use std::sync::Arc;
use suprnova::magnetar_integration::install_magnetar_oauth_engine;


// These values must be in the scope that constructed the custom host engine.
let oauth = host_engine.oauth_service(oauth_host_config)?;
install_magnetar_oauth_engine(Arc::new(oauth))?;
```

`MagnetarOAuthHostConfig` takes an explicit list of
`MagnetarOAuthProviderConfig` values, an HTTP transport, an abuse limiter,
authorization policy, and an auto-link policy. The provider registry becomes
authoritative when installed. An unknown provider fails closed instead of
falling through to another authentication implementation.

Provider implementations and their client-authentication dossiers come from
the `suprnova-magnetar` crate. Applications that construct the OAuth engine
must add that crate as a direct dependency with the provider features they use.
The framework does not infer OAuth client IDs or secrets from environment
variables. Read them through application configuration or a secret manager and
build the provider registry during bootstrap.

## Session binding

OAuth begin requires `SessionMiddleware`. Magnetar binds the ceremony to a
digest of the initiating framework session, so the callback cannot be moved to
another browser session.

Successful password, magic-link, passkey, and OAuth sign-in rotates the
framework session ID and CSRF token, records the application user ID, and stores
an opaque Magnetar web binding. Remember-me hydration rotates both the Magnetar
credential and the framework session binding.

## Start an OAuth flow

Use `begin` in the provider's start handler:

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// Return an HTTP redirect to kickoff.authorization_url.
```

The returned `OAuthKickoff` contains:

- `authorization_url`, the URL to send to the browser.
- `state`, the single-use selector bound to the initiating session.

Magnetar owns state generation, PKCE policy, ceremony persistence, provider
exchange, identity verification, and abuse limiting. The host controller owns
the HTTP redirect and callback route.

## Verify or complete the callback

The callback has two entry points:

| Method | Result | Side effects |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | Verifies the provider proof and returns the provider, subject, verified email, and display name without creating an application session. |
| `complete(code, state)` | `(User, Session)` | Resolves the identity through the installed host engine, applies account-link policy and the factor gate, rotates the framework session, and returns the framework-owned user and Magnetar session values. |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` is present only when the provider supplied a verified
email. Persist the provider and subject as the stable external identity. Email
is not a stable provider identifier.

## Account-link policy

OAuth completion does not treat possession of an unverified email string as
proof that the caller owns an existing application account.

The completion result can require more work instead of issuing a session:

- **Email completion required** returns HTTP 409 when the provider identity
  needs a separate verified-email ceremony.
- **Explicit link required** returns HTTP 409 when an existing verified account
  must authorize the link.
- **Factor required** returns HTTP 401 when account policy requires a second
  factor before session issuance.

A verified-email completion that wins the first-email-proof boundary reclaims
an unverified squatted account atomically. The transaction advances the auth
epoch, removes provisional credentials, revokes old sessions and remember
credentials, and attaches the verified provider account. A verified account is
never auto-linked by email alone.

## Sign in with Apple

Apple uses the same `Auth::oauth("apple")` facade, but its callback commonly
uses `response_mode=form_post`. Register the callback as a `POST` route and
pass the optional Apple `user` form field through the Apple-specific methods:

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` includes the stable subject, optional verified email,
`email_verified`, and `is_private_email`. Persist the subject as the stable key.
Apple can supply the display name only during the first authorization, so the
provider adapter must preserve that first `form_post` value.

Apple token and identity verification belongs to the installed provider
implementation. Current Magnetar providers require signature, issuer,
audience, expiry, and nonce checks rather than trusting an ID token's decoded
JSON.

## Magic-link login

Magic-link login uses the installed Magnetar password/session engine. The
framework returns the plaintext single-use token, while the application owns
mail composition and URL shape:

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` applies the authentication abuse budget before token issuance. `consume`
is single-use, applies the factor gate, binds the resulting session into the
framework request session, and returns the user and Magnetar session.

For an unverified pre-existing account, successful magic-link consumption is a
first email proof. The transaction reclaims the account and removes provisional
password, passkey, linked-account, two-factor, session, and remember state so a
prior squatter cannot retain access.

## Routes to add

A typical application adds these routes:

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

Apply `SessionMiddleware` to every OAuth and passkey start/callback route. The
session carries the ceremony selector and binds the round trip to the browser
that started it.

## Authentication migration

The `suprnova-magnetar` crate includes a shape-aware migration engine for
Torii, Suprnova web, Suprnova API, and existing Magnetar schemas. It is a
library surface and example, not a `suprnova` CLI subcommand.

Enable the `migration` feature plus the source database driver and run a dry
plan before applying. For PostgreSQL:

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

Use `seaorm-mysql` or `seaorm-sqlite` instead when that is the source and
application database driver.

Add `--apply` to apply the reviewed plan. The runner rechecks source and schema
fingerprints before import, records retry state, refuses identity collisions,
and uses transactional imports. MySQL same-database migrations use a
write-barrier-protected shadow swap with resumable restore and abort paths.

Keep the generated plan and report in deployment records. Do not apply a plan
whose source fingerprint changed after review.

## Reference

- Default boot: `MagnetarConfig`, `PasskeyConfig`, and `init_magnetar`.
- Facades: `Auth::oauth(provider)` and `Auth::magic_link()`.
- OAuth installation:
  `suprnova::magnetar_integration::install_magnetar_oauth_engine` and the
  configuration types in `suprnova::magnetar_integration::engine`.
- Migration library: `magnetar::migration` from the `suprnova-magnetar` crate.
- Bearer authentication: `BearerTokenMiddleware`.

## Next

- [Authentication](authentication.md) covers password, passkey, guards,
  framework sessions, and engine initialization.
- [Auth flows](auth-flows.md) covers email verification, password reset,
  lockout, and two-factor authentication.
- [Mail](mail.md) covers application-owned magic-link delivery.
- [Session](session.md) covers the browser session that binds OAuth and passkey
  ceremonies.
