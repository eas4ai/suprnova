# Images

Suprnova ships a Laravel-shaped image pipeline: build it in a handler,
chain the operations you want, and finish with a terminal that hands you
bytes, a response, or a stored file.

```rust
use suprnova::{Image, OutputFormat, Response, handler};

#[handler]
pub async fn thumbnail() -> Response {
    Ok(Image::from_path("storage/photos/hero.jpg")
        .cover(320, 320)
        .to_format(OutputFormat::WebP)
        .quality(80)
        .to_response()
        .await?)
}
```

That handler decodes the JPEG, fills a 320x320 box, crops the overflow
from the centre, encodes WebP, and returns a `200` with
`Content-Type: image/webp`.

The subsystem lives in `suprnova::media`, behind the default-on `media`
feature. Everything you normally reach for - `Image`, `OutputFormat`,
`ImageDriver`, `ImageConfig` - is re-exported flat at the crate root, so
`use suprnova::Image;` is the import you want. The module name is
plural-in-spirit on purpose: it is where the OxideAV-backed audio and
video surfaces will live too.

If you are upgrading, note that the upload validator that used to be
called `Image` is now `ImageFile`, which frees the plain name for this
pipeline type. That mirrors Laravel, where the validation rule is
`ImageFile` and the manipulation type is `Image`. See
[Requests](requests.md) for the validator.

## The pipeline is lazy

Constructing an `Image` reads nothing and decodes nothing. Operations
record themselves; the source is opened only when a terminal runs. So
this is free:

```rust
use suprnova::Image;

let pipeline = Image::from_disk("uploads", "avatars/42.png").resize(64, 64);
```

Nothing has touched the disk yet. `Image` is `Clone`, and a clone
re-runs the pipeline from its source rather than sharing a result.

