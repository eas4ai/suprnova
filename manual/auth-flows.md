# Auth Flows

`suprnova::auth_flows` is the lifecycle layer on top of
[authentication](authentication.md). Where `auth::*` answers "who is this
request", `auth_flows::*` covers mailbox proof, password recovery, account
lockout, and framework TOTP challenges.

Five surfaces ship under the namespace:

- `EmailVerification` mints and consumes framework `auth_flow_tokens`, sends
  mail through the [`Mail`](mail.md) facade, and marks the authenticated token
  owner verified through the configured `UserProvider`.
- `PasswordReset` uses the installed Magnetar engine when available. Without
  Magnetar, verified accounts can reset through the configured `UserProvider`
  and framework `auth_flow_tokens`; unverified accounts fail closed because a
  generic provider cannot perform Magnetar's atomic first-email-proof policy.
- `BruteForce` and `LoginThrottleMiddleware` delegate account lockout state to
  the installed Magnetar engine.
- `TwoFactor` is the framework-owned TOTP facade over
  `two_factor_credentials`. It provides enrollment, confirmation, verification,
  recovery codes, secret rotation, challenge promotion, and timestep replay
  protection.
- `remember_me` re-exports the legacy framework remember module for namespace
  compatibility. When Magnetar is installed, normal `Auth` and
  `SessionMiddleware` remember flows use Magnetar credentials instead.

Two route-gate middleware ship in the same namespace:

- `EnsureEmailVerifiedMiddleware` composes after `AuthMiddleware` and gates
  routes on `email_verified_at`.
- `TwoFactorChallengeMiddleware` composes before `AuthMiddleware` and redirects
  a session with a pending framework TOTP challenge to the challenge form.

Transactional messages always use the framework [`Mail`](mail.md) facade.
Magnetar supplies security engines and storage contracts; it does not install a
second application mail transport.

### Where state lives

Email-verification tokens live in the framework's `auth_flow_tokens` table and
the verified timestamp is written through the configured `UserProvider`.
Verification is actor-bound: the current authenticated user must own the token.

Password-reset tokens, password credentials, lockout rows, opaque sessions,
remember credentials, passkey ceremonies, OAuth ceremonies, and auth epochs
belong to the installed Magnetar host engine. Password reset, magic link, and
OAuth verified-email completion share Magnetar's atomic first-email-proof
boundary for reclaiming unverified accounts.

The public `TwoFactor` facade in this chapter retains its framework-owned
`two_factor_credentials` schema. Magnetar also has a factor engine used by the
integrated password, magic-link, passkey, OAuth, and session flows. Do not
assume the two stores are interchangeable: use one enrollment surface
consistently for a given application.

Suprnova continues to own HTTP middleware, cookies, outbound mail, events, and
the `UserProvider` bridge. Application code uses framework facades rather than
calling storage engines directly.

## Failure semantics across flows

Every facade follows one ordering rule: the durable state change
commits first, then notification side effects fire. A listener panic, a
transient mail-transport failure, or a dispatcher error after the
mutation cannot roll the mutation back.

- `EmailVerification::verify` requires the authenticated token owner, consumes
  the token, and marks the user verified before firing `EmailVerified`.
- `PasswordReset::complete` commits through the installed Magnetar engine when
  available, including first-proof policy, auth-epoch advancement, and atomic
  revocation. The provider fallback is verified-account-only: it consumes the
  framework token, rotates the provider password, then reports framework
  session and remember revocation outcomes. Mail and events run afterward.
- `BruteForce::unlock_account` commits the unlock before firing
  `AccountUnlocked`.
- `TwoFactor::confirm` stamps `confirmed_at` before firing
  `TwoFactorEnrolled`; `TwoFactor::disable` deletes the row before
  firing `TwoFactorDisabled`; `TwoFactor::complete_challenge`
  promotes pending → authed before dispatching the standard
  `auth::Login` + `auth::Authenticated` pair followed by
  `TwoFactorChallenged`.

