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

/// MEM-003: deciding `.` and `..` without decoding a segment keeps the same
/// answers: a dot segment is exactly one or two dots, encoded or not.
#[test]
fn mounted_document_path_tells_dot_segments_from_dotted_names() {
    for path in ["/%2E", "/.%2e", "/a/%2e", "/."] {
        assert!(
            MountedDocumentPath::parse(path).is_err(),
            "a dot segment must be rejected: {path}"
        );
    }
    for path in ["/%2e%2e%2e", "/.a", "/a.", "/...", "/a..b"] {
        assert!(
            MountedDocumentPath::parse(path).is_ok(),
            "a dotted name is not a dot segment: {path}"
        );
    }
}
