//! End-to-end encryption for `private-encrypted-` channels.
//!
//! Pusher's encrypted channels use the NaCl secretbox
//! (XSalsa20-Poly1305) with a per-channel key derived from one master
//! key, so the Pusher service only ever relays ciphertext.

use crate::FrameworkError;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crypto_secretbox::aead::Aead;
use crypto_secretbox::{Key, KeyInit, Nonce, XSalsa20Poly1305};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The wire shape of an encrypted event's `data`. A struct rather than
/// `json!` so the field order is fixed: `nonce`, then `ciphertext`.
#[derive(Serialize)]
struct EncryptedPayload {
    nonce: String,
    ciphertext: String,
}

/// The per-channel shared secret: `sha256(channel_name ++ master_key)`,
/// where `channel_name` is the full wire name.
///
/// Deriving it per channel means a client authorized for one encrypted
/// channel holds a key that opens no other channel.
pub(crate) fn shared_secret(channel_name: &str, master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(channel_name.as_bytes());
    hasher.update(master_key);
    hasher.finalize().into()
}

/// Encrypt `plaintext` for `channel_name` and return the JSON string
/// `{"nonce":"<base64>","ciphertext":"<base64>"}` that Pusher clients
/// decrypt.
///
/// Each call draws a fresh 24-byte nonce straight from the operating
/// system's RNG, the source `crate::crypto` draws its AES-GCM nonces
/// from. It reads it through the fallible call, so a failed read is an
/// error rather than a panic, and there is no user-space RNG state that
/// a forked process could share with its parent. XSalsa20's 192-bit
/// nonce makes a random nonce safe to use without a counter.
pub(crate) fn encrypt(
    channel_name: &str,
    master_key: &[u8; 32],
    plaintext: &[u8],
) -> Result<String, FrameworkError> {
    let nonce = nonce_from(&mut rand::rngs::SysRng)?;
    encrypt_with_nonce(channel_name, master_key, plaintext, nonce)
}

/// Encrypt with a caller-chosen nonce. Outside the tests only
/// [`encrypt`] calls it, always with a fresh random nonce: reusing a
/// nonce under one key breaks the cipher. The seam exists so a
/// known-answer test can pin the exact bytes pusher-js decrypts.
///
/// The ciphertext is the NaCl secretbox layout: the 16-byte tag, then
/// the encrypted bytes.
pub(crate) fn encrypt_with_nonce(
    channel_name: &str,
    master_key: &[u8; 32],
    plaintext: &[u8],
    nonce: [u8; 24],
) -> Result<String, FrameworkError> {
    let key = Key::from(shared_secret(channel_name, master_key));
    let cipher = XSalsa20Poly1305::new(&key);
    let ciphertext = cipher
        .encrypt(&Nonce::from(nonce), plaintext)
        .map_err(|_| {
            FrameworkError::internal(format!(
                "Pusher encryption for channel '{channel_name}' failed"
            ))
        })?;
    let payload = EncryptedPayload {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    };
    serde_json::to_string(&payload).map_err(|e| {
        FrameworkError::from_external_with("serializing an encrypted Pusher payload failed", e)
    })
}

/// Fill a nonce from `rng`, mapping a failed read to an error. Generic
/// so a test can stand in an RNG that fails.
fn nonce_from<R: rand::TryRng>(rng: &mut R) -> Result<[u8; 24], FrameworkError> {
    let mut nonce = [0u8; 24];
    rng.try_fill_bytes(&mut nonce).map_err(|e| {
        FrameworkError::internal(format!(
            "drawing a Pusher encryption nonce from the OS random number generator failed: {e}"
        ))
    })?;
    Ok(nonce)
}