A listener that needs durability should buffer its work (queue a
job from the listener body); the facade itself never retries.

## Bootstrapping

Initialize Magnetar after `DB::init` and after `APP_KEY` has initialized
`Crypt`:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` creates the default auth schema unless migrations are disabled,
then installs password/session and passkey adapters atomically. Calling it a
second time returns an error. Tests that need process-global installation
should use a dedicated integration-test binary because an installed engine is
not replaceable.

### Email verification

Email verification needs:

1. A registered `UserProvider` that can retrieve users by email and mark the
   verification timestamp.
2. `MustVerifyEmail` on the application user type.
3. A nullable `email_verified_at` column.
4. The framework `auth_flow_tokens` table.

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

The verification handler must run inside authenticated session scope. A valid
token for another user is rejected without being consumed.

### Password reset and lockout

`BruteForce` requires the installed Magnetar password engine. Password reset
prefers that engine, but a provider-backed application can reset already
verified users without installing Magnetar when its `UserProvider` explicitly
supports password reset. `EloquentUserProvider<M>` opts in automatically when
`M` implements `MustVerifyEmail + CanResetPassword`. Unverified users receive
no reset link on the provider path; install Magnetar to use password reset as
an atomic first mailbox proof.

`MagnetarConfig::lockout_config` accepts
`magnetar::password::lockout::LockoutConfig`. The default policy enables
lockout after five failed attempts for 15 minutes, retains audit rows for seven
days, and fails closed when the lockout backend is unavailable.

Password reset normalizes an unknown or provider-backed unverified address to
`Ok(())` only after the abuse-limiter, mail configuration, provider/engine, and
storage checks succeed. Configuration and storage failures still surface.
Magnetar completion uses the atomic first-email-proof store. Provider fallback
completion returns a `PasswordResetOutcome` with explicit framework session and
remember-revocation results.

### Registering the 2FA migrations

The framework ships the schema; your app opts in by listing both
migrations in its own migrator:

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... your own migrations ...

            // Creates `two_factor_credentials`.
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // Adds `last_used_timestep` for TOTP replay protection.
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

Both are idempotent against an already-applied database (the v1 uses
`CREATE TABLE IF NOT EXISTS`; the v2 is a column add). Re-running
`suprnova migrate` against a production database that already has the
schema is a no-op.

### Environment

The transactional mailables read two environment variables at send
time:

| Var | Default | Used for |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | Subject branding and the `otpauth://` issuer label that authenticator apps display. |
| `MAIL_FROM` | none - **errors when unset** | Envelope `From` on every outgoing message. Set to a verified sender domain. |

`MAIL_FROM` deliberately has no default. Defaulting to a placeholder
like `noreply@example.com` would silently break DMARC / SPF in
production and ship from a domain the operator doesn't control, so the
facade fails closed instead. `EmailVerification::send_link` and
`PasswordReset::send_link` surface the error as `Err`;
`PasswordReset::complete` logs via `tracing::warn!` and continues
(the password change has already committed, so the notification path
cannot roll it back).

Apps additionally set `APP_URL` so controllers can derive the base URL
used in `send_link` calls; the framework facade itself takes the base
URL as a parameter.

The mail driver is configured separately via `MAIL_DRIVER` - see the
[Mail](mail.md) docs.

## Email Verification

`EmailVerification` mints, checks, and consumes verification tokens
against the `auth_flow_tokens` table and marks the user verified through
the configured provider. Four operations cover the lifecycle:

| Method | Signature | Notes |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | Mint + mail, given a user already in hand. |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | Normalizes an unknown provider result to `Ok(())`; token storage and mail failures still return `Err`, and execution time is not equalized. |
| `check` | `check(token: &str) -> Result<bool>` | Non-consuming - safe to call on a landing page. |
| `verify` | `verify(token: &str) -> Result<String>` | Actor-bound and single-use: the authenticated user must own the token; success consumes it, marks the user verified, and returns that user ID. |

