//! Security boundaries for the signed framework document-path snapshot extension.

use suprnova_live::snapshot::MountedDocumentPath;

#[test]
fn mounted_document_path_rejects_browser_normalization_boundaries() {
    for path in [
        "/catalog/../admin",
        "/catalog/./admin",
        "/catalog/%2e%2e/admin",
        "/catalog/%2E%2e/admin",
        "/catalog/.%2E/admin",
        "/catalog/%2e./admin",
        r"/catalog\..\admin",
        "/catalog/%5c../admin",
        "/catalog/%5C../admin",
        "/catalog/%2fadmin",
        "/catalog/%2Fadmin",
        "/catalog/%",
        "/catalog/%2",
        "/catalog/%zz",
    ] {
        assert!(
            MountedDocumentPath::parse(path).is_err(),
            "normalizable path must be rejected: {path}"
        );
    }
}

#[test]
fn mounted_document_path_keeps_valid_parameterized_paths() {
    let path = MountedDocumentPath::parse("/catalog/rust%20books/edition-2")
        .expect("valid normalized parameterized path");

    assert_eq!(path.as_str(), "/catalog/rust%20books/edition-2");
}
