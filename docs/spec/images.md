# Images

Status: Agreed 2026-10-05
Prefix: IMG

Drafted 2026-10-05 from issue #145 (EXIF orientation and the rest of
Laravel's Image API) and the developer's acceptance of the recommendations
("I'll accept your changes for the Image changes"). The plan was posted on
the issue. Orientation and the metadata defects (IMG-001, IMG-002) change
what applications store today and come first. The Observed section
describes framework `2bd4bd53d` (v3.2.1), checked against the code by an
independent reader the same day and again at `c34ee7b15`, where nothing
under `framework/src/media/` had changed; Laravel references cite
`reference/framework-13.27.0/src/Illuminate/` and Intervention Image 4.

The `images` mechanism requires the host's ImageMagick 7 for the `magick`
half of each requirement and runs those tests even though the gate leaves
them ignored; a missing binary fails the mechanism rather than skipping
it (ruled 2026-10-05).

## Observed at 2bd4bd53d

Status: Observed

- Neither driver applies the EXIF orientation: `oxideav` decodes,
  transforms and encodes with no orientation step
  (`framework/src/media/oxideav.rs:281-314,426-442,695-741`), and `magick`
  runs without `-auto-orient` (`framework/src/media/magick.rs:442-498,563-594`).
- `oxideav` decodes JPEG with zune-jpeg, lossless JPEG with oxideav-mjpeg
  (`oxideav.rs:17-23,305-312`), and WebP with `decode_webp_image`, which
  returns pixels without metadata (`oxideav.rs:298-300`). EXIF appears only
  where the sniffer counts the JPEG APP1 segment against the memory budget
  (`framework/src/media/sniff.rs:330,521-532`; `framework/src/media/oxideav/peak.rs:393`).
- Sources for the tag: zune-jpeg's `exif()` returns the TIFF bytes after
  the headers are decoded, and keeps the last of several Exif APP1
  segments (zune-jpeg 0.5.16-rc2 `headers.rs:736`); PNG's `eXIf` chunk is
  uncompressed TIFF; WebP's `EXIF` chunk is reachable through the
  container parser and often carries an `Exif\0\0` prefix. A lossless
  JPEG never reaches zune-jpeg's `exif()`, and the sniffer's walk, which
  every JPEG takes first (`sniff.rs:305-312`), visits each APP1 segment
  but keeps only their byte count (`sniff.rs:414-416`). oxideav-png's
  `parse_metadata` also inflates every compressed text and profile chunk
  with no limit. The `image` crate 0.25.10 is compiled in transitively,
  and its `metadata::Orientation::from_exif_chunk` takes TIFF bytes
  without the `Exif\0\0` prefix.
- The decode estimate runs before zune-jpeg reads the headers
  (`oxideav.rs:254-263`, against `decode_headers` at `719`) and has no
  term for orientation (`peak.rs:170-186`), while a turn or flip through
  `filter` allocates a second full-size plane while the first is held
  (`oxideav.rs:872-878`).
- `oxideav`'s quarter-turn rotation is exact and both flips are wired
  (`oxideav.rs:816-818`), so all eight orientations can be applied without
  resampling.
- `oxideav` writes only pixels, dropping all metadata, the ICC profile
  included (`oxideav.rs:366-416,1031-1042,1115-1126`); `magick` passes no
  `-strip` or `+profile` and keeps all of it, GPS position, the
  `Orientation` tag, JPEG `COM` segments and PNG text chunks included
  (`magick.rs:563-594`), so its output looks upright today only because
  the tag survives.