```rust
use suprnova::auth_flows::EmailVerification;

// After a fresh signup, with the freshly-created user in hand:
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// Optional landing-page check - non-consuming, so a page refresh
// does not burn the token.
let valid: bool = EmailVerification::check(&token_str).await?;

// The click-through handler runs behind authentication. `verify`
// consumes the token only when `Auth::id()` matches its owner.
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` fires `EmailVerified` on success - listeners are the right
place to unlock additional functionality (welcome email, default
follows, "complete your profile" CTA) without coupling them to the
verification handler. The event carries the provider's user id.

### The resend endpoint (anti-enumeration)

`resend` takes only the email and looks up the user through the active
provider. An unknown provider result is normalized to `Ok(())`. For a known
account, the facade mints a token and sends the mail.
`EmailVerification::resend` likewise normalizes an unknown provider result to
`Ok(())`; it does not guarantee identical timing or identical behavior when
token storage or mail delivery fails. A handler can still return a neutral
message after either successful result:

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` performs the lookup and normalizes an unknown address to `Ok(())`.
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` and `resend` both build the URL as
`{base_url}?token={plaintext_token}`. A trailing slash on `base_url` is
trimmed before the query string is appended, so
`https://app.example.com/verify/` and `https://app.example.com/verify`
both produce a clean URL.

The click-through handler must run behind `AuthMiddleware`. It pulls the token
from the query string and calls `verify`:

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;
    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` checks `Auth::id()` against the token owner before consumption. A
token belonging to another account returns the same invalid-token response and
remains unused. On success, the provider marks the authenticated owner verified
and the facade fires `EmailVerified`.

### Verified-only routes: `EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` gates routes on the authenticated
user's `email_verified_at`. Compose it after `AuthMiddleware` and the
chain blocks any request whose user has not yet completed the verify
step.

The choice between **403 JSON** and **302 HTML redirect** is made at
route-registration time via the constructor - there is no
request-content sniffing, matching the pattern set by
`AuthMiddleware::new` / `AuthMiddleware::redirect_to`:

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// API surface - 403 with a JSON body.
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Web surface - 302 (or 409 + X-Inertia-Location for Inertia visits).
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

If no user is authenticated, the middleware falls into the same response
branch as "authed but not verified" - matching Laravel's
`! $request->user() || ! hasVerifiedEmail()` shape. Compose
`AuthMiddleware` first when you want a separate `401` for unauthed
requests.

For in-handler branching (e.g. conditionally rendering a "please
verify" CTA without redirecting), load the typed user through the
session guard and read the trait method:

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // branch on it
}
```

## Password Reset

`PasswordReset` has four operations:

| Method | Signature | Notes |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | Uses Magnetar when installed; otherwise issues a framework token only for a verified user from an explicitly reset-capable provider. Unknown and unverified addresses return `Ok(())`. |
| `check` | `check(token: &str) -> Result<bool>` | Non-consuming validation through Magnetar or the framework token store used by the provider fallback. |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | Runs Magnetar's atomic first-proof transaction when installed; otherwise rotates a verified provider user and revokes framework session/remember state. |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | Returns committed Magnetar counts or the provider fallback's explicit framework revocation outcomes. |

```rust
use suprnova::auth_flows::PasswordReset;

// From the "forgot password" form. An unknown address returns `Ok(())`
// after prerequisite checks succeed; configuration and backend errors still surface.
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// Optional landing-page check before rendering the new-password form.
let valid: bool = PasswordReset::check(&token).await?;

// The click-through handler, after the user submits a new password:
// consume the token + rotate the password, returning the user id.
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` passes the plaintext password through `SecretString`; Magnetar hashes
it inside the credential engine. Do not pre-hash it. An empty or whitespace-only
password returns HTTP 400 before the engine is called.

### Bounded anti-enumeration behavior

