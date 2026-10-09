//! Laravel's validation messages and image rules that Suprnova lacked or
//! worded differently: the `ip` message, the numeric `max` message,
//! `Rule::requiredIf`, the `image` rule's type list and the `dimensions`
//! rule.
//!
//! The upload validators are driven directly with the sniff buffer the
//! multipart extractor hands them, as `framework/src/http/upload/validators.rs`
//! does in its own tests; each image fixture is a header of the format,
//! checked against `infer` first so a test proves the type it names.

use std::cell::Cell;
use std::sync::Arc;

use suprnova::http::upload::validators::UploadValidator;
use suprnova::rules::{Required, RequiredIf};
use suprnova::testing::TestContainer;
use suprnova::{
    DimensionLimits, Dimensions, FrameworkError, ImageFile, MimeAllowlist, MimeType, Rule,
    ValidationErrors, ValidationMessage, validate,
};
use validator::Validate;

// ── the English catalog ─────────────────────────────────────────────

/// Bind a translator over the framework's English catalog for the current
/// `TestContainer::scope`: an application `lang/en/validation.ftl` that
/// overrides nothing.
#[cfg(feature = "localization")]
fn english_catalog() -> tempfile::TempDir {
    use suprnova::{FluentTranslator, Locale, LocalizationConfig, Translator};

    let dir = tempfile::tempdir().expect("a lang directory");
    std::fs::create_dir_all(dir.path().join("en")).expect("lang/en");
    std::fs::write(dir.path().join("en").join("validation.ftl"), "")
        .expect("lang/en/validation.ftl");
    let config = LocalizationConfig {
        default_locale: Locale::parse("en").expect("en"),
        fallback_locale: Locale::parse("en").expect("en"),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
        parents: Default::default(),
    };
    let translator = FluentTranslator::from_dir(dir.path(), &config).expect("the catalog loads");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));
    dir
}

/// `field`'s messages rendered from the English catalog.
#[cfg(feature = "localization")]
async fn english(errors: ValidationErrors, field: &str) -> Vec<String> {
    let field = field.to_string();
    TestContainer::scope(async move {
        let _catalog = english_catalog();
        errors.messages_for(&field)
    })
    .await
}

/// One message for `field`, as a refused upload reports it.
fn bag(field: &str, message: ValidationMessage) -> ValidationErrors {
    let mut errors = ValidationErrors::new();
    errors.add(field, message);
    errors
}

// ── `ip` and the numeric `max` message ──────────────────────────────

#[derive(Validate)]
struct Server {
    #[validate(ip)]
    address: String,
    #[validate(ip(v4))]
    gateway: String,
    #[validate(range(max = 10))]
    replicas: i64,
    #[validate(range(min = 1))]
    workers: i64,
    #[validate(length(max = 3))]
    region: String,
}

fn server() -> Server {
    Server {
        address: "10.0.0.1".into(),
        gateway: "10.0.0.254".into(),
        replicas: 10,
        workers: 1,
        region: "eu".into(),
    }
}

fn derive_errors(form: &Server) -> ValidationErrors {
    match form.validate() {
        Ok(()) => ValidationErrors::new(),
        Err(errors) => ValidationErrors::from_validator(errors),
    }
}

