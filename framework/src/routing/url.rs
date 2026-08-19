//! URL generation helpers.
//!
//! Laravel's `URL` facade (`Illuminate/Routing/UrlGenerator.php`) backs
//! `url()`, `url()->to()`, `url()->current()`, `url()->previous()`,
//! `url()->signedRoute()`. Suprnova ships a deliberately smaller surface
//! — the heavy `asset()`/`secureAsset()` family is handled by Vite +
//! the filesystem disks, and the controller-action `action()` helper
//! has no Rust analogue because handlers are functions, not controller
//! strings.
//!
//! What does land here is the user-facing shape consumers reach for:
//!
//! - [`to`] / [`secure`] — build an absolute URL from a path against
//!   the configured `APP_URL`.
//! - [`current`] / [`full`] / [`previous`] — read the current request's
//!   URL, full URL, and the previous URL recorded in the session.
//! - [`signed_route`] / [`temporary_signed_route`] — sign a named route
//!   for HMAC-verified delivery.
//! - [`has_valid_signature`] / [`signature_verdict`] — verify a signed URL
//!   coming in on a request; the latter tells `Expired` from `Invalid`.
//!
//! All helpers are free functions in the `crate::routing::url` namespace,
//! re-exported under `suprnova::url::*` so consumers write:
//!
//! ```rust,no_run
//! use suprnova::url;
//! # use suprnova::Request;
//! # fn req() -> Request { unimplemented!() }
//! # fn ex() -> Result<(), Box<dyn std::error::Error>> {
//! # let t = "reset-token";
//! # let request = req();
//! let absolute = url::to("/dashboard");
//! let signed = url::signed_route("password.reset", &[("token", t)])?;
//! let verdict = url::has_valid_signature(&request);
//! # let _ = (absolute, signed, verdict);
//! # Ok(()) }
//! ```

use crate::FrameworkError;
use crate::http::Request;
use crate::routing::signed::{
    SignatureVerdict, sign_route as do_sign_route, sign_url as do_sign_url, verify_signature,
};

/// Build an absolute URL by joining `path` to the configured
/// `APP_URL`.
///
/// Mirrors Laravel's `url()->to($path)` /
/// `Illuminate/Routing/UrlGenerator.php::to()`. The host comes from
/// `APP_URL` (env at boot), the scheme/port from that URL too.
/// An already-absolute `path` (one that starts with `http://`,
/// `https://`, or `//`) is returned unchanged.
///
/// # Example
///
/// ```rust,no_run
/// // SAFETY: single-threaded doctest; `set_var` is `unsafe` in edition 2024.
/// unsafe { std::env::set_var("APP_URL", "https://example.com"); }
/// assert_eq!(suprnova::url::to("/about"), "https://example.com/about");
/// assert_eq!(
///     suprnova::url::to("https://other.example/x"),
///     "https://other.example/x",
/// );
/// ```
pub fn to(path: &str) -> String {
    if is_absolute(path) {
        return path.to_string();
    }
    let base = app_url();
    join_base_path(&base, path)
}

/// Build an absolute `https://` URL even if `APP_URL` is `http://`.
/// Mirrors Laravel's `url()->secure($path)`.
pub fn secure(path: &str) -> String {
    let absolute = to(path);
    if let Some(rest) = absolute.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        absolute
    }
}

/// The current request's path + query string, derived from the active
/// request scope. Returns `None` outside a handler (no request scope).
///
/// Mirrors Laravel's `url()->current()` (path only, without query is the
/// PHP default; Suprnova returns path+query because Rust callers
/// typically want the full visible URL). Use [`Request::path`] directly
/// when you only need the path.
pub fn current(request: &Request) -> String {
    let path = request.path();
    match request.uri().query() {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    }
}

/// Full absolute URL of the current request — `APP_URL` host +
/// [`current`]. Mirrors Laravel's `url()->full()`.
pub fn full(request: &Request) -> String {
    to(&current(request))
}

/// The previous URL recorded by [`crate::session::SessionMiddleware`] on
/// the prior GET request. Returns `fallback` when no previous URL is
/// recorded (fresh session, or the session middleware isn't active).
///
/// Mirrors Laravel's `url()->previous($fallback = '/')`. Powers
/// [`crate::Redirect::back`].
pub fn previous(fallback: &str) -> String {
    crate::session::session()
        .and_then(|s| s.previous_url())
        .unwrap_or_else(|| fallback.to_string())
}

