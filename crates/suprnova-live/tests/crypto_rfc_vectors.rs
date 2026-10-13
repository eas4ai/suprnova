//! Pinned RustCrypto verification against the normative HKDF and HMAC vectors.

use hex_literal::hex;
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

#[test]
fn rfc_5869_sha256_case_one() {
    let input_key_material = hex!("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
    let salt = hex!("000102030405060708090a0b0c");
    let info = hex!("f0f1f2f3f4f5f6f7f8f9");
    let expected = hex!(
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), &input_key_material);
    let mut output = [0_u8; 42];

    assert_eq!(input_key_material.len(), 22);

    hkdf.expand(&info, &mut output)
        .expect("the RFC vector output length is valid for HKDF-SHA-256");

    assert_eq!(output, expected);
}

#[test]
fn rfc_4231_hmac_sha256_case_one() {
    let key = [0x0b_u8; 20];
    let expected = hex!("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("HMAC accepts the RFC key");

    assert_eq!(key.len(), 20);

    mac.update(b"Hi There");

    assert_eq!(mac.finalize().into_bytes().as_slice(), expected);
}