#[test]
fn an_ip_field_reports_the_ip_key_for_every_ip_form() {
    let form = Server {
        address: "999.1.1.1".into(),
        gateway: "::1".into(),
        ..server()
    };
    let errors = derive_errors(&form);
    assert_eq!(errors.errors["address"][0].key, "validation-ip");
    assert_eq!(
        errors.errors["gateway"][0].key, "validation-ip",
        "`ip(v4)` reports the same code"
    );

    assert!(
        derive_errors(&server()).is_empty(),
        "valid addresses and bounds pass"
    );
    let v6 = Server {
        address: "2001:db8::1".into(),
        ..server()
    };
    assert!(derive_errors(&v6).is_empty(), "an IPv6 address is an IP");
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn an_invalid_ip_reads_laravels_ip_message() {
    let form = Server {
        address: "999.1.1.1".into(),
        ..server()
    };
    assert_eq!(
        english(derive_errors(&form), "address").await,
        ["The address field must be a valid IP address."]
    );
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn a_number_over_its_max_reads_not_greater_than() {
    let form = Server {
        replicas: 11,
        ..server()
    };
    let messages = english(derive_errors(&form), "replicas").await;
    assert_eq!(
        messages,
        ["The replicas field must not be greater than 10."]
    );
    assert!(!messages[0].contains("must be at most"), "{messages:?}");
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn the_other_bound_messages_keep_their_wording() {
    let form = Server {
        workers: 0,
        region: "europe".into(),
        ..server()
    };
    let errors = derive_errors(&form);
    assert_eq!(
        english(errors.clone(), "workers").await,
        ["The workers field must be at least 1."]
    );
    assert_eq!(
        english(errors, "region").await,
        ["The region field must be at most 3 characters."],
        "a string's max keeps its own message"
    );
}

// ── `RequiredIf::when` ──────────────────────────────────────────────

#[test]
fn required_if_when_true_requires_the_field() {
    let rule = RequiredIf::when(true);
    for empty in ["", "   "] {
        let refused = rule.passes(empty).expect_err("an empty value is refused");
        assert_eq!(refused.key, "validation-required", "{empty:?}");
    }
    assert!(rule.passes("4111 1111 1111 1111").is_ok());
}

#[test]
fn required_if_when_false_passes_every_value() {
    assert!(RequiredIf::when(false).passes("").is_ok());
    assert!(RequiredIf::when(false).passes("anything").is_ok());
    assert!(RequiredIf::when(|| false).passes("").is_ok());
}

#[test]
fn required_if_when_reads_its_closure_on_every_check() {
    let admin = Cell::new(false);
    let rule = RequiredIf::when(|| admin.get());
    assert!(
        rule.passes("").is_ok(),
        "not required while the closure answers false"
    );
    admin.set(true);
    assert_eq!(
        rule.passes("")
            .expect_err("required once it answers true")
            .key,
        "validation-required"
    );
    assert!(rule.passes("ops").is_ok());
}

#[test]
fn required_if_when_refuses_with_requireds_own_message() {
    let when = RequiredIf::when(|| true).passes("").expect_err("required");
    let required = Required.passes("").expect_err("required");
    assert_eq!(when.key, required.key);
    assert_eq!(when.args, required.args);
}

struct Checkout {
    billing_type: String,
    card_number: Option<String>,
    note: String,
}

impl Checkout {
    fn check(&self, card_required: bool) -> Result<(), ValidationErrors> {
        validate! { self =>
            card_number ?=> RequiredIf::when(card_required);
            note => RequiredIf::when(|| self.billing_type == "invoice");
        }
    }
}

#[test]
fn required_if_when_runs_in_validate_rows() {
    let form = Checkout {
        billing_type: "invoice".into(),
        card_number: None,
        note: String::new(),
    };
    let errors = form.check(true).expect_err("both fields are required");
    let mut fields: Vec<&str> = errors.errors.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(fields, ["card_number", "note"]);

    let card = Checkout {
        billing_type: "card".into(),
        card_number: None,
        note: String::new(),
    };
    assert!(
        card.check(false).is_ok(),
        "neither condition holds, so nothing is required: {:?}",
        card.check(false).err().map(|errors| errors.errors)
    );
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn required_if_when_reads_the_required_message() {
    let refused = RequiredIf::when(true).passes("").expect_err("required");
    assert_eq!(
        english(bag("card_number", refused), "card_number").await,
        ["The card number field is required."]
    );
}

// ── image fixtures ──────────────────────────────────────────────────

/// A PNG signature and IHDR chunk: the header `infer` and a dimension
/// read both stop at.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    // Bit depth, colour type, compression, filter, interlace, then the CRC.
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

/// A baseline JPEG: start of image, a JFIF APP0 segment, then the start
/// of frame that holds the height and the width.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    bytes.extend(b"JFIF\0");
    bytes.extend([1, 1, 0, 0, 1, 0, 1, 0, 0]);
    bytes.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]);
    bytes.extend(height.to_be_bytes());
    bytes.extend(width.to_be_bytes());
    bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    bytes
}

/// A JPEG whose first segment, an APP1 of 64 KiB as a camera's metadata
/// and thumbnail make it, runs past the 16 KiB sniff buffer, so the start
/// of frame is never in view.
fn jpeg_with_large_metadata() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF];
    bytes.extend(b"Exif\0\0");
    bytes.resize(16 * 1024, 0);
    bytes
}

fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes.extend([0xF7, 0, 0]);
    bytes
}

