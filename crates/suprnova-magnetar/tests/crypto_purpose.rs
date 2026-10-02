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
fn labels_are_stable_and_distinct() {
    let expected = [
        (
            CryptoPurpose::CeremonyState,
            "magnetar/crypto/ceremony-state/v1",
        ),
        (CryptoPurpose::TwoFactorSecret, "suprnova:2fa:secret:v1"),
        (CryptoPurpose::TwoFactorRecovery, "suprnova:2fa:recovery:v1"),
        (
            CryptoPurpose::ProviderToken,
            "magnetar/crypto/provider-token/v1",
        ),
        (
            CryptoPurpose::RefreshToken,
            "magnetar/crypto/refresh-token/v1",
        ),
        (
            CryptoPurpose::SessionGrant,
            "magnetar/crypto/session-grant/v1",
        ),
    ];
    let labels = expected.map(|(purpose, label)| {
        assert_eq!(purpose.label_str(), label);
        assert_eq!(purpose.label(), label.as_bytes());
        label
    });
    for (index, label) in labels.iter().enumerate() {
        assert!(labels[index + 1..].iter().all(|other| other != label));
    }
}

#[test]
fn matching_purpose_round_trips_and_nonce_is_randomized() {
    let encryptor = AeadEncryptor::new([0x42; 32]);
    let plaintext = b"purpose-bound secret";
    let first = encryptor
        .encrypt(CryptoPurpose::TwoFactorSecret, plaintext)
        .expect("encryption should succeed");
    let second = encryptor
        .encrypt(CryptoPurpose::TwoFactorSecret, plaintext)
        .expect("encryption should succeed");

    assert_ne!(first, second, "each encryption must use a fresh nonce");
    assert_eq!(
        encryptor
            .decrypt(CryptoPurpose::TwoFactorSecret, &first)
            .expect("matching purpose should decrypt"),
        plaintext
    );
}

#[test]
fn every_mismatched_purpose_is_rejected() {
    let encryptor = AeadEncryptor::new([0x99; 32]);
    let ciphertext = encryptor
        .encrypt(CryptoPurpose::TwoFactorSecret, b"secret")
        .expect("encryption should succeed");

    for purpose in PURPOSES {
        if purpose == CryptoPurpose::TwoFactorSecret {
            continue;
        }
        assert!(
            encryptor.decrypt(purpose, &ciphertext).is_err(),
            "purpose {purpose:?} must not decrypt a two-factor secret"
        );
    }
}

#[test]
fn tampering_and_truncated_ciphertexts_are_rejected() {
    let encryptor = AeadEncryptor::new([0x17; 32]);
    let ciphertext = encryptor
        .encrypt(CryptoPurpose::ProviderToken, b"provider token")
        .expect("encryption should succeed");

    for length in 0..ciphertext.len() {
        assert!(
            encryptor
                .decrypt(CryptoPurpose::ProviderToken, &ciphertext[..length])
                .is_err(),
            "truncated ciphertext of length {length} must fail"
        );
    }

    for index in 0..ciphertext.len() {
        let mut tampered = ciphertext.clone();
        tampered[index] ^= 1;
        assert!(
            encryptor
                .decrypt(CryptoPurpose::ProviderToken, &tampered)
                .is_err(),
            "tampering at byte {index} must fail"
        );
    }
}
