//! Password rule family parity: strength checks mirror Laravel's regexes;
//! uncompromised() speaks HIBP's k-anonymity range API and fails open.
//!
//! `Password` implements both `Rule` (sync, strength-only) and `AsyncRule`
//! (strength + the HIBP check), both named `passes`. With both traits in
//! scope, `rule.passes(...)` is ambiguous to the compiler regardless of
//! sync/async context (method resolution picks a candidate by name before
//! it ever looks at `.await`), so every call below is fully qualified as
//! `Rule::passes(&rule, ...)` / `AsyncRule::passes(&rule, ...).await`.
//!
//! `Http::assert_sent`/`fake_response` are also free functions
//! (`suprnova::assert_sent`), not `Http::` associated items - only
//! `Http::fake`, `Http::fake_response_text`, and the request builders
//! (`Http::get`/`post`/...) are associated with `Http` itself.

use std::sync::Arc;
use suprnova::{
    AsyncRule, FrameworkError, Http, Password, Rule, UncompromisedVerifier, assert_sent,
};
use tracing_test::traced_test;

#[test]
fn min_floors_at_one_and_checks_length() {
    assert!(
        Rule::passes(&Password::min(0), "x").is_ok(),
        "min(0) floors to 1"
    );
    let err = Rule::passes(&Password::min(8), "short").expect_err("too short");
    assert_eq!(err.key, "validation-min");
}

#[test]
fn min_counts_unicode_scalars_not_bytes() {
    // "pässwör" is 7 Unicode scalar values but 9 UTF-8 bytes (ä and ö are
    // each 2 bytes) - `Password::min` must count `char`s, not bytes, or a
    // multi-byte string could satisfy a length floor with fewer real
    // characters than the floor intends.
    let value = "pässwör";
    assert_eq!(value.chars().count(), 7);
    assert_eq!(value.len(), 9);
    let err = Rule::passes(&Password::min(8), value).expect_err("7 chars is under min 8");
    assert_eq!(err.key, "validation-min");
    assert!(Rule::passes(&Password::min(7), value).is_ok());
}

#[test]
fn strength_flags_mirror_laravels_regexes() {
    let rule = Password::min(1).mixed_case();
    assert!(Rule::passes(&rule, "aB").is_ok());
    assert_eq!(
        Rule::passes(&rule, "ab").expect_err("no upper").key,
        "validation-password-mixed"
    );

    let rule = Password::min(1).letters();
    assert_eq!(
        Rule::passes(&rule, "1234").expect_err("no letter").key,
        "validation-password-letters"
    );

    let rule = Password::min(1).numbers();
    assert_eq!(
        Rule::passes(&rule, "abcd").expect_err("no number").key,
        "validation-password-numbers"
    );

    let rule = Password::min(1).symbols();
    assert_eq!(
        Rule::passes(&rule, "abc1").expect_err("no symbol").key,
        "validation-password-symbols"
    );
    assert!(
        Rule::passes(&rule, "ab c").is_ok(),
        "space is \\p{{Z}}: a separator counts as a symbol, per Laravel"
    );
}

/// `sha1("password")` = `5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8`:
/// prefix `5BAA6`, suffix `1E4C9B93F3F0682250B6CF8331B7EE68FD8`, reported
/// count `3730471`. Shared by every test below that drives the range API
/// with "password" so the fixture data can't drift between them.
const LEAKED_RANGE_BODY: &str =
    "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n1E4C9B93F3F0682250B6CF8331B7EE68FD8:3730471\r\n";

