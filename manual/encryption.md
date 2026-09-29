# Encryption

Suprnova ships application-level encryption as a process-wide facade
named `Crypt`. It encrypts strings or any `Serialize` value under
AES-256-GCM, keyed by your `APP_KEY`. Reach for it whenever you need
to put something sensitive into storage you don't fully trust - a
column, a cookie, a pagination cursor - and need to read it back
intact later.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
assert_eq!(plain, "ssn-123-45-6789");
```

The framework itself uses `Crypt` for encrypted cookies, encrypted
pagination cursors, 2FA secrets, recovery codes, and the
`AsEncrypted*` Eloquent casts. The same facade is available to your
code with no extra wiring once `APP_KEY` is configured (see
[configuration.md](configuration.md#the-env-file)).

## The wire format

`encrypt_string` and `encrypt` both return URL-safe base64 (no
padding) over `nonce || ciphertext_with_tag`:

```
base64url( [12-byte random nonce] || [ciphertext] || [16-byte GCM tag] )
```

Each call samples a fresh 12-byte nonce from the OS RNG, so two
encryptions of the same plaintext under the same key produce distinct
ciphertexts. There is no padding oracle to leak length information
beyond the plaintext itself.

The output is safe to put in URL query strings, JSON bodies, headers,
and cookies without further encoding. A minimum valid wire is 28 bytes
(12 nonce + 16 tag) - anything shorter is rejected up front.

## `APP_KEY` - the one secret that matters

Suprnova reads a single 32-byte symmetric key from the `APP_KEY`
environment variable. The expected format is URL-safe base64, no
padding, decoding to exactly 32 bytes (43 base64 characters):

```env
APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Generate one with the CLI:

```bash
suprnova key:generate
# Generated a new APP_KEY (AES-256, base64 URL-safe, no padding):
#
#     hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
#
# Add it to your .env (or your secrets manager):
#
#     APP_KEY=hQ7rW0X9_NkSi8Cw5fF8j6V_K6JzgB3y2Hq9LpL9-Wo
```

Or pipe straight into the environment:

```bash
echo "APP_KEY=$(suprnova key:generate --show)" >> .env
```

### Boot-time validation - fail closed

`Server::from_config` validates `APP_KEY` **on every boot**, not just
the first one. The rules:

| Environment | `APP_KEY` unset | `APP_KEY` malformed |
|---|---|---|
| `local`, `development`, `testing` | Generated transient key, warn in logs | Hard error - fails boot |
| `staging`, `production`, anything else | Hard error - fails boot | Hard error - fails boot |

A malformed key is **always** a hard error, even in `local` - better to
fail boot than mask a typo. A `Custom` environment value the framework
doesn't recognise (e.g. `APP_ENV=k8s`) is treated as production-like:
no `APP_KEY`, no boot.

The diagnostic points at the fix:

```
APP_KEY is required when APP_ENV=production. Generate one with
`suprnova key:generate` and set it in your environment (e.g. .env
or your secrets manager). Suprnova refuses to boot without an
encryption key outside of local/development/testing because session
cookies and pagination cursors would otherwise be unsigned and
forgeable.
```

## `CryptPurpose` - domain separation through AAD

Every `Crypt::*` call takes a `CryptPurpose`. The variant maps to a
stable byte label that is bound into the AES-GCM authentication tag
as Associated Data (AAD):

```rust
pub enum CryptPurpose {
    Cookie,            // bound to the cookie name, see below
    Cursor,            // suprnova:cursor:v1
    TwoFactorSecret,   // suprnova:2fa:secret:v1
    TwoFactorRecovery, // suprnova:2fa:recovery:v1
    Cast,              // suprnova:cast:v1
}
```

The label is **not** stored in the wire. GCM mixes the AAD into the
authentication tag without including it in the ciphertext, so:

- The on-wire format is unchanged - still
  `base64(nonce || ciphertext || tag)`.
- A wire produced under one purpose is **rejected** by
  any decrypt call that supplies a different purpose. The GCM tag
  check fails before any post-decrypt parsing runs.
- Adding a new surface (a future queue payload encryption, an
  encrypted file header) means adding a new variant - not changing
  the wire format.

