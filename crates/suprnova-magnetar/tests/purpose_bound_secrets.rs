//! Every encrypted value lives under a distinct purpose; a ciphertext can
//! never decrypt under another purpose, and the deployed Suprnova labels
//! stay stable for unchanged ciphertext migration.

use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};

const PURPOSES: [CryptoPurpose; 6] = [
    CryptoPurpose::CeremonyState,
    CryptoPurpose::TwoFactorSecret,
    CryptoPurpose::TwoFactorRecovery,
    CryptoPurpose::ProviderToken,
    CryptoPurpose::RefreshToken,
    CryptoPurpose::SessionGrant,
];

#[test]
fn every_cross_purpose_decrypt_fails_closed() {
    let encryptor = AeadEncryptor::new([42; 32]);
    for minted_under in PURPOSES {
        let ciphertext = encryptor
            .encrypt(minted_under, b"factor secret material")
            .expect("encryption succeeds");
        for attempted_under in PURPOSES {
            let result = encryptor.decrypt(attempted_under, &ciphertext);
            if attempted_under == minted_under {
                assert_eq!(
                    result.expect("same-purpose decrypt succeeds"),
                    b"factor secret material"
                );
            } else {
                assert!(
                    result.is_err(),
                    "{minted_under:?} ciphertext must not open under {attempted_under:?}"
                );
            }
        }
    }
}

#[test]
fn a_foreign_key_never_opens_any_purpose() {
    let minted = AeadEncryptor::new([1; 32]);
    let other = AeadEncryptor::new([2; 32]);
    for purpose in PURPOSES {
        let ciphertext = minted.encrypt(purpose, b"secret").unwrap();
        assert!(other.decrypt(purpose, &ciphertext).is_err());
    }
}

#[test]
fn deployed_labels_stay_stable_for_unchanged_ciphertext_migration() {
    // The 2FA labels are the deployed Suprnova values: existing rows must
    // decrypt without transformation after the swap.
    assert_eq!(
        CryptoPurpose::TwoFactorSecret.label_str(),
        "suprnova:2fa:secret:v1"
    );
    assert_eq!(
        CryptoPurpose::TwoFactorRecovery.label_str(),
        "suprnova:2fa:recovery:v1"
    );
    // Magnetar-native purposes are versioned under the crate's namespace.
    assert_eq!(
        CryptoPurpose::CeremonyState.label_str(),
        "magnetar/crypto/ceremony-state/v1"
    );
}
