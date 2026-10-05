# Images

Status: Draft
Prefix: IMG

Drafted 2026-10-05 from issue #145 (EXIF orientation and the rest of
Laravel's Image API) and the developer's acceptance of the recommendations
("I'll accept your changes for the Image changes"). The plan was posted on
the issue. Orientation and the metadata defects (IMG-001, IMG-002) change
what applications store today and come first. The Observed section
describes framework `2bd4bd53d` (v3.2.1), checked against the code by an
independent reader the same day; Laravel references cite
`reference/framework-13.27.0/src/Illuminate/` and Intervention Image 4.

## Observed at 2bd4bd53d

Status: Observed

- Neither driver applies the EXIF orientation: `oxideav` decodes,
  transforms and encodes with no orientation step
  (`framework/src/media/oxideav.rs:281-314,426-442,695-741`), and `magick`
  runs without `-auto-orient` (`framework/src/media/magick.rs:442-498,563-593`).
- `oxideav` decodes JPEG with zune-jpeg, lossless JPEG with oxideav-mjpeg
  (`oxideav.rs:17-23,305-312`), and WebP with `decode_webp_image`, which
  returns pixels without metadata (`oxideav.rs:298-300`). EXIF appears only
  where the sniffer counts the JPEG APP1 segment against the memory budget
  (`framework/src/media/sniff.rs:330,521-532`; `framework/src/media/oxideav/peak.rs:393`).
- Sources for the tag: zune-jpeg's `exif()` returns the TIFF bytes after
  the headers are decoded; PNG's `eXIf` chunk is uncompressed TIFF;
  WebP's `EXIF` chunk is reachable through the container parser and often
  carries an `Exif\0\0` prefix; a lossless JPEG's APP1 is the segment the
  sniffer already finds. oxideav-png's `parse_metadata` also inflates every
  compressed text and profile chunk with no limit. The `image` crate 0.25.10
  is compiled in transitively, and its `metadata::Orientation::from_exif_chunk`
  takes TIFF bytes without the `Exif\0\0` prefix.
- `oxideav`'s quarter-turn rotation is exact and both flips are wired
  (`oxideav.rs:816-818`), so all eight orientations can be applied without
  resampling.
- `oxideav` writes only pixels, dropping all metadata, the ICC profile
  included (`oxideav.rs:366-416,1031-1041,1115-1125`); `magick` keeps all
  of it, GPS position and the `Orientation` tag included
  (`magick.rs:563-593`), so its output looks upright today only because
  the tag survives.
- `Image` lacks `orient`, a background for `rotate`, the `flip`/`flop`
  aliases, and padding in `contain`, which the manual already lists as a
  divergence (`framework/src/media/mod.rs:420-497,464-467`). One
  process-wide driver serves every image (`mod.rs:256,289-321,526`); the
  only extension point is a whole `ImageDriver` over a closed
  `Transformation` enum (`framework/src/media/driver.rs:98-159,209-229`);
  `store(disk, path)` needs the full path and returns nothing
  (`mod.rs:561-566`).
- Rotation fills exposed corners with transparent black
  (`oxideav.rs:918`; `magick.rs:479-485`), and JPEG encoding drops alpha
  (`oxideav.rs:385-389`), so rotated corners and any transparent area come
  out black in JPEG. Laravel fills with white and flattens alpha onto
  white for JPEG.
- `dimensions()` measures the processed output (`mod.rs:568-575`).
- `Transformation` and `ImageConfig` are not `#[non_exhaustive]`
  (`driver.rs:98`; `mod.rs:149-171`).

## Orientation and metadata

