#![cfg(all(feature = "filesystem", feature = "testing"))]

//! `Storage::url`: the public URL of a file on a disk that has a public
//! base URL, and an error for a disk that has none.

use suprnova::Storage;

#[test]
fn the_url_is_the_base_of_the_disk_and_the_path() {
    let _storage = Storage::fake();
    Storage::register_memory("public");
    Storage::set_public_url("public", "https://cdn.example.com/files").expect("a base URL");

    assert_eq!(
        Storage::url("public", "avatars/7.png").expect("a public disk"),
        "https://cdn.example.com/files/avatars/7.png"
    );
    assert_eq!(
        Storage::url("public", "/avatars//7.png").expect("a public disk"),
        "https://cdn.example.com/files/avatars/7.png",
        "a slash in front and an empty segment are no segments"
    );
}

#[test]
fn a_path_of_the_own_host_is_a_base_as_well() {
    let _storage = Storage::fake();
    Storage::register_memory("public");
    Storage::set_public_url("public", "/storage/").expect("a base URL");

    assert_eq!(
        Storage::url("public", "report.pdf").expect("a public disk"),
        "/storage/report.pdf"
    );
}

#[test]
fn a_segment_stays_one_segment() {
    let _storage = Storage::fake();
    Storage::register_memory("public");
    Storage::set_public_url("public", "https://cdn.example.com").expect("a base URL");

    // Written as they are, `?` and `#` would end the path, and `%2F`
    // would be read as a slash by whoever decodes the URL twice.
    assert_eq!(
        Storage::url("public", "reports/q3 2026#final?.pdf").expect("a public disk"),
        "https://cdn.example.com/reports/q3%202026%23final%3F.pdf"
    );
    // S3 reads a `+` in a path as a space.
    assert_eq!(
        Storage::url("public", "c++ notes.pdf").expect("a public disk"),
        "https://cdn.example.com/c%2B%2B%20notes.pdf"
    );
    // Some servers read what is behind a `;` as a parameter of the
    // segment, and `..;` as `..`.
    assert_eq!(
        Storage::url("public", "..;/admin/x").expect("a public disk"),
        "https://cdn.example.com/..%3B/admin/x"
    );
    assert_eq!(
        Storage::url("public", "a\\b/c:d@e.txt").expect("a public disk"),
        "https://cdn.example.com/a%5Cb/c%3Ad%40e.txt"
    );
    assert_eq!(
        Storage::url("public", "plain-name_1.0~draft.txt").expect("a public disk"),
        "https://cdn.example.com/plain-name_1.0~draft.txt",
        "what a URL gives no meaning to stays as it is"
    );
    assert_eq!(
        Storage::url("public", "a%2Fb.txt").expect("a public disk"),
        "https://cdn.example.com/a%252Fb.txt"
    );
    assert_eq!(
        Storage::url("public", "fotos/caf\u{e9}.png").expect("a public disk"),
        "https://cdn.example.com/fotos/caf%C3%A9.png"
    );
}

#[test]
fn a_private_disk_hands_out_no_url() {
    let _storage = Storage::fake();
    Storage::register_memory("invoices");

    let error = Storage::url("invoices", "2026/0001.pdf").expect_err("no public base URL");
    let message = error.to_string();
    assert!(message.contains("has no public URL"), "{message}");
    assert!(message.contains("temporary_url"), "{message}");
}

#[test]
fn a_disk_that_is_not_registered_is_named_as_that() {
    let _storage = Storage::fake();

    let error = Storage::url("nowhere", "a.png").expect_err("no such disk");
    assert!(error.to_string().contains("not registered"), "{error}");

    let error =
        Storage::set_public_url("nowhere", "https://cdn.example.com").expect_err("no such disk");
    assert!(error.to_string().contains("not registered"), "{error}");
}

#[test]
fn a_path_that_a_browser_would_resolve_is_refused() {
    let _storage = Storage::fake();
    Storage::register_memory("public");
    Storage::set_public_url("public", "https://cdn.example.com/files").expect("a base URL");

    for path in [
        "../secret.txt",
        "a/../../b.txt",
        "./a.txt",
        "a/./b.txt",
        "",
        "/",
        "//",
    ] {
        assert!(
            Storage::url("public", path).is_err(),
            "`{path}` must be refused"
        );
    }
    assert_eq!(
        Storage::url("public", "my..file.txt").expect("dots inside a name are a name"),
        "https://cdn.example.com/files/my..file.txt"
    );
}

