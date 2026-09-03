//! Test seams; hidden from documentation and never used by application code.

use super::registry::RenderCachePolicyTable;

/// The router's registered RenderCache policy table.
#[must_use]
pub fn policy_table(router: &crate::Router) -> RenderCachePolicyTable {
    router.render_cache_policies().clone()
}