`PasswordReset::send_link` returns `Ok(())` for an unknown address only after
the abuse-limiter, mail configuration, engine, and storage checks succeed.
Configuration, limiter, storage, and mail failures still return `Err`. The
dogfood controller gives successful known- and unknown-account requests the
same HTTP status and body, but the implementation does not equalize their
execution time.

### `complete` side effects

Magnetar commits password reset in one transaction:

1. Consume the single-use reset token.
2. Apply first-email-proof policy when the account is still unverified.
3. Hash and replace the password.
4. Advance the authentication epoch.
5. Revoke old opaque sessions and remember credentials.
6. Remove provisional credentials when this reset is the account's first
   mailbox proof.

After commit, the framework sends `PasswordChangedMail` and dispatches
`PasswordResetCompleted`. Mail or listener failure cannot roll back the reset.

On an already verified account, reset preserves legitimate passkeys, linked
accounts, and confirmed two-factor enrollment. On an unverified squatted
account, first proof removes provisional credentials so the prior registrant
cannot retain access.

## Brute-Force Protection

The brute-force layer has two parts: the `BruteForce` facade that
records and queries lockout state, and the `LoginThrottleMiddleware`
that short-circuits at the HTTP layer before the handler is invoked.

### The `BruteForce` facade

Call `record_failed_attempt` from the failed-auth branch of your login
handler, and `reset_attempts` from the success branch:

```rust
use suprnova::auth_flows::BruteForce;

// In the failed-auth path:
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // Optionally surface a custom response. The middleware will do
    // this for you on the *next* request - see below.
}

// In the success path:
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` returns the updated `LockoutStatus`
(`is_locked`, `failed_attempts`, and `locked_until` when locked). Pass
the optional `ip` for audit logs; pass `None` if your transport doesn't
surface a client IP cleanly.

Two additional operations:

```rust
// Read-only - safe on emails with no history.
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// Admin / forced unlock. Fires `AccountUnlocked` only on a real state
// transition (no-op unlock on an already-unlocked account does not fire).
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

`unlock_account` returns `true` when the account had been locked at the
time of the call, `false` otherwise. The `AccountUnlocked` event fires
only on `true` - a `false` return is the no-op it is, not an audit
event.

### `LoginThrottleMiddleware`

The middleware reads the lockout state for whichever email a request is
targeting and short-circuits with `429 Too Many Requests` when the
account is locked. The login handler is never invoked, so a locked
account does not even get to attempt a credentials check:

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// The email extractor is a sync closure over `&Request`. Reading
// JSON/form body is async and consumes `Request`, so the closure
// cannot read the body - pull from a header, query string, or
// route param instead.
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

Practical extraction surfaces:

- A header (`X-Login-Email`), set by a preceding pre-processor - the
  pattern used in the dogfood app.
- A query string parameter (`?email=…`).
- A route parameter (`/login/{email}`).

Returning `None` from the extractor is the explicit "I have nothing to
check" signal - the middleware passes the request through unchanged.
This makes the middleware safe to install on routes that occasionally
see anonymous traffic (e.g. the same `POST /login` endpoint that also
handles a no-email "request password reset" sub-action).

On lock the middleware returns:

- Status `429 Too Many Requests`.
- `Retry-After` header - seconds, computed from the lockout's
  `locked_until` via `LockoutStatus::retry_after_seconds`. Falls back
  to `900` (15 minutes, Magnetar's default lockout period) if the
  timestamp is somehow absent.
- Body: `"Account locked due to too many failed login attempts. Try
  again later."`

### Backend errors (fail closed by default)

If `get_lockout_status` returns an error, `LoginThrottleMiddleware` logs the
failure and, by default, returns HTTP `503 Service Unavailable` with
`Retry-After: 1` without invoking the login handler. To keep login available
during a lockout-backend outage, opt in explicitly with
`.on_backend_error(BackendErrorPolicy::FailOpen)`; only that policy passes the
request to the handler.

### Layering with `RateLimitMiddleware`

`LoginThrottleMiddleware` is per-account - it gates a single email
when the threshold is crossed. For per-IP quotas, layer it with
[`RateLimitMiddleware`](rate-limiting.md). The two compose naturally:

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