- `Image` lacks `orient`, a background for `rotate`, the `flip`/`flop`
  aliases, and padding in `contain`, which the manual already lists as a
  divergence (`framework/src/media/mod.rs:420-497,464-467`). One
  process-wide driver serves every image (`mod.rs:256,289-321,526`); the
  only extension point is a whole `ImageDriver` over a closed
  `Transformation` enum (`framework/src/media/driver.rs:98-159,209-229`),
  which derives `Copy` and `PartialEq` and which both drivers copy step
  by step (`oxideav.rs:359`; `magick.rs:572`); `store(disk, path)` needs
  the full path and returns nothing (`mod.rs:561-566`). The filesystem
  has no per-file visibility: a file is public when its disk has a public
  base URL (`manual/filesystem.md:295-337`).
- Rotation fills exposed corners with transparent black
  (`oxideav.rs:918`; `magick.rs:479-485`), JPEG encoding drops alpha
  (`oxideav.rs:385-389`), and `oxideav`'s GIF encoder maps every pixel to
  an opaque colour (`oxideav.rs:1108-1114`), so rotated corners and any
  transparent area come out black in JPEG, and in GIF from `oxideav`.
  Laravel fills with white and flattens alpha onto white for JPEG.
- `dimensions()` measures the processed output (`mod.rs:568-575`).
- `Transformation` and `ImageConfig` are not `#[non_exhaustive]`
  (`driver.rs:98`; `mod.rs:149-171`).

## Orientation and metadata

[IMG-001] Both drivers MUST apply the EXIF orientation on decode by
default, read from the same place: the JPEG's last APP1 Exif segment (the
one zune-jpeg keeps, lossless JPEGs included), PNG's uncompressed `eXIf`
chunk and WebP's `EXIF` chunk (its `Exif\0\0` prefix removed), parsed
with `image::metadata::Orientation` as a direct dependency. `oxideav`
applies it with its exact rotations and flips; `magick` applies the same
tag as the matching fixed rotation or flip after the input, so both
drivers turn one file alike, and uses `-auto-orient` only for a format
the framework cannot read (HEIC, TIFF). Reading the tag MUST NOT inflate
compressed chunks. `oxideav`'s decode estimate MUST count the full-size
copy a non-identity orientation allocates. `orient()` MUST exist as an
explicit transformation that applies the tag when decode has not, and
`ImageConfig` MUST offer an opt-out, read from `IMAGE_AUTO_ORIENT`
(default `true`) like its other fields.
Falsifier: a JPEG, PNG or WebP with any of the eight orientations comes out of either driver with pixels that are not the exact orientation permutation of its decoded pixels; `oxideav` orients a JPEG with two Exif APP1 segments by any but the last; a PNG with a large compressed chunk is inflated to read the tag; an oriented decode peaks above the estimate its refusal names; `orient()` is missing, does not apply the tag under the opt-out, or turns an image its decode already oriented; the opt-out does not keep the sensor's pixels; or `IMAGE_AUTO_ORIENT=false` does not turn orientation on decode off; or `magick` takes the orientation of a JPEG, PNG or WebP from anywhere but the places `oxideav` reads it.
Mechanism: `images`.
Rationale: Laravel's drivers orient on decode by default (Intervention `Config::autoOrientation`). Ruled 2026-10-05: MEM-003 gains an exception for the image bytes this changes on purpose. Ruled 2026-10-06 (escalation 5ef445fa): ImageMagick's own reading takes the first Exif segment, PNG raw-profile text and its `orNT` chunk, so `magick` applies the framework's reading.
Status: Agreed 2026-10-06

