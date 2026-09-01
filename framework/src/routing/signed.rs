//! Signed URL generation and verification.
//!
//! Laravel's `URL::signedRoute()` / `URL::temporarySignedRoute()` /
//! `URL::hasValidSignature()` family in `Illuminate/Routing/UrlGenerator.php`.
//! Suprnova's port lives here so the routing module owns the full
//! `route(name) → URL` surface end to end.
//!
//! ## Wire format
//!
//! Given a generated URL `/path?foo=1&bar=2` (after route-name substitution +
//! per-segment percent-encoding from [`crate::routing::route`]) and optional
//! expiration `expires_at` (epoch seconds):
//!
//! 1. Append `expires` if present: `?foo=1&bar=2&expires=1748800000`
//! 2. Sort query pairs lexicographically by `(key, value)` so equivalent
//!    URLs hash identically regardless of caller insertion order. Sorting
//!    on the value too - not the key alone - is what makes the order total
//!    when a key repeats.
//! 3. Build the canonical string `path?<sorted_kv>` (omit the `?` when no
//!    pairs exist). **Every pair is carried, including repeated keys.**
//!    Collapsing them into a map was SEC-04: the verifier hashed the last
//!    value for a repeated key while the handler read the first, so
//!    prepending a value to a legitimately signed URL left the signature
//!    intact and changed what the handler did.
//! 4. HMAC-SHA256 with the framework's APP_KEY; hex-encode the result.
//! 5. Append `&signature=<hex>` (or `?signature=<hex>` if no other params).
//!
//! Verification reverses the build: strip `signature`, recompute the HMAC over
//! the canonical form, and compare in constant time. Expired signatures
//! verify cleanly but report `expired` separately so callers can render a
//! refresh flow.
//!
//! ## Why HMAC over the path + sorted query
//!
//! - **Path** binds the URL to its route - switching `/orders/1` to
//!   `/orders/2` invalidates the signature even when query parameters match.
//! - **Sorted query** prevents trivial reorderings from producing different
//!   signatures for the same effective URL (matching Laravel's
//!   `ksort($queryString)` policy).
//! - **`expires` inside the signed payload** binds the expiration to the
//!   signature itself - a client cannot strip or extend the expiration
//!   without invalidating the HMAC.
//! - **HMAC-SHA256, hex** - a 32-byte digest rendered as 64 lowercase hex
//!   characters, the same primitive and encoding Laravel uses.
//!
//! ## Not byte-compatible with Laravel's default signatures
//!
//! Suprnova signs the **path + sorted query** (host-independent). Laravel's
//! default `UrlGenerator` signs the **absolute URL** - scheme, host, path,
//! and query together. Because the signed payloads differ, a signature
//! minted by one side will **not** verify on the other even when the
//! `APP_KEY` is identical: same primitive, different message. Signing
//! path+query keeps Suprnova links portable across hostnames (proxies,
//! preview domains, local vs. production) without re-minting, which is the
//! deliberate divergence - at the cost of cross-framework wire interchange.
//!
//! ## Key source
//!
//! [`signed_url_key`] resolves the signing key from the framework's
//! [`Crypt`][crate::crypto::Crypt] keyring. Laravel uses `APP_KEY` for both
//! encryption and URL signing; Suprnova does the same so users get one
//! rotation story.

use crate::FrameworkError;
use crate::crypto::Crypt;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Reserved query-parameter name for the signature value. Reserved
/// because we strip it on verification; a route that legitimately
/// expects a `signature` query param would collide.
pub const SIGNATURE_KEY: &str = "signature";

/// Reserved query-parameter name for the expiration timestamp (epoch
/// seconds). Same reservation rule as [`SIGNATURE_KEY`].
pub const EXPIRES_KEY: &str = "expires";

/// Outcome of [`verify_signature`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureVerdict {
    /// Signature is valid and (if present) not yet expired.
    Valid,
    /// Signature is structurally well-formed and matches the recomputed
    /// HMAC, but the `expires` timestamp is in the past.
    Expired,
    /// Signature is missing, malformed, or does not match the recomputed
    /// HMAC. Treat as untrusted - do not trust the embedded `expires`
    /// value either.
    Invalid,
}

impl SignatureVerdict {
    /// `true` when the URL is safe to act on. Equivalent to
    /// `matches!(self, SignatureVerdict::Valid)`.
    pub fn is_valid(self) -> bool {
        matches!(self, SignatureVerdict::Valid)
    }

    /// `true` when the signature was correct but the URL has expired.
    /// Useful for rendering "request a fresh link" UX.
    pub fn is_expired(self) -> bool {
        matches!(self, SignatureVerdict::Expired)
    }
}

