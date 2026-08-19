//! `FrameworkError::External` carries its source, and the chain renderer
//! makes that source visible in logs. Without the renderer the variant is
//! a diagnostics regression, so both are pinned here together.

use std::error::Error;
use suprnova::FrameworkError;

#[derive(Debug)]
struct Inner(&'static str);

impl std::fmt::Display for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inner failure: {}", self.0)
    }
}

impl Error for Inner {}

#[test]
fn from_external_uses_the_errors_display_as_the_message() {
    let e = FrameworkError::from_external(Inner("disk"));
    assert_eq!(e.to_string(), "inner failure: disk");
    assert_eq!(e.status_code(), 500);
}

#[test]
fn from_external_with_overrides_the_message_and_keeps_the_source() {
    let e = FrameworkError::from_external_with("saving user", Inner("disk"));
    assert_eq!(e.to_string(), "saving user");
    assert_eq!(e.status_code(), 500);
    assert_eq!(
        e.external_source().map(|s| s.to_string()),
        Some("inner failure: disk".to_string())
    );
}

#[test]
fn external_source_downcasts_to_the_concrete_error_type() {
    // `source()` hands back the `Arc` node, not the wrapped error, so
    // `source().downcast_ref::<Inner>()` is `None`. `external_source()`
    // exists precisely so a retry policy can still probe the concrete type.
    let e = FrameworkError::from_external_with("saving user", Inner("disk"));
    let src = e.external_source().expect("external carries a source");
    assert!(src.downcast_ref::<Inner>().is_some());
}

#[test]
fn error_source_is_populated() {
    let e = FrameworkError::from_external_with("saving user", Inner("disk"));
    assert!(Error::source(&e).is_some());
}

#[test]
fn non_external_variants_have_no_external_source() {
    assert!(FrameworkError::internal("boom").external_source().is_none());
    assert!(Error::source(&FrameworkError::internal("boom")).is_none());
}

#[test]
fn context_preserves_the_external_source() {
    // The `context` match ends in a catch-all that flattens to `Domain`.
    // Without an explicit `External` arm the `Arc` is discarded silently -
    // no compiler error, no warning.
    let e =
        FrameworkError::from_external_with("saving user", Inner("disk")).context("http handler");
    assert_eq!(e.to_string(), "http handler: saving user");
    assert_eq!(e.status_code(), 500);
    assert_eq!(
        e.external_source().map(|s| s.to_string()),
        Some("inner failure: disk".to_string()),
        "context() dropped the source"
    );
}

#[test]
fn clone_shares_the_source() {
    let e = FrameworkError::from_external_with("saving user", Inner("disk"));
    let c = e.clone();
    assert_eq!(
        c.external_source().map(|s| s.to_string()),
        Some("inner failure: disk".to_string())
    );
}

#[test]
fn rendered_chain_includes_the_source_text() {
    // This is the regression guard for the whole item: a migrated call site
    // must log at least as much as the `format!` it replaced.
    let e = FrameworkError::from_external_with("verify query failed", Inner("disk"));
    let rendered = suprnova::render_error_chain(&e);
    assert!(
        rendered.contains("verify query failed"),
        "chain lost the context: {rendered}"
    );
    assert!(
        rendered.contains("inner failure: disk"),
        "chain lost the source: {rendered}"
    );
}

#[test]
fn rendered_chain_does_not_stutter_when_message_came_from_the_source() {
    // `from_external` sets `message = err.to_string()`, so a naive walker
    // emits "inner failure: disk: inner failure: disk".
    let e = FrameworkError::from_external(Inner("disk"));
    assert_eq!(suprnova::render_error_chain(&e), "inner failure: disk");
}

#[test]
fn rendered_chain_of_a_sourceless_error_is_just_its_display() {
    let e = FrameworkError::internal("boom");
    assert_eq!(
        suprnova::render_error_chain(&e),
        "Internal server error: boom"
    );
}

#[test]
fn external_renders_as_a_sanitised_500() {
    // 5xx sanitisation still applies: the client gets a generic message,
    // the detail goes to logs.
    let resp = suprnova::HttpResponse::from(FrameworkError::from_external_with(
        "saving user",
        Inner("disk"),
    ));
    assert_eq!(resp.status_code(), 500);
}

#[test]
fn migrated_call_site_logs_at_least_what_the_format_did() {
    // The shape the three dogfood sites used to build by hand, and the
    // shape they build now. The rendered chain must not lose either half.
    #[derive(Debug)]
    struct DbLike;
    impl std::fmt::Display for DbLike {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "UNIQUE constraint failed: bench_job_runs.job_id")
        }
    }
    impl Error for DbLike {}

    let old = format!("verify query failed: {}", DbLike);
    let new = suprnova::render_error_chain(&FrameworkError::from_external_with(
        "verify query failed",
        DbLike,
    ));

    assert_eq!(new, old, "migration changed what an operator sees");
}