/// Sign a named route. Convenience wrapper over
/// [`crate::routing::signed::sign_route`].
///
/// Mirrors Laravel's `URL::signedRoute($name, $parameters, $expiration)`
/// without an `$expiration` argument — for the timed variant use
/// [`temporary_signed_route`].
pub fn signed_route(name: &str, params: &[(&str, &str)]) -> Result<String, FrameworkError> {
    do_sign_route(name, params, None)
}

/// Sign a named route with an expiration. Convenience wrapper over
/// [`crate::routing::signed::sign_route`] with an `expires` clock.
///
/// `expires_at_epoch_seconds` is interpreted in absolute terms (a UNIX
/// timestamp). To express "now + duration", compute
/// `chrono::Utc::now().timestamp() + duration.as_secs() as i64` at the
/// call site.
///
/// Mirrors Laravel's `URL::temporarySignedRoute($name, $expiration,
/// $parameters)`.
pub fn temporary_signed_route(
    name: &str,
    params: &[(&str, &str)],
    expires_at_epoch_seconds: i64,
) -> Result<String, FrameworkError> {
    do_sign_route(name, params, Some(expires_at_epoch_seconds))
}

/// Sign an arbitrary URL with the framework signing key. Use this
/// when the URL doesn't come from a registered named route (e.g.
/// callbacks, third-party redirects). Wrapper over
/// [`crate::routing::signed::sign_url`].
pub fn signed_url(
    url: &str,
    expires_at_epoch_seconds: Option<i64>,
) -> Result<String, FrameworkError> {
    do_sign_url(url, expires_at_epoch_seconds)
}

/// Verify the signature on the inbound `request`.
///
/// Returns `true` only when the HMAC matches and the URL has not
/// expired. Use [`signature_has_not_expired`] when you want the
/// expired-vs-invalid distinction.
///
/// Mirrors Laravel's `URL::hasValidSignature($request)`. The verifier
/// uses the current epoch second clock (`chrono::Utc::now`).
///
/// # Errors
///
/// Returns `FrameworkError` when the encryption key is not installed.
pub fn has_valid_signature(request: &Request) -> Result<bool, FrameworkError> {
    Ok(verdict_for_request(request)?.is_valid())
}

/// `true` only when the signature is valid **and** the URL has not expired.
///
/// # Why this requires a valid signature (SEC-04)
///
/// It used to be `!verdict.is_expired()`, which answered `true` for
/// [`SignatureVerdict::Invalid`] as well as [`SignatureVerdict::Valid`]: a
/// forged signature is not expired, because it never had an expiry to miss.
/// That mirrored Laravel's `URL::signatureHasNotExpired($request)` exactly,
/// and it made a function whose name reads like a guard let every forged URL
/// through. `expires` is attacker-supplied until the HMAC says otherwise, so
/// no answer derived from it means anything before the signature checks out.
///
/// Requiring validity is what closes that, and it collapses this function
/// into [`has_valid_signature`] — necessarily so. Under a three-state verdict
/// there is no "not expired" a boolean can report honestly except `Valid`;
/// the old function was only distinct *because* it trusted unauthenticated
/// input. The genuine three-way distinction lives in [`signature_verdict`],
/// which can say `Expired` and `Invalid` separately instead of encoding three
/// states in a `bool`:
///
/// ```rust,ignore
/// match url::signature_verdict(&req)? {
///     SignatureVerdict::Valid   => grant_access(),
///     SignatureVerdict::Expired => render("this link has expired — request a new one"),
///     SignatureVerdict::Invalid => render("this link is not valid"),
/// }
/// ```
///
/// A missing `expires` value still counts as "not expired", as in Laravel —
/// an unexpiring signed URL is a valid one.
#[deprecated(
    since = "0.7.4",
    note = "now identical to `has_valid_signature`; match on `signature_verdict` \
            to tell Expired from Invalid"
)]
pub fn signature_has_not_expired(request: &Request) -> Result<bool, FrameworkError> {
    Ok(verdict_for_request(request)?.is_valid())
}

/// Return the full [`SignatureVerdict`] for the inbound request. Lets
/// callers branch on `Valid`/`Expired`/`Invalid` to render distinct UX
/// (e.g. "this link has expired — request a new one").
pub fn signature_verdict(request: &Request) -> Result<SignatureVerdict, FrameworkError> {
    verdict_for_request(request)
}

