//! An encrypter that holds its own key.
//!
//! [`Crypt`](crate::Crypt) reads one key ring for the whole process, sealed
//! at boot from `APP_KEY`. Some values belong under another key: a
//! tenant's own key, a key you share with one other service, a key read
//! from a secrets manager after boot. [`Encrypter`] holds the key it is
//! built with, as Laravel's `new Encrypter($key)` does, and touches no
//! process-wide state.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use super::{
    CryptPurpose, EncryptionKey, KeyRing, aead, decrypt_bytes_for_with_ring, json_decode_error,
};
use crate::FrameworkError;

/// Encrypts and decrypts under the one key it holds, with the payload
/// format and the purpose labels of [`Crypt`](crate::Crypt).
///
/// Build one with [`Encrypter::new`]. Nothing is installed:
/// [`Crypt`](crate::Crypt) keeps the key ring `APP_KEY` gave it, and an
/// encrypter never reads that ring. Because the payload is the same, a
/// value [`Crypt::encrypt_string_for`](crate::Crypt::encrypt_string_for)
/// wrote under a key opens with `Encrypter::new` of that key, and a value
/// an encrypter wrote under the current `APP_KEY` opens with `Crypt`.
///
/// There is one cipher, AES-256-GCM, and one key size, 32 bytes. No
/// argument picks another: [`EncryptionKey`] refuses every other length,
/// so a weaker key never reaches the cipher.
///
/// The `Debug` output prints the key as `[REDACTED]`.
///
/// ```rust
/// use suprnova::{CryptPurpose, EncryptionKey, Encrypter};
///
/// # fn main() -> Result<(), suprnova::FrameworkError> {
/// let encrypter = Encrypter::new(EncryptionKey::try_generate()?);
/// let wire = encrypter.encrypt_string_for(CryptPurpose::Cast, "tenants.api_token", "tok_123")?;
/// let plain = encrypter.decrypt_string_for(CryptPurpose::Cast, "tenants.api_token", &wire)?;
/// assert_eq!(plain, "tok_123");
/// # Ok(()) }
/// ```
pub struct Encrypter {
    /// A ring of one key, so a contexted read goes through the same two
    /// passes as `Crypt`'s.
    ring: KeyRing,
}

impl Encrypter {
    /// Build an encrypter that holds `key` and nothing else.
    ///
    /// It installs nothing and reads no environment, so it works before
    /// boot, in a test, and next to a [`Crypt`](crate::Crypt) that holds
    /// another key.
    pub fn new(key: EncryptionKey) -> Self {
        Self {
            ring: KeyRing {
                current: key,
                previous: Vec::new(),
            },
        }
    }

    /// Encrypt `plaintext` under `purpose` and `context`, as
    /// [`Crypt::encrypt_string_for`](crate::Crypt::encrypt_string_for)
    /// does, with this encrypter's key.
    ///
    /// Returns URL-safe base64 without padding over
    /// `nonce || ciphertext || tag`, with a fresh random nonce for each
    /// call. The purpose and the context are bound into the tag, so the
    /// value opens only under the same pair. For [`CryptPurpose::Cookie`]
    /// the context is the logical cookie name.
    ///
    /// # Errors
    ///
    /// When the cipher refuses the plaintext.
    pub fn encrypt_string_for(
        &self,
        purpose: CryptPurpose,
        context: &str,
        plaintext: &str,
    ) -> Result<String, FrameworkError> {
        self.seal(&purpose.aad_for(context), plaintext.as_bytes())
    }

    /// Decrypt a value that [`Self::encrypt_string_for`] or
    /// [`Crypt::encrypt_string_for`](crate::Crypt::encrypt_string_for)
    /// wrote under this encrypter's key, `purpose` and `context`.
    ///
    /// The read is the one `Crypt` does. It tries the label with the
    /// context first. For every purpose but [`CryptPurpose::Cookie`] it
    /// then tries the label without a context, which a value stored
    /// before its context was bound carries. A cookie opens under the name
    /// it was written for and under no other.
    ///
    /// Unlike `Crypt`, it logs nothing. An encrypter has no previous keys,
    /// and a value under the label without a context is yours to write
    /// again with [`Self::encrypt_string_for`].
    ///
    /// # Errors
    ///
    /// When the wire is not base64, the key, the purpose or the context is
    /// not the one the value was written with, the bytes were changed, or
    /// the plaintext is not UTF-8.
    pub fn decrypt_string_for(
        &self,
        purpose: CryptPurpose,
        context: &str,
        wire: &str,
    ) -> Result<String, FrameworkError> {
        let bytes = decode_wire(wire)?;
        let (plain, _origin) = decrypt_bytes_for_with_ring(&self.ring, purpose, context, &bytes)?;
        String::from_utf8(plain).map_err(|e| {
            FrameworkError::internal(format!("Encrypter decrypted bytes not UTF-8: {e}"))
        })
    }

