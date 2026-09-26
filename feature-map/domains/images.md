# Feature map: `manual/images.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 58 checked.

## Rust API: suprnova

### `suprnova::media::driver` (private module; items are public through re-exports)

- [ ] struct `suprnova::ImagePipeline` · framework/src/media/driver.rs:141 (feature: `media`; also `suprnova::media::ImagePipeline`)
  - Public fields: `transformations`, `format`, `quality`
- [ ] enum `suprnova::OutputFormat` · framework/src/media/driver.rs:30 (feature: `media`; also `suprnova::media::OutputFormat`)
  - Variants: `Jpeg`, `Png`, `WebP`, `Gif`, `Bmp`
  - [ ] fn `suprnova::OutputFormat::mime_type` · framework/src/media/driver.rs:49
  - [ ] fn `suprnova::OutputFormat::extension` · framework/src/media/driver.rs:60
- [ ] enum `suprnova::Transformation` · framework/src/media/driver.rs:77 (feature: `media`; also `suprnova::media::Transformation`)
  - Variants: `Resize`, `ResizeWidth`, `ResizeHeight`, `Scale`, `ScaleWidth`, `ScaleHeight`, `Crop`, `Cover`, `Contain`, `Rotate`, `FlipVertically`, `FlipHorizontally`, `Blur`, `Sharpen`, `Grayscale`
- [ ] trait `suprnova::ImageDriver` · framework/src/media/driver.rs:184 (feature: `media`; also `suprnova::media::ImageDriver`)
  - Implemented here by: `MagickCliDriver`, `OxideAvImageDriver`
  - [ ] fn `suprnova::ImageDriver::process` · framework/src/media/driver.rs:188 (required)
  - [ ] fn `suprnova::ImageDriver::dimensions` · framework/src/media/driver.rs:195 (required)
  - [ ] fn `suprnova::ImageDriver::dominant_color` · framework/src/media/driver.rs:200 (required)
  - [ ] fn `suprnova::ImageDriver::name` · framework/src/media/driver.rs:203 (required)
- [ ] const `suprnova::DEFAULT_IMAGE_QUALITY` · framework/src/media/driver.rs:21 (feature: `media`; also `suprnova::media::DEFAULT_IMAGE_QUALITY`)

### `suprnova::media::magick` (private module; items are public through re-exports)

- [ ] struct `suprnova::MagickCliDriver` · framework/src/media/magick.rs:80 (feature: `media`; also `suprnova::media::MagickCliDriver`)
  - Implements: `suprnova::ImageDriver`
  - [ ] fn `suprnova::MagickCliDriver::new` · framework/src/media/magick.rs:86
  - [ ] fn `suprnova::MagickCliDriver::from_env` · framework/src/media/magick.rs:93
  - [ ] fn `suprnova::MagickCliDriver::binary` · framework/src/media/magick.rs:102

### `suprnova::media::oxideav` (private module; items are public through re-exports)

- [ ] struct `suprnova::OxideAvImageDriver` · framework/src/media/oxideav.rs:172 (feature: `media`; also `suprnova::media::OxideAvImageDriver`)
  - Implements: `suprnova::ImageDriver`
  - [ ] fn `suprnova::OxideAvImageDriver::new` · framework/src/media/oxideav.rs:182

### `suprnova::media` (feature: `media`)

