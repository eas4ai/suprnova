//! Closed recovery state shared by later endpoint adapters.

/// Whether the exact action body may be attempted again automatically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryLegality {
    /// No accepted or possibly committed effect blocks transport retry.
    Allowed,
    /// The request path must fresh-render and never replay the action.
    Prohibited,
}