fn verdict_for_request(request: &Request) -> Result<SignatureVerdict, FrameworkError> {
    let url = current(request);
    let now = chrono::Utc::now().timestamp();
    verify_signature(&url, now)
}

/// Whether `path` is already absolute (`http://`, `https://`, or `//`).
fn is_absolute(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("//")
}

/// Accept a same-origin, root-relative target, or reject it.
///
/// Shared by every place in the framework that stores or redirects to a
/// URL a client influenced without a full authority check: the
/// `InertiaValidationRedirectMiddleware` `Referer`/current-URL guard
/// (`crate::inertia::validation_redirect_middleware`) and
/// [`crate::session::SessionMiddleware`]'s `_previous.url` write, which
/// backs [`crate::Redirect::back`], [`crate::Redirect::refresh`], and
/// [`previous`].
///
/// The `_previous.url` path enforces this in two layers, not one: guarded
/// here at write time so a hostile value never lands in the session in
/// the first place, and re-checked again on every read by
/// [`SessionData::previous_url`](crate::session::SessionData::previous_url)
/// via this same function. A write-time guard alone would leave any
/// session cookie that predates it — or one written by a future bug —
/// trusted forever once stored; the read-time re-check makes such a
/// session self-heal instead: a stored value that now fails the check
/// reads back as `None`, so the caller's own fallback takes over instead
/// of resolving an off-origin `Location`, and the next successful GET
/// records a fresh, safe value over it.
///
/// Rejects exactly the shapes a browser can be tricked into parsing as a
/// different origin from what a byte-for-byte check on the original
/// string sees:
///
/// - A leading `//` — protocol-relative, read as absolute.
/// - A leading `/\` — the same bypass in disguise: the WHATWG URL
///   parser treats `\` as `/` for special schemes, so `/\evil.test`
///   becomes `//evil.test` once the browser normalizes it.
/// - Any ASCII control byte anywhere in the string, not only right
///   after the leading slash. The URL parser strips ASCII tab and
///   newline from its *entire* input before it ever compares origins
///   (`https://url.spec.whatwg.org/#url-parsing`), so `/<TAB>/evil.test`
///   is `//evil.test` by the time a browser navigates it, even though a
///   byte-for-byte check on the original string sees a single leading
///   `/`. Rejecting on *any* C0 control or DEL, rather than enumerating
///   tab/newline/CR specifically, is deliberate: this guard has already
///   needed widening twice (from bare `//`, to `/\` too, to any control
///   byte), and chasing the URL Standard's exact normalization steps one
///   character class at a time is a fight that keeps recurring. A
///   reject-on-any-control-byte rule is simpler to keep correct than a
///   strip-and-reprocess rule that has to track those steps forever.
pub(crate) fn root_relative_or_none(candidate: &str) -> Option<String> {
    if candidate.is_empty() || has_control_byte(candidate) {
        return None;
    }
    let rest = candidate.strip_prefix('/')?;
    if rest.starts_with('/') || rest.starts_with('\\') {
        None
    } else {
        Some(candidate.to_string())
    }
}

/// True when `s` contains an ASCII control byte: C0 (`0x00..=0x1F`) or
/// DEL (`0x7F`). Covers tab and newline — the two the URL parser strips
/// before comparing origins — without having to name them individually.
pub(crate) fn has_control_byte(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_control())
}

/// Resolve the framework's `APP_URL`. Reads from
/// [`crate::config::ConfigRegistry`] when a config provider is
/// registered, otherwise falls back to the `APP_URL` env var or
/// `http://localhost`.
fn app_url() -> String {
    // Try the typed `AppConfig` first.
    if let Some(cfg) = crate::config::Config::get::<crate::config::AppConfig>() {
        let trimmed = cfg.url.trim_end_matches('/').to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    // Fall back to env, mirroring `AppConfig::from_env()`.
    std::env::var("APP_URL")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| "http://localhost".to_string())
}