[IMG-002] Processed output from both drivers MUST NOT carry the source's
EXIF, XMP or IPTC metadata, GPS position included, or its text metadata:
JPEG `COM` segments and PNG `tEXt`, `zTXt` and non-XMP `iTXt` chunks,
including the date and time chunks an encoder adds by itself (`tIME`,
ImageMagick's `date:` text).
When orientation was not applied (the opt-out, with no `orient()` in the
pipeline), the `Orientation` tag MUST be kept in output that can hold
EXIF (JPEG, PNG, WebP), so a viewer can still correct the image. The
output MUST keep its colour: it carries the source's ICC profile where
the format can hold one (JPEG as APP2, PNG `iCCP`, WebP, BMP), or its
pixels are converted to sRGB (GIF). `magick`'s output MUST meet all of
this as `oxideav`'s does: no comment and no text chunk survives, the
`date:` chunks ImageMagick adds to a PNG included, and the only profiles
it carries are the ICC profile and, under the opt-out, an EXIF that holds
the `Orientation` tag alone. Every argument that selects the metadata
`magick` keeps is a fixed string the driver writes, never text a caller
supplies. Reading a compressed profile MUST count its inflated size
against `IMAGE_MAX_ALLOC_BYTES`, and a profile whose colour space no
longer matches the output's pixels MUST be converted away, not carried. A PNG's
`sRGB`, `cICP`, `cHRM` and `gAMA` chunks describe its colour and are kept
with the profile, and dropped when it is converted away.
Falsifier: output from either driver carries an Exif or XMP APP1, an APP13, a JPEG `COM` segment, a PNG `eXIf`, `tEXt`, `zTXt`, `iTXt` or `tIME` chunk, or a WebP `EXIF` or `XMP ` chunk, other than the opt-out's Orientation-only EXIF; output carries an `Orientation` tag after orientation was applied, or lacks it under the opt-out in JPEG, PNG or WebP; a source's ICC profile that matches the output's pixels is not byte-identical in JPEG, PNG, WebP or BMP output; output carries an ICC profile whose colour space differs from its pixels'; a flat Display P3 source's GIF output is not its sRGB conversion within one 8-bit step per channel; `oxideav` allocates past `IMAGE_MAX_ALLOC_BYTES` inflating a PNG `iCCP`; or a PNG's `sRGB`, `cICP`, `cHRM` or `gAMA` chunk that still describes the output's pixels is dropped, or one is kept after its profile is converted away.
Mechanism: `images`.
Rationale: A divergence from Laravel, whose default keeps metadata with Imagick and drops all of it, ICC included, with GD; stripping location and text by default protects users' uploads, and stripping before orienting is applied would turn `magick` output sideways, so IMG-001 and IMG-002 land together. Ruled 2026-10-05: text metadata is stripped in both drivers, and MEM-003 gains an exception for the image bytes this changes on purpose. Keeping only the ICC profile is not enough for `magick`: ImageMagick 7 writes a source's comment back as a JPEG `COM` segment or a PNG `tEXt` chunk, adds `date:` `tEXt` chunks to every PNG it writes, and drops the `Orientation` tag with the EXIF profile.
Status: Agreed 2026-10-05

## The rest of the Image API

[IMG-003] An image MUST be able to choose its driver, overriding the
process-wide default for that image only.
Falsifier: an image that chooses a driver is processed by another one (with `IMAGE_MAGICK_BINARY` naming a missing binary, an image choosing `magick` does not fail naming it while the same image without the choice succeeds), or the choice changes the driver of another image or the process default.
Mechanism: `images`.
Rationale: Laravel's `using()`; HEIC uploads can go to `magick` while the rest stay on `oxideav`.
Status: Agreed 2026-10-05

[IMG-004] An application MUST be able to register a custom transformation
without implementing a whole driver, and both drivers MUST apply it at
its place in the pipeline. It MUST receive decoded pixels under both
drivers, as they stand at that place, and MUST NOT contribute ImageMagick
arguments, so the manual's promise that user input reaches no argument
position holds: under `magick`, each custom step runs between two
ImageMagick runs that pass the image over stdin and stdout, and the
driver writes no temporary file. Output from a pipeline with a custom
step MUST meet IMG-001 and IMG-002 as one without it does. A custom
transformation with no registration MUST fail with an error naming it.
Falsifier: a custom transformation registered without an `ImageDriver` impl is missing from the output, or lands at another place in the pipeline, under `oxideav` or `magick`; it receives anything but the image's pixels at its place; the `magick` argv carries anything it supplied; the driver writes a temporary file for a `magick` pipeline with a custom step, or that pipeline loses the source's ICC profile or the opt-out's `Orientation` tag; or a custom transformation with no registration does not fail with an error naming it.
Mechanism: `images`.
Rationale: Laravel's `transform()` and `transformUsing`. Ruled 2026-10-05: a custom transformation works on decoded pixels under both drivers and never contributes ImageMagick arguments.
Status: Agreed 2026-10-05

[IMG-005] An image MUST be storable on a disk with `store` in a
directory under a generated name (40 random alphanumerics and the output
format's extension) and with `store_as` under a given name, each
returning the stored path. A call written against today's
`store(disk, path)` MUST fail to compile rather than store elsewhere.
Falsifier: the returned path does not read back the processed bytes from that disk; a generated name is not 40 random alphanumerics plus the output format's extension; two images stored in one directory get the same name; `store_as` does not write exactly `dir/name`; or a call in today's `store(disk, path)` form compiles.
Mechanism: `images`.
Rationale: Laravel's `store`, `storeAs`, `hashName` and `extension`. Ruled 2026-10-05: there is no `storePublicly` or `storePubliclyAs`, because Suprnova sets visibility per disk, so storing on a public disk is what public means.
Status: Agreed 2026-10-05

[IMG-006] `rotate` MUST take a background colour for the exposed corners,
white by default when the output is JPEG or GIF and transparent when it
is PNG, WebP or BMP. Encoding to JPEG or GIF MUST flatten transparency
onto the rotation's background, or onto white when the pipeline does not
rotate. The colour MUST be a typed value that the driver writes into
ImageMagick's arguments itself, so no text a caller supplies reaches an
argument.
Falsifier: default corners, or flattened transparency, in JPEG or GIF output from either driver are not white (JPEG within its compression loss); default corners in PNG, WebP or BMP output are not fully transparent; a given colour is not used for the corners and the flatten; or a colour reaches `magick`'s arguments as text the caller wrote rather than the driver's formatting of the value.
Mechanism: `images`.
Rationale: Laravel fills with white for every format and flattens alpha onto white for JPEG; keeping transparency where the format holds it is the divergence, as today's output does. Ruled 2026-10-05: MEM-003 gains an exception for the image bytes this changes on purpose.
Status: Agreed 2026-10-05

[IMG-007] The image MUST offer `to_base64` and `to_data_uri`, `width()`
and `height()`, the shortcuts `to_webp`, `to_jpg`, `to_jpeg`, `to_png`,
`to_gif` and `to_bmp`, `optimize(format, quality)`, and `flip` and
`flop` as aliases of `flip_vertically` and `flip_horizontally`.
Falsifier: one is missing; `to_base64` does not decode to `to_bytes`; `to_data_uri` does not carry `mime_type()` and that payload; `width` or `height` differs from `dimensions()`; a shortcut or `optimize` records a different format or quality than `to_format` and `quality`; or `flip` or `flop` mirrors on the other axis.
Mechanism: `images`.
Rationale: Laravel's `toBase64`, `toDataUri`, `width`, `height`, `toWebp` and siblings, `optimize`, `flip` and `flop`.
Status: Agreed 2026-10-05

[IMG-008] `Transformation` and `ImageConfig` MUST be `#[non_exhaustive]`.
The manual MUST document orientation, metadata stripping and the new
API, and MUST keep its promise that user input reaches no ImageMagick
argument position.
Falsifier: a struct literal of `ImageConfig` or an exhaustive match on `Transformation` compiles outside the crate; `manual/images.md` lacks orientation, metadata stripping, or a method IMG-003 to IMG-007 adds; `IMAGE_AUTO_ORIENT` is missing from `manual/images.md` or `manual/env-vars.md`; or `manual/images.md` drops the promise that no argument position is reachable.
Mechanism: `images`.
Rationale: Ruled 2026-10-05: the manual's promise that no ImageMagick argument position is reachable stays.
Status: Agreed 2026-10-05
