//! A small searchable model owned by the search component's dogfood page.

use chrono::{DateTime, Utc};

/// Dedicated rows exercise search without adding write hooks to ordinary todo operations.
#[suprnova::model(table = "dogfood_search_notes", fillable = ["title", "body", "done"], timestamps,
    casts = { done = ::suprnova::AsBool })]
#[derive(suprnova::Searchable)]
#[searchable(index = "dogfood_search_notes")]
pub struct SearchNote {
    /// The database identity becomes the result key.
    pub id: i64,
    /// Titles carry native lexical relevance and escaped highlight fragments.
    #[searchable(text)]
    pub title: String,
    /// Bodies prove several declared text fields can appear in a result.
    #[searchable(text)]
    pub body: Option<String>,
    /// A native checkbox filters one declared facet.
    #[searchable(filterable, facet)]
    pub done: bool,
    /// Creation timestamps remain model data rather than public search attributes.
    pub created_at: DateTime<Utc>,
    /// Monotonic versions let the index reject stale writes.
    pub updated_at: DateTime<Utc>,
}
