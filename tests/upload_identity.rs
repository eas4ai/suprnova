//! Upload identity and transfer-grant contract tests.

use suprnova_live::upload::{UploadErrorKind, UploadHandle};

const HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-123456789abc";

#[test]
fn upload_handle_accepts_only_canonical_non_nil_v4_uuid() {
    let handle = UploadHandle::parse(HANDLE).expect("canonical v4 handle");

    assert_eq!(handle.to_string(), HANDLE);
    assert_eq!(
        serde_json::to_string(&handle).expect("serialize handle"),
        format!("\"{HANDLE}\"")
    );
    assert_eq!(
        serde_json::from_str::<UploadHandle>(&format!("\"{HANDLE}\"")).expect("deserialize handle"),
        handle
    );

    for rejected in [
        "00000000-0000-0000-0000-000000000000",
        "018f8f3a-7b2c-1d5e-8f90-123456789abc",
        "018F8F3A-7B2C-4D5E-8F90-123456789ABC",
        "018f8f3a7b2c4d5e8f90123456789abc",
        "not-a-handle",
    ] {
        assert_eq!(
            UploadHandle::parse(rejected)
                .expect_err("noncanonical handle must fail")
                .kind(),
            UploadErrorKind::InvalidHandle,
            "accepted {rejected}"
        );
    }
}

#[test]
fn upload_handle_debug_is_opaque() {
    let handle = UploadHandle::parse(HANDLE).expect("canonical v4 handle");
    let debug = format!("{handle:?}");

    assert_eq!(debug, "<UploadHandle>");
    assert!(!debug.contains(HANDLE));
}