/// A BMP with a 40-byte info header. A negative `height` is a top-down
/// bitmap.
fn bmp(width: i32, height: i32) -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend(54u32.to_le_bytes());
    bytes.extend([0, 0, 0, 0]);
    bytes.extend(54u32.to_le_bytes());
    bytes.extend(40u32.to_le_bytes());
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(24u16.to_le_bytes());
    bytes.resize(54, 0);
    bytes
}

/// A BMP with the 12-byte OS/2 header, whose sides are 16-bit.
fn bmp_os2(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend(26u32.to_le_bytes());
    bytes.extend([0, 0, 0, 0]);
    bytes.extend(26u32.to_le_bytes());
    bytes.extend(12u32.to_le_bytes());
    bytes.extend(width.to_le_bytes());
    bytes.extend(height.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(24u16.to_le_bytes());
    bytes
}

/// A RIFF container holding one WebP chunk.
fn webp_chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((4 + 8 + payload.len() as u32).to_le_bytes());
    bytes.extend(b"WEBP");
    bytes.extend(fourcc);
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    bytes
}

/// An extended WebP: the canvas size in the VP8X chunk.
fn webp(width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 4];
    payload.extend(&(width - 1).to_le_bytes()[..3]);
    payload.extend(&(height - 1).to_le_bytes()[..3]);
    webp_chunk(b"VP8X", &payload)
}

/// A lossless WebP: the size packed into the VP8L header.
fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![0x2F];
    payload.extend(((width - 1) | ((height - 1) << 14)).to_le_bytes());
    payload.push(0);
    webp_chunk(b"VP8L", &payload)
}

/// A lossy WebP: the size after the VP8 frame's start code.
fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
    let mut payload = vec![0x10, 0x02, 0x00, 0x9D, 0x01, 0x2A];
    payload.extend(width.to_le_bytes());
    payload.extend(height.to_le_bytes());
    webp_chunk(b"VP8 ", &payload)
}

