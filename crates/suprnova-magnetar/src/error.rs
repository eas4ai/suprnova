//! Error and result primitives shared by the crate's foundation modules.

use core::fmt;

/// The error categories shared by Magnetar operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An input value does not satisfy the operation's contract.
    InvalidInput {
        /// The input field associated with the failure.
        field: String,
        /// The reason the value was rejected.
        message: String,
    },
    /// The requested resource could not be found.
    NotFound {
        /// The kind of resource that was requested.
        resource: String,
        /// The identifier used for the lookup.
        identifier: String,
    },
    /// The requested operation conflicts with existing state.
    Conflict {
        /// The kind of resource whose state conflicts.
        resource: String,
        /// The reason the operation conflicts.
        message: String,
    },
    /// A required dependency is unavailable and the operation must fail
    /// closed.
    DependencyUnavailable {
        /// The dependency that could not satisfy the operation.
        dependency: String,
        /// The dependency failure detail.
        message: String,
    },
    /// An otherwise unclassified internal failure.
    Internal {
        /// The internal failure detail.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { field, message } => {
                write!(formatter, "invalid input for {field}: {message}")
            }
            Self::NotFound {
                resource,
                identifier,
            } => write!(formatter, "{resource} not found: {identifier}"),
            Self::Conflict { resource, message } => {
                write!(formatter, "conflict for {resource}: {message}")
            }
            Self::DependencyUnavailable {
                dependency,
                message,
            } => write!(formatter, "dependency {dependency} unavailable: {message}"),
            Self::Internal { message } => write!(formatter, "internal error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// The result type returned by Magnetar operations.
pub type Result<T> = core::result::Result<T, Error>;
