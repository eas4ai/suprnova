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

/// Strong comparison of `If-None-Match` against the represented validator.
#[must_use]
pub fn evaluate_conditional(
    if_none_match: Option<&str>,
    validator: &Validator,
) -> ConditionalOutcome {
    let Some(header) = if_none_match else {
        return ConditionalOutcome::Full;
    };
    if header.trim() == "*" {
        return ConditionalOutcome::NotModified;
    }
    let etag = validator.etag();
    let matched = header
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == etag);
    if matched {
        ConditionalOutcome::NotModified
    } else {
        ConditionalOutcome::Full
    }
}

/// `Cache-Control` for a class, shared policy, freshness, and optional seed
/// deadline (milliseconds remaining); private classes are never public.
/// Shell-stitched responses take the private treatment until the stitching
/// work defines their shared-cache behavior, so the default is conservative
/// rather than accidental.
#[must_use]
pub fn cache_control_value(
    class: RepresentationClass,
    shared: SharedCachePolicy,
    freshness: &FreshnessPolicy,
    seed_remaining_ms: Option<u64>,
) -> String {
    let mut max_age = freshness.fresh_ms() / 1_000;
    if let Some(remaining) = seed_remaining_ms {
        max_age = max_age.min(remaining / 1_000);
    }
    match (class, shared) {
        (RepresentationClass::PublicShared, SharedCachePolicy::SMaxAge { seconds }) => {
            let s_maxage =
                seed_remaining_ms.map_or(u64::from(seconds), |r| u64::from(seconds).min(r / 1_000));
            format!("public, max-age={max_age}, s-maxage={s_maxage}")
        }
        _ => format!("private, max-age={max_age}"),
    }
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