    /// Encode `value` as JSON and encrypt it under `purpose`, as
    /// [`Crypt::encrypt`](crate::Crypt::encrypt) does, with this
    /// encrypter's key.
    ///
    /// # Errors
    ///
    /// [`CryptPurpose::Cookie`] is refused before anything is encrypted. A
    /// cookie value is bound to the name of its cookie, and this function
    /// has no name to bind: use [`Self::encrypt_string_for`] with the name.
    /// Otherwise, when the value does not encode as JSON or the cipher
    /// refuses.
    pub fn encrypt<T: Serialize>(
        &self,
        purpose: CryptPurpose,
        value: &T,
    ) -> Result<String, FrameworkError> {
        if purpose == CryptPurpose::Cookie {
            return Err(cookie_encrypt_without_name());
        }
        let json = serde_json::to_vec(value)
            .map_err(|e| FrameworkError::internal(format!("Encrypter JSON encode failed: {e}")))?;
        self.seal(purpose.aad(), &json)
    }

    /// Decrypt a value that [`Self::encrypt`] or
    /// [`Crypt::encrypt`](crate::Crypt::encrypt) wrote under this
    /// encrypter's key and `purpose`, and decode its JSON as `T`.
    ///
    /// # Errors
    ///
    /// [`CryptPurpose::Cookie`] is refused before anything is decrypted.
    /// Otherwise, when the wire is not base64, the key or the purpose is
    /// not the one the value was written with, the bytes were changed, or
    /// the plaintext does not decode as `T`. The decode error names the
    /// kind of mistake and its position, and never quotes the value.
    pub fn decrypt<T: DeserializeOwned>(
        &self,
        purpose: CryptPurpose,
        wire: &str,
    ) -> Result<T, FrameworkError> {
        if purpose == CryptPurpose::Cookie {
            return Err(cookie_decrypt_without_name());
        }
        let bytes = decode_wire(wire)?;
        let plain = aead::decrypt(&self.ring.current, purpose.aad(), &bytes)?;
        serde_json::from_slice(&plain).map_err(|e| json_decode_error("Encrypter", &e))
    }

    /// Encrypt `plaintext` under this encrypter's key with `aad` bound
    /// into the tag, and encode the result as `Crypt` does.
    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<String, FrameworkError> {
        let wire = aead::encrypt(&self.ring.current, aad, plaintext)?;
        Ok(URL_SAFE_NO_PAD.encode(wire))
    }
}

impl Clone for Encrypter {
    /// A second encrypter with the same key. Cloning copies the 32 key
    /// bytes, and each copy scrubs them when it drops.
    fn clone(&self) -> Self {
        Self::new(self.ring.current.clone())
    }
}

impl std::fmt::Debug for Encrypter {
    /// Prints `Encrypter { key: EncryptionKey("[REDACTED]") }`, so an
    /// encrypter inside a logged struct never leaks its key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Encrypter")
            .field("key", &self.ring.current)
            .finish()
    }
}

/// Decode a base64 wire, naming `Encrypter` in the error so a failed read
/// points at the call that made it.
fn decode_wire(wire: &str) -> Result<Vec<u8>, FrameworkError> {
    URL_SAFE_NO_PAD
        .decode(wire.trim())
        .map_err(|e| FrameworkError::internal(format!("Encrypter base64 decode failed: {e}")))
}

/// The error of [`Encrypter::encrypt`] for [`CryptPurpose::Cookie`]. It
/// names the function that binds the cookie name, and nothing of the
/// value.
fn cookie_encrypt_without_name() -> FrameworkError {
    FrameworkError::internal(
        "Encrypter refuses to encrypt a cookie value without the name of its cookie: call \
         Encrypter::encrypt_string_for(CryptPurpose::Cookie, name, value)",
    )
}

/// The error of [`Encrypter::decrypt`] for [`CryptPurpose::Cookie`]. Only
/// a read with the cookie name checks the name the value was written for,
/// so a read without it opens nothing.
fn cookie_decrypt_without_name() -> FrameworkError {
    FrameworkError::internal(
        "Encrypter refuses to decrypt a cookie value without the name of its cookie: call \
         Encrypter::decrypt_string_for(CryptPurpose::Cookie, name, wire)",
    )
}
