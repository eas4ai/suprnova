//! Bounded canonical JSON values used by signed Live contracts.

mod parser;
mod serializer;
mod value;

pub use parser::parse_canonical_value;
pub use serializer::to_canonical_bytes;
pub use value::{CanonicalError, CanonicalErrorKind, CanonicalNumber, CanonicalValue};