/// Resolve the signing key for URL signatures.
///
/// Returns the active encryption key's raw 32 bytes. Falls back to a
/// `FrameworkError` if no key is installed - signed URLs are a
/// trust-boundary feature and silently signing with a missing key would
/// produce unverifiable links. The caller (route helpers, middleware)
/// should treat the error as a 500-equivalent boot misconfiguration.
fn signed_url_key() -> Result<Vec<u8>, FrameworkError> {
    if !Crypt::is_initialized() {
        return Err(FrameworkError::internal(
            "Cannot sign URLs: encryption key not installed. \
             Boot the framework via `Server::from_config(...)` so APP_KEY \
             is loaded before signed-URL helpers run.",
        ));
    }
    Crypt::current_key_bytes().ok_or_else(|| {
        FrameworkError::internal("Cannot sign URLs: active encryption key unavailable")
    })
}

/// Compute the HMAC-SHA256 over the canonical payload bytes and return
/// the hex-encoded digest. Pure function - no global state.
fn hmac_hex(key: &[u8], payload: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(key).expect("HMAC accepts any key length - input is fine");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

/// Decompose `url` into `(path, query_pairs)` where the path is everything
/// up to the first `?`. Fragment handling: a `#fragment` is dropped from the
/// canonical form because browsers never transmit it back to the server, so
/// signing over it would invalidate every link the moment a client adds an
/// anchor.
fn split_url(url: &str) -> (String, Vec<(String, String)>) {
    // Strip fragment first.
    let url = match url.find('#') {
        Some(i) => &url[..i],
        None => url,
    };
    match url.find('?') {
        Some(i) => {
            let path = url[..i].to_string();
            let pairs: Vec<(String, String)> =
                url::form_urlencoded::parse(&url.as_bytes()[i + 1..])
                    .into_owned()
                    .collect();
            (path, pairs)
        }
        None => (url.to_string(), Vec::new()),
    }
}

/// Reassemble `path` + sorted query pairs back into a canonical URL string.
///
/// `pairs` must already be sorted by [`sort_pairs`].
///
/// This takes a **slice, not a map**, and that is the whole point. It used
/// to take a `BTreeMap<String, String>`, which silently kept only the last
/// value for a repeated key. The verifier therefore hashed the last value
/// while [`crate::http::Request::query_param`] handed the handler the
/// first, so an attacker could prepend their own value to a legitimately
/// signed URL and have the signature still verify (SEC-04):
///
/// ```text
/// signed:    /promote?user=victim
/// attacked:  /promote?user=attacker&user=victim&signature=<unchanged>
/// ```
///
/// Carrying every pair means the signature covers the exact multiset of
/// parameters, so adding a value changes the payload and breaks the HMAC.
///
/// For a URL with no repeated keys this emits byte-identical output to the
/// map version it replaces, so signatures minted before the fix still
/// verify - the format did not change, only what it refuses to lose.
fn canonicalize(path: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return path.to_string();
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        serializer.append_pair(k, v);
    }
    let mut out = String::with_capacity(path.len() + 64);
    out.push_str(path);
    out.push('?');
    out.push_str(&serializer.finish());
    out
}

/// Order query pairs into the canonical sequence both signing and
/// verification hash over.
///
/// Sorted by `(key, value)` rather than by key alone: sorting by key leaves
/// repeated keys in caller order, which would make the signature depend on
/// something a proxy is free to reorder. With the value as tiebreak the
/// ordering is total, so equivalent URLs always canonicalise identically.
fn sort_pairs(pairs: &mut [(String, String)]) {
    pairs.sort();
}

/// Sign a URL with the framework signing key.
///
/// Returns the URL with a `signature` (and optional `expires`) query
/// parameter appended. The input may already contain query parameters;
/// they're preserved alphabetically alongside any new ones.
///
/// `expires_at_epoch_seconds = Some(ts)` produces a temporary signed URL;
/// `None` produces a permanent signed URL.
///
/// # Errors
///
/// Returns `FrameworkError` when the encryption key is not installed
/// (see `signed_url_key`).
pub fn sign_url(
    url: &str,
    expires_at_epoch_seconds: Option<i64>,
) -> Result<String, FrameworkError> {
    let key = signed_url_key()?;
    let (path, mut pairs) = split_url(url);

    // Strip any pre-existing `signature` so we never sign-over-sign;
    // strip pre-existing `expires` so the caller's argument wins.
    pairs.retain(|(k, _)| k != SIGNATURE_KEY && k != EXPIRES_KEY);

    if let Some(ts) = expires_at_epoch_seconds {
        pairs.push((EXPIRES_KEY.to_string(), ts.to_string()));
    }

    // Canonical order. Every pair survives, including repeated keys - a
    // caller signing `?tag=a&tag=b` gets both values covered by the HMAC
    // rather than silently losing one.
    sort_pairs(&mut pairs);
    let canonical = canonicalize(&path, &pairs);

    let signature = hmac_hex(&key, canonical.as_bytes());

    // Append signature OUTSIDE the canonicalised payload - verifiers
    // recompute over everything except `signature`, so position is
    // semantically irrelevant; we append last for human readability.
    let mut out = canonical;
    if pairs.is_empty() {
        out.push('?');
    } else {
        out.push('&');
    }
    out.push_str(SIGNATURE_KEY);
    out.push('=');
    out.push_str(&signature);
    Ok(out)
}

