//! Deterministic route and group policy resolution.

use std::collections::BTreeMap;

use suprnova_live::render_cache::{PolicyPatch, RenderCachePolicy, VarianceDimension};

use crate::FrameworkError;

/// Whether this host gives `dimension` a value when it builds a key.
///
/// The match names every dimension, so a dimension that the engine gains
/// does not compile here until somebody decides which side it is on.
pub(crate) fn has_a_producer(dimension: &VarianceDimension) -> bool {
    match dimension {
        VarianceDimension::Host
        | VarianceDimension::Locale
        | VarianceDimension::Media
        | VarianceDimension::Encoding
        | VarianceDimension::Tenant
        | VarianceDimension::Principal => true,
        VarianceDimension::FeatureVersion
        | VarianceDimension::ConfigVersion
        | VarianceDimension::Application(_) => false,
    }
}

/// Refuses a policy that varies on a dimension without a producer.
///
/// The key of a stored response carries one value for every dimension its
/// policy declares. A route whose policy declares a dimension that nothing
/// gives a value can never build its key, so every request for it goes
/// past the cache, and nothing about the route says so. The engine accepts
/// such a policy, because another host can have the producer. This host
/// refuses it where the application registers it, which is at boot.
fn refuse_a_dimension_without_a_producer(
    target: &str,
    policy: &RenderCachePolicy,
) -> Result<(), FrameworkError> {
    match policy
        .vary()
        .iter()
        .find(|dimension| !has_a_producer(dimension))
    {
        Some(dimension) => Err(FrameworkError::internal(format!(
            "RenderCache policy for `{target}` varies on {dimension:?}, and nothing gives \
             that dimension a value, so no response for `{target}` could be cached. \
             Remove the dimension from the policy."
        ))),
        None => Ok(()),
    }
}

/// A group's policy: a full policy at the root of a subtree or a patch of an
/// enclosing group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GroupPolicy {
    /// A complete policy.
    Policy(RenderCachePolicy),
    /// A narrowing patch of the enclosing group.
    Patch(PolicyPatch),
}

impl From<RenderCachePolicy> for GroupPolicy {
    fn from(policy: RenderCachePolicy) -> Self {
        Self::Policy(policy)
    }
}

impl From<PolicyPatch> for GroupPolicy {
    fn from(patch: PolicyPatch) -> Self {
        Self::Patch(patch)
    }
}

/// Registered policies: exact route patterns and group prefixes.
#[derive(Clone, Debug, Default)]
pub struct RenderCachePolicyTable {
    routes: BTreeMap<String, GroupPolicy>,
    groups: BTreeMap<String, GroupPolicy>,
}

impl RenderCachePolicyTable {
    /// Registers a group prefix once.
    pub fn register_group(
        &mut self,
        prefix: &str,
        policy: GroupPolicy,
    ) -> Result<(), FrameworkError> {
        if self.groups.contains_key(prefix) {
            return Err(FrameworkError::internal(
                "RenderCache group policy registered twice",
            ));
        }
        match &policy {
            GroupPolicy::Policy(full) => refuse_a_dimension_without_a_producer(prefix, full)?,
            GroupPolicy::Patch(patch) => self.validate_patch(
                prefix,
                prefix,
                patch,
                "RenderCache group patch has no enclosing policy",
                "RenderCache group patch widens sharing",
            )?,
        }
        self.groups.insert(prefix.to_owned(), policy);
        Ok(())
    }

    /// Registers one route pattern once; the pattern must already be routed.
    pub fn register_route(
        &mut self,
        pattern: &str,
        policy: GroupPolicy,
    ) -> Result<(), FrameworkError> {
        if self.routes.contains_key(pattern) {
            return Err(FrameworkError::internal(
                "RenderCache route policy registered twice",
            ));
        }
        match &policy {
            GroupPolicy::Policy(full) => refuse_a_dimension_without_a_producer(pattern, full)?,
            GroupPolicy::Patch(patch) => self.validate_patch(
                pattern,
                "",
                patch,
                "RenderCache route patch has no group policy",
                "RenderCache route patch widens sharing",
            )?,
        }
        self.routes.insert(pattern.to_owned(), policy);
        Ok(())
    }

    /// Resolves the enclosing policy for `key` (excluding `exclude`) and
    /// checks that `patch` only narrows it, and that the policy the patch
    /// makes varies only on dimensions this host gives a value. Shared by
    /// `register_group` and `register_route`, which differ only in the
    /// exclusion and the error wording for their respective contexts.
    fn validate_patch(
        &self,
        key: &str,
        exclude: &str,
        patch: &PolicyPatch,
        missing_enclosing_message: &str,
        widens_message: &str,
    ) -> Result<(), FrameworkError> {
        let enclosing = self
            .enclosing(key, exclude)
            .ok_or_else(|| FrameworkError::internal(missing_enclosing_message))?;
        let patched = enclosing
            .apply(patch)
            .map_err(|_| FrameworkError::internal(widens_message))?;
        refuse_a_dimension_without_a_producer(key, &patched)
    }

    /// The effective policy for a matched route pattern, or `None`.
    #[must_use]
    pub fn effective_policy(&self, pattern: &str) -> Option<RenderCachePolicy> {
        match self.routes.get(pattern) {
            Some(GroupPolicy::Policy(policy)) => Some(policy.clone()),
            Some(GroupPolicy::Patch(patch)) => self
                .enclosing(pattern, "")
                .and_then(|group| group.apply(patch).ok()),
            None => self.enclosing(pattern, ""),
        }
    }

    /// Resolves groups from the longest prefix inward, excluding `exclude`.
    fn enclosing(&self, pattern: &str, exclude: &str) -> Option<RenderCachePolicy> {
        let mut chain: Vec<(&String, &GroupPolicy)> = self
            .groups
            .iter()
            .filter(|(prefix, _)| {
                prefix.as_str() != exclude
                    && (pattern == prefix.as_str()
                        || pattern.starts_with(&format!("{}/", prefix.trim_end_matches('/'))))
            })
            .collect();
        chain.sort_by_key(|(prefix, _)| prefix.len());
        let mut effective: Option<RenderCachePolicy> = None;
        for (_, policy) in chain {
            effective = match (effective, policy) {
                (_, GroupPolicy::Policy(policy)) => Some(policy.clone()),
                (Some(parent), GroupPolicy::Patch(patch)) => parent.apply(patch).ok(),
                (None, GroupPolicy::Patch(_)) => None,
            };
        }
        effective
    }
}