[IMG-001] Both drivers MUST apply the EXIF orientation on decode by
default: `magick` with `-auto-orient` after the input, `oxideav` by
reading the tag from zune-jpeg's EXIF, the lossless JPEG's APP1 segment,
PNG's uncompressed `eXIf` chunk and WebP's `EXIF` chunk (its `Exif\0\0`
prefix removed), parsing it with `image::metadata::Orientation` as a
direct dependency, and applying it with its exact rotations and flips.
Reading the tag MUST NOT inflate compressed chunks. `orient()` MUST exist
as an explicit transformation, and `ImageConfig` MUST offer an opt-out.
Falsifier: a portrait JPEG, PNG or WebP with orientation 6 comes out sideways from either driver; any of the eight orientations is resampled or wrong; a PNG with a large compressed chunk is inflated to read the tag; `orient()` is missing; or the opt-out does not keep the sensor's pixels.
Mechanism: `images`.
Rationale: Laravel's drivers orient on decode by default (Intervention `Config::autoOrientation`).
Status: Draft

[IMG-002] Processed output from both drivers MUST NOT carry the source's
EXIF, XMP or IPTC metadata, GPS position included, except that with the
orientation opt-out the `Orientation` tag MUST be kept so a viewer can
still correct the image. The output MUST keep its colour: it carries the
source's ICC profile where the format can hold one (JPEG as APP2, PNG
`iCCP`, WebP, BMP; `magick` keeps profiles with `+profile '!icc,*'`), or
its pixels are converted to sRGB (GIF).
Falsifier: output from either driver carries GPS data, or an `Orientation` tag when orientation was applied; output under the opt-out loses the tag; or a wide-gamut source shifts colour in the output.
Mechanism: `images`.
Rationale: A divergence from Laravel, whose default keeps metadata with Imagick and drops all of it, ICC included, with GD; stripping location by default protects users' uploads. Stripping before orienting is applied would turn `magick` output sideways, so IMG-001 and IMG-002 land together.
Status: Draft

## The rest of the Image API

[IMG-003] An image MUST be able to choose its driver, overriding the
process-wide default for that image only.
Falsifier: choosing `magick` for one image changes the driver of another.
Mechanism: `images`.
Rationale: Laravel's `using()`; HEIC uploads can go to `magick` while the rest stay on `oxideav`.
Status: Draft

[IMG-004] An application MUST be able to register a custom transformation
for a driver without implementing a whole driver.
Falsifier: adding one operation requires a full `ImageDriver`.
Mechanism: `images`.
Rationale: Laravel's `transform()` and `transformUsing`.
Status: Draft

[IMG-005] An image MUST be storable under a generated name with the
output format's extension, returning the stored path, alongside variants
that take a name and that store publicly.
Falsifier: storing without a name fails, the extension differs from the output format, or no path is returned.
Mechanism: `images`.
Rationale: Laravel's `store`, `storeAs`, `storePublicly`, `storePubliclyAs`, `hashName` and `extension`.
Status: Draft

[IMG-006] `rotate` MUST take a background colour for the exposed corners,
white by default for formats without alpha and transparent for formats
with it, and encoding to a format without alpha MUST flatten transparency
onto that background.
Falsifier: a rotated JPEG's corners, or the transparent area of a PNG converted to JPEG, come out black by default; or a given colour is not used.
Mechanism: `images`.
Rationale: Laravel fills with white for every format and flattens alpha onto white for JPEG; keeping transparency where the format holds it is the divergence, as today's output does.
Status: Draft

[IMG-007] The image output MUST offer base64 and data-URI encodings,
`width()`, `height()`, format shortcuts, `optimize`, and the `flip`/`flop`
aliases.
Falsifier: one of them is missing.
Mechanism: `images`.
Rationale: Laravel's `toBase64`, `toDataUri`, `width`, `height`, `toWebp` and siblings, `optimize`, `flip` and `flop`.
Status: Draft

[IMG-008] `Transformation` and `ImageConfig` MUST be `#[non_exhaustive]`,
and the manual MUST document orientation, metadata stripping and the new
API.
Falsifier: either type is exhaustive, or the manual omits a requirement above.
Mechanism: `images`, `manual-check`.
Status: Draft
