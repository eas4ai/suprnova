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
| `scale(w, h)` | Fit inside the box, preserving aspect ratio. **Never enlarges** |
| `scale_width(w)` / `scale_height(h)` | Scale down to at most one dimension. Never enlarges |
| `crop(w, h, x, y)` | Cut a rectangle out. Errors if it falls outside the image |
| `cover(w, h)` | Fill the box exactly, cropping the overflow from the centre |
| `contain(w, h)` | Fit inside the box, preserving aspect ratio. No padding |
| `rotate(degrees)` | Rotate clockwise by any angle, growing the canvas to fit |
| `flip_vertically()` / `flip_horizontally()` | Laravel's `flip` and `flop` |
| `blur(amount)` | Gaussian blur, `0..=100`. `0` is a no-op |
| `sharpen(amount)` | Unsharp mask, `0..=100`. `0` is a no-op. `50` is the classic strength |
| `grayscale()` | Desaturate. Spelled the Laravel way |
| `to_format(format)` | Choose the output container |
| `quality(q)` | Encode quality, clamped to `1..=100`, default `70` |

Values that would be nonsense are clamped rather than rejected:
`blur(500)` records `100`, `quality(0)` records `1`. A crop that falls
outside the image is a real error, not a clamp, because silently moving
someone's crop box is worse than telling them.

`rotate` takes arbitrary angles. A 90-degree multiple takes an exact
axis-aligned path with no resampling; anything else is bilinear, and the
canvas grows so no pixel is clipped. The exposed corners are transparent
where the output format has an alpha channel.

## Terminals

Every terminal is `async`, consumes the `Image`, and runs the decode,
transform, and encode work on a blocking thread so it never stalls the
runtime. Source I/O happens before that hop, so a slow disk never
occupies a blocking worker.

| Terminal | Returns |
|---|---|
| `to_bytes()` | `Vec<u8>` of the encoded file |
| `to_response()` | An `HttpResponse` with the right `Content-Type` |
| `save(path)` | Writes to the filesystem |
| `store(disk, path)` | Writes to a `Storage` disk |
| `dimensions()` | `(width, height)` of the **processed** image |
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

Five formats are read and written today: **PNG, JPEG, WebP, GIF, and
BMP**.

| Format | Reads | Writes | Quality knob |
|---|---|---|---|
| PNG | yes | yes | ignored (lossless) |
| JPEG | yes | yes | honoured |
| WebP | yes | yes (lossless) | no effect today |
| GIF | yes | yes | ignored (palette) |
| BMP | yes | yes | ignored (lossless) |

AVIF is neither read nor written yet. The in-house AV1 encoder it
depends on has not published, and shipping an `OutputFormat::Avif` that
always failed would be a promise the framework could not keep. It
arrives with that publish, as a new enum variant and nothing else.

GIF output is palette-quantised to at most 256 colours with
Floyd-Steinberg dithering before encoding, so a photographic source
converts cleanly rather than erroring.

WebP is written losslessly, so `quality()` currently has no effect on
WebP output. Use JPEG when you need a size/quality dial.

## Storage

`from_disk` and `store` work against any registered `Storage` disk, so a
resize-and-restore round trip never touches local paths:

```rust
use suprnova::{FrameworkError, Image};

async fn make_web_copy() -> Result<(), FrameworkError> {
    Image::from_disk("uploads", "originals/42.png")
        .scale(1024, 1024)
        .store("uploads", "web/42.png")
        .await
}
```

See [File Storage](filesystem.md) for registering disks.

## Decode limits

Decoding is where hostile input does damage: a few kilobytes can declare
a 40000x40000 canvas and ask a server to allocate six gigabytes for it.
Suprnova refuses that before allocating anything.

| Var | Default | Purpose |
|---|---|---|
| `IMAGE_MAX_DIMENSION` | `16384` | Cap on width and height in pixels |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456` (256 MiB) | Cap on the decoded RGBA footprint, and on the size of the source file itself |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | Wall-clock ceiling on one ImageMagick invocation (`magick` driver only) |

The framework parses the input's own header - a few dozen bytes, no
allocation - reads the declared dimensions, and rejects oversized input
before a decoder is constructed. The same caps apply to resize targets,
because `resize(50_000, 50_000)` allocates just as much whether the
numbers came from an attacker or a typo.

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

### `IMAGE_DRIVER=oxideav`

The default. Pure Rust, built on the [OxideAV](https://github.com/OxideAV)
codec family: no native library, nothing to install, nothing to
configure. It is the right choice for almost every application, and it
is what a scaffolded app gets.

### `IMAGE_DRIVER=magick`

Opt-in. Runs a host-installed ImageMagick 7 binary, piping the image in
over stdin and reading the result back over stdout - no temp files. The
binary name comes from `IMAGE_MAGICK_BINARY` and defaults to `magick`;
a missing binary is a clear error at first use, not a silent fallback.

Choose it when you need input formats the pure-Rust driver does not
carry - HEIC being the common one. The cost is a host dependency: the
operator installs ImageMagick and its delegates, and owns their
licensing. The framework links nothing and compiles nothing native
either way.

Arguments are always a fixed array handed straight to the process, never
a shell string, and every numeric argument is formatted from an
already-validated field. There is no argument position user input can
reach.

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
        // to `pipeline.format` at `pipeline.quality`.
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
ingestion works today: install ImageMagick with libheif through your
package manager and flip the env var. The licensing sits where it
belongs, with the host.

When the `oxideav` driver meets a HEIC file it says so by name, points
at this chapter, and names both ways forward, rather than returning a
generic "unsupported format".

**AVIF is pending, not skipped.** It is royalty-free and it is the
modern-format answer we want; the in-house AV1 encoder simply has not
published yet. WebP is the modern-format path in the meantime.

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