/// Reverse [`encrypt`]. Only the tests need it: the server never reads
/// its own ciphertext back.
#[cfg(test)]
pub(crate) fn decrypt(
    channel_name: &str,
    master_key: &[u8; 32],
    payload: &str,
) -> Result<Vec<u8>, FrameworkError> {
    let parsed: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| FrameworkError::from_external_with("encrypted payload is not JSON", e))?;
    let field = |name: &str| -> Result<Vec<u8>, FrameworkError> {
        let text = parsed[name]
            .as_str()
            .ok_or_else(|| FrameworkError::internal(format!("encrypted payload has no {name}")))?;
        STANDARD.decode(text).map_err(|_| {
            FrameworkError::internal(format!("encrypted payload {name} is not base64"))
        })
    };
    let nonce: [u8; 24] = field("nonce")?
        .try_into()
        .map_err(|_| FrameworkError::internal("encrypted payload nonce is not 24 bytes"))?;
    let ciphertext = field("ciphertext")?;
    let key = Key::from(shared_secret(channel_name, master_key));
    XSalsa20Poly1305::new(&key)
        .decrypt(&Nonce::from(nonce), ciphertext.as_slice())
        .map_err(|_| FrameworkError::internal("encrypted payload failed authentication"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    fn master_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = i as u8;
        }
        key
    }

    #[test]
    fn pusher_shared_secret_matches_vector() {
        assert_eq!(
            STANDARD.encode(master_key()),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
        );
        let secret = shared_secret("private-encrypted-orders.42", &master_key());
        assert_eq!(
            STANDARD.encode(secret),
            "1p2xOYUKMFi7esECwtVrgbYWw5p/Jyo0M/uighBM4/Y="
        );
    }

    #[test]
    fn pusher_encrypt_then_decrypt_round_trips() {
        let channel = "private-encrypted-orders.42";
        let plaintext = br#"{"id":42}"#;
        let payload = encrypt(channel, &master_key(), plaintext).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let nonce = STANDARD
            .decode(parsed["nonce"].as_str().expect("nonce is a string"))
            .unwrap();
        assert_eq!(nonce.len(), 24, "the nonce is 24 bytes");
        let ciphertext = STANDARD
            .decode(
                parsed["ciphertext"]
                    .as_str()
                    .expect("ciphertext is a string"),
            )
            .unwrap();
        // NaCl secretbox: a 16-byte Poly1305 tag, then the ciphertext.
        assert_eq!(ciphertext.len(), plaintext.len() + 16);
        assert!(
            !payload.contains(r#""id":42"#),
            "the plaintext never appears in the payload: {payload}"
        );

        assert_eq!(
            decrypt(channel, &master_key(), &payload).unwrap(),
            plaintext
        );

        // A fresh nonce per message.
        let again = encrypt(channel, &master_key(), plaintext).unwrap();
        assert_ne!(payload, again);
    }

    #[test]
    fn pusher_decrypt_with_another_master_key_fails() {
        let channel = "private-encrypted-orders.42";
        let payload = encrypt(channel, &master_key(), b"secret").unwrap();
        let mut other = master_key();
        other[0] ^= 0xff;
        assert!(decrypt(channel, &other, &payload).is_err());
        // The channel name is part of the key, too.
        assert!(decrypt("private-encrypted-orders.43", &master_key(), &payload).is_err());
    }

    #[test]
    fn pusher_encrypt_matches_a_tweetnacl_known_answer() {
        // Computed independently with tweetnacl 1.0.3, the secretbox
        // implementation pusher-js decrypts with.
        let mut nonce = [0u8; 24];
        for (i, byte) in nonce.iter_mut().enumerate() {
            *byte = 100 + i as u8;
        }
        assert_eq!(STANDARD.encode(nonce), "ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7");
        let payload = encrypt_with_nonce(
            "private-encrypted-orders.42",
            &master_key(),
            br#"{"id":42}"#,
            nonce,
        )
        .unwrap();
        assert_eq!(
            payload,
            r#"{"nonce":"ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7","ciphertext":"ojLzGvW7n1CLOTLeOjevBcFfbdPawbcJYQ=="}"#
        );
    }

    /// An RNG whose every draw fails, standing in for an OS RNG that
    /// cannot be read.
    struct FailingRng;
    impl rand::TryRng for FailingRng {
        type Error = std::fmt::Error;
        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            Err(std::fmt::Error)
        }
        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            Err(std::fmt::Error)
        }
        fn try_fill_bytes(&mut self, _dst: &mut [u8]) -> Result<(), Self::Error> {
            Err(std::fmt::Error)
        }
    }

    #[test]
    fn pusher_nonce_rng_failure_is_err_not_panic() {
        let err = nonce_from(&mut FailingRng).expect_err("a failed draw is an error");
        assert!(err.to_string().contains("nonce"), "{err}");
    }

    #[test]
    fn pusher_nonce_from_the_os_rng_is_fresh() {
        let first = nonce_from(&mut rand::rngs::SysRng).unwrap();
        let second = nonce_from(&mut rand::rngs::SysRng).unwrap();
        assert_ne!(first, second);
    }
}
