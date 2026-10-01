//! File responses: `HttpResponse::file`, `HttpResponse::download`,
//! `HttpResponse::download_bytes`, and the RFC 6266 `Content-Disposition`
//! builder they share (`ContentDisposition::header_value`).
//!
//! Every response case is driven through the real `Router` and
//! `MiddlewareRegistry` via `handle_request`, so what is asserted is what
//! reaches the wire: a header hyper would refuse (a raw CR or LF, say) is
//! dropped by `into_hyper`, and the tests would see it missing.

use crate::common::incoming_get_request;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::HeaderMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use suprnova::{
    ContentDisposition, FrameworkError, HttpResponse, MiddlewareRegistry, Router, handle_request,
};

const WAIT: Duration = Duration::from_secs(10);

/// The name the falsifier of PAR-002 names: a middle dot and an accented
/// letter, both outside ASCII.
const CATALAN_NAME: &str = "Certificat\u{b7}Joan P\u{e9}rez.pdf";

/// Answer one `GET path` through `handle_request` on a router whose only
/// route is `path`, and collect the whole body.
async fn serve<F, Fut>(path: &'static str, handler: F) -> (u16, HeaderMap, Bytes)
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = suprnova::Response> + Send + 'static,
{
    let handler = Arc::new(handler);
    let router: Router = Router::new()
        .get(path, move |_req| {
            let handler = handler.clone();
            async move { handler().await }
        })
        .into();
    let req = incoming_get_request(path, &[]).await;
    let resp = tokio::time::timeout(
        WAIT,
        handle_request(Arc::new(router), Arc::new(MiddlewareRegistry::new()), req),
    )
    .await
    .expect("handle_request timed out");
    let status = resp.status().as_u16();
    let (parts, body) = resp.into_parts();
    let body = tokio::time::timeout(WAIT, body.collect())
        .await
        .expect("collecting the body timed out")
        .expect("body collects")
        .to_bytes();
    (status, parts.headers, body)
}

/// `expect_err` for a file response: `HttpResponse` has no `Debug`, so the
/// standard one is not available.
trait ExpectError {
    fn expect_error(self, why: &str) -> FrameworkError;
}

impl ExpectError for Result<HttpResponse, FrameworkError> {
    fn expect_error(self, why: &str) -> FrameworkError {
        match self {
            Ok(response) => panic!("{why}; got status {}", response.status_code()),
            Err(error) => error,
        }
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("response must carry `{name}`; it had {headers:?}"))
        .to_str()
        .expect("header is visible ASCII")
}

fn write_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write fixture file");
    path
}

/// A `Content-Disposition` value split the way an RFC 6266 parser splits
/// it: the disposition type, then each `name=value` parameter, with a
/// quoted-string value unescaped. Panics on anything that does not parse,
/// so a name that broke the header's structure fails the test here.
fn parse_disposition(value: &str) -> (String, Vec<(String, String)>) {
    let bytes = value.as_bytes();
    let token_end = |from: usize| {
        let mut j = from;
        while j < bytes.len() && !matches!(bytes[j], b';' | b'=' | b' ' | b'"') {
            j += 1;
        }
        j
    };
    let mut i = token_end(0);
    let kind = value[..i].to_string();
    let mut params = Vec::new();
    while i < bytes.len() {
        assert_eq!(bytes[i], b';', "expected `;` at {i} in {value:?}");
        i += 1;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let name_end = token_end(i);
        let name = value[i..name_end].to_string();
        assert!(!name.is_empty(), "empty parameter name in {value:?}");
        i = name_end;
        assert_eq!(
            bytes.get(i),
            Some(&b'='),
            "expected `=` after {name} in {value:?}"
        );
        i += 1;
        let mut out = String::new();
        if bytes.get(i) == Some(&b'"') {
            i += 1;
            loop {
                match bytes.get(i) {
                    Some(b'\\') => {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                    }
                    Some(b'"') => {
                        i += 1;
                        break;
                    }
                    Some(&b) => {
                        out.push(b as char);
                        i += 1;
                    }
                    None => panic!("unterminated quoted-string in {value:?}"),
                }
            }
        } else {
            let end = token_end(i);
            out.push_str(&value[i..end]);
            i = end;
        }
        params.push((name, out));
    }
    (kind, params)
}

