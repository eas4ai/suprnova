//! Opaque temporary upload identities and secret transfer capabilities.

mod identity;

pub use identity::{
    IssuedTransferGrant, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, UploadError, UploadErrorKind, UploadHandle, VerifiedTransferGrant,
};
