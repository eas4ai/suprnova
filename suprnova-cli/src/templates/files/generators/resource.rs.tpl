//! The `{resource_type}` JSON:API resource.

use suprnova::Data;
use validator::Validate;

/// The `{resource_type}` resource. `id` is its JSON:API `id` and every other
/// field is one of its `attributes`. Render it with
/// `Resource::single(resource).render().await`, or a list of them with
/// `Resource::collection`.
#[derive(Debug, Clone, Data, Validate)]
#[json_resource("{resource_type}")]
pub struct {name} {
    /// The JSON:API `id`.
    pub id: u64,
}
