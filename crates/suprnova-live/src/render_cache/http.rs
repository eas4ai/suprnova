//! Conditional evaluation and cache metadata for represented variants.

use super::entry::Validator;
use super::policy::{FreshnessPolicy, RepresentationClass, SharedCachePolicy};
use super::variance::VarianceDescriptor;

/// Whether a conditional request is satisfied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConditionalOutcome {
    /// Return 304 with metadata and no body.
    NotModified,
    /// Return the full representation.
    Full,
}

/// Strong comparison of `If-None-Match` against an already formed entity
/// tag; [`evaluate_conditional`] is this rule applied to a validator. A hot
/// hit holds its tag as text already, so it compares without forming one.
#[must_use]
pub fn conditional_matches(if_none_match: Option<&str>, etag: &str) -> ConditionalOutcome {
    let Some(header) = if_none_match else {
        return ConditionalOutcome::Full;
    };
    if header.trim() == "*" {
        return ConditionalOutcome::NotModified;
    }
    if header
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == etag)
    {
        ConditionalOutcome::NotModified
    } else {
        ConditionalOutcome::Full
    }
}

/// Strong comparison of `If-None-Match` against the represented validator;
/// exactly [`conditional_matches`] over the validator's entity tag.
#[must_use]
pub fn evaluate_conditional(
    if_none_match: Option<&str>,
    validator: &Validator,
) -> ConditionalOutcome {
    conditional_matches(if_none_match, &validator.etag())
}

/// Writes the `Cache-Control` for a class, shared policy, freshness, and
/// optional seed deadline (milliseconds remaining) into any formatter;
/// private classes are never public. Shell-stitched responses are assembled
/// per request and take the private treatment by definition: the shared
/// shell is what the server caches, never what a downstream cache may
/// share.
///
/// This is the only place the directive text is formed.
/// [`cache_control_value`] is this function over a `String`, and the hot
/// path is this function over a fixed stack buffer, so a request-formed
/// value and an allocated one can never disagree.
pub(super) fn write_cache_control(
    out: &mut impl core::fmt::Write,
    class: RepresentationClass,
    shared: SharedCachePolicy,
    freshness: &FreshnessPolicy,
    seed_remaining_ms: Option<u64>,
) -> core::fmt::Result {
    let mut max_age = freshness.fresh_ms() / 1_000;
    if let Some(remaining) = seed_remaining_ms {
        max_age = max_age.min(remaining / 1_000);
    }
    match (class, shared) {
        (RepresentationClass::PublicShared, SharedCachePolicy::SMaxAge { seconds }) => {
            let s_maxage =
                seed_remaining_ms.map_or(u64::from(seconds), |r| u64::from(seconds).min(r / 1_000));
            write!(out, "public, max-age={max_age}, s-maxage={s_maxage}")
        }
        _ => write!(out, "private, max-age={max_age}"),
    }
}

/// `Cache-Control` for a class, shared policy, freshness, and optional seed
/// deadline (milliseconds remaining), as an owned `String`; private classes
/// are never public. Shell-stitched responses are assembled per request and
/// take the private treatment by definition: the shared shell is what the
/// server caches, never what a downstream cache may share.
///
/// One private writer inside this module forms the directive text, and the
/// hot path drives that same writer over a fixed stack buffer rather than a
/// `String`, so a request-formed value and this one can never disagree.
#[must_use]
pub fn cache_control_value(
    class: RepresentationClass,
    shared: SharedCachePolicy,
    freshness: &FreshnessPolicy,
    seed_remaining_ms: Option<u64>,
) -> String {
    let mut out = String::new();
    write_cache_control(&mut out, class, shared, freshness, seed_remaining_ms)
        .expect("formatting into a String cannot fail");
    out
}

/// `Vary` from the declared variance, if any header participates.
#[must_use]
pub fn vary_value(descriptor: &VarianceDescriptor) -> Option<String> {
    let headers = descriptor.vary_headers();
    if headers.is_empty() {
        None
    } else {
        Some(headers.join(", "))
    }
}
