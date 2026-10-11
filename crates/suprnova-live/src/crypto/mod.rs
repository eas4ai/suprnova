//! Purpose-separated HKDF-SHA-256 and HMAC-SHA-256 capability integrity.

mod key;
mod key_ring;
mod signature;

pub use key::{KeyError, KeyErrorKind, KeyRecord, RootKey, SnapshotPurpose};
pub use key_ring::SnapshotKeyRing;
pub use signature::{SignedMac, SnapshotSignature};
