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
        filter.validate(descriptor).is_ok() && filter.matches(document)
    })
}
