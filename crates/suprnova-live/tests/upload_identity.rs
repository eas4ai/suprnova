//! Upload identity and transfer-grant contract tests.

use suprnova_live::upload::{UploadErrorKind, UploadHandle};

const HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-123456789abc";
const FIXTURE_V7_HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";

#[test]
fn upload_handle_accepts_canonical_non_nil_v4_and_v7_uuid() {
    let handle = UploadHandle::parse(HANDLE).expect("canonical v4 handle");
    let fixture_handle =
        UploadHandle::parse(FIXTURE_V7_HANDLE).expect("locked fixture's canonical v7 handle");

    assert_eq!(handle.to_string(), HANDLE);
    assert_eq!(fixture_handle.to_string(), FIXTURE_V7_HANDLE);
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
