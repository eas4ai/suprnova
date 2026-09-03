//! Deterministic route and group policy resolution.

use std::collections::BTreeMap;

use suprnova_live::render_cache::{PolicyPatch, RenderCachePolicy};

use crate::FrameworkError;

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
        if let GroupPolicy::Patch(patch) = &policy {
            let enclosing = self.enclosing(prefix, prefix).ok_or_else(|| {
                FrameworkError::internal("RenderCache group patch has no enclosing policy")
            })?;
            enclosing
                .apply(patch)
                .map_err(|_| FrameworkError::internal("RenderCache group patch widens sharing"))?;
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
        if let GroupPolicy::Patch(patch) = &policy {
            let group = self.enclosing(pattern, "").ok_or_else(|| {
                FrameworkError::internal("RenderCache route patch has no group policy")
            })?;
            group
                .apply(patch)
                .map_err(|_| FrameworkError::internal("RenderCache route patch widens sharing"))?;
        }
        self.routes.insert(pattern.to_owned(), policy);
        Ok(())
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