- [ ] fn `suprnova::media::config` · framework/src/media/mod.rs:237
- [ ] fn `suprnova::media::default_driver` · framework/src/media/mod.rs:263
- [ ] fn `suprnova::media::set_default_driver` · framework/src/media/mod.rs:289
- [ ] struct `suprnova::Image` · framework/src/media/mod.rs:323 (also `suprnova::media::Image`)
  - [ ] fn `suprnova::Image::from_bytes` · framework/src/media/mod.rs:344
  - [ ] fn `suprnova::Image::from_path` · framework/src/media/mod.rs:349
  - [ ] fn `suprnova::Image::from_disk` · framework/src/media/mod.rs:356
  - [ ] fn `suprnova::Image::from_upload` · framework/src/media/mod.rs:368
  - [ ] fn `suprnova::Image::from_stream` · framework/src/media/mod.rs:383
  - [ ] fn `suprnova::Image::resize` · framework/src/media/mod.rs:411
  - [ ] fn `suprnova::Image::resize_width` · framework/src/media/mod.rs:416
  - [ ] fn `suprnova::Image::resize_height` · framework/src/media/mod.rs:421
  - [ ] fn `suprnova::Image::scale` · framework/src/media/mod.rs:426
  - [ ] fn `suprnova::Image::scale_width` · framework/src/media/mod.rs:431
  - [ ] fn `suprnova::Image::scale_height` · framework/src/media/mod.rs:436
  - [ ] fn `suprnova::Image::crop` · framework/src/media/mod.rs:441
  - [ ] fn `suprnova::Image::cover` · framework/src/media/mod.rs:451
  - [ ] fn `suprnova::Image::contain` · framework/src/media/mod.rs:456
  - [ ] fn `suprnova::Image::rotate` · framework/src/media/mod.rs:461
  - [ ] fn `suprnova::Image::flip_vertically` · framework/src/media/mod.rs:466
  - [ ] fn `suprnova::Image::flip_horizontally` · framework/src/media/mod.rs:471
  - [ ] fn `suprnova::Image::blur` · framework/src/media/mod.rs:476
  - [ ] fn `suprnova::Image::sharpen` · framework/src/media/mod.rs:481
  - [ ] fn `suprnova::Image::grayscale` · framework/src/media/mod.rs:486
  - [ ] fn `suprnova::Image::to_format` · framework/src/media/mod.rs:491
  - [ ] fn `suprnova::Image::quality` · framework/src/media/mod.rs:499
  - [ ] fn `suprnova::Image::to_bytes` · framework/src/media/mod.rs:526
  - [ ] fn `suprnova::Image::to_response` · framework/src/media/mod.rs:533
  - [ ] fn `suprnova::Image::save` · framework/src/media/mod.rs:544
  - [ ] fn `suprnova::Image::store` · framework/src/media/mod.rs:553
  - [ ] fn `suprnova::Image::dimensions` · framework/src/media/mod.rs:560
  - [ ] fn `suprnova::Image::mime_type` · framework/src/media/mod.rs:572
  - [ ] fn `suprnova::Image::dominant_color` · framework/src/media/mod.rs:582
- [ ] struct `suprnova::ImageConfig` · framework/src/media/mod.rs:135 (also `suprnova::media::ImageConfig`)
  - Public fields: `max_dimension`, `max_alloc_bytes`, `magick_timeout_secs`
  - [ ] fn `suprnova::ImageConfig::from_env` · framework/src/media/mod.rs:205
- [ ] enum `suprnova::ImageDriverKind` · framework/src/media/mod.rs:89 (also `suprnova::media::ImageDriverKind`)
  - Variants: `OxideAv`, `Magick`
  - [ ] fn `suprnova::ImageDriverKind::parse` · framework/src/media/mod.rs:103
  - [ ] fn `suprnova::ImageDriverKind::from_env` · framework/src/media/mod.rs:114
- [ ] const `suprnova::DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS` · framework/src/media/mod.rs:85 (also `suprnova::media::DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS`)
- [ ] const `suprnova::DEFAULT_IMAGE_MAX_ALLOC_BYTES` · framework/src/media/mod.rs:78 (also `suprnova::media::DEFAULT_IMAGE_MAX_ALLOC_BYTES`)
- [ ] const `suprnova::DEFAULT_IMAGE_MAX_DIMENSION` · framework/src/media/mod.rs:75 (also `suprnova::media::DEFAULT_IMAGE_MAX_DIMENSION`)