```rust
use suprnova::{Crypt, CryptPurpose};

let wire = Crypt::encrypt_string(CryptPurpose::Cast, "ssn-123-45-6789")?;

// Same key, same wire, different purpose - fails.
let result = Crypt::decrypt_string(CryptPurpose::Cursor, &wire);
assert!(result.is_err());

// Same purpose - succeeds.
let plain = Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

`CryptPurpose::Cookie` is the one purpose that has no context-free form. A
cookie value is always bound to the name of its cookie (see
[Cookie-name-bound AAD](#cookie-name-bound-aad)). `Crypt::encrypt_string` and
`Crypt::encrypt` with `CryptPurpose::Cookie` return an error, and so do
`Crypt::decrypt_string`, `Crypt::decrypt_string_with_origin`, `Crypt::decrypt`
and `Crypt::decrypt_with_origin`. For a cookie, use `Cookie::encrypted` and
`Cookie::read_encrypted_for`, or the `_for` functions with the cookie name.

### Why Suprnova diverges

Laravel's `Crypt::encryptString` does not take a purpose. The single
`APP_KEY` is reused across cookies, signed URLs, signed expiry
tokens, and any user calls to `Crypt::encrypt`, with no domain
separation at the crypto layer. If two surfaces happen to accept
ciphertext of the same plaintext shape, a value minted for one
surface can be replayed into the other.

Suprnova reuses the same `APP_KEY` for the same reason - operators
manage one secret - but binds each surface to its own AAD label.
Cross-surface ciphertext replay is rejected at the GCM tag check,
before any parsing runs. The cost to the caller is one extra enum
parameter; the gain is a property the wire format alone cannot break.

The version suffix on each label gives every surface its own label. A change
of the label of one surface invalidates the old ciphertext of that surface
**only** and leaves cursors, 2FA secrets, and cast columns alone.

## Cookie-name-bound AAD

An encrypted cookie is bound to its logical name.
`Cookie::encrypted("suprnova_session", value)` binds
`suprnova:cookie:v2:suprnova_session` into the GCM tag, and
`Cookie::read_encrypted_for("suprnova_session", wire)` supplies the same
context on the way back:

```rust
use suprnova::Cookie;

let cookie = Cookie::encrypted("suprnova_session", "session-id")?;
let wire = cookie.value().to_string();
assert_eq!(
    Cookie::read_encrypted_for("suprnova_session", &wire)?,
    "session-id"
);
assert!(Cookie::read_encrypted_for("other_cookie", &wire).is_err());
```

The bound name is logical, not rendered. A later `__Host-` or `__Secure-`
wire-name prefix therefore does not change the AAD and does not log users out.
The prefix is a browser and header concern; the cookie name is the
cryptographic domain.

### A cookie opens under its own name and in no other way

The wire format has no version byte and no name. The reader supplies the name,
and the GCM tag check fails when it is not the name the writer used. A value
from one cookie therefore does not open in another cookie, and a value that was
written without a cookie name does not open in any cookie. Both fail with the
same error as a tampered value, and the error quotes neither the plaintext nor
the ciphertext.

Key rotation works for cookies as it does for every other purpose.
`Cookie::read_encrypted_for` tries the name-bound context across the whole key
ring, current key first. A cookie that a previous key wrote opens and reports
`KeyOrigin::Previous`.

The functions that take no name refuse `CryptPurpose::Cookie` before they touch
a key:

```rust,no_run
use suprnova::{Crypt, CryptPurpose};

assert!(Crypt::encrypt_string(CryptPurpose::Cookie, "session-id").is_err());
assert!(Crypt::decrypt_string(CryptPurpose::Cookie, "any-wire").is_err());

