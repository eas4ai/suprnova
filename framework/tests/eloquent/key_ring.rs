#![cfg(feature = "testing")]

//! The one encryption key ring of this binary.
//!
//! `Crypt` holds one ring per process, sealed by whichever test installs
//! a key first. Under plain `cargo test` the tests of this binary are
//! threads of one process, so every test here that needs a key installs
//! this ring, through [`rotation_keys`]: the rotation tests need a
//! current key and two previous ones, and a test that only needs some
//! key is served by the same ring. A test that installed a key of its
//! own would seal the ring before the rotation tests could install
//! theirs, in whatever order the tests happened to start.

use std::sync::OnceLock;

use suprnova::EncryptionKey;

/// Keys used by every test in this binary. Materialised once; the
/// installed ring is `current = B`, `previous = [A, A2]` (A2 for the
/// multi-step-rotation test, harmless for the single-fallback tests).
///
/// `current` (B) - used for any new encrypt issued via `Crypt::encrypt_string`.
/// `previous[0]` (A) - the oldest fallback; tests that simulate "data
///                     was written under A" use this.
/// `previous[1]` (A2) - a second fallback to prove the ring walks the
///                      full list instead of stopping at index 0.
pub struct RotationKeys {
    pub current: EncryptionKey,
    pub previous_oldest: EncryptionKey,
    pub previous_middle: EncryptionKey,
}

/// Install the ring of this binary on the first call, and return its
/// keys on every call.
pub fn rotation_keys() -> &'static RotationKeys {
    static KEYS: OnceLock<RotationKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let keys = RotationKeys {
            current: EncryptionKey::generate(),
            previous_oldest: EncryptionKey::generate(),
            previous_middle: EncryptionKey::generate(),
        };
        let installed = suprnova::testing::install_test_encryption_keyring(
            keys.current.clone(),
            vec![keys.previous_oldest.clone(), keys.previous_middle.clone()],
        );
        assert!(
            installed,
            "a key was installed before this binary's ring; install keys only through \
             key_ring::rotation_keys"
        );
        keys
    })
}