Two constructors have to be eager, and say so in their docs:
`from_upload` (an upload's temp file does not outlive the request) and
`from_stream` (a stream can only be consumed once).

## Construction

| Constructor | Source | Eager? |
|---|---|---|
| `Image::from_bytes(bytes)` | anything `Into<Bytes>` | no |
| `Image::from_path(path)` | the filesystem | no |
| `Image::from_disk(disk, path)` | a `Storage` disk | no |
| `Image::from_upload(&file).await?` | an `UploadedFile` | yes |
| `Image::from_stream(stream).await?` | a `Stream<Item = io::Result<Bytes>>` | yes |

`from_stream` enforces `IMAGE_MAX_ALLOC_BYTES` *while* collecting, so an
endless stream is cut off rather than discovered after it has already
filled memory.

## Operations

| Method | Effect |
|---|---|
| `resize(w, h)` | Exact dimensions, aspect ratio ignored |
| `resize_width(w)` / `resize_height(h)` | One dimension, the other derived from the aspect ratio |
| `resize_width_only(w)` / `resize_height_only(h)` | One dimension, the other kept at its current size, so the aspect ratio changes (Laravel's `resize(width: ...)`) |
| `scale(w, h)` | Fit inside the box, preserving aspect ratio. **Never enlarges** |
| `scale_width(w)` / `scale_height(h)` | Scale down to at most one dimension. Never enlarges |
| `crop(w, h, x, y)` | Cut a rectangle out. Errors if it falls outside the image |
| `cover(w, h)` | Fill the box exactly, cropping the overflow from the centre |
| `contain(w, h)` | Fit inside the box, preserving aspect ratio. No padding |
| `rotate(degrees)` | Rotate clockwise by any angle, growing the canvas to fit |
| `rotate_with_background(degrees, color)` | Rotate, filling the exposed corners with a `Color` |
| `flip_vertically()` / `flip_horizontally()` | Mirror top to bottom, or left to right |
| `flip()` / `flop()` | Laravel's names for the same two mirrors |
| `orient()` | Apply the source's EXIF orientation here, if decoding did not |
| `transform(transformation)` | Add any `Transformation`, a custom one included |
| `transform_with(name, settings)` | Add a custom transformation and hand its function `settings` |
| `blur(amount)` | Gaussian blur, `0..=100`. `0` is a no-op |
| `sharpen(amount)` | Unsharp mask, `0..=100`. `0` is a no-op. `50` is the classic strength |
| `grayscale()` | Desaturate. Spelled the Laravel way |
| `to_format(format)` | Choose the output container: `Jpeg`, `Png`, `WebP`, `WebPLossless`, `Gif` or `Bmp` |
| `to_webp()`, `to_jpg()`, `to_jpeg()`, `to_png()`, `to_gif()`, `to_bmp()` | Shortcuts for `to_format` |
| `quality(q)` | Encode quality, clamped to `1..=100`, default `70` |
| `optimize(format, q)` | `to_format(format).quality(q)` in one call |
| `using(driver)` | Process this image with the `oxideav` or `magick` driver (see [Backends](#backends)) |

Values that would be nonsense are clamped rather than rejected:
`blur(500)` records `100`, `quality(0)` records `1`, and a side of `0`
given to `resize`, `cover` or a one-side resize records `1`, as Laravel's
`Image` clamps it, so no driver, a custom one included, receives a zero
side. A crop that falls outside the image is a real error, not a clamp,
because silently moving someone's crop box is worse than telling them.

The "only" resizes keep the other side at the size the image has at that
point of the pipeline, not the source's, so they compose with the steps
before them:

```rust
use suprnova::{FrameworkError, Image};

async fn banner_strip() -> Result<(u32, u32), FrameworkError> {
    // 40x20 -> 30x12 -> 10x12
    Image::from_path("storage/photos/banner.png")
        .resize(30, 12)
        .resize_width_only(10)
        .dimensions()
        .await
}
```

The new size meets the same decode limits as any resize target, under
either driver.

`rotate` takes arbitrary angles. A 90-degree multiple takes an exact
axis-aligned path with no resampling; anything else is bilinear, and the
canvas grows so no pixel is clipped.

The corners a rotation exposes are white when the output is JPEG or GIF,
which cannot hold transparency, and transparent when it is PNG, WebP or
BMP. `rotate_with_background` names the colour instead. JPEG and GIF
output also flattens any transparency in the image onto that colour, or
onto white when the pipeline does not rotate:

```rust
use suprnova::{Color, FrameworkError, Image};

async fn tilted() -> Result<Vec<u8>, FrameworkError> {
    Image::from_path("storage/photos/logo.png")
        .rotate_with_background(15.0, Color::from_hex("#1e293b")?)
        .to_jpg()
        .to_bytes()
        .await
}
```

`Color` is a typed value: `Color::rgb`, `Color::rgba`, `Color::WHITE`,
`Color::BLACK`, `Color::TRANSPARENT`, or `Color::from_hex` for a colour
that arrives as text (`rgb`, `rrggbb` or `rrggbbaa`, with or without
`#`). A string that is not a hex colour fails at `from_hex`, in your
code, rather than inside a driver.

## Orientation

A phone stores a photo's pixels as the sensor read them and records, in
the EXIF `Orientation` tag, how to turn them for display. Both drivers
apply the tag as they decode, as Laravel's do, so a portrait photo comes
out upright. All eight orientations are quarter turns and mirrors, so
applying one moves pixels and never resamples them.

The tag is read from a JPEG's last Exif `APP1` segment, wherever it sits
(a progressive JPEG can carry one between its scans), a PNG's `eXIf`
chunk, and a WebP's `EXIF` chunk. Reading it never inflates a compressed
chunk. Both drivers read the tag in these places, with the same reader,
and turn the image with a fixed quarter turn or mirror, so a file comes
out the same way under either. ImageMagick's own orientation sources, a
PNG's `Raw profile type exif` text chunk and its `orNT` chunk, are not
used. For a format the framework cannot read (HEIC, TIFF), the `magick`
driver leaves orientation to ImageMagick's `-auto-orient`.

Turning the image holds a second full-size plane beside the decoded one,
and the decode estimate counts it (see
[What a decode costs](#what-a-decode-costs)). `ImageDriver::dimensions`
reports a tagged source at its turned size, and `dominant_color` at its
average, without turning the pixels.

| Var | Default | Purpose |
|---|---|---|
| `IMAGE_AUTO_ORIENT` | `true` | Apply the EXIF orientation on decode. `false`, `0`, `no` or `off` turn it off |

With `IMAGE_AUTO_ORIENT=false` the pixels keep the sensor's orientation,
and the output keeps the `Orientation` tag, so a viewer can still turn
the image. `orient()` then applies the tag at the point in the pipeline
where you put it:

```rust
use suprnova::{FrameworkError, Image};

// With IMAGE_AUTO_ORIENT=false: crop in sensor coordinates, then turn.
async fn crop_then_turn() -> Result<Vec<u8>, FrameworkError> {
    Image::from_path("storage/photos/scan.jpg")
        .crop(1200, 800, 40, 40)
        .orient()
        .to_bytes()
        .await
}
```

When decoding has already oriented the image, `orient()` does nothing, so
an image is never turned twice.

## Metadata

Processed output keeps two things of the source's metadata and drops the
rest:

- **The ICC profile**, so the colours read the same. JPEG output carries
  it in `APP2` segments, PNG in an `iCCP` chunk, WebP in an `ICCP` chunk,
  and BMP in a V5 header. A PNG's `cHRM`, `gAMA`, `sRGB` and `cICP`
  chunks describe its colour too, and PNG output keeps them with the
  profile.
- **An `Orientation` tag**, alone in its own EXIF block, when orientation
  was not applied: `IMAGE_AUTO_ORIENT=false` with no `orient()` in the
  pipeline. Only JPEG, PNG and WebP can hold one.

Everything else goes: EXIF (and with it the GPS position), XMP, IPTC,
JPEG comments, PNG `tEXt`, `zTXt` and `iTXt` chunks, and the date and
time an encoder adds by itself, such as PNG's `tIME` chunk and
ImageMagick's `date:` text. A user's upload does not publish where it was
taken.

A profile must describe the pixels it travels with. GIF holds no profile
here, so GIF output has its colours converted from the profile to sRGB.
The default driver writes every format as RGB, so a grey profile no
longer describes its output: the pixels are converted from it to sRGB,
and the profile and its PNG colour chunks are dropped. The `magick`
driver keeps a grey profile on output it writes as grey.

A profile is carried only when its length is the size its own header
gives. A profile is a few kilobytes, but a PNG's is compressed, and a
small file can hold one that inflates a thousand times over. So the
driver reads the header first, and charges `IMAGE_MAX_ALLOC_BYTES` for
what it holds for the profile beside the pixels: the profile itself when
it has to join its pieces or inflate it, the compressed copy PNG output
makes of it, and the copy the output carries. Every inflate counts the
profile's inflated size, even one that only checks a PNG profile's
length. A profile that sits whole in the file is read where it stands,
and a JPEG's or a GIF's that the output drops costs nothing. A profile
whose copies do not fit is refused like any other decode over the limit.
The `magick` driver reads no more than the header of a source profile,
and checks the one ImageMagick writes without holding it, except a GIF's
RGB or grey profile, which it joins to convert the palette from.

A CMYK or Lab profile cannot describe RGB output, so it is dropped. The
default driver does not read CMYK JPEGs at all. Under `magick`,
ImageMagick converts CMYK pixels to RGB with its own formula, not through
the profile, so colours can shift.

## Custom transformations

A custom transformation is a function over decoded pixels that you
register by name, and both drivers run it at its place in the pipeline:

```rust
use suprnova::{FrameworkError, Image, ImagePixels, Transformation, register_transformation};

pub fn register() {
    // Invert every colour, keeping the alpha.
    register_transformation("invert", |mut pixels: ImagePixels| {
        for pixel in pixels.pixels_mut().chunks_mut(4) {
            for channel in &mut pixel[..3] {
                *channel = !*channel;
            }
        }
        Ok(pixels)
    });
}

async fn negative() -> Result<Vec<u8>, FrameworkError> {
    Image::from_path("storage/photos/hero.jpg")
        .resize(800, 600)
        .transform(Transformation::custom("invert"))
        .to_png()
        .to_bytes()
        .await
}
```

`ImagePixels` holds packed 8-bit RGBA, rows top to bottom. The function
receives the pixels as the steps before it left them and returns the
pixels the steps after it work on; to change the size, build a new
`ImagePixels` with `ImagePixels::new(width, height, bytes)`. The returned
size meets the same `IMAGE_MAX_DIMENSION` and `IMAGE_MAX_ALLOC_BYTES`
caps a resize does.

Register during bootstrap. Registering a name again replaces its
function. An image that names a transformation nothing is registered
under fails with an error that names it.

### Settings per call

A transformation that needs values per image, such as the block size of
a pixelate, takes them as settings. Register it with
`register_transformation_with`, naming the settings type in the function,
and record the step with `transform_with(name, settings)`:

```rust
use suprnova::{FrameworkError, Image, ImagePixels, register_transformation_with};

pub struct Pixelate {
    pub size: u32,
}

pub fn register() {
    register_transformation_with("pixelate", |mut pixels: ImagePixels, settings: &Pixelate| {
        let width = pixels.width() as usize;
        let height = pixels.height() as usize;
        let size = settings.size.max(1) as usize;
        let rgba = pixels.pixels_mut();
        for y in 0..height {
            for x in 0..width {
                // Each pixel takes the colour of the top-left pixel of its block.
                let block = ((y - y % size) * width + (x - x % size)) * 4;
                rgba.copy_within(block..block + 4, (y * width + x) * 4);
            }
        }
        Ok(pixels)
    });
}

async fn coarse(path: &str) -> Result<Vec<u8>, FrameworkError> {
    Image::from_path(path)
        .transform_with("pixelate", Pixelate { size: 8 })
        .to_png()
        .to_bytes()
        .await
}
```

The image holds the settings, not the step, so `Transformation` stays
`Copy`. Clones of the image share them, and they are released when the
last clone is dropped. A step whose settings are not the type the
transformation was registered with fails the image with an error that
names the transformation. So does a settings transformation recorded
with `transform(Transformation::custom(name))`, which carries no
settings, and a transformation registered with `register_transformation`
that is handed settings.

Under the `magick` driver a custom step runs in Rust between two
ImageMagick runs, which pass the image over stdout and stdin as a PNG
that keeps the ICC profile and, while orientation is still to be applied,
the EXIF. A pipeline with a custom step ends with the same profile and
orientation tag as one without. The step contributes no ImageMagick
argument, and its settings reach only your function.

## Terminals

Every terminal is `async`, consumes the `Image`, and runs the decode,
transform, and encode work on a blocking thread so it never stalls the
runtime. Source I/O happens before that hop, so a slow disk never
occupies a blocking worker.

| Terminal | Returns |
|---|---|
| `to_bytes()` | `Vec<u8>` of the encoded file |
| `to_response()` | An `HttpResponse` with the right `Content-Type` |
| `to_base64()` | The encoded file as standard, padded base64 |
| `to_data_uri()` | A `data:` URI: the media type and the base64 bytes |
| `save(path)` | Writes to the filesystem |
| `store(directory, disk)` | Writes to a `Storage` disk under a generated name, returning the path |
| `store_as(directory, name, disk)` | Writes to a `Storage` disk as `directory/name`, returning the path |
| `dimensions()` | `(width, height)` of the **processed** image |
| `width()` / `height()` | One side of `dimensions()` |
| `mime_type()` | The **processed** image's media type |
| `dominant_color()` | The average colour, as `#rrggbb` |

`dimensions()`, `mime_type()`, and `dominant_color()` all describe the
finished image, not the source - the same contract Laravel has. Asking
for the mime type still runs the pipeline, because reporting a type for
an image that cannot actually be produced is a lie the caller would only
discover later.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn describe() -> Result<(), FrameworkError> {
    let banner = Image::from_path("hero.png").resize(1200, 400);

    // Reads (1200, 400), not the source's dimensions.
    let (width, height) = banner.clone().dimensions().await?;
    println!("{width}x{height}");

    let accent = banner.to_format(OutputFormat::Jpeg).dominant_color().await?;
    println!("{accent}");

    Ok(())
}
```

## Formats

Five formats are read and written: **PNG, JPEG, WebP, GIF, and
BMP**.

| Format | Reads | Writes | Quality knob |
|---|---|---|---|
| PNG | yes | yes | ignored (lossless) |
| JPEG | yes | yes | honoured |
| WebP (`OutputFormat::WebP`) | yes | yes | honoured (lossy; see below for the lossless cases) |
| WebP (`OutputFormat::WebPLossless`) | yes | yes | ignored (lossless) |
| GIF | yes | yes | ignored (palette) |
| BMP | yes | yes | ignored (lossless) |

AVIF is neither read nor written: there is no AVIF encoder crate with a
license compatible with Suprnova's. WebP is the modern-format path.

GIF output is palette-quantised to at most 256 colours with
Floyd-Steinberg dithering before encoding, so a photographic source
converts cleanly rather than erroring.

### WebP

`OutputFormat::WebP` is lossy and uses the quality of the pipeline, the
same dial JPEG has. The quality is `70` when you set none.
`OutputFormat::WebPLossless` is always lossless and ignores the quality.
Use it when the pixels of the file must be exact. Both variants have the
content type `image/webp` and the extension `webp`.

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

async fn encode() -> Result<(), FrameworkError> {
    // Lossy at quality 80.
    let small = Image::from_path("storage/photos/hero.jpg")
        .to_format(OutputFormat::WebP)
        .quality(80)
        .to_bytes()
        .await?;

    // Lossless: the quality has no effect.
    let exact = Image::from_path("storage/photos/hero.jpg")
        .to_format(OutputFormat::WebPLossless)
        .to_bytes()
        .await?;

    Ok(())
}
```

The built-in driver writes `OutputFormat::WebP` lossless, and ignores the
quality, in two cases. In both the lossy form of WebP cannot hold the
image:

- A pixel is not fully opaque. The lossy encoder has no alpha channel,
  so a lossy file would lose the transparency.
- A side is longer than 16383 px, the largest side a lossy frame can
  have.

The ImageMagick driver writes `WebP` lossy at every quality, and keeps
the alpha channel. It writes `WebPLossless` lossless.

A WebP source that you resize and do not convert is written as `WebP`.
An opaque one is therefore written lossy.

## Storage

`from_disk`, `store` and `store_as` work against any registered `Storage`
disk, so a resize-and-restore round trip never touches local paths.

`store(directory, disk)` stores the result in `directory` under a
generated name: 40 random letters and digits and the output format's
extension. `store_as(directory, name, disk)` stores it as
`directory/name`. Both return the path they wrote, with the slashes at
the ends of `directory` and `name` trimmed. `disk` names a disk, and
`None` means the application's default disk (`FILESYSTEM_DISK`):

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copies() -> Result<(String, String), FrameworkError> {
    // For example "web/8f1Qx...Zt2.webp".
    let generated = Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .to_webp()
        .store("web", Some("uploads"))
        .await?;

    // Exactly "web/42.png", on the default disk.
    let named = Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store_as("web", "42.png", None)
        .await?;

    Ok((generated, named))
}
```

There is no `store_publicly`: Suprnova sets visibility per disk, so
storing on a disk with a public base URL is what makes a file public.
See [File Storage](filesystem.md) for registering disks and their URLs.

## Decode limits

Decoding is where hostile input does damage: a few kilobytes can declare
a 40000x40000 canvas and ask a server to allocate six gigabytes for it.
Suprnova refuses that before allocating anything.

| Var | Default | Purpose |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Cap on width and height in pixels |
| `IMAGE_MAX_ALLOC_BYTES` | `1073741824` (1 GiB) | Cap on the memory one decode may allocate, and on the size of the source file itself |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Wall-clock ceiling on one ImageMagick invocation (`magick` driver only) |

The framework parses the input's own header - a few dozen bytes, no
allocation - reads the declared dimensions, and rejects oversized input
before a decoder is constructed. The same caps apply to resize targets,
because `resize(50_000, 50_000)` allocates just as much whether the
numbers came from an attacker or a typo.

A header can also declare a small image over data that asks for far
more, and the default driver bounds that too:

- PNG pixel data that inflates past the size its header declares is
  refused when the inflate reaches that size. A few kilobytes of
  compressed data can expand to gigabytes.
- Only the first frame of an animated GIF is decoded, because the
  pipeline only uses the first frame, and decoding stops the moment that
  frame is complete. A first frame larger than the GIF's logical screen
  is refused before it is decoded.
- A lossless WebP, or a WebP's lossless alpha plane, is read as far as
  its last prefix code before it decodes, and the tables those codes build
  count toward the limit. How many tables there are is written in the
  compressed data, not in a header: 160 KiB of codes can ask for 250 MB
  of tables for a 4x4 image. The read keeps no pixels and holds one
  group's tables at a time, at most about 17 KiB.
- A file or stored source is read no further than
  `IMAGE_MAX_ALLOC_BYTES`, even when the size its storage reports is
  wrong or missing, as it is for a pipe.
- A JPEG's Extended XMP segments are counted before it decodes. The JPEG
  decoder keeps every segment of an unfinished series and re-reads all of
  them after each marker, so the work grows with the square of the
  segment count: 100,000 one-byte segments, about 8 MB, ask for billions
  of comparisons. The default driver counts the bytes those passes would
  read and refuses the JPEG when that is over `IMAGE_MAX_ALLOC_BYTES`. A
  complete series, as cameras and editors write one, is far below it.

### What a decode costs

`IMAGE_MAX_ALLOC_BYTES` is the most memory one decode may allocate, not
only the size of the decoded image. Decoders hold more than the pixels
they return: an inflated PNG next to its unfiltered rows, a progressive
JPEG's coefficients, the canvases a GIF frame is composed on. So the
default driver works out, from the image's headers, how many bytes its
decode will allocate, and refuses the image when that is over the limit.
The refusal names the estimate:

```text
image exceeds configured decode limits: decoding this 8000x6000 image/png
needs about 1923381182 bytes, over the IMAGE_MAX_ALLOC_BYTES limit of 1073741824
```

As a guide, a decode needs about this many times width x height x 4
bytes:

| Format | Times |
|---|---|
| PNG, 8-bit | 1.3 (grey or palette) to 5 (incompressible RGBA) |
| PNG, 16-bit | 2.5 (grey) to 10 (incompressible RGBA). At the 1 GiB default, 16-bit RGBA tops out at about 27 megapixels and 16-bit RGB at about 36 |
| GIF | 1.0 to 1.1 |
| JPEG, sequential (baseline, extended, arithmetic) | 1.0 to 1.1 |
| JPEG, progressive, or one component a scan | 1.5 (grey) to 2.5 (4:4:4) |
| JPEG, lossless | 1.3 (grey) to 4 (RGB) |
| WebP | 1.4 to 2.4 |
| BMP | 1.0 (32-bit) to 1.75 (24-bit) |

An image that decoding turns by its EXIF orientation needs at least 2
times width x height x 4 bytes, the decoded plane and the turned one,
or the table's figure when that is more.

So the default 1 GiB decodes a 48-megapixel photo (8000x6000) in every
8-bit format, a progressive 4:4:4 JPEG and a PNG of incompressible RGBA
included, turned or not. 16-bit PNG holds more and tops out lower, as
the table says.
Raise `IMAGE_MAX_ALLOC_BYTES` if your users upload larger images, or
lower it on a small host.

A limit hit is a 4xx-shaped `FrameworkError::param`, because oversized
input is a client problem, not a server fault.

Out-of-range configuration clamps with a warning rather than failing
boot: `IMAGE_MAX_DIMENSION=0` would reject every image in the
application, which is not what anyone meant to configure.

### One bound is not configurable

A WebP declares its real decoded size in its innermost bitstream chunk,
not in the canvas header, so the framework walks the container to find
it. That walk stops after **4096 chunks per level** and follows nesting
**two levels deep**, and a file that exceeds either is refused outright
rather than measured.

It is refused rather than measured on purpose. Reporting a number from a
walk that did not reach the end of the file would be a gate that a large
enough pile of filler chunks could step around, so an unfinishable walk
has no answer to give.

Neither number is tunable, and no `IMAGE_MAX_*` variable affects them -
the error says so, rather than saying "configured", precisely so nobody
spends an afternoon raising `IMAGE_MAX_ALLOC_BYTES` and watching nothing
change. In practice only a deliberately hostile file gets near it: a
300-frame animation passes comfortably, and a 4100-frame one does not.

## Backends

Like Laravel, the image surface is two drivers, chosen with
`IMAGE_DRIVER`.

| Driver | Value | Needs | Reads |
|---|---|---|---|
| OxideAV | `oxideav` (default) | nothing | PNG, JPEG, WebP, GIF, BMP |
| ImageMagick | `magick` | ImageMagick 7 on the host | whatever the host's delegates provide |

`IMAGE_DRIVER` sets the driver for the whole process. One image can
choose its own with `using`, which changes the driver of that image and
its clones only:

```rust
use suprnova::{FrameworkError, Image, ImageDriverKind};

// HEIC goes to ImageMagick; every other image stays on the default.
async fn thumbnail(upload: Vec<u8>, is_heic: bool) -> Result<Vec<u8>, FrameworkError> {
    let image = Image::from_bytes(upload).cover(320, 320).to_jpg();
    let image = if is_heic {
        image.using(ImageDriverKind::Magick)
    } else {
        image
    };
    image.to_bytes().await
}
```

### `IMAGE_DRIVER=oxideav`

The default. Pure Rust, built on the [OxideAV](https://github.com/OxideAV)
codec family, with [zune-jpeg](https://github.com/etemesi254/zune-image)
decoding JPEG: no native library, nothing to install, nothing to
configure. It is the right choice for almost every application, and it
is what a scaffolded app gets.

It reads 8-bit JPEGs in every coding: baseline, progressive and
arithmetic, greyscale, YCbCr at 4:4:4, 4:2:2, 4:2:0, 4:4:0 or 4:1:1, RGB,
and lossless. CMYK and 12-bit JPEGs need the `magick` driver, and so does a
lossless JPEG of more than 67,108,864 samples (width x height x
components), which its decoder, oxideav-mjpeg, refuses.

### `IMAGE_DRIVER=magick`

Opt-in. Runs a host-installed ImageMagick 7 binary, piping the image in
over stdin and reading the result back over stdout. The driver writes no
temporary file. ImageMagick itself copies its stdin into its own
temporary directory before it decodes, and deletes the copy when it
exits. The binary name comes from `IMAGE_MAGICK_BINARY` and defaults to
`magick`; a missing binary is a clear error at first use, not a silent
fallback.

Choose it when you need input formats the pure-Rust driver does not
carry - HEIC being the common one. The cost is a host dependency: the
operator installs ImageMagick and its delegates, and owns their
licensing. The framework links nothing and compiles nothing native
either way.

Like the default driver, it works on the first frame of an animation
and drops the rest. A GIF's first frame is composed onto the GIF's
logical screen first, so both drivers produce the same image and report
the same size for it.

Arguments are always a fixed array handed straight to the process, never
a shell string, and every numeric argument is formatted from an
already-validated field. A rotation background is a typed `Color` that
the driver formats itself. The arguments that choose which metadata
ImageMagick keeps are fixed strings. A custom transformation adds no
argument: it runs on the pixels between two runs. There is no argument
position user input can reach.

When the framework recognises the input, the decoder is named on the
command line - `png:-` rather than a bare `-`. That matters: given a
bare `-`, ImageMagick picks a coder from the bytes it is handed, so a
file whose magic says MVG or MSL is read as a *script* regardless of
what your application believed it was accepting. Pinning the coder makes
a mislabelled file fail instead of becoming something else.

**Input the framework cannot name still relies on your `policy.xml`.**
Reading those formats is the whole reason this driver exists, so that
path cannot pin a coder. Harden the host's ImageMagick policy - at
minimum disabling the `MVG`, `MSL`, `URL`, `HTTPS`, `EPHEMERAL`, and
`TEXT` coders - if you accept arbitrary uploads under
`IMAGE_DRIVER=magick`.

Decode limits are enforced twice under this driver. For the five formats
the framework can parse, the header check above runs before the process
is spawned. For everything else a pre-parse is impossible, so every
invocation carries ImageMagick's own `-limit` flags derived from the
same configuration, including a wall-clock `-limit time`.

That flag is not the whole story, because ImageMagick enforces it with
its own resource monitor, and a process wedged inside a delegate before
that monitor engages never trips it. So Suprnova also holds its own
deadline: past `IMAGE_MAGICK_TIMEOUT_SECS` (plus a couple of seconds of
grace for IM's own limit to fire first) it kills the process group -
delegates included, not just the process it started - and stops waiting
on the pipes. A stalled delegate therefore cannot pin a worker thread.
Delegates that stay in the process group die with it; one that leaves the
group, or a host with no `kill` binary, can outlive the request - that
residual is what host process supervision is for.

A kill surfaces as a 5xx `FrameworkError::internal`, not a 4xx, even
though a request triggered it. Something wedged the image path badly
enough to need killing, which belongs in server-error monitoring where
an operator will see it - classifying it as a client error would file
away the one condition here worth paging on.

## Custom drivers

`ImageDriver` is the extension point: `&[u8]` in, `Vec<u8>` out, no
codec type crossing the boundary.

```rust
use suprnova::{FrameworkError, ImageDriver, ImagePipeline};

struct MyDriver;

impl ImageDriver for MyDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        // Decode `contents`, replay `pipeline.transformations`, then encode
        // to `pipeline.format` at `pipeline.quality`. Give every
        // `OutputFormat` variant an arm, `WebPLossless` included: the
        // enum is not `#[non_exhaustive]`. `Transformation` is, so its
        // match ends in a wildcard arm that returns an error naming the
        // step. Run a `Transformation::Custom` step with
        // `apply_with(pixels, &pipeline.settings)`, which hands a step
        // recorded with `transform_with` its settings.
        todo!()
    }

    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        todo!()
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        todo!()
    }

    fn name(&self) -> &'static str {
        "mine"
    }
}
```

Install it during `bootstrap()`, before the first image is processed:

```rust
use suprnova::FrameworkError;

pub fn register() -> Result<(), FrameworkError> {
    suprnova::media::set_default_driver(Box::new(MyDriver))
}
```

A conforming driver enforces the configured `ImageConfig` limits before
allocating for a decode. The framework cannot do it on a driver's
behalf, because it never sees the decoded buffer.

`ImageConfig` is `#[non_exhaustive]`: start from
`ImageConfig::default()` or `ImageConfig::from_env()` and set the fields
you change, rather than writing a struct literal.

### Reaching more formats

If the built-in five are not enough, there are three routes, in rough
order of how much you take on:

1. **The built-in `magick` driver.** Set `IMAGE_DRIVER=magick`. Format
   breadth comes from the host's ImageMagick delegates, and there is no
   build dependency to manage.
2. **A custom driver around libvips**, for example via the
   [libvips-rust-bindings](https://github.com/olxgroup-oss/libvips-rust-bindings)
   crate (MIT). libvips is the engine behind Node's `sharp`, with a very
   wide format range - JPEG, JPEG XL, TIFF, PNG, WebP, HEIC, AVIF, PDF,
   SVG, GIF, and more, plus ImageMagick delegation - and strong
   streaming performance. It binds the libvips C library, so your app
   installs libvips at build and run time and owns that dependency,
   which is exactly why it belongs behind the trait rather than in the
   framework. One practical note: the binding's `VipsImage` is not
   thread safe, which the one-image-per-`process()`-call driver shape
   already accommodates.
3. **Any CLI tool**, wrapped the way the `magick` driver is: a fixed
   argument array handed to `std::process::Command`, image bytes over
   stdin and out over stdout, never a shell string.

Suprnova endorses the trait boundary, not any particular dependency
behind it. What sits back there is your call, and so is its licensing.

## Testing

The subsystem needs no fixtures on disk - it is its own fixture factory
once decode and encode round-trip:

```rust
use suprnova::{FrameworkError, Image, OutputFormat};

/// Grow a 1x1 byte-literal fixture into whatever size a test needs.
async fn fixture(source: &[u8]) -> Result<Vec<u8>, FrameworkError> {
    Image::from_bytes(source.to_vec())
        .resize(4, 2)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
}
```

Tests that tighten the decode limits must be serialised: the limits are
process-global, so a parallel sibling would decode under the tightened
cap.

### Why Suprnova diverges

**No HEIC in the default driver, and the reason is patents.** HEVC, the
codec inside HEIC, is patent-encumbered - the Access Advance pool among
others. Suprnova installs no native libraries, so a built-in decoder
would have to be pure Rust and would carry that exposure directly, and
the one credible pure-Rust decoder is dual AGPL-3.0/commercial, which is
a per-application legal obligation rather than something an MIT
framework gets to default anybody into.

Both frameworks make HEIC a host-provisioning concern; Suprnova's
version just has one fewer moving part. Laravel's default driver, GD,
cannot read HEIC at all, and its Imagick path needs the libheif delegate
compiled into **both** the system ImageMagick binary and the PHP
`imagick` extension. In Suprnova the default driver does not read HEIC,
and `IMAGE_DRIVER=magick` reads it whenever the host's ImageMagick
carries the libheif delegate - no extension layer in between. So HEIC
ingestion works: install ImageMagick with libheif through your
package manager, and send HEIC uploads to it with
`using(ImageDriverKind::Magick)`, or set `IMAGE_DRIVER=magick` for every
image. The licensing sits where it belongs, with the host.

When the `oxideav` driver meets a HEIC file it says so by name, points
at this chapter, and names both ways forward, rather than returning a
generic "unsupported format".

**No base64 or URL constructors.** Laravel's `ImageManager` has
`->read($base64)` and `->read($url)`. `from_bytes` composes with
whatever produced the bytes, including the [HTTP client](http-client.md),
and keeping a URL fetch out of the image subsystem keeps its timeouts,
retries, and SSRF policy in one place instead of two.

**`from_stream` is eager, with a cap.** Laravel's contents are a lazy
closure. A stream cannot be replayed, so this one is drained at
construction, counting bytes against `IMAGE_MAX_ALLOC_BYTES` as it goes.

**`contain` does not pad.** It fits the image inside the box and stops
there; it does not letterbox onto a background. Compose it with a
background yourself if you need one.

**Metadata is stripped by default.** Laravel's Imagick driver keeps
every profile, the GPS position included, and its GD driver drops all of
them, the colour profile included. Suprnova keeps the ICC profile and
drops the rest, in both drivers, so a stored upload neither leaks where
it was taken nor shifts colour. Laravel also returns an upload's original
bytes when the pipeline changes nothing; Suprnova always re-encodes, so
every output is stripped.

**Rotation keeps transparency where the format holds it.** Laravel fills
exposed corners with white in every format. Suprnova fills them with
white for JPEG and GIF and leaves them transparent for PNG, WebP and BMP,
unless you name a colour.

**A custom transformation is a pixel function.** Laravel's
`transformUsing` adds a handler to one driver, written against that
driver's library. Suprnova's `register_transformation` takes a function
over decoded RGBA that both drivers run, so changing `IMAGE_DRIVER` never
drops a step, and no custom code reaches an ImageMagick argument.
Laravel's handler receives the transformation object with its fields;
here those fields are the settings `transform_with` records, which the
image holds so that a step stays plain `Copy` data.

**No `store_publicly`.** Visibility belongs to the disk in Suprnova, so
`store` and `store_as` on a public disk are the public variants.

**Resize uses bilinear resampling.** The backend's filter set ships
nearest-neighbour and bilinear; bilinear is its documented default for
natural images.

**Images are never serialisable.** Laravel throws on `__serialize` and
Suprnova simply does not implement it. Store the path or the disk key
and rebuild the pipeline.

## Next

- [File Storage](filesystem.md) for the disks `from_disk` and `store` read and write.
- [HTTP Responses](responses.md) for what `to_response()` hands back.
- [Environment Variables](env-vars.md) for the full list of image settings.