// With the name, a cookie value round-trips.
let wire = Crypt::encrypt_string_for(CryptPurpose::Cookie, "suprnova_session", "session-id")?;
let plain = Crypt::decrypt_string_for(CryptPurpose::Cookie, "suprnova_session", &wire)?;
assert_eq!(plain, "session-id");
```

The error names the function to use instead.

A cookie that does not open is a request without that cookie. It is never an
error response:

- The session middleware starts a new session. The user signs in again.
- The remember-me hydration does not sign the user in and clears the cookie.
- The maintenance mode treats the request as one without the bypass cookie, so
  it gets the maintenance response. The operator visits the secret URL again.

A cookie value that was written without a cookie name does not open, so the
users who hold one sign in again.

### Reading `DecryptOrigin`

Three functions return the value and a `DecryptOrigin`. Use them when your
code has to act on where a value came from. They serve the purposes other than
`CryptPurpose::Cookie`, and the two that take no context return an error for a
cookie:

| Function | Reads like |
|---|---|
| `Crypt::decrypt_string_with_origin(purpose, wire)` | `decrypt_string` |
| `Crypt::decrypt_string_for_with_origin(purpose, context, wire)` | `decrypt_string_for` |
| `Crypt::decrypt_with_origin::<T>(purpose, wire)` | `decrypt` |

Each returns `Result<(value, DecryptOrigin), FrameworkError>` and has the
errors of the function it reads like. None of them logs a rotation warning,
because you have the origin.

`DecryptOrigin` has two independent axes. `DecryptOrigin`, `KeyOrigin` and
`AadVersion` are exported from the crate root.

- `origin.key = KeyOrigin::Previous(index)` means the value still depends on
  `APP_KEY_PREVIOUS[index]`. Re-encrypt the value under the current key and
  remove that previous key only after the rotation tail is gone.
- `origin.aad = AadVersion::Legacy` means a `_for` read opened a value that was
  written without a context. That read falls back to the additional data
  without a name for the purposes that store values which do not expire as a
  cookie does, such as a cast or a cursor. Read the value with
  `decrypt_string_for_with_origin` and write it again with
  `encrypt_string_for`. A cookie never reports `Legacy`: it opens under its
  name or not at all.

Both axes can be stale together. `origin.needs_reencryption()` returns `true`
when either axis is stale, so you do not have to check both.

### Re-encrypting a value after a rotation

The job that ends a rotation reads every stored value, writes again the ones
that need it, and removes the previous key when none is left:

```rust,no_run
use suprnova::{Crypt, CryptPurpose, FrameworkError};

fn refresh(stored: &str) -> Result<Option<String>, FrameworkError> {
    let (plain, origin) = Crypt::decrypt_string_with_origin(CryptPurpose::Cast, stored)?;
    if origin.needs_reencryption() {
        return Ok(Some(Crypt::encrypt_string(CryptPurpose::Cast, &plain)?));
    }
    Ok(None)
}
```

Write the value again with the function that pairs with the one you read
it with:

- A value read with `decrypt_string_for_with_origin` is written by
  `Crypt::encrypt_string_for` with the same context. That gives it the
  current key and the current label.
- `Crypt::encrypt_string` writes the label without a context. Read that value
  with a context again and it matches the legacy label, so
  `needs_reencryption()` stays `true` and a job that runs until it is
  `false` never ends.

The plain `decrypt_string` logs a warning for each stale axis and returns the
value alone. The warning is for a person who reads the log. A program cannot
act on it.

## The two encrypt / decrypt pairs

There are two shapes for two use cases.

### Strings - `encrypt_string` / `decrypt_string`

For UTF-8 strings:

```rust
use suprnova::{Crypt, CryptPurpose};

let wire: String =
    Crypt::encrypt_string(CryptPurpose::Cast, "alice@example.com")?;

let plain: String =
    Crypt::decrypt_string(CryptPurpose::Cast, &wire)?;
```

The decrypt path returns a `String` - non-UTF-8 bytes (which a normal
encrypt run can't produce, but which a corrupt or attacker-supplied
wire might) surface as a clear `FrameworkError::Internal`.

### Anything `Serialize` - `encrypt` / `decrypt`

For structured values, JSON-encode-then-encrypt in one call:

```rust
use serde::{Serialize, Deserialize};
use suprnova::{Crypt, CryptPurpose};

#[derive(Serialize, Deserialize)]
struct Secret {
    api_key: String,
    last_rotated_at: chrono::DateTime<chrono::Utc>,
}

let value = Secret {
    api_key: "sk_live_…".into(),
    last_rotated_at: chrono::Utc::now(),
};

