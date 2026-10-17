//! Viewer scope is applied before retrieval and checked again against live rows.

use super::{document::SearchDocument, query::Filter, registry::IndexDescriptor};
use crate::{Auth, FrameworkError, auth::Authenticatable};
use std::fmt;
use std::sync::Arc;

/// A named viewer prevents background reads from accidentally becoming unscoped.
#[derive(Clone)]
pub enum Viewer {
    /// An authenticated identity gives scope functions the same user as the request.
    Authenticated {
        /// String identifiers support the framework's integer and UUID user keys.
        id: String,
        /// The authenticated user allows scopes to consult roles and permissions.
        user: Arc<dyn Authenticatable>,
    },
    /// Missing request identity always receives the guest scope.
    Guest,
}

impl fmt::Debug for Viewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authenticated { id, .. } => f
                .debug_struct("Authenticated")
                .field("id", id)
                .finish_non_exhaustive(),
            Self::Guest => f.write_str("Guest"),
        }
    }
}

impl Viewer {
    /// Read through Auth so provider failures propagate rather than bypassing scope.
    pub async fn current() -> Result<Self, FrameworkError> {
        match Auth::user().await? {
            Some(user) => Ok(Self::Authenticated {
                id: user.get_auth_identifier(),
                user,
            }),
            None => Ok(Self::Guest),
        }
    }

    /// Optional identity is convenient for scope functions without inventing guest ids.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Authenticated { id, .. } => Some(id),
            Self::Guest => None,
        }
    }
}

/// Combine the code-owned scope with user filters before any engine read.
pub fn apply_scope(
    descriptor: &IndexDescriptor,
    viewer: &Viewer,
    filter: Option<Filter>,
) -> Result<Option<Filter>, FrameworkError> {
    if let Some(filter) = &filter {
        filter.validate(descriptor)?;
    }
    let scope = descriptor.scope.map(|scope| scope(viewer));
    if let Some(scope) = &scope {
        scope.validate(descriptor)?;
        reads_only_scope_fields(descriptor, scope)?;
    }
    Ok(match (filter, scope) {
        (Some(filter), Some(scope)) => Some(Filter::And(vec![filter, scope])),
        (filter, None) => filter,
        (None, scope) => scope,
    })
}

/// Check a freshly documented row to close the interval between indexing and hydration.
pub fn evaluate_scope(
    descriptor: &IndexDescriptor,
    viewer: &Viewer,
    document: &SearchDocument,
) -> bool {
    descriptor.scope.is_none_or(|scope| {
        let filter = scope(viewer);
        filter.validate(descriptor).is_ok()
            && reads_only_scope_fields(descriptor, &filter).is_ok()
            && filter.matches(document)
    })
}

/// A recommendation event records the target's scope-field values as its access boundary,
/// so a scope that reads another field would leave part of that boundary unrecorded.
fn reads_only_scope_fields(
    descriptor: &IndexDescriptor,
    scope: &Filter,
) -> Result<(), FrameworkError> {
    let mut pending = vec![scope];
    while let Some(filter) = pending.pop() {
        let field = match filter {
            Filter::And(children) | Filter::Or(children) => {
                pending.extend(children);
                continue;
            }
            Filter::Not(child) => {
                pending.push(child);
                continue;
            }
            Filter::Eq(field, _)
            | Filter::Ne(field, _)
            | Filter::In(field, _)
            | Filter::NotIn(field, _)
            | Filter::Gt(field, _)
            | Filter::Gte(field, _)
            | Filter::Lt(field, _)
            | Filter::Lte(field, _)
            | Filter::Between(field, _, _)
            | Filter::IsNull(field)
            | Filter::IsNotNull(field)
            | Filter::GeoRadius(field, _, _) => field,
        };
        if !descriptor.scope_fields.contains(&field.as_str()) {
            return Err(FrameworkError::param(format!(
                "index '{}' scope reads '{field}', which its scope_fields do not name",
                descriptor.name
            )));
        }
    }
    Ok(())
}