/// Verify a signed URL.
///
/// Reverses [`sign_url`]: strip the `signature` query parameter, recompute
/// the HMAC over the canonical form, and compare in constant time.
///
/// Behaviour:
/// - Returns [`SignatureVerdict::Invalid`] when `signature` is missing,
///   malformed (non-hex, wrong length), or does not match the recomputed
///   HMAC under the current key OR any `APP_KEY_PREVIOUS` entry.
/// - Returns [`SignatureVerdict::Expired`] when some key in the ring
///   produces a matching HMAC but the embedded `expires` value is in
///   the past relative to `now_epoch_seconds`.
/// - Returns [`SignatureVerdict::Valid`] otherwise.
///
/// ## Key rotation
///
/// The current key is tried first; on a mismatch, each `APP_KEY_PREVIOUS`
/// entry is tried in registration order (mirroring the AEAD rotation
/// fallback in [`crate::crypto::Crypt::decrypt_string`]). A previous-key
/// hit emits a `tracing::warn!` carrying the zero-based ring index so an
/// operator running a log search for "APP_KEY_PREVIOUS" sees one
/// consistent rotation-in-progress signal across the crypto surface,
/// then continues to validate / expire-check the URL normally. This
/// keeps outstanding signed URLs verifiable across an `APP_KEY` flip;
/// without the fallback every minted link would invalidate the instant
/// the operator rotates.
///
/// Pass `now_epoch_seconds` so the caller controls the clock (testability +
/// monotonic-test parity with Laravel's `Carbon::now()->getTimestamp()` in
/// `UrlGenerator::signatureHasNotExpired`).
///
/// # Errors
///
/// Returns `FrameworkError` when the encryption key is not installed.
pub fn verify_signature(
    url: &str,
    now_epoch_seconds: i64,
) -> Result<SignatureVerdict, FrameworkError> {
    let current_key = signed_url_key()?;
    let previous_keys = Crypt::previous_key_bytes();
    Ok(verify_signature_with_keys(
        url,
        now_epoch_seconds,
        &current_key,
        &previous_keys,
    ))
}

/// Pure verification primitive. Takes the keyring explicitly so the
/// rotation fallback is exercisable from a unit test without seeding
/// the process-global [`Crypt`] ring (which is sealed by OnceLock at
/// boot and not safe to mutate from a test that races with other
/// `Crypt::init` callers in the same lib-test binary).
///
/// Tries `current_key` first. If that misses, walks `previous_keys`
/// in order - mirroring [`crate::crypto::Crypt::decrypt_string`]'s
/// fallback - and emits a single `tracing::warn!` with the matching
/// previous-ring index on a hit so an operator running a log search
/// for "APP_KEY_PREVIOUS" sees one consistent rotation-in-progress
/// signal across the crypto surface.
fn verify_signature_with_keys(
    url: &str,
    now_epoch_seconds: i64,
    current_key: &[u8],
    previous_keys: &[Vec<u8>],
) -> SignatureVerdict {
    let (path, pairs) = split_url(url);

    // Extract the candidate signature and the expires value.
    let mut sig: Option<String> = None;
    let mut expires: Option<i64> = None;
    let mut rest: Vec<(String, String)> = Vec::with_capacity(pairs.len());
    let mut signature_count = 0usize;
    let mut expires_count = 0usize;
    for (k, v) in pairs {
        if k == SIGNATURE_KEY {
            signature_count += 1;
            sig = Some(v);
        } else {
            if k == EXPIRES_KEY {
                expires_count += 1;
                expires = v.parse::<i64>().ok();
            }
            rest.push((k, v));
        }
    }

    // A repeated control parameter has no legitimate meaning and leaves
    // the authoritative value ambiguous - whichever one we picked would be
    // a guess about what the *handler* will read. Refuse instead of
    // guessing. Ordinary parameters may legitimately repeat and are
    // covered by the canonical payload below.
    if signature_count > 1 || expires_count > 1 {
        return SignatureVerdict::Invalid;
    }

    let Some(sig) = sig else {
        return SignatureVerdict::Invalid;
    };

    // Canonical recomputation. Build once - the payload is identical
    // across every key in the ring.
    sort_pairs(&mut rest);
    let canonical = canonicalize(&path, &rest);

    // Current key first.
    let expected_current = hmac_hex(current_key, canonical.as_bytes());
    if signatures_match(&sig, &expected_current) {
        return verdict_for_expiry(expires, now_epoch_seconds);
    }

    // Walk APP_KEY_PREVIOUS in ring order.
    for (index, key) in previous_keys.iter().enumerate() {
        let expected = hmac_hex(key, canonical.as_bytes());
        if signatures_match(&sig, &expected) {
            tracing::warn!(
                previous_index = index,
                "signed URL verified against APP_KEY_PREVIOUS[{index}]; \
                 re-mint outstanding signed links so the rotation can complete",
            );
            return verdict_for_expiry(expires, now_epoch_seconds);
        }
    }

    SignatureVerdict::Invalid
}