let wire = Crypt::encrypt(CryptPurpose::Cast, &value)?;
let round_trip: Secret = Crypt::decrypt(CryptPurpose::Cast, &wire)?;
```

The wire format is the same - base64 over `nonce || ciphertext ||
tag` - the only difference is that the plaintext is `serde_json` bytes
of `value` instead of UTF-8 of a string. Use this for any record
shape: a config blob, a session payload, a queue argument tuple.

### `appears_encrypted` - shape check, not tamper check

For middleware that needs to skip already-encrypted values on the
egress pass (matching Laravel's `EncryptCookies` behaviour),
`Crypt::appears_encrypted` does a cheap heuristic check:

```rust
if Crypt::appears_encrypted(cookie_value) {
    // pass through - already wrapped
} else {
    // encrypt before sending
}
```

It returns `true` when the input decodes as URL-safe base64 and the
decoded length is at least 28 bytes (nonce + tag). It never calls
into AES-GCM, so it **cannot** distinguish a valid ciphertext from
random bytes of the right shape. Callers that need authentication
must call `decrypt_string` / `decrypt` and handle the error.

## Key rotation - the keyring

Suprnova supports zero-downtime rotation through a key *ring*: one
current key (used for every new encryption) plus an ordered list of
previous keys (tried as fallbacks on decrypt). You roll `APP_KEY`
without re-encrypting every column in lock-step.

Set `APP_KEY_PREVIOUS` to a comma-separated list of base64 keys,
oldest to newest:

```env
APP_KEY=<new key>
APP_KEY_PREVIOUS=<old key>
# Or for multi-step rotation (older → newer):
APP_KEY_PREVIOUS=<oldest>,<middle>,<previous>
```

`APP_KEY_PREVIOUS` is Suprnova's canonical name.
`APP_PREVIOUS_KEYS` is accepted as a Laravel-compatible alias. If both
variables are set, `APP_KEY_PREVIOUS` wins. When their trimmed values
differ, boot logs a warning and ignores `APP_PREVIOUS_KEYS`.

Encryption **always** uses the current key. Decryption tries the
current key first; if that fails, each previous key is tried in
order. On a previous-key hit, `Crypt` emits a `tracing::warn!`:

```
WARN previous_index=0 Crypt decrypted a value with APP_KEY_PREVIOUS[0];
re-encrypt (load + save) this row under the current APP_KEY and remove
the corresponding APP_KEY_PREVIOUS entry once the rotation completes.
```

The log line deliberately excludes both the plaintext and the
ciphertext - only the fact-of-rotation plus an actionable hint
travels. Operators running a log search for `APP_KEY_PREVIOUS` land
on every column still depending on an old key.

### The cap - `MAX_PREVIOUS_KEYS = 8`

`APP_KEY_PREVIOUS` is capped at 8 entries. A realistic rotation chain
is 1-3 entries (one in-flight roll, maybe one stalled prior roll the
operator hasn't cleaned up); 8 leaves generous headroom. Past the
cap, boot **fails loudly** with a diagnostic that names both the
count and the cap:

```
APP_KEY_PREVIOUS holds 12 keys; the maximum is 8. A realistic
rotation chain is 1-3 entries - a longer list is almost always a
config-templating accident. Trim the list to the keys still needed
for in-flight rotation; once a re-encrypt job has migrated every
row off an old key, drop that entry.
```

Silent truncation would drop a key the operator may still depend on,
leaving columns undecryptable with no diagnostic. The hard cap is
intentional.

Empty entries are tolerated:
`APP_KEY_PREVIOUS=,,,old1,,,old2,,,` parses to two real keys. A
malformed entry (typo, wrong length, bad base64) is a hard error -
half-rotated secrets fail boot, not silently drop a fallback.

### Rotation procedure

```bash
# 1. Mint a new key.
NEW=$(suprnova key:generate --show)

# 2. Move the current key to APP_KEY_PREVIOUS, install the new one.
#    Edit your .env or secrets manager:
#
#      APP_KEY_PREVIOUS=<old_value_of_APP_KEY>
#      APP_KEY=<NEW>

# 3. Deploy. New writes use the new key; existing rows continue
#    to decrypt via the previous-key fallback. Logs identify
#    columns still on the old key.

# 4. Run a re-encrypt pass. For each model with encrypted casts:
#
#      User::query().chunk(500, |batch| async {
#          for mut row in batch { row.save().await?; }
#          Ok(())
#      }).await?;
#
#    `Cast::to_storage` always uses the current key, so a no-op
#    load-then-save migrates the row.

# 5. Once warnings stop appearing in logs, drop APP_KEY_PREVIOUS
#    and deploy again.
```

The whole procedure is online - at no point is there a window where
new requests fail.

### Observing the ring

For operator dashboards or health checks:

```rust
use suprnova::Crypt;

