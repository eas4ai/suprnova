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
//! (`suprnova::assert_sent`), not `Http::` associated items — only
//! `Http::fake`, `Http::fake_response_text`, and the request builders
//! (`Http::get`/`post`/...) are associated with `Http` itself.

use std::sync::Arc;
use suprnova::{
    AsyncRule, FrameworkError, Http, Password, Rule, UncompromisedVerifier, assert_sent,
};

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

#[tokio::test]
async fn uncompromised_flags_a_leaked_password_via_the_range_api() {
    // sha1("password") = 5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8
    // prefix 5BAA6, suffix 1E4C9B93F3F0682250B6CF8331B7EE68FD8
    Http::fake(|| async {
        Http::fake_response_text(
            "GET",
            "api.pwnedpasswords.com/range/5BAA6",
            200,
            "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n1E4C9B93F3F0682250B6CF8331B7EE68FD8:3730471\r\n",
        );
        let rule = Password::min(1).uncompromised();
        let err = AsyncRule::passes(&rule, "password").await.expect_err("leaked");
        assert_eq!(err.key, "validation-password-uncompromised");

        // The threshold comparison is strictly greater-than: a threshold at or
        // above the reported count (3_730_471) lets the password pass.
        let rule = Password::min(1).uncompromised_with_threshold(4_000_000);
        assert!(AsyncRule::passes(&rule, "password").await.is_ok());

        assert_sent(|req| {
            req.url.contains("/range/5BAA6")
                && !req.url.to_uppercase().contains("1E4C9B93")
                && req.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("add-padding") && v == "true")
        });
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
async fn empty_value_is_compromised_without_a_network_call() {
    Http::fake(|| async {
        let _guard = suprnova::http_client::FailOnRealCallsGuard::install();
        let rule = Password::min(1).uncompromised();
        // min(1) already rejects ""; call the verifier directly to pin the
        // Laravel rule that an empty value reports compromised, not clean.
        let verifier = suprnova::HibpVerifier::default();
        assert!(!verifier.verify("", 0).await.expect("no network needed"));
        let _ = rule;
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
    let rule = Password::min(1)
        .uncompromised()
        .verifier(Arc::new(AlwaysLeaked));
    assert!(AsyncRule::passes(&rule, "whatever").await.is_err());
}