Together they cover the realistic shapes of credential stuffing:
distributed (one email × many IPs) is the rate limit's job; focused
(many attempts × one email) is the throttle middleware's job.

### Configuration

`MagnetarConfig` accepts a `LockoutConfig`. The default is five failed attempts,
a 15-minute counting and lockout period, seven-day attempt retention, and
`BackendErrorPolicy::FailClosed`:

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

Use `LockoutConfig::disabled()` only when another fail-closed identity control
replaces account lockout.

## Two-Factor (TOTP)

`TwoFactor` covers TOTP-based 2FA - the kind that pairs with any
standards-compliant authenticator app (Google Authenticator, 1Password,
Bitwarden, Authy). The flow is enrollment → confirmation → ongoing
verification, plus single-use recovery codes for when the user loses
their device, plus the challenge flow that stitches everything into the
login lifecycle.

### The `TwoFactorUser` trait

The framework cannot reach into your application's user storage, so
callers implement a small trait to bridge from their user model to the
2FA facade:

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` is an opaque storage key. It can be a numeric application ID rendered
as text, a UUID, or a Magnetar `UserId`. The framework TOTP table has no foreign
key to the application user table.

`email` is folded into the `otpauth://` URL's `account_name` segment so the
authenticator app displays a recognizable account label.

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### Storage