if Crypt::has_previous_keys() {
    let n = Crypt::previous_key_count();
    tracing::info!(previous_keys = n, "APP_KEY rotation in progress");
}
```

The key bytes themselves are never accessible from public API.
`EncryptionKey`'s `Debug` impl prints `"[REDACTED]"`, and there is no
accessor that surfaces a raw key outside of the crate.

## Eloquent integration - the `AsEncrypted*` casts

Application-level encryption is most useful at the column boundary.
The `AsEncrypted*` family of casts wraps `Crypt::encrypt_string` so
your model fields stay typed plaintext at runtime and ciphertext at
rest:

```rust
use suprnova::{model, Model};
use suprnova::eloquent::casts::{
    AsEncrypted, AsEncryptedArray, AsEncryptedObject, AsEncryptedCollection,
};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ApiKey {
    pub provider: String,
    pub secret: String,
}

#[model(table = "users", casts = {
    api_token     = AsEncrypted,
    api_keys      = AsEncryptedArray<ApiKey>,
    billing       = AsEncryptedObject<BillingDetails>,
    ssh_keys      = AsEncryptedCollection<String>,
})]
pub struct User {
    pub id: i64,
    pub api_token: String,
    pub api_keys: Vec<ApiKey>,
    pub billing: BillingDetails,
    pub ssh_keys: suprnova::eloquent::Collection<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

| Cast | Runtime type | Storage shape |
|---|---|---|
| `AsEncrypted` | `String` | encrypted string |
| `AsEncryptedArray<T>` | `Vec<T>` | JSON → encrypted string |
| `AsEncryptedObject<T>` | `T` | JSON → encrypted string |
| `AsEncryptedCollection<T>` | `Collection<T>` | JSON → encrypted string |

All four route through `CryptPurpose::Cast`. A wire minted by an
encrypted cast is rejected by any code that tries to decrypt it as a
cookie or cursor - even though `APP_KEY` is the same, the AAD label
differs.

For the full cast surface, table of failure modes, and re-encryption
recipes see [eloquent.md](eloquent.md). The encryption mechanics are
the same as the facade above - the cast is sugar that runs
`Crypt::encrypt_string(CryptPurpose::Cast, …)` on the storage
boundary.

### Encryption vs hashing - pick the right tool

`AsEncrypted` is **reversible**. The plaintext can be recovered with
`APP_KEY`. Use it for data your application needs to read back: API
tokens you display in a settings page, third-party secrets you
forward to upstream services, addresses you ship orders to.

For data your application only ever needs to *verify* - passwords,
API key prefixes you compare against incoming tokens - use a hash
instead. Hashes are one-way: there is no plaintext to leak even if
`APP_KEY` is compromised. See [hashing.md](hashing.md) for the
Bcrypt / Argon2id facade and the `AsHashed` cast.

## Where else `Crypt` is used inside the framework

You don't have to do anything to opt into these - they are wired
automatically once `APP_KEY` is configured.

- **Encrypted cookies** - `Cookie::encrypted(...)` /
  `Cookie::read_encrypted_for(...)` use `CryptPurpose::Cookie`. The
  session cookie, the remember-me cookie, and the maintenance-mode
  bypass cookie all ride this. See [responses.md](responses.md) and
  [session.md](session.md).
- **Cursor pagination** - `CursorPaginator` encodes the cursor under
  `CryptPurpose::Cursor` so the on-wire `?cursor=…` value cannot be
  forged or replayed across surfaces. See
  [eloquent.md](eloquent.md#cursor-pagination).
- **2FA secrets** - the encrypted base32 TOTP secret on
  `two_factor_authentications.secret` uses
  `CryptPurpose::TwoFactorSecret`; recovery codes use
  `CryptPurpose::TwoFactorRecovery`. Distinct purposes prevent
  within-row cross-column ciphertext replay. See
  [auth-flows.md](auth-flows.md).
- **HMAC-derived signing** - signed URLs and password-reset tokens
  derive an HMAC key from `APP_KEY` rather than encrypting under it.
  The raw key bytes are not exported; the derivation lives inside
  the framework. See [routing.md](routing.md#signed-urls).

## Testing with `Crypt`

The `Crypt` facade is `OnceLock`-backed, so the first installer in a
test binary wins. The testing helpers handle the boilerplate:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn encrypts_and_round_trips() {
    install_test_encryption_key(); // idempotent - safe to call from every test

    let wire = suprnova::Crypt::encrypt_string(
        suprnova::CryptPurpose::Cast,
        "hello",
    ).unwrap();

    let plain = suprnova::Crypt::decrypt_string(
        suprnova::CryptPurpose::Cast,
        &wire,
    ).unwrap();

    assert_eq!(plain, "hello");
}
```

The test key is deterministic, so tests can decrypt stable fixtures and
exercise rotation against a known key. Ciphertext strings must not be
compared for equality across calls or runs: every encryption still uses a
fresh random nonce.

For rotation tests, install a keyring directly and mint historical
ciphertext with `suprnova::crypto::testing::encrypt_string_under`. The
function is compiled with the `testing` feature:

```rust
use suprnova::testing::install_test_encryption_keyring;
use suprnova::EncryptionKey;

let current = EncryptionKey::generate();
let old = EncryptionKey::generate();

install_test_encryption_keyring(current, vec![old.clone()]);

// Simulate a value written when `old` was current.
let legacy_wire = suprnova::crypto::testing::encrypt_string_under(
    &old,
    suprnova::CryptPurpose::Cast,
    "legacy",
).unwrap();

// The current ring decrypts it via the previous-key fallback and
// reports the origin.
let (plain, origin) = suprnova::Crypt::decrypt_string_with_origin(
    suprnova::CryptPurpose::Cast,
    &legacy_wire,
).unwrap();

assert_eq!(plain, "legacy");
assert!(origin.needs_reencryption());
```

`encrypt_string_for_under(key, purpose, context, plaintext)` is the same for a
value that `encrypt_string_for` writes. Both helpers are compiled out of
production binaries when the `testing` feature is disabled
(`default-features = false`).

## Failure modes - what errors look like

Every fallible `Crypt::*` call returns `Result<_, FrameworkError>`.
The five errors you can see:

| Cause | Where | Surface |
|---|---|---|
| `Crypt` not initialised | Any call before boot | `FrameworkError::Internal("Crypt is not initialized - set APP_KEY before serving")` |
| Wire is not valid base64 | `decrypt_string`, `decrypt` | `FrameworkError::Internal("Crypt base64 decode failed: …")` |
| Wire too short (< 28 bytes) | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD wire too short …")` |
| Tag check fails - wrong key, wrong AAD, tampered bytes | `decrypt_string`, `decrypt` | `FrameworkError::Internal("AEAD decrypt failed: …")` |
| JSON encode / decode fails | `encrypt`, `decrypt` | `FrameworkError::Internal("Crypt JSON {encode,decode} failed: …")` |

There is no silent fallback to garbage. A wrong key against an
existing ciphertext is always a hard error, both at the facade
level and at the cast level. This matches Laravel's `Encrypter`
behaviour and is the property that lets rotation be safe: a missed
column would surface immediately, not return plausible-but-wrong
plaintext.

A decode error names the kind of mistake and its position, for example
`Crypt JSON decode failed: the value is not JSON (line 1, column 3)`. It
never quotes the decrypted value, because an error is logged and can reach a
client.

When a previous key successfully decrypts a wire, the call still
returns `Ok(...)` - but the `tracing::warn!` line fires alongside,
so log-driven alerting catches the rotation tail before
`APP_KEY_PREVIOUS` is removed. To act on it in code, read the origin with
[`decrypt_string_with_origin`](#reading-decryptorigin).

## Next

- [configuration.md](configuration.md) - `APP_KEY`, `APP_ENV`, and
  the rest of the boot environment.
- [eloquent.md](eloquent.md) - the `AsEncrypted*` casts, the full
  cast table, and rotation procedure for model columns.
- [hashing.md](hashing.md) - one-way alternative when you need to
  *verify* not *recover*; bcrypt and Argon2id facades plus
  `AsHashed`.
- [auth-flows.md](auth-flows.md) - 2FA secret and recovery code
  storage, which ride `Crypt` under their own purposes.
- [session.md](session.md) - the session cookie, encrypted and
  signed by `Crypt` via `CryptPurpose::Cookie`.