/// Constant-time signature comparison guarded by a length check so the
/// hex-encoding step never near `ct_eq` on a malformed `signature` query
/// param.
fn signatures_match(actual: &str, expected: &str) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Resolve `Valid` / `Expired` for a successful HMAC match. Called once
/// per ring hit so the verdict path is identical whether the URL was
/// signed under the current key or a rotated previous key.
fn verdict_for_expiry(expires: Option<i64>, now_epoch_seconds: i64) -> SignatureVerdict {
    if let Some(ts) = expires
        && now_epoch_seconds > ts
    {
        return SignatureVerdict::Expired;
    }
    SignatureVerdict::Valid
}

/// Convenience: sign a named route lookup.
///
/// Looks `name` up via [`crate::routing::route`], applies the optional
/// expiration, and signs the result. Fails with `FrameworkError` when the
/// route name is not registered or the encryption key is missing.
pub fn sign_route(
    name: &str,
    params: &[(&str, &str)],
    expires_at_epoch_seconds: Option<i64>,
) -> Result<String, FrameworkError> {
    let url = crate::routing::route(name, params).ok_or_else(|| {
        FrameworkError::internal(format!(
            "Cannot sign route '{name}': name is not registered. \
             Register via `.name(\"{name}\")` or `routes!{{}}`.",
        ))
    })?;
    sign_url(&url, expires_at_epoch_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Crypt, EncryptionKey};

    fn ensure_key() {
        if !Crypt::is_initialized() {
            Crypt::init(EncryptionKey::generate());
        }
    }

    /// The three verdicts are genuinely three, and `Invalid` is not
    /// `Expired` - a forged signature never had an expiry to miss.
    ///
    /// That is a true statement about the enum and it stays. It is also
    /// exactly why `url::signature_has_not_expired` could not be built on
    /// `!is_expired()`: that expression answers `true` for `Invalid`, so a
    /// function whose name reads like a guard let every forged URL through
    /// (SEC-04). The fix belongs in the helper, which now requires
    /// `is_valid()`; this test pins the enum semantics the helper must not
    /// go back to relying on.
    #[test]
    fn invalid_is_a_third_state_not_a_flavour_of_expired() {
        assert!(
            !SignatureVerdict::Invalid.is_expired(),
            "an Invalid verdict is not Expired - which is why `!is_expired()` \
             is not a safe basis for an expiry helper"
        );
        assert!(
            !SignatureVerdict::Invalid.is_valid(),
            "…while `is_valid()` correctly rejects it, and that is what the \
             helper is built on now"
        );
        assert!(SignatureVerdict::Expired.is_expired());
        assert!(!SignatureVerdict::Expired.is_valid());
        assert!(SignatureVerdict::Valid.is_valid());
        assert!(!SignatureVerdict::Valid.is_expired());
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn sign_then_verify_round_trips() {
        ensure_key();
        let url = "/orders/42?foo=1&bar=2";
        let signed = sign_url(url, None).expect("sign");
        assert!(signed.contains("signature="));
        let verdict = verify_signature(&signed, 0).expect("verify");
        assert_eq!(verdict, SignatureVerdict::Valid);
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn sign_is_order_independent_over_query_params() {
        ensure_key();
        let a = sign_url("/x?b=2&a=1", None).expect("sign a");
        let b = sign_url("/x?a=1&b=2", None).expect("sign b");
        // Canonical form is keyed by sort order, so the signature must match.
        let sig_a = a.rsplit("signature=").next().unwrap();
        let sig_b = b.rsplit("signature=").next().unwrap();
        assert_eq!(sig_a, sig_b, "param order must not change the signature");
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn tampered_path_fails_verification() {
        ensure_key();
        let signed = sign_url("/orders/42", None).expect("sign");
        let tampered = signed.replace("/orders/42", "/orders/43");
        assert_eq!(
            verify_signature(&tampered, 0).unwrap(),
            SignatureVerdict::Invalid,
            "tampered path must not validate",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn tampered_query_fails_verification() {
        ensure_key();
        let signed = sign_url("/x?u=alice", None).expect("sign");
        let tampered = signed.replace("u=alice", "u=eve");
        assert_eq!(
            verify_signature(&tampered, 0).unwrap(),
            SignatureVerdict::Invalid,
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn expired_signature_reports_expired_not_invalid() {
        ensure_key();
        let signed = sign_url("/reset", Some(1000)).expect("sign");
        let verdict = verify_signature(&signed, 2000).expect("verify");
        assert_eq!(verdict, SignatureVerdict::Expired);
        assert!(verdict.is_expired());
        assert!(!verdict.is_valid());
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn unexpired_signature_validates() {
        ensure_key();
        let signed = sign_url("/reset", Some(5000)).expect("sign");
        let verdict = verify_signature(&signed, 1000).expect("verify");
        assert_eq!(verdict, SignatureVerdict::Valid);
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn stripping_signature_fails_verification() {
        ensure_key();
        let signed = sign_url("/x", None).expect("sign");
        // Strip the signature query param entirely.
        let no_sig = signed.split("?signature=").next().unwrap().to_string();
        assert_eq!(
            verify_signature(&no_sig, 0).unwrap(),
            SignatureVerdict::Invalid,
            "missing signature must be Invalid (not Valid by accident)",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn fragment_is_stripped_from_canonical_form() {
        ensure_key();
        let with_frag = sign_url("/about#section", None).expect("sign");
        // The signature is computed over `/about`, so re-signing without
        // the fragment yields the same signature.
        let without_frag = sign_url("/about", None).expect("sign-again");
        let s1 = with_frag.rsplit("signature=").next().unwrap();
        let s2 = without_frag.rsplit("signature=").next().unwrap();
        assert_eq!(
            s1, s2,
            "fragment must not influence the signature - browsers don't echo it back",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install, route_registry)]
    fn sign_route_resolves_named_route() {
        ensure_key();
        crate::routing::clear_route_names_for_test();
        crate::routing::register_route_name("signed.test.route", "/items/{id}");
        let signed = sign_route("signed.test.route", &[("id", "42")], None).expect("sign route");
        assert!(signed.starts_with("/items/42?signature="));
        assert_eq!(
            verify_signature(&signed, 0).unwrap(),
            SignatureVerdict::Valid,
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn sign_route_errors_on_unknown_name() {
        ensure_key();
        let err = sign_route("signed.test.does_not_exist_xyz", &[], None).unwrap_err();
        assert!(
            err.to_string().contains("is not registered"),
            "error must explain the missing name; got {err}",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn malformed_signature_hex_is_invalid_not_panic() {
        ensure_key();
        // Bare junk in the signature slot.
        let url = "/x?signature=not-hex-at-all";
        assert_eq!(verify_signature(url, 0).unwrap(), SignatureVerdict::Invalid,);
    }

    /// Mint a URL signed against an arbitrary key, bypassing the
    /// `sign_url` keyring resolution. Mirrors `sign_url`'s canonical
    /// build so the rotation-fallback tests can simulate a URL that
    /// was minted under what is now an `APP_KEY_PREVIOUS` entry.
    fn sign_url_with_key(url: &str, key: &[u8], expires: Option<i64>) -> String {
        let (path, mut pairs) = split_url(url);
        pairs.retain(|(k, _)| k != SIGNATURE_KEY && k != EXPIRES_KEY);
        if let Some(ts) = expires {
            pairs.push((EXPIRES_KEY.to_string(), ts.to_string()));
        }
        sort_pairs(&mut pairs);
        let canonical = canonicalize(&path, &pairs);
        let signature = hmac_hex(key, canonical.as_bytes());
        let mut out = canonical;
        if pairs.is_empty() {
            out.push('?');
        } else {
            out.push('&');
        }
        out.push_str(SIGNATURE_KEY);
        out.push('=');
        out.push_str(&signature);
        out
    }

    // M22 - Signed URL APP_KEY_PREVIOUS fallback. The lib-test binary
    // shares one `Crypt` OnceLock across every test module, so we
    // can't reliably install a multi-key ring from here. Instead we
    // drive [`verify_signature_with_keys`] directly with the ring
    // laid out per-test. The `verify_signature` public path is
    // already covered by the existing tests above; the keys-explicit
    // primitive is the single point where the rotation walk lives.

    #[test]
    fn rotation_fallback_validates_url_signed_by_previous_key() {
        // Operator rotated APP_KEY; the now-current key was not the
        // key that signed this URL, but a previous key (still
        // installed via APP_KEY_PREVIOUS) was. The verifier must
        // walk the ring and accept the URL - otherwise every
        // outstanding signed link breaks the moment the operator
        // rotates.
        let current = EncryptionKey::generate();
        let prev = EncryptionKey::generate();
        let signed = sign_url_with_key("/orders/42?foo=1", prev.as_bytes(), None);
        let verdict =
            verify_signature_with_keys(&signed, 0, current.as_bytes(), &[prev.as_bytes().to_vec()]);
        assert_eq!(
            verdict,
            SignatureVerdict::Valid,
            "URL signed by APP_KEY_PREVIOUS must validate via the ring fallback so \
             outstanding signed links survive an APP_KEY rotation"
        );
    }

    #[test]
    fn rotation_fallback_walks_multiple_previous_keys_in_order() {
        // Multi-step rotation: the URL was signed by the oldest
        // previous key. The walk must reach it, not stop at the
        // first miss.
        let current = EncryptionKey::generate();
        let mid = EncryptionKey::generate();
        let oldest = EncryptionKey::generate();
        let signed = sign_url_with_key("/x?a=1&b=2", oldest.as_bytes(), None);
        let previous = vec![oldest.as_bytes().to_vec(), mid.as_bytes().to_vec()];
        let verdict = verify_signature_with_keys(&signed, 0, current.as_bytes(), &previous);
        assert_eq!(verdict, SignatureVerdict::Valid);
    }

    #[test]
    fn rotation_fallback_preserves_expiry_verdict() {
        // The fallback must NOT downgrade the verdict - a previous-
        // key-signed URL that has since elapsed `expires=` returns
        // Expired (not Valid, not Invalid).
        let current = EncryptionKey::generate();
        let prev = EncryptionKey::generate();
        let signed = sign_url_with_key("/reset", prev.as_bytes(), Some(1000));
        let verdict = verify_signature_with_keys(
            &signed,
            2000,
            current.as_bytes(),
            &[prev.as_bytes().to_vec()],
        );
        assert_eq!(
            verdict,
            SignatureVerdict::Expired,
            "expiry must apply equally to previous-key-signed URLs"
        );
    }

    #[test]
    fn rotation_fallback_rejects_url_signed_by_key_outside_ring() {
        // A URL signed by a key that is in NEITHER `current` nor
        // `previous` must be rejected as Invalid - the fallback is
        // bounded by the installed ring, not an open sesame.
        let current = EncryptionKey::generate();
        let prev = EncryptionKey::generate();
        let unrelated = EncryptionKey::generate();
        let signed = sign_url_with_key("/orders/42", unrelated.as_bytes(), None);
        let verdict =
            verify_signature_with_keys(&signed, 0, current.as_bytes(), &[prev.as_bytes().to_vec()]);
        assert_eq!(verdict, SignatureVerdict::Invalid);
    }

    #[test]
    fn rotation_fallback_current_key_match_does_not_walk_previous() {
        // Sanity: when the current key matches, the previous list is
        // ignored (no rotation warning, no waste of HMAC ops). We
        // can't easily assert "no log emitted" without a tracing
        // subscriber, but we can pin the verdict alongside an empty
        // previous list and prove the answer doesn't shift when a
        // non-matching previous key is added.
        let current = EncryptionKey::generate();
        let signed = sign_url_with_key("/x", current.as_bytes(), None);
        let no_prev = verify_signature_with_keys(&signed, 0, current.as_bytes(), &[]);
        let other = EncryptionKey::generate();
        let with_prev = verify_signature_with_keys(
            &signed,
            0,
            current.as_bytes(),
            &[other.as_bytes().to_vec()],
        );
        assert_eq!(no_prev, SignatureVerdict::Valid);
        assert_eq!(with_prev, SignatureVerdict::Valid);
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn trailing_slash_is_part_of_the_signed_path() {
        ensure_key();
        let without_slash = sign_url("/orders/1?scope=read", None).expect("sign without slash");
        let with_slash = sign_url("/orders/1/?scope=read", None).expect("sign with slash");

        assert_eq!(
            verify_signature(&without_slash, 0).unwrap(),
            SignatureVerdict::Valid,
            "the URL without a trailing slash must validate itself",
        );
        assert_eq!(
            verify_signature(&with_slash, 0).unwrap(),
            SignatureVerdict::Valid,
            "the URL with a trailing slash must validate itself",
        );

        let without_slash_signature = without_slash
            .rsplit_once("signature=")
            .expect("signed URL without slash carries a signature")
            .1;
        let with_slash_signature = with_slash
            .rsplit_once("signature=")
            .expect("signed URL with slash carries a signature")
            .1;

        assert_eq!(
            verify_signature(
                &format!("/orders/1?scope=read&signature={with_slash_signature}"),
                0,
            )
            .unwrap(),
            SignatureVerdict::Invalid,
            "the slash-path signature must not validate the non-slash path",
        );
        assert_eq!(
            verify_signature(
                &format!("/orders/1/?scope=read&signature={without_slash_signature}"),
                0,
            )
            .unwrap(),
            SignatureVerdict::Invalid,
            "the non-slash-path signature must not validate the slash path",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn trailing_slash_is_part_of_the_signed_path_without_query() {
        ensure_key();
        let without_slash = sign_url("/orders/1", None).expect("sign without slash");
        let with_slash = sign_url("/orders/1/", None).expect("sign with slash");

        assert_eq!(
            verify_signature(&without_slash, 0).unwrap(),
            SignatureVerdict::Valid,
            "the URL without a trailing slash must validate itself",
        );
        assert_eq!(
            verify_signature(&with_slash, 0).unwrap(),
            SignatureVerdict::Valid,
            "the URL with a trailing slash must validate itself",
        );

        let without_slash_signature = without_slash
            .rsplit_once("signature=")
            .expect("signed URL without slash carries a signature")
            .1;
        let with_slash_signature = with_slash
            .rsplit_once("signature=")
            .expect("signed URL with slash carries a signature")
            .1;

        assert_eq!(
            verify_signature(&format!("/orders/1?signature={with_slash_signature}"), 0,).unwrap(),
            SignatureVerdict::Invalid,
            "the slash-path signature must not validate the non-slash path",
        );
        assert_eq!(
            verify_signature(
                &format!("/orders/1/?signature={without_slash_signature}"),
                0,
            )
            .unwrap(),
            SignatureVerdict::Invalid,
            "the non-slash-path signature must not validate the slash path",
        );
    }

    #[test]
    #[serial_test::serial(crypt_install)]
    fn root_path_is_preserved_exactly() {
        ensure_key();
        let signed = sign_url("/", None).expect("sign root");
        assert!(
            signed.starts_with("/?signature="),
            "the exact root path must remain '/' in the signed URL; got {signed}",
        );
        assert_eq!(
            verify_signature(&signed, 0).unwrap(),
            SignatureVerdict::Valid,
            "the exact root path must validate itself",
        );
    }

    // ------------------------------------------------------------------
    // SEC-04 - duplicate query keys
    // ------------------------------------------------------------------

    /// The attack the lossless canonical form exists to stop.
    ///
    /// Take a legitimately signed `?user=victim`, prepend a second value
    /// for the same key, and send the original signature untouched. Under
    /// the map-based canonical form the verifier hashed only the *last*
    /// value - `victim` - so the signature still matched, while
    /// `Request::query_param` handed the handler the *first*. Verified and
    /// executed were different URLs.
    #[test]
    fn a_prepended_duplicate_value_no_longer_verifies() {
        let key = vec![7u8; 32];
        let signed = sign_url_with_key("/promote?user=victim", &key, None);
        let sig = signed
            .rsplit_once("signature=")
            .expect("signed URL carries a signature")
            .1;

        let attacked = format!("/promote?user=attacker&user=victim&signature={sig}");

        assert_eq!(
            verify_signature_with_keys(&attacked, 0, &key, &[]),
            SignatureVerdict::Invalid,
            "adding a value for an already-signed key must break the HMAC; \
             it verified, which means the canonical form is losing values again"
        );
        // …and the untouched URL still verifies, so the test is failing on
        // the substitution rather than on a broken signer.
        assert_eq!(
            verify_signature_with_keys(&signed, 0, &key, &[]),
            SignatureVerdict::Valid,
        );
    }

    /// Appending rather than prepending is the same attack against the
    /// other accessor. Neither ordering may verify.
    #[test]
    fn an_appended_duplicate_value_no_longer_verifies() {
        let key = vec![7u8; 32];
        let signed = sign_url_with_key("/promote?user=victim", &key, None);
        let sig = signed.rsplit_once("signature=").expect("signature").1;

        let attacked = format!("/promote?user=victim&user=attacker&signature={sig}");
        assert_eq!(
            verify_signature_with_keys(&attacked, 0, &key, &[]),
            SignatureVerdict::Invalid,
        );
    }

    /// A repeated key is legitimate in ordinary URLs (`?tag=a&tag=b`), so
    /// the fix must carry every value into the signature rather than
    /// refuse the URL outright - otherwise "reject duplicates" would break
    /// list parameters for everyone to stop one attack.
    #[test]
    fn a_genuinely_repeated_key_signs_and_verifies_with_every_value() {
        let key = vec![9u8; 32];
        let signed = sign_url_with_key("/feed?tag=a&tag=b", &key, None);
        assert_eq!(
            verify_signature_with_keys(&signed, 0, &key, &[]),
            SignatureVerdict::Valid,
            "both values must survive into the canonical payload"
        );

        // Dropping one of them is a different URL and must not verify.
        let sig = signed.rsplit_once("signature=").expect("signature").1;
        assert_eq!(
            verify_signature_with_keys(&format!("/feed?tag=a&signature={sig}"), 0, &key, &[]),
            SignatureVerdict::Invalid,
            "removing a signed value must invalidate too, not just adding one"
        );
    }

    /// Reordering equivalent parameters must not change the signature -
    /// the property the sort exists for, now that ordering includes the
    /// value as a tiebreak.
    #[test]
    fn repeated_values_canonicalise_independently_of_wire_order() {
        let key = vec![3u8; 32];
        let one = sign_url_with_key("/feed?tag=b&tag=a", &key, None);
        let two = sign_url_with_key("/feed?tag=a&tag=b", &key, None);
        assert_eq!(
            one, two,
            "two spellings of the same parameter set must canonicalise identically"
        );
    }

    /// A duplicated control parameter leaves the authoritative value
    /// ambiguous. `signature` is stripped before canonicalisation, so
    /// nothing else would catch this one.
    #[test]
    fn a_duplicated_signature_parameter_is_refused() {
        let key = vec![7u8; 32];
        let signed = sign_url_with_key("/promote?user=victim", &key, None);
        let sig = signed.rsplit_once("signature=").expect("signature").1;

        assert_eq!(
            verify_signature_with_keys(
                &format!("/promote?user=victim&signature={sig}&signature=deadbeef"),
                0,
                &key,
                &[]
            ),
            SignatureVerdict::Invalid,
        );
        assert_eq!(
            verify_signature_with_keys(
                &format!("/promote?user=victim&signature=deadbeef&signature={sig}"),
                0,
                &key,
                &[]
            ),
            SignatureVerdict::Invalid,
            "and the ordering that would otherwise pick the good one must \
             not be a way in either"
        );
    }

    /// Same rule for `expires`: two expiries means two answers to "has
    /// this lapsed?", and the verifier must not be the one guessing.
    ///
    /// Unlike the `signature` case this is **defence in depth, not the
    /// load-bearing guard** - `expires` lives *inside* the signed payload,
    /// so a second copy already changes the canonical form and breaks the
    /// HMAC. Deleting the `expires_count` check alone leaves this test
    /// green; deleting the lossless canonical form fails it. Said plainly
    /// so nobody reads a passing test as proof the explicit check works.
    #[test]
    fn a_duplicated_expires_parameter_is_refused() {
        let key = vec![7u8; 32];
        let signed = sign_url_with_key("/report", &key, Some(1_000));
        let sig = signed.rsplit_once("signature=").expect("signature").1;

        assert_eq!(
            verify_signature_with_keys(
                &format!("/report?expires=1000&expires=99999999999&signature={sig}"),
                0,
                &key,
                &[]
            ),
            SignatureVerdict::Invalid,
        );
    }

    /// Signatures minted before the lossless canonical form must still
    /// verify. For a URL with no repeated keys, sorting the pair list by
    /// `(key, value)` yields exactly the order `BTreeMap` iteration gave,
    /// so the payload bytes are unchanged - this pins that, because a
    /// canonical-form change that silently invalidated every outstanding
    /// password-reset link would be a far worse outage than the bug.
    #[test]
    fn the_canonical_form_is_unchanged_for_urls_without_repeated_keys() {
        let pairs = vec![
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
            ("c".to_string(), "3".to_string()),
        ];
        let mut sorted = pairs.clone();
        sort_pairs(&mut sorted);

        let legacy: std::collections::BTreeMap<String, String> = pairs.into_iter().collect();
        let legacy_form = {
            let mut s = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in &legacy {
                s.append_pair(k, v);
            }
            format!("/p?{}", s.finish())
        };

        assert_eq!(canonicalize("/p", &sorted), legacy_form);
    }
}
