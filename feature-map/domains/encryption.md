# Feature map: `manual/encryption.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 34 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova key:generate` · suprnova-cli/src/main.rs:349
  - Generate a new APP_KEY (32-byte AES-256, base64 URL-safe, no padding)

## Rust API: suprnova

### Re-exported from other crates

- [ ] type alias `suprnova::SecretString` re-exports `secrecy::SecretString` (feature: `magnetar-oauth`)

### `suprnova::crypto::key`

- [ ] struct `suprnova::EncryptionKey` · framework/src/crypto/key.rs:18 (also `suprnova::crypto::EncryptionKey`, `suprnova::crypto::key::EncryptionKey`)
  - [ ] fn `suprnova::EncryptionKey::from_env` · framework/src/crypto/key.rs:26
  - [ ] fn `suprnova::EncryptionKey::generate` · framework/src/crypto/key.rs:36
  - [ ] fn `suprnova::EncryptionKey::from_base64` · framework/src/crypto/key.rs:44
  - [ ] fn `suprnova::EncryptionKey::to_base64` · framework/src/crypto/key.rs:60
  - [ ] fn `suprnova::EncryptionKey::as_bytes` · framework/src/crypto/key.rs:65

### `suprnova::crypto`

- [ ] fn `suprnova::crypto::resolve_boot_key` · framework/src/crypto/mod.rs:716
- [ ] fn `suprnova::crypto::resolve_boot_keyring` · framework/src/crypto/mod.rs:776
- [ ] struct `suprnova::crypto::BootKeyRing` · framework/src/crypto/mod.rs:945
  - Public fields: `current`, `previous`
  - [ ] fn `suprnova::crypto::BootKeyRing::is_current_generated` · framework/src/crypto/mod.rs:957
  - [ ] fn `suprnova::crypto::BootKeyRing::into_keys` · framework/src/crypto/mod.rs:963
- [ ] struct `suprnova::Crypt` · framework/src/crypto/mod.rs:297 (also `suprnova::crypto::Crypt`)
  - [ ] fn `suprnova::Crypt::init` · framework/src/crypto/mod.rs:305
  - [ ] fn `suprnova::Crypt::init_with_keyring` · framework/src/crypto/mod.rs:316
  - [ ] fn `suprnova::Crypt::is_initialized` · framework/src/crypto/mod.rs:323
  - [ ] fn `suprnova::Crypt::encrypt_string` · framework/src/crypto/mod.rs:345
  - [ ] fn `suprnova::Crypt::encrypt_string_for` · framework/src/crypto/mod.rs:357
  - [ ] fn `suprnova::Crypt::decrypt_string` · framework/src/crypto/mod.rs:396
  - [ ] fn `suprnova::Crypt::decrypt_string_for` · framework/src/crypto/mod.rs:413
  - [ ] fn `suprnova::Crypt::encrypt` · framework/src/crypto/mod.rs:443
  - [ ] fn `suprnova::Crypt::decrypt` · framework/src/crypto/mod.rs:459
  - [ ] fn `suprnova::Crypt::appears_encrypted` · framework/src/crypto/mod.rs:484
  - [ ] fn `suprnova::Crypt::previous_key_count` · framework/src/crypto/mod.rs:501
  - [ ] fn `suprnova::Crypt::has_previous_keys` · framework/src/crypto/mod.rs:508
- [ ] struct `suprnova::crypto::DecryptOrigin` · framework/src/crypto/mod.rs:265
  - Public fields: `key`, `aad`
- [ ] enum `suprnova::crypto::AadVersion` · framework/src/crypto/mod.rs:239
  - Variants: `Current`, `Legacy`
- [ ] enum `suprnova::crypto::BootKey` · framework/src/crypto/mod.rs:915
  - Variants: `Configured`, `GeneratedTransient`
  - [ ] fn `suprnova::crypto::BootKey::into_key` · framework/src/crypto/mod.rs:926
  - [ ] fn `suprnova::crypto::BootKey::is_generated` · framework/src/crypto/mod.rs:935
- [ ] enum `suprnova::CryptPurpose` · framework/src/crypto/mod.rs:120 (also `suprnova::crypto::CryptPurpose`)
  - Variants: `Cookie`, `Cursor`, `TwoFactorSecret`, `TwoFactorRecovery`, `MagnetarCeremonyState`, `MagnetarProviderToken`, `MagnetarRefreshToken`, `MagnetarSessionGrant`, `Cast`
- [ ] enum `suprnova::crypto::KeyOrigin` · framework/src/crypto/mod.rs:228
  - Variants: `Current`, `Previous`
- [ ] const `suprnova::crypto::MAX_PREVIOUS_KEYS` · framework/src/crypto/mod.rs:860