/// Decode an RFC 8187 `UTF-8''...` extended value.
fn decode_ext_value(value: &str) -> String {
    let encoded = value
        .strip_prefix("UTF-8''")
        .unwrap_or_else(|| panic!("filename* must start with UTF-8'': {value:?}"));
    percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .expect("filename* decodes to UTF-8")
        .into_owned()
}

/// Assert a header value is one structurally sound disposition: visible
/// ASCII only, the expected type, an ASCII `filename` fallback equal to
/// `fallback`, and a `filename*` that decodes to `extended` (or none).
fn assert_disposition(value: &str, kind: &str, fallback: &str, extended: Option<&str>) {
    assert!(
        value.bytes().all(|b| (0x20..=0x7e).contains(&b)),
        "every byte of the header must be visible ASCII: {value:?}"
    );
    let (parsed_kind, params) = parse_disposition(value);
    assert_eq!(parsed_kind, kind, "disposition type of {value:?}");
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    match extended {
        Some(_) => assert_eq!(names, ["filename", "filename*"], "params of {value:?}"),
        None => assert_eq!(names, ["filename"], "params of {value:?}"),
    }
    assert_eq!(params[0].1, fallback, "ASCII fallback of {value:?}");
    if let Some(extended) = extended {
        assert_eq!(
            decode_ext_value(&params[1].1),
            extended,
            "filename* of {value:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The Content-Disposition builder
// ---------------------------------------------------------------------------

#[test]
fn a_non_ascii_name_gets_an_ascii_fallback_and_a_percent_encoded_filename_star() {
    let value = ContentDisposition::Attachment.header_value(CATALAN_NAME);
    assert_eq!(
        value,
        "attachment; filename=\"Certificat_Joan P_rez.pdf\"; \
         filename*=UTF-8''Certificat%C2%B7Joan%20P%C3%A9rez.pdf"
    );
    assert_disposition(
        &value,
        "attachment",
        "Certificat_Joan P_rez.pdf",
        Some(CATALAN_NAME),
    );
}

#[test]
fn a_plain_ascii_name_has_only_the_filename_parameter() {
    let value = ContentDisposition::Attachment.header_value("report-2026.pdf");
    assert_eq!(value, "attachment; filename=\"report-2026.pdf\"");
    let value = ContentDisposition::Inline.header_value("logo.png");
    assert_eq!(value, "inline; filename=\"logo.png\"");
}

#[test]
fn quotes_and_backslashes_are_escaped_and_cannot_close_the_quoted_string() {
    let value = ContentDisposition::Attachment.header_value("say \"hi\"; x=1.pdf");
    assert_eq!(
        value,
        "attachment; filename=\"say \\\"hi\\\"; x=1.pdf\"; \
         filename*=UTF-8''say%20%22hi%22%3B%20x%3D1.pdf"
    );
    assert_disposition(
        &value,
        "attachment",
        "say \"hi\"; x=1.pdf",
        Some("say \"hi\"; x=1.pdf"),
    );

    let value = ContentDisposition::Attachment.header_value("C:\\temp\\x.pdf\\");
    assert_eq!(
        value,
        "attachment; filename=\"C:\\\\temp\\\\x.pdf\\\\\"; \
         filename*=UTF-8''C%3A%5Ctemp%5Cx.pdf%5C"
    );
    assert_disposition(
        &value,
        "attachment",
        "C:\\temp\\x.pdf\\",
        Some("C:\\temp\\x.pdf\\"),
    );
}

#[test]
fn control_characters_never_reach_the_header() {
    let value =
        ContentDisposition::Attachment.header_value("report\r\nSet-Cookie: pwned=1\0\t\u{7f}.pdf");
    assert!(
        !value.contains(['\r', '\n', '\0', '\t', '\u{7f}']),
        "no control character may reach the header: {value:?}"
    );
    assert_disposition(
        &value,
        "attachment",
        "report__Set-Cookie: pwned=1___.pdf",
        None,
    );

    // A C1 control is a control character too, and an encoded control
    // character must not appear in `filename*` either.
    let value = ContentDisposition::Inline.header_value("caf\u{e9}\u{85}\r.txt");
    assert!(
        !value.contains("%0D") && !value.contains("%C2%85"),
        "{value:?}"
    );
    assert_disposition(&value, "inline", "caf___.txt", Some("caf\u{e9}__.txt"));
}

#[test]
fn a_percent_sign_is_kept_out_of_the_fallback() {
    let value = ContentDisposition::Attachment.header_value("100%25 done.csv");
    assert_disposition(
        &value,
        "attachment",
        "100_25 done.csv",
        Some("100%25 done.csv"),
    );
}

#[test]
fn an_empty_name_writes_the_disposition_type_alone() {
    assert_eq!(
        ContentDisposition::Attachment.header_value(""),
        "attachment"
    );
    assert_eq!(ContentDisposition::Inline.header_value(""), "inline");
}

// ---------------------------------------------------------------------------
// HttpResponse::file
// ---------------------------------------------------------------------------

#[tokio::test]
async fn file_serves_a_pdf_inline_with_its_own_name_and_content_type() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "invoice.pdf", b"%PDF-1.7 invoice body");

    let (status, headers, body) = serve("/invoice", move || {
        let path = path.clone();
        async move {
            HttpResponse::file(&path, None)
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert_eq!(
        header(&headers, "content-disposition"),
        "inline; filename=\"invoice.pdf\""
    );
    assert_eq!(header(&headers, "content-length"), "21");
    assert_eq!(&body[..], b"%PDF-1.7 invoice body");
}

#[tokio::test]
async fn file_takes_an_optional_inline_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "a1b2c3.png", b"\x89PNG\r\n\x1a\nfake");

    let (status, headers, body) = serve("/logo", move || {
        let path = path.clone();
        async move {
            HttpResponse::file(&path, Some("Logo P\u{e9}rez.png"))
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "image/png");
    assert_disposition(
        header(&headers, "content-disposition"),
        "inline",
        "Logo P_rez.png",
        Some("Logo P\u{e9}rez.png"),
    );
    assert_eq!(&body[..], b"\x89PNG\r\n\x1a\nfake");
}

#[tokio::test]
async fn the_content_type_comes_from_the_extension_and_is_never_sniffed() {
    let dir = tempfile::tempdir().unwrap();
    // Markup in a file whose extension says nothing: sniffing it would
    // serve `image/svg+xml` inline, which runs the script.
    let unknown = write_file(dir.path(), "upload.xyz", b"<svg onload=\"alert(1)\"></svg>");
    let bare = write_file(dir.path(), "README", b"<svg></svg>");
    let upper = write_file(dir.path(), "SCAN.PDF", b"%PDF-1.4");
    let text = write_file(dir.path(), "notes.txt", b"hello");

    for (path, expected) in [
        (unknown, "application/octet-stream"),
        (bare, "application/octet-stream"),
        (upper, "application/pdf"),
        (text, "text/plain; charset=utf-8"),
    ] {
        let response = HttpResponse::file(&path, None)
            .await
            .expect("an existing file answers");
        assert_eq!(
            response.header_value("Content-Type"),
            Some(expected),
            "content type of {}",
            path.display()
        );
        assert_eq!(
            response
                .header_value("Content-Disposition")
                .map(|v| v.starts_with("inline")),
            Some(true)
        );
    }
}

#[tokio::test]
async fn a_large_file_is_streamed_and_arrives_whole() {
    let dir = tempfile::tempdir().unwrap();
    let len = 5 * 1024 * 1024 + 123;
    let contents: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let path = write_file(dir.path(), "archive.zip", &contents);

    let response = HttpResponse::file(&path, None).await.unwrap();
    assert!(
        response.is_streaming(),
        "a file above the buffering limit must stream, not be read into memory"
    );

    let (status, headers, body) = serve("/archive", move || {
        let path = path.clone();
        async move {
            HttpResponse::download(&path, None)
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/zip");
    assert_eq!(header(&headers, "content-length"), len.to_string());
    assert_eq!(body.len(), len);
    assert!(
        body[..] == contents[..],
        "the streamed body must match the file"
    );
}

#[tokio::test]
async fn a_missing_file_is_a_404_and_does_not_name_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("gone.pdf");

    let err = HttpResponse::file(&missing, None)
        .await
        .expect_error("a missing file must be an error, not a panic");
    assert_eq!(err.status_code(), 404);
    let err = HttpResponse::download(&missing, Some("x.pdf"))
        .await
        .expect_error("a missing file must be an error");
    assert_eq!(err.status_code(), 404);

    let dir_path = dir.path().to_string_lossy().into_owned();
    let (status, _headers, body) = serve("/gone", move || {
        let missing = missing.clone();
        async move {
            HttpResponse::file(&missing, None)
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;
    assert_eq!(status, 404);
    let body = String::from_utf8_lossy(&body);
    assert!(
        !body.contains(&dir_path),
        "the 404 body must not reveal the server path: {body}"
    );
}

#[tokio::test]
async fn a_directory_is_not_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let err = HttpResponse::file(dir.path(), None)
        .await
        .expect_error("a directory is not a file response");
    assert_eq!(err.status_code(), 404);
}

// ---------------------------------------------------------------------------
// HttpResponse::download
// ---------------------------------------------------------------------------

#[tokio::test]
async fn download_defaults_to_the_file_own_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "invoice.pdf", b"%PDF-1.7");

    let (status, headers, body) = serve("/dl", move || {
        let path = path.clone();
        async move {
            HttpResponse::download(&path, None)
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert_eq!(
        header(&headers, "content-disposition"),
        "attachment; filename=\"invoice.pdf\""
    );
    assert_eq!(&body[..], b"%PDF-1.7");
}

#[tokio::test]
async fn download_writes_the_caller_name_per_rfc_6266() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "7f3e.pdf", b"%PDF-1.7 certificate");

    let (status, headers, body) = serve("/certificate", move || {
        let path = path.clone();
        async move {
            HttpResponse::download(&path, Some(CATALAN_NAME))
                .await
                .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "application/pdf");
    assert_eq!(
        header(&headers, "content-disposition"),
        "attachment; filename=\"Certificat_Joan P_rez.pdf\"; \
         filename*=UTF-8''Certificat%C2%B7Joan%20P%C3%A9rez.pdf"
    );
    assert_eq!(&body[..], b"%PDF-1.7 certificate");
}

#[tokio::test]
async fn a_hostile_download_name_cannot_add_a_header_or_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "report.pdf", b"%PDF-1.7");

    let (status, headers, _body) = serve("/hostile", move || {
        let path = path.clone();
        async move {
            HttpResponse::download(
                &path,
                Some("a\"; filename=evil.exe\r\nSet-Cookie: pwned=1\\.pdf"),
            )
            .await
            .map_err(HttpResponse::from)
        }
    })
    .await;

    assert_eq!(status, 200);
    assert!(
        headers.get("set-cookie").is_none(),
        "a name must never add a header"
    );
    assert_eq!(
        headers.get_all("content-disposition").iter().count(),
        1,
        "exactly one Content-Disposition reaches the wire"
    );
    assert_disposition(
        header(&headers, "content-disposition"),
        "attachment",
        "a\"; filename=evil.exe__Set-Cookie: pwned=1\\.pdf",
        Some("a\"; filename=evil.exe__Set-Cookie: pwned=1\\.pdf"),
    );
}

// ---------------------------------------------------------------------------
// HttpResponse::download_bytes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn download_bytes_sends_generated_content_with_its_type_and_name() {
    let (status, headers, body) = serve("/export", || async {
        Ok(HttpResponse::download_bytes(
            Bytes::from_static(b"id,name\n1,Joan\n"),
            "users-2026-09-30.csv",
            "text/csv; charset=utf-8",
        ))
    })
    .await;

    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), "text/csv; charset=utf-8");
    assert_eq!(
        header(&headers, "content-disposition"),
        "attachment; filename=\"users-2026-09-30.csv\""
    );
    assert_eq!(&body[..], b"id,name\n1,Joan\n");
}

#[tokio::test]
async fn download_bytes_encodes_a_non_ascii_name() {
    let (status, headers, _body) = serve("/export-ca", || async {
        Ok(HttpResponse::download_bytes(
            b"%PDF-1.7".to_vec(),
            CATALAN_NAME,
            "application/pdf",
        ))
    })
    .await;

    assert_eq!(status, 200);
    assert_disposition(
        header(&headers, "content-disposition"),
        "attachment",
        "Certificat_Joan P_rez.pdf",
        Some(CATALAN_NAME),
    );
}