#[tokio::test]
async fn uncompromised_flags_a_leaked_password_via_the_range_api() {
    Http::fake(|| async {
        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            LEAKED_RANGE_BODY,
        );
        let rule = Password::min(1).uncompromised();
        let err = AsyncRule::passes(&rule, "password")
            .await
            .expect_err("leaked");
        assert_eq!(err.key, "validation-password-uncompromised");

        // Each canned response is consumed on match (`fake.rs`), so the
        // threshold-boundary check below needs its own queued entry per
        // call - queuing only one response for two calls would let the
        // second fall through to the fake's default empty `200 {}` and
        // pass via the "nothing matched" path instead of the threshold
        // comparison, making the assertion pass under either polarity.
        //
        // The comparison is strict `>`: a threshold exactly AT the
        // reported count (3_730_471) must pass, and a threshold one below
        // it must still fail. This pins the true boundary, not just "some
        // big number passes."
        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            LEAKED_RANGE_BODY,
        );
        let rule = Password::min(1).uncompromised_with_threshold(3_730_471);
        assert!(
            AsyncRule::passes(&rule, "password").await.is_ok(),
            "threshold == count must pass: count > threshold is false"
        );

        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            LEAKED_RANGE_BODY,
        );
        let rule = Password::min(1).uncompromised_with_threshold(3_730_470);
        let err = AsyncRule::passes(&rule, "password")
            .await
            .expect_err("threshold one below count must still fail");
        assert_eq!(err.key, "validation-password-uncompromised");

        assert_sent(|req| {
            req.url.contains("/range/5BAA6")
                && !req.url.to_uppercase().contains("1E4C9B93")
                && req
                    .headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("add-padding") && v == "true")
        });
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
async fn uncompromised_skips_response_lines_without_a_colon() {
    Http::fake(|| async {
        // A malformed line with no `:` must be skipped (the `continue` at
        // the `split_once` failure), not abort the scan - the real match
        // on the next line must still be found.
        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            "this line has no colon and must be skipped\r\n1E4C9B93F3F0682250B6CF8331B7EE68FD8:5\r\n",
        );
        let rule = Password::min(1).uncompromised();
        let err = AsyncRule::passes(&rule, "password")
            .await
            .expect_err("the malformed first line must not hide the real match below it");
        assert_eq!(err.key, "validation-password-uncompromised");
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
async fn uncompromised_unparseable_count_on_a_matched_suffix_is_treated_as_compromised() {
    Http::fake(|| async {
        // A matched suffix with a count that doesn't parse as u64 is the
        // deliberate fail-CLOSED exception to the rule's overall fail-open
        // posture: the suffix genuinely matched, so treat it as
        // compromised rather than guessing.
        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            "1E4C9B93F3F0682250B6CF8331B7EE68FD8:not-a-number\r\n",
        );
        let rule = Password::min(1).uncompromised();
        let err = AsyncRule::passes(&rule, "password")
            .await
            .expect_err("an unparseable count on a matched suffix must fail closed");
        assert_eq!(err.key, "validation-password-uncompromised");
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
async fn uncompromised_non_2xx_status_fails_open() {
    Http::fake(|| async {
        Http::fake_response_text("GET", "api.pwnedpasswords.com/range/5BAA6", 503, "");
        let rule = Password::min(1).uncompromised();
        assert!(
            AsyncRule::passes(&rule, "password").await.is_ok(),
            "a non-2xx HIBP response must fail open, same as a transport error"
        );
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
async fn uncompromised_fails_open_when_hibp_is_unreachable() {
    Http::fake(|| async {
        // No fake entry matches + FailOnRealCallsGuard => the client call errs,
        // which is the same arm a transport error or timeout takes. The fake
        // cannot simulate a timeout (interception happens before transport).
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        let rule = Password::min(1).uncompromised();
        assert!(
            AsyncRule::passes(&rule, "definitely-unique-p@ssw0rd-77")
                .await
                .is_ok(),
            "HIBP unreachable must fail open, per NotPwnedVerifier"
        );
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
#[traced_test]
async fn uncompromised_fail_open_log_never_leaks_the_prefix() {
    Http::fake(|| async {
        // Same unreachable-HIBP setup as `uncompromised_fails_open_when_hibp_is_unreachable`,
        // but this test inspects the actual `tracing::warn!` output: the
        // transport error `FailOnRealCallsGuard` produces embeds the full
        // request URL (see `http_client/mod.rs`'s fail-on-real-calls
        // message), and that URL contains the k-anonymity prefix. The rule
        // documented on `HibpVerifier` is that the prefix never appears in
        // a log line - this pins that the fail-open branch actually scrubs
        // it rather than logging the transport error's `Display` verbatim.
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        let verifier = suprnova::HibpVerifier::default();
        // sha1("password") = 5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8; the
        // 5-character prefix sent over the wire is "5BAA6".
        let clean = verifier
            .verify("password", 0)
            .await
            .expect("an unreachable HIBP must fail open (Ok), not Err");
        assert!(clean);
        assert!(
            logs_contain("failing open"),
            "the fail-open branch must actually log a warning"
        );
        assert!(
            !logs_contain("5BAA6"),
            "the k-anonymity prefix must never appear in the fail-open log line, \
             even though it is embedded in the transport error's own Display text"
        );
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
async fn empty_value_is_compromised_without_a_network_call() {
    Http::fake(|| async {
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        // min(1) already rejects ""; call the verifier directly to pin the
        // Laravel rule that an empty value reports compromised, not clean.
        let verifier = suprnova::HibpVerifier::default();
        assert!(!verifier.verify("", 0).await.expect("no network needed"));
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[test]
fn sync_use_of_uncompromised_is_a_loud_error() {
    let rule = Password::min(1).uncompromised();
    let err = Rule::passes(&rule, "anything").expect_err("must not silently skip the check");
    assert!(err.to_string().contains("after_validation_async"));
}

#[tokio::test]
async fn custom_verifier_overrides_hibp() {
    struct AlwaysLeaked;
    #[async_trait::async_trait]
    impl UncompromisedVerifier for AlwaysLeaked {
        async fn verify(&self, _v: &str, _t: u32) -> Result<bool, FrameworkError> {
            Ok(false)
        }
    }
    Http::fake(|| async {
        // A configured custom verifier must never reach the network at
        // all. The guard turns a regression in verifier selection
        // (falling through to the default HibpVerifier instead of this
        // one) into a fast local error instead of a live call to
        // api.pwnedpasswords.com from the unattended gate.
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        let rule = Password::min(1)
            .uncompromised()
            .verifier(Arc::new(AlwaysLeaked));
        assert!(AsyncRule::passes(&rule, "whatever").await.is_err());
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}

#[tokio::test]
#[traced_test]
async fn a_broken_custom_verifier_never_leaks_its_error_into_the_422_body() {
    // `Err` from a verifier is an implementation bug, not a user problem.
    // Its detail belongs to the operator (the log), and the client must see
    // only the fixed, translatable "could not be checked" message - never an
    // infrastructure string that would otherwise ride a 4xx body straight
    // around the 5xx sanitisation every other operational fault gets.
    struct Broken;
    #[async_trait::async_trait]
    impl UncompromisedVerifier for Broken {
        async fn verify(&self, _v: &str, _t: u32) -> Result<bool, FrameworkError> {
            Err(FrameworkError::internal(
                "connect to hibp-proxy.internal:8443 refused",
            ))
        }
    }
    Http::fake(|| async {
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        let rule = Password::min(1).uncompromised().verifier(Arc::new(Broken));
        let err = AsyncRule::passes(&rule, "whatever")
            .await
            .expect_err("a broken verifier fails the check rather than passing it");
        assert_eq!(err.key, "validation-password-unverifiable");
        let rendered = err.to_string();
        assert!(
            !rendered.contains("hibp-proxy") && !rendered.contains("8443"),
            "verifier detail leaked into the user-facing message: {rendered}"
        );
        assert!(logs_contain("hibp-proxy.internal:8443"));
        assert!(logs_contain("the check did not run"));
        Ok::<_, FrameworkError>(())
    })
    .await
    .expect("fake scope");
}
