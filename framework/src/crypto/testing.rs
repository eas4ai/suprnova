//! Ciphertext for a test of a key rotation.
//!
//! [`Crypt`](super::Crypt) encrypts with the current key and the current
//! label and with nothing else, so no call of it writes a value the way
//! it was written before a rotation. A test of a rotation needs such a
//! value: one under the key that is a previous key now, or under the
//! label that values had before the name of their context was a part of
//! it. The two functions here write them.
//!
//! The module is compiled with the `testing` feature. An application
//! that ships without the feature has none of it.
//!
//! ```rust
//! use suprnova::crypto::testing::encrypt_string_under;
//! use suprnova::{CryptPurpose, EncryptionKey};
//!
//! # fn main() -> Result<(), suprnova::FrameworkError> {
//! // The key the application had before the rotation.
//! let old_key = EncryptionKey::generate();
//! let stored = encrypt_string_under(&old_key, CryptPurpose::Cast, "a value")?;
//! assert!(suprnova::Crypt::appears_encrypted(&stored));
//! # Ok(()) }
//! ```

use super::{CryptPurpose, EncryptionKey, aead};
use crate::FrameworkError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

/// Encrypt `plaintext` with `key` and the label of `purpose`, the way
/// [`Crypt::encrypt_string`](super::Crypt::encrypt_string) does with the
/// current key.
///
/// With a key that is one of the previous keys of the ring, the result is
/// a value that was written before the rotation: it decrypts with
/// [`KeyOrigin::Previous`](super::KeyOrigin::Previous). Read through a
/// function that takes a context, such as
/// [`Crypt::decrypt_string_for`](super::Crypt::decrypt_string_for), it is
/// a value under the legacy label as well.
///
/// With [`CryptPurpose::Cookie`] it writes what `Crypt` refuses to write:
/// a cookie value under the v1 label, which has no cookie name in it. No
/// read of a cookie opens it, under any key of the ring; a test makes it
/// to check exactly that.
///
/// # Errors
///
/// When the cipher refuses, which it does for no input a test gives it.
pub fn encrypt_string_under(
    key: &EncryptionKey,
    purpose: CryptPurpose,
    plaintext: &str,
) -> Result<String, FrameworkError> {
    let wire = aead::encrypt(key, purpose.aad(), plaintext.as_bytes())?;
    Ok(URL_SAFE_NO_PAD.encode(wire))
}

/// Encrypt `plaintext` with `key` and the label of `purpose` and
/// `context`, the way
/// [`Crypt::encrypt_string_for`](super::Crypt::encrypt_string_for) does
/// with the current key.
///
/// With a previous key this is a value under a previous key and the
/// current label, which no call of `Crypt` writes.
///
/// # Errors
///
/// When the cipher refuses, which it does for no input a test gives it.
pub fn encrypt_string_for_under(
    key: &EncryptionKey,
    purpose: CryptPurpose,
    context: &str,
    plaintext: &str,
) -> Result<String, FrameworkError> {
    let wire = aead::encrypt(key, &purpose.aad_for(context), plaintext.as_bytes())?;
    Ok(URL_SAFE_NO_PAD.encode(wire))
}