#[test]
fn what_is_no_base_url_is_refused() {
    let _storage = Storage::fake();
    Storage::register_memory("public");

    // Each base with a piece of it that the error must not repeat. The
    // host is none the error uses for its own example.
    for (base, never_shown) in [
        ("", "internal.test"),
        ("files.internal.test", "internal.test"),
        ("ftp://files.internal.test", "internal.test"),
        ("//files.internal.test/files", "internal.test"),
        // A browser reads a backslash as a slash: this is `//evil.test`.
        ("/\\evil.test/files", "evil.test"),
        ("/private\\..", "private"),
        ("https:\\\\files.internal.test", "internal.test"),
        ("https://", "internal.test"),
        ("https:///private", "private"),
        (
            "https://files.internal.test/files?token=a-signed-token",
            "a-signed-token",
        ),
        ("https://files.internal.test/files#top", "internal.test"),
        ("https://files.internal.test/my files", "internal.test"),
        // Every link would show the password.
        (
            "https://user:a-password@files.internal.test/files",
            "a-password",
        ),
        ("https://someone@files.internal.test/files", "someone"),
        ("https://files.internal.test/files/..", "internal.test"),
        (
            "https://files.internal.test/files/../other",
            "internal.test",
        ),
        (
            "https://files.internal.test/files/%2e%2e/other",
            "internal.test",
        ),
        ("https://files.internal.test/files/%2E%2e", "internal.test"),
        ("/private/../etc", "private"),
        ("/private/./x", "private"),
        ("/private\u{0}", "private"),
        ("/private\u{1}x", "private"),
        ("javascript:alert(1)", "alert"),
        ("data:text/html,x", "text/html"),
    ] {
        let error = Storage::set_public_url("public", base)
            .err()
            .unwrap_or_else(|| panic!("`{base}` must be refused"));
        let message = error.to_string();
        assert!(
            !message.contains(never_shown),
            "`{base}`: the error must not repeat the URL: {message}"
        );
    }
    assert!(
        Storage::url("public", "a.png").is_err(),
        "a base that was refused is not the base of the disk"
    );
}

#[test]
fn a_base_url_is_kept_the_way_a_browser_reads_it() {
    let _storage = Storage::fake();
    Storage::register_memory("public");

    for (base, expected) in [
        (
            "HTTPS://CDN.Example.com/Files/",
            "https://cdn.example.com/Files/a.png",
        ),
        (
            "https://cdn.example.com:8443/files",
            "https://cdn.example.com:8443/files/a.png",
        ),
        (
            "http://[2001:db8::1]:9000/bucket",
            "http://[2001:db8::1]:9000/bucket/a.png",
        ),
        ("https://cdn.example.com", "https://cdn.example.com/a.png"),
        (
            "https://cdn.example.com///",
            "https://cdn.example.com/a.png",
        ),
        ("/storage///", "/storage/a.png"),
        ("/", "/a.png"),
    ] {
        Storage::set_public_url("public", base)
            .unwrap_or_else(|e| panic!("`{base}` is a base URL: {e}"));
        assert_eq!(
            Storage::url("public", "a.png").expect("a public disk"),
            expected,
            "{base}"
        );
    }
}

#[test]
fn a_disk_that_is_registered_again_is_a_private_disk_again() {
    let _storage = Storage::fake();
    Storage::register_memory("files");
    Storage::set_public_url("files", "https://cdn.example.com").expect("a base URL");
    assert!(Storage::url("files", "a.png").is_ok());

    // The name now stands for another disk. The URL was the URL of the
    // one before.
    Storage::register_memory("files");
    assert!(Storage::url("files", "a.png").is_err());

    Storage::set_public_url("files", "https://cdn.example.com").expect("a base URL");
    assert!(Storage::forget("files"));
    Storage::register_memory("files");
    assert!(Storage::url("files", "a.png").is_err());
}