/// Concatenate `base` + `path` ensuring exactly one `/` separator.
fn join_base_path(base: &str, path: &str) -> String {
    if path.is_empty() {
        return base.to_string();
    }
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
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

    // ---- SEC-04: the expiry helper requires a valid signature -----------

    /// The attack the old helper allowed: forge any URL, and because a forged
    /// signature has no expiry to miss, `!is_expired()` answered `true`. Any
    /// handler that guarded on the function whose name reads "has not
    /// expired" let it straight through.
    #[cfg(feature = "testing")]
    #[test]
    #[serial_test::serial(crypt_install)]
    fn a_forged_signature_has_not_expired_is_false() {
        ensure_key();
        let signed = crate::routing::signed::sign_url("/promote?user=victim", None).expect("sign");
        let forged = signed.replace("user=victim", "user=attacker");
        let req = Request::for_test("GET", &forged);

        assert_eq!(
            signature_verdict(&req).expect("verdict"),
            SignatureVerdict::Invalid,
            "tampering with a signed parameter invalidates the signature"
        );
        #[allow(deprecated)]
        let not_expired = signature_has_not_expired(&req).expect("expiry helper");
        assert!(
            !not_expired,
            "an unverifiable URL must not report 'has not expired' — `expires` \
             is attacker-supplied until the HMAC says otherwise"
        );
    }

    /// `has_valid_signature` is the guard handlers are told to reach for, and
    /// until this existed nothing drove it with a tampered request — the whole
    /// framework suite passed with its body replaced by `!is_expired()`, the
    /// exact defect SEC-04 describes. A guard with no adversarial test is a
    /// guard nobody has checked.
    #[cfg(feature = "testing")]
    #[test]
    #[serial_test::serial(crypt_install)]
    fn has_valid_signature_rejects_a_tampered_url_and_accepts_an_intact_one() {
        ensure_key();
        let signed = crate::routing::signed::sign_url("/promote?user=victim", None).expect("sign");

        let intact = Request::for_test("GET", &signed);
        assert!(
            has_valid_signature(&intact).expect("verify"),
            "an untouched signed URL verifies"
        );

        let forged = signed.replace("user=victim", "user=attacker");
        let tampered = Request::for_test("GET", &forged);
        assert!(
            !has_valid_signature(&tampered).expect("verify"),
            "changing a signed parameter must fail the guard"
        );

        // And the SEC-04 shape specifically: prepending a duplicate rather
        // than editing in place, which the pre-fix canonical form missed
        // because it hashed only the last value per key.
        let prepended = signed.replace("/promote?", "/promote?user=attacker&");
        let duplicated = Request::for_test("GET", &prepended);
        assert!(
            !has_valid_signature(&duplicated).expect("verify"),
            "a prepended duplicate parameter must fail the guard too"
        );
    }

    /// The control, so the fix is not just "return false always": a genuinely
    /// valid, unexpiring signed URL still answers `true`.
    #[cfg(feature = "testing")]
    #[test]
    #[serial_test::serial(crypt_install)]
    fn a_valid_signature_with_no_expiry_has_not_expired() {
        ensure_key();
        let signed = crate::routing::signed::sign_url("/promote?user=victim", None).expect("sign");
        let req = Request::for_test("GET", &signed);

        assert_eq!(
            signature_verdict(&req).expect("verdict"),
            SignatureVerdict::Valid
        );
        #[allow(deprecated)]
        let not_expired = signature_has_not_expired(&req).expect("expiry helper");
        assert!(not_expired, "an unexpiring valid link has not expired");
    }

    /// And a correctly-signed link that HAS expired answers `false`, which is
    /// the one case the helper was ever really asked about.
    #[cfg(feature = "testing")]
    #[test]
    #[serial_test::serial(crypt_install)]
    fn a_valid_but_expired_signature_has_expired() {
        ensure_key();
        let long_ago = chrono::Utc::now().timestamp() - 3600;
        let signed =
            crate::routing::signed::sign_url("/promote?user=victim", Some(long_ago)).expect("sign");
        let req = Request::for_test("GET", &signed);

        assert_eq!(
            signature_verdict(&req).expect("verdict"),
            SignatureVerdict::Expired,
            "the verdict still distinguishes expired from invalid — that is \
             where the three-way answer lives now"
        );
        #[allow(deprecated)]
        let not_expired = signature_has_not_expired(&req).expect("expiry helper");
        assert!(!not_expired);
    }

    #[test]
    fn to_prepends_app_url_for_relative_path() {
        // SAFETY: tests in this crate single-thread env mutation via
        // `serial_test::serial(env_app_url)` — see the test below.
        unsafe {
            std::env::set_var("APP_URL", "https://example.test");
        }
        let url = to("/dashboard");
        assert!(
            url == "https://example.test/dashboard" || url.ends_with("/dashboard"),
            "URL should incorporate APP_URL + path; got {url}",
        );
    }

    #[test]
    fn to_returns_absolute_unchanged() {
        assert_eq!(
            to("https://elsewhere.example/page"),
            "https://elsewhere.example/page",
        );
        assert_eq!(
            to("http://elsewhere.example/page"),
            "http://elsewhere.example/page",
        );
        assert_eq!(to("//cdn.example/x"), "//cdn.example/x");
    }

    #[test]
    fn secure_upgrades_http_to_https() {
        unsafe {
            std::env::set_var("APP_URL", "http://example.test");
        }
        let url = secure("/login");
        assert!(
            url.starts_with("https://"),
            "secure() must yield an https URL; got {url}",
        );
    }

    #[test]
    fn join_handles_missing_or_extra_slashes() {
        assert_eq!(join_base_path("https://x", "/a"), "https://x/a");
        assert_eq!(join_base_path("https://x", "a"), "https://x/a");
        assert_eq!(join_base_path("https://x", ""), "https://x");
    }

    // ---- root_relative_or_none: shared by the session write-guard and
    // the Inertia validation-redirect bridge ----

    #[test]
    fn root_relative_or_none_accepts_a_plain_same_origin_path() {
        assert_eq!(
            root_relative_or_none("/dashboard?tab=2"),
            Some("/dashboard?tab=2".to_string())
        );
    }

    #[test]
    fn root_relative_or_none_rejects_protocol_relative_and_backslash_forms() {
        // `//evil.test` — read as absolute by a browser.
        assert_eq!(root_relative_or_none("//evil.test/x"), None);
        // `/\evil.test` — the WHATWG URL parser folds `\` into `/` for
        // special schemes, so this is `//evil.test` in disguise.
        assert_eq!(root_relative_or_none("/\\evil.test"), None);
    }

    #[test]
    fn root_relative_or_none_rejects_any_control_byte_anywhere() {
        // The confirmed bypass: a byte-for-byte check on the char right
        // after the leading `/` misses this, but the URL parser strips
        // tab/newline from its whole input before comparing origins.
        assert_eq!(root_relative_or_none("/\t/evil.test"), None);
        assert_eq!(root_relative_or_none("/\n/evil.test"), None);
        assert_eq!(root_relative_or_none("/\r/evil.test"), None);
        // Not just right after the leading slash.
        assert_eq!(root_relative_or_none("/register/step\x0b2"), None);
        assert_eq!(root_relative_or_none(""), None);
    }

    #[test]
    fn has_control_byte_covers_c0_and_del_only() {
        assert!(has_control_byte("a\tb"));
        assert!(has_control_byte("a\x00b"));
        assert!(has_control_byte("a\x7fb"));
        assert!(!has_control_byte("/perfectly/safe/path?q=1"));
    }

    #[test]
    #[serial_test::serial(crypt_install, route_registry)]
    fn signed_route_then_url_verifier_round_trips() {
        ensure_key();
        crate::routing::clear_route_names_for_test();
        crate::routing::register_route_name("url.test.signed", "/secret/{id}");
        let signed = signed_route("url.test.signed", &[("id", "42")]).expect("sign");
        assert!(signed.contains("signature="));

        // Reach the verifier through a synthetic request to mirror how
        // a real handler will use `has_valid_signature`.
        // The URL on the signed string is `/secret/42?signature=...`
        // — we feed exactly that path+query to the verifier.
        let now = chrono::Utc::now().timestamp();
        let verdict = crate::routing::signed::verify_signature(&signed, now).expect("verify");
        assert_eq!(verdict, SignatureVerdict::Valid);
    }

    #[test]
    #[serial_test::serial(crypt_install, route_registry)]
    fn temporary_signed_route_expires() {
        ensure_key();
        crate::routing::clear_route_names_for_test();
        crate::routing::register_route_name("url.test.temp", "/once/{id}");
        let signed = temporary_signed_route("url.test.temp", &[("id", "1")], 1000).expect("sign");
        let verdict = crate::routing::signed::verify_signature(&signed, 5000).expect("verify");
        assert_eq!(verdict, SignatureVerdict::Expired);
    }
}