2FA state lives in the framework-owned `two_factor_credentials` table.
Secrets and recovery codes are encrypted at rest with
`crate::crypto::Crypt::encrypt_string`, which requires a process-global
`EncryptionKey`. Apps opt into the schema by listing both migrations
in their `Migrator::migrations()` - see [Bootstrapping](#bootstrapping).

### Enroll, confirm, verify

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. Enrollment: generate a fresh secret + 10 recovery codes, persist
//    them encrypted, return everything needed to render the QR code.
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - `otpauth://totp/...` deep link
// response.qr_code_svg - <svg> wrapping a base64 PNG, embed inline
// response.recovery_codes - Vec<String>, 10 plaintext codes - show ONCE

// 2. Confirm: the user opens the authenticator app and types in the
//    6-digit code. `confirm` validates it and stamps `confirmed_at`.
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// fires `TwoFactorEnrolled`

// 3. On subsequent logins, gate the session on `verify`:
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` returns plaintext recovery codes **exactly once**. There is
no API to retrieve them later - the encrypted column is one-way from
this point on. Show them on the enrollment success page, encourage the
user to save them, and don't store the plaintext anywhere else.

`enroll` refuses to overwrite a **confirmed** enrollment - it returns a
`409` to push the caller toward `re_enroll`, which requires proof of
possession. Re-enrolling on an unconfirmed (pending) row is allowed:
the prior enrollment never became authoritative.

### Replay protection

`verify` writes the current TOTP timestep to `last_used_timestep` on
success. Subsequent verifies where `current_timestep <=
last_used_timestep` are rejected even when the code itself is
structurally valid, defeating a stolen-code replay inside the 30-second
window.

The timestep claim is atomic. The stamp lands via a conditional
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep <
:current`, and the verify only succeeds when the statement affects
exactly one row. Two concurrent verifies in the same timestep cannot
both win: the first flips the column, the second's predicate no
longer matches, and the second is treated as a replay. A plain
read-modify-write would be a TOCTOU race - both verifies read the
pre-stamp row, both validate the same code, both stamp, both succeed.
Concurrent racers are also counted as failed attempts so the
brute-force counter records them.

### Recovery codes

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

Single-use: a matching code is removed from the row before the call
returns, so a second attempt against the same code returns `false`.
Codes are 12 decimal digits in `NNNNNN-NNNNNN` shape (~40 bits of
entropy each, matching Laravel Fortify's format).

`consume_recovery_code` only accepts codes when 2FA is fully confirmed -
it short-circuits to `Ok(false)` while `confirmed_at` is NULL.
Without this gate, an attacker who triggered enrollment on a victim
account (or any flow that creates the row without confirming) could
authenticate using only a fresh recovery code, bypassing TOTP entirely.
The contract is symmetric with `verify`'s "confirmed enrollment only"
guard.

### Rotating recovery codes and secrets

When a user exhausts their recovery codes, or wants to rotate them
after a suspected compromise:

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` must validate as either a current TOTP code or an unused
recovery code. Without the proof check, a session-hijacked attacker
could silently blow away the legitimate user's recovery codes
(denial-of-service against account recovery). The fresh codes replace
the persisted set; the existing secret and `confirmed_at` are
preserved, so the user's authenticator app keeps working without
re-pairing. Errors:

- `400` - no confirmed enrollment exists; call `enroll`/`confirm` first.
- `401` - `proof` validates as neither a TOTP code nor an unused
  recovery code.
- `429` - the account is locked by brute-force throttling.

To rotate the **secret** (re-pair to a new device) without disabling
2FA first:

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

Same proof model as `regenerate_recovery_codes`. The row is rewritten
with a fresh secret + 10 fresh recovery codes; `confirmed_at` resets to
NULL so the user must `confirm` with a code from the new authenticator
before 2FA is active again.

### Disable

```rust
TwoFactor::disable(&user_2fa).await?;
// fires `TwoFactorDisabled` only if a row was removed
```

Idempotent: a disable on a user who never enrolled is not an error.
The `TwoFactorDisabled` event fires only on a real state transition,
so audit listeners see one entry per actual disable rather than one
per click on a no-op button.

### Challenge flow (gating login on the second factor)

The enroll / confirm / verify primitives are the building blocks; the
**challenge flow** stitches them into the login lifecycle so a user
with 2FA enabled cannot reach protected pages on password alone.

The flow:

1. Password login resolves a user.
2. If `TwoFactor::is_enabled_by_id(&user_id)` returns `true`, the login
   handler calls `TwoFactor::start_challenge(user_id, remember)` -
   that stashes the user-id as **pending** in the session, clears the
   fully-authenticated slot, revokes any remember-me cookie issued by
   `Auth::attempt`, and remembers whether the user opted into
   remember-me so the cookie can be re-issued after the challenge
   completes. `Auth::id()` returns `None` from this point until the
   challenge completes.
3. The handler redirects to a `/two-factor-challenge` route that shows
   the code form.
4. The challenge POST handler calls
   `TwoFactor::complete_challenge(code)` - verifies the code (TOTP
   **or** an unused recovery code, matching Fortify's challenge
   controller), promotes pending → authed, rotates the session id
   (defeating session fixation) and the CSRF token, re-issues the
   remember-me cookie when the user opted in, and dispatches the
   standard `auth::Login` + `auth::Authenticated` lifecycle events
   plus the 2FA-specific `TwoFactorChallenged`.

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // Demote to "pending": auth slot cleared, pending set,
                // remember-me cookie revoked. Pass through the form's
                // remember flag so `complete_challenge` can re-issue
                // the cookie on success.
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // Session id + CSRF have rotated; remember-me has been re-issued
    // if the original login form set it. Listeners that hook
    // `auth::Login` / `auth::Authenticated` saw a normal login.
    redirect!("/dashboard").into()
}
```

`complete_challenge` rotates the session id and CSRF token as part of
the promotion to authed. That closes the classic session-fixation
attack where an attacker plants a known session id on a victim before
they log in - after the rotation, the planted id is dead and only the
freshly-generated id carries the authenticated state. The contract
matches `Auth::login_id` / `Auth::login_using_id`, so 2FA logins are
indistinguishable from no-2FA logins in terms of session state and
listener observability.

Gate every protected route group with `TwoFactorChallengeMiddleware`
**before** `AuthMiddleware` so a pending session is bounced to the
challenge page rather than the login page:

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

The challenge page itself (the GET that renders the form, the POST
that calls `complete_challenge`) must NOT install
`TwoFactorChallengeMiddleware` - it is the destination. The POST
handler typically also checks `TwoFactor::pending_user_id().is_some()`
up front so a stale link does not reach the verify logic with an
empty session.

`TwoFactor::cancel_challenge()` clears both pending slots without
authenticating anyone - wire it to a "back to login" link on the
challenge page.

**Recovery code fallback.** `complete_challenge(code)` tries the TOTP
path first and falls back to consuming a recovery code, so a user who
lost their authenticator can still get in. Each recovery code is
single-use.

**Brute-force linkage.** Failed challenge codes feed the per-account
brute-force counter through `BruteForce::record_failed_attempt`, the
same way bare `TwoFactor::verify` does. An attacker grinding the
challenge form will trip `AccountLocked` after the configured
threshold. A single bad submission counts as **one** failed attempt
even though `complete_challenge` tries both the TOTP and recovery-code
paths internally - the silent-validation cores skip the brute-force
counter so the outer layer records the canonical attempt exactly once.

**Lockout gate.** `complete_challenge` checks `BruteForce::is_locked`
up front and returns `429 Too Many Requests` if the account is
already locked - even when the submitted code is correct. Without
this in-method gate an attacker who tripped the lockout could still
get in by submitting the right code on the next request: the
brute-force counter is keyed on the user's email but `verify` itself
doesn't consult it. The password path's `LoginThrottleMiddleware`
enforces the same constraint at the route layer; composing it in
front of the challenge POST route is fine - both gates are
idempotent.

**Failure event.** `complete_challenge` dispatches
`TwoFactorChallengeFailed { user_id }` on a bad code (or a locked
account), distinct from the password path's `auth::Failed`. Listeners
watching for "user tried 2FA and failed" subscribe to the new event;
listeners watching for "password didn't authenticate" stay on
`auth::Failed`. The two surfaces are kept separate so a 2FA mistype
does not look like a password failure to audit pipelines.

### Why Suprnova diverges

The framework TOTP `user_id` is a `String`. A fixed `i64`, UUID, or Magnetar
identifier type would tie the reusable facade to one application schema. The
string boundary lets an app choose any stable identifier at the cost of one
conversion at the call site.

Magnetar's integrated factor gate is separate from this retained facade. The
separation preserves compatibility for applications using
`two_factor_credentials`, but applications should not enroll the same account
through both stores.

## Remember-me

`suprnova::auth_flows::remember_me` re-exports the legacy
`suprnova::auth::remember` module for compatibility.

When Magnetar is installed, ordinary `Auth::attempt(..., true)`,
`Auth::issue_remember_cookie`, and `SessionMiddleware` hydration use Magnetar's
purpose-bound remember credentials. Magnetar stores verifier digests, checks the
auth epoch, rotates credentials on successful use, revokes them with the user
session, and reports replay or malformed-credential anomalies without exposing
the secret.

The browser-facing cookie remains owned by the framework. It is encrypted with
the logical `remember_me` name, follows `SESSION_COOKIE_PREFIX`, and is cleared
before backend revocation so a storage failure cannot leave the browser sending
the old credential.

The legacy database-row implementation remains available when no Magnetar
engine is installed. New applications should initialize Magnetar and treat the
legacy re-export as a transition surface.

## Events

Nine events fire across the flows, one per security-state transition:

| Event | Fired by | Carries |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` on success | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` on success - anti-enumeration silent for absent emails | `user_id: String`, `email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` on success | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` on the unlocked → locked transition | `email: String`, `failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` when an actual unlock occurred | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` on success | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` promoted pending → authed | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` rejected a bad code or refused a locked account | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` when a row was actually removed | `user_id: String` |

Every event is `Debug + Clone + 'static`, carries no sensitive data
(no plaintext tokens, no IPs), and uses stringy identifiers so
listeners can serialize them across task boundaries without leaking
type information from the user-storage backend.

### Listening

Subscribe via the standard event API - same surface as every other
in-process event:

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ... Slack notification, audit table append, etc.
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

Listeners run on Tokio's runtime and are dispatched in registration
order. See the [Events](events.md) chapter for the full surface.

## Testing

Three fakes cover the auth-flows surface, and they compose.

### `Mail::fake()`

Installs a process-local capture transport. Every send during the
guard's lifetime lands in an in-memory buffer instead of going out:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ... drive the flow ...
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` exposes `assert_sent`, `assert_not_sent`,
`assert_sent_count`, plus the raw `captured()` and `count()`
accessors. When the guard drops, the previously-bound transport is
restored - tests that interleave fakes with explicit transport
binding do not leak state.

### `EventFacade::fake()`

The same shape, but for events:

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ... drive the flow ...
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

The fake records dispatched events without invoking listeners, so a
listener that talks to an external service will not fire during the
test. The companion `assert_not_dispatched::<E>(pred)` asserts the
negative; `dispatched_count::<E>(pred)` returns the raw count for
finer-grained assertions.

### Integration tests for email verification and password reset

Email-verification tests create `auth_flow_tokens`, register a `UserProvider`,
establish the authenticated token owner, set `MAIL_FROM`, and drive the facade
under `Mail::fake()`.

Password-reset tests install a `MagnetarPasswordAuthEngine` test adapter and
assert issue, non-consuming check, atomic completion, session revocation, and
single-use behavior.

Canonical source examples are:

- `framework/tests/email_verify.rs` for actor-bound verification and
  single-use tokens.
- `framework/tests/password_reset.rs` for Magnetar delegation and completion
  outcomes.
- `framework/tests/magnetar_default_engine.rs` for real default-engine setup.
- `framework/tests/brute_force.rs` for lockout lifecycle.
- `framework/tests/two_factor_challenge_flow.rs` for the retained framework
  TOTP challenge flow.
- `framework/tests/magnetar_remember_middleware.rs` for remember rotation and
  dual-session binding.

Process-global Magnetar installation is intentionally one-shot. Put tests that
need different engines in separate integration-test binaries, or install one
test adapter once for the whole binary.


## Reference

| Symbol | Purpose |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`, `resend`, `check`, and actor-bound `verify`; `verify` returns the user ID. |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` for 403 JSON and `redirect_to(path)` for browser or Inertia redirects. |
| `suprnova::auth_flows::PasswordReset` | Magnetar-first reset with a verified-account `UserProvider` fallback over framework `auth_flow_tokens`. |
| `suprnova::MustVerifyEmail` | Application-user contract for the framework verification facade. |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | SeaORM table definition for framework verification tokens. |
| `suprnova::auth_flows::BruteForce` | Magnetar-backed account lockout facade. |
| `suprnova::auth_flows::LoginThrottleMiddleware` | HTTP middleware that returns 429 before the login handler when the account is locked. |
| `suprnova::auth_flows::TwoFactor` | Retained framework TOTP enrollment, verification, recovery, and challenge facade. |
| `suprnova::auth_flows::TwoFactorUser` | Application-user bridge for the framework TOTP facade. |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | Gate for sessions waiting on the framework TOTP challenge. |
| `suprnova::auth_flows::remember_me` | Compatibility re-export of the legacy framework remember module. |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | Default Magnetar engine configuration and one-shot installation. |
| `suprnova::auth_flows::events::*` | Authentication lifecycle events. |

## Next

- [Authentication](authentication.md) - guards, providers, the
  `Auth` facade, `AuthMiddleware`.
- [Mail](mail.md) - the transport layer the `send_link` calls
  dispatch through.
- [Events](events.md) - registering listeners for the nine
  auth-flow events.
- [Rate Limiting](rate-limiting.md) - pair
  `RateLimitMiddleware::ip_based` with `LoginThrottleMiddleware` for
  layered defence.
- [Session](session.md) - what `start_challenge` /
  `complete_challenge` touch when they rotate the session id.