/// An ISO base media file of brand `brand` whose `meta` box states one
/// image size in an `ispe` property, as AVIF and HEIC files do.
fn isobmff(brand: &[u8; 4], width: u32, height: u32) -> Vec<u8> {
    let mut ispe = 20u32.to_be_bytes().to_vec();
    ispe.extend(b"ispe");
    ispe.extend([0, 0, 0, 0]);
    ispe.extend(width.to_be_bytes());
    ispe.extend(height.to_be_bytes());
    let boxed = |kind: &[u8; 4], body: &[u8], full: bool| {
        let header = if full { 12 } else { 8 };
        let mut out = ((header + body.len()) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        if full {
            out.extend([0, 0, 0, 0]);
        }
        out.extend(body);
        out
    };
    let ipco = boxed(b"ipco", &ispe, false);
    let iprp = boxed(b"iprp", &ipco, false);
    let meta = boxed(b"meta", &iprp, true);

    let mut bytes = 24u32.to_be_bytes().to_vec();
    bytes.extend(b"ftyp");
    bytes.extend(brand);
    bytes.extend([0, 0, 0, 0]);
    bytes.extend(b"mif1");
    bytes.extend(brand);
    bytes.extend(meta);
    bytes
}

fn avif(width: u32, height: u32) -> Vec<u8> {
    isobmff(b"avif", width, height)
}

fn heic(width: u32, height: u32) -> Vec<u8> {
    isobmff(b"heic", width, height)
}

fn tiff() -> Vec<u8> {
    let mut bytes = b"II*\0".to_vec();
    bytes.extend(8u32.to_le_bytes());
    bytes.extend([1, 0, 0, 1, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    bytes
}

fn psd() -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend([
        0, 1, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 8, 0, 3,
    ]);
    bytes
}

fn ico() -> Vec<u8> {
    let mut bytes = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
    bytes.extend(1128u32.to_le_bytes());
    bytes.extend(22u32.to_le_bytes());
    bytes
}

fn jpeg_xl() -> Vec<u8> {
    vec![0xFF, 0x0A, 0xFA, 0x7F, 0x01, 0x90, 0x08, 0x00]
}

const SVG: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect/></svg>";

/// Each fixture is the type it is named for, by the magic bytes the
/// validators read, so a test that refuses or accepts it proves the rule
/// for that type.
#[test]
fn the_fixtures_are_the_types_they_name() {
    let mime = |bytes: &[u8]| infer::get(bytes).map(|kind| kind.mime_type());
    for (name, bytes, expected) in [
        ("png", png(1, 1), "image/png"),
        ("jpeg", jpeg(1, 1), "image/jpeg"),
        ("jpeg metadata", jpeg_with_large_metadata(), "image/jpeg"),
        ("gif", gif(1, 1), "image/gif"),
        ("bmp", bmp(1, 1), "image/bmp"),
        ("bmp os/2", bmp_os2(1, 1), "image/bmp"),
        ("webp", webp(1, 1), "image/webp"),
        ("webp lossless", webp_lossless(1, 1), "image/webp"),
        ("webp lossy", webp_lossy(1, 1), "image/webp"),
        ("avif", avif(1, 1), "image/avif"),
        ("heic", heic(1, 1), "image/heif"),
        ("tiff", tiff(), "image/tiff"),
        ("psd", psd(), "image/vnd.adobe.photoshop"),
        ("ico", ico(), "image/vnd.microsoft.icon"),
        ("jpeg xl", jpeg_xl(), "image/jxl"),
    ] {
        assert_eq!(mime(&bytes), Some(expected), "{name}");
    }
}

fn final_check<V: UploadValidator>(bytes: &[u8]) -> Result<(), FrameworkError> {
    V::default().validate_final(bytes, bytes.len() as u64, None)
}

fn refused(result: Result<(), FrameworkError>) -> ValidationMessage {
    match result {
        Err(FrameworkError::InvalidUpload(message)) => *message,
        other => panic!("expected a refused file, got {other:?}"),
    }
}

// ── `ImageFile` ─────────────────────────────────────────────────────

#[test]
fn image_file_accepts_the_types_laravels_image_rule_accepts() {
    for (name, bytes) in [
        ("jpeg", jpeg(64, 64)),
        ("png", png(64, 64)),
        ("gif", gif(64, 64)),
        ("bmp", bmp(64, 64)),
        ("webp", webp(64, 64)),
        ("avif", avif(64, 64)),
        ("heic", heic(64, 64)),
    ] {
        assert!(final_check::<ImageFile>(&bytes).is_ok(), "{name}");
    }
}

#[test]
fn image_file_refuses_tiff_psd_ico_and_jpeg_xl() {
    for (name, bytes) in [
        ("tiff", tiff()),
        ("psd", psd()),
        ("ico", ico()),
        ("jpeg xl", jpeg_xl()),
    ] {
        assert_eq!(
            refused(final_check::<ImageFile>(&bytes)).key,
            "validation-image",
            "{name}"
        );
    }
}

#[derive(Default)]
struct SvgAllowed;

impl MimeAllowlist for SvgAllowed {
    fn allowed() -> &'static [&'static str] {
        &["image/svg+xml"]
    }
}

#[test]
fn svg_is_no_image_file_and_passes_only_an_allowlist_naming_it() {
    assert_eq!(
        refused(final_check::<ImageFile>(SVG)).key,
        "validation-image"
    );
    let declared = MimeType::<SvgAllowed>::default().validate_final(
        SVG,
        SVG.len() as u64,
        Some("image/svg+xml"),
    );
    assert!(declared.is_ok(), "{declared:?}");
}

// ── `Dimensions` ────────────────────────────────────────────────────

#[derive(Default)]
struct AtMost100Wide;

impl DimensionLimits for AtMost100Wide {
    fn max_width() -> Option<u32> {
        Some(100)
    }
}

#[test]
fn dimensions_max_width_refuses_one_pixel_over_and_passes_the_limit() {
    assert_eq!(
        refused(final_check::<Dimensions<AtMost100Wide>>(&png(101, 50))).key,
        "validation-dimensions"
    );
    assert!(final_check::<Dimensions<AtMost100Wide>>(&png(100, 50)).is_ok());
    assert!(final_check::<Dimensions<AtMost100Wide>>(&png(1, 5000)).is_ok());
}

#[derive(Default)]
struct Banner;

impl DimensionLimits for Banner {
    fn min_width() -> Option<u32> {
        Some(600)
    }
    fn min_height() -> Option<u32> {
        Some(100)
    }
    fn max_height() -> Option<u32> {
        Some(200)
    }
}

#[test]
fn dimensions_check_every_bound_together() {
    assert!(final_check::<Dimensions<Banner>>(&png(600, 100)).is_ok());
    assert!(final_check::<Dimensions<Banner>>(&png(2000, 200)).is_ok());
    for (width, height) in [(599, 150), (800, 99), (800, 201)] {
        assert_eq!(
            refused(final_check::<Dimensions<Banner>>(&png(width, height))).key,
            "validation-dimensions",
            "{width}x{height}"
        );
    }
}

#[derive(Default)]
struct Exactly640By480;

impl DimensionLimits for Exactly640By480 {
    fn width() -> Option<u32> {
        Some(640)
    }
    fn height() -> Option<u32> {
        Some(480)
    }
}

#[test]
fn dimensions_read_every_type_image_file_accepts() {
    for (name, bytes) in [
        ("png", png(640, 480)),
        ("jpeg", jpeg(640, 480)),
        ("gif", gif(640, 480)),
        ("bmp", bmp(640, 480)),
        ("bmp top-down", bmp(640, -480)),
        ("bmp os/2", bmp_os2(640, 480)),
        ("webp", webp(640, 480)),
        ("webp lossless", webp_lossless(640, 480)),
        ("webp lossy", webp_lossy(640, 480)),
        ("avif", avif(640, 480)),
        ("heic", heic(640, 480)),
    ] {
        assert!(
            final_check::<Dimensions<Exactly640By480>>(&bytes).is_ok(),
            "{name} reads as 640x480"
        );
    }
    for (name, bytes) in [
        ("png", png(480, 640)),
        ("jpeg", jpeg(641, 480)),
        ("bmp top-down", bmp(640, -481)),
        ("webp", webp(640, 479)),
        ("heic", heic(480, 640)),
    ] {
        assert_eq!(
            refused(final_check::<Dimensions<Exactly640By480>>(&bytes)).key,
            "validation-dimensions",
            "{name}"
        );
    }
}

#[derive(Default)]
struct ThreeByTwo;

impl DimensionLimits for ThreeByTwo {
    fn ratio() -> Option<f64> {
        Some(3.0 / 2.0)
    }
}

#[derive(Default)]
struct Square;

impl DimensionLimits for Square {
    fn ratio() -> Option<f64> {
        Some(1.0)
    }
}

#[test]
fn dimensions_ratio_allows_laravels_one_pixel_tolerance() {
    assert!(final_check::<Dimensions<ThreeByTwo>>(&png(300, 200)).is_ok());
    assert!(final_check::<Dimensions<ThreeByTwo>>(&png(1500, 1000)).is_ok());
    assert!(final_check::<Dimensions<ThreeByTwo>>(&png(301, 200)).is_err());
    assert!(final_check::<Dimensions<ThreeByTwo>>(&png(200, 300)).is_err());

    assert!(final_check::<Dimensions<Square>>(&png(1000, 1000)).is_ok());
    assert!(
        final_check::<Dimensions<Square>>(&png(1001, 1000)).is_err(),
        "one pixel over is outside the tolerance at this size"
    );
}

#[derive(Default)]
struct Anything;

impl DimensionLimits for Anything {}

#[test]
fn dimensions_refuse_a_file_whose_size_cannot_be_read() {
    for (name, bytes) in [
        ("tiff", tiff()),
        ("psd", psd()),
        ("svg", SVG.to_vec()),
        (
            "jpeg metadata past the sniff buffer",
            jpeg_with_large_metadata(),
        ),
        ("truncated png", png(10, 10)[..18].to_vec()),
        ("zero-width png", png(0, 10)),
        ("not an image", b"%PDF-1.7\n".to_vec()),
    ] {
        assert_eq!(
            refused(final_check::<Dimensions<Anything>>(&bytes)).key,
            "validation-dimensions",
            "{name}"
        );
    }
    assert!(
        final_check::<Dimensions<Anything>>(&png(10, 10)).is_ok(),
        "with no limit set, a readable image passes"
    );
}

#[test]
fn image_file_and_dimensions_compose_in_order() {
    type Avatar = (ImageFile, Dimensions<AtMost100Wide>);
    assert_eq!(
        refused(final_check::<Avatar>(&tiff())).key,
        "validation-image",
        "the type check runs first"
    );
    assert_eq!(
        refused(final_check::<Avatar>(&png(101, 10))).key,
        "validation-dimensions"
    );
    assert!(final_check::<Avatar>(&jpeg(100, 10)).is_ok());
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn a_refused_size_reads_laravels_dimensions_message() {
    let message = refused(final_check::<Dimensions<AtMost100Wide>>(&png(101, 1)));
    assert_eq!(
        english(bag("avatar", message), "avatar").await,
        ["The avatar field has invalid image dimensions."]
    );
}
