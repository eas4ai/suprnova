//! The background colour a rotation fills its exposed corners with.
//!
//! A typed value rather than a string on purpose. The `magick` driver writes
//! the colour into ImageMagick's arguments, and the module's safety promise
//! is that no text a caller supplies ever reaches an argument position. A
//! [`Color`] holds four bytes, and the driver formats them itself
//! (`#rrggbbaa`), so whatever string an application parsed the colour from
//! never reaches the command line.

use crate::error::FrameworkError;

/// An 8-bit RGBA colour, for the corners a rotation exposes and the
/// background transparency is flattened onto.
///
/// Build one with [`Color::rgb`], [`Color::rgba`], one of the constants, or
/// [`Color::from_hex`] for a value that arrives as text, such as a form
/// field. Alpha `255` is opaque and `0` fully transparent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl Color {
    /// Opaque white, the default background for JPEG and GIF output, which
    /// cannot hold transparency.
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Fully transparent, the default background for PNG, WebP and BMP
    /// output, which can.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    /// An opaque colour.
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, u8::MAX)
    }

    /// A colour with an alpha channel, `255` opaque and `0` transparent.
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Parse a hex colour: `rgb`, `rrggbb` or `rrggbbaa`, with or without a
    /// leading `#`, in either case.
    ///
    /// Laravel's `rotate` takes its background as a string. This is where
    /// such a string becomes a value: it fails here, at the call that has the
    /// text, rather than inside a driver.
    pub fn from_hex(value: &str) -> Result<Self, FrameworkError> {
        let invalid = || {
            FrameworkError::param(format!(
                "image colour {value:?} is not a hex colour; expected rgb, rrggbb or rrggbbaa"
            ))
        };
        let digits = value.trim().strip_prefix('#').unwrap_or(value.trim());
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let pair = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|_| invalid());
        let single = |at: usize| {
            u8::from_str_radix(&digits[at..at + 1], 16)
                .map(|nibble| nibble * 17)
                .map_err(|_| invalid())
        };
        match digits.len() {
            3 => Ok(Self::rgb(single(0)?, single(1)?, single(2)?)),
            6 => Ok(Self::rgb(pair(0)?, pair(2)?, pair(4)?)),
            8 => Ok(Self::rgba(pair(0)?, pair(2)?, pair(4)?, pair(6)?)),
            _ => Err(invalid()),
        }
    }

    /// The red channel.
    pub const fn red(self) -> u8 {
        self.red
    }

    /// The green channel.
    pub const fn green(self) -> u8 {
        self.green
    }

    /// The blue channel.
    pub const fn blue(self) -> u8 {
        self.blue
    }

    /// The alpha channel: `255` opaque, `0` fully transparent.
    pub const fn alpha(self) -> u8 {
        self.alpha
    }

    /// The four channels in RGBA order.
    pub const fn to_rgba(self) -> [u8; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }

    /// `#rrggbbaa`, the form the `magick` driver writes into an argument.
    pub fn to_hex(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            self.red, self.green, self.blue, self.alpha
        )
    }

    /// This colour composited over opaque white: the opaque colour a
    /// flatten paints where this colour is not fully opaque.
    ///
    /// A JPEG or GIF cannot hold transparency, so the pipeline flattens onto
    /// the rotation's background. That background can itself be translucent
    /// (or [`Color::TRANSPARENT`]), so it is first put over white, Laravel's
    /// own flatten colour.
    pub(crate) fn over_white(self) -> Color {
        let alpha = u16::from(self.alpha);
        let channel = |value: u8| -> u8 {
            let blended = u16::from(value) * alpha + 255 * (255 - alpha);
            // Round to nearest: `(x + 127) / 255` for x in 0..=65025.
            ((blended + 127) / 255) as u8
        };
        Color::rgb(channel(self.red), channel(self.green), channel(self.blue))
    }
}

/// Composite every pixel of a packed RGBA buffer over the opaque `background`
/// and make it opaque, in place.
///
/// The JPEG and GIF encoders keep no alpha, so a transparent pixel would
/// otherwise come out as whatever its colour channels hold, usually black.
pub(crate) fn flatten_rgba(pixels: &mut [u8], background: Color) {
    let background = background.over_white();
    let base = [background.red, background.green, background.blue];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for (channel, under) in pixel[..3].iter_mut().zip(base) {
            let blended = u16::from(*channel) * alpha + u16::from(under) * (255 - alpha);
            *channel = ((blended + 127) / 255) as u8;
        }
        pixel[3] = u8::MAX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn img_006_hex_colours_parse_in_every_documented_form() {
        assert_eq!(Color::from_hex("#fff").unwrap(), Color::WHITE);
        assert_eq!(Color::from_hex("FF0000").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(
            Color::from_hex("#11223344").unwrap(),
            Color::rgba(0x11, 0x22, 0x33, 0x44)
        );
        assert_eq!(Color::from_hex(" #000 ").unwrap(), Color::BLACK);
        for bad in ["", "#", "#12", "#1234", "#gggggg", "red", "#ffffff;rm"] {
            assert!(Color::from_hex(bad).is_err(), "{bad:?} must not parse");
        }
    }

    #[test]
    fn img_006_the_driver_writes_the_colour_from_its_bytes() {
        // What reaches an ImageMagick argument is this formatting of the
        // four bytes, whatever text the colour was parsed from.
        let parsed = Color::from_hex("#ABC").unwrap();
        assert_eq!(parsed.to_hex(), "#aabbccff");
        assert_eq!(Color::TRANSPARENT.to_hex(), "#00000000");
    }

    #[test]
    fn img_006_flatten_composites_over_the_background_put_over_white() {
        let mut pixels = vec![
            255, 0, 0, 255, // opaque red stays red
            10, 20, 30, 0, // transparent takes the background
            0, 0, 0, 128, // half black over white is mid grey
        ];
        flatten_rgba(&mut pixels, Color::TRANSPARENT);
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[255, 255, 255, 255]);
        assert_eq!(&pixels[8..12], &[127, 127, 127, 255]);

        let mut onto_blue = vec![1, 2, 3, 0];
        flatten_rgba(&mut onto_blue, Color::rgb(0, 0, 255));
        assert_eq!(onto_blue, [0, 0, 255, 255]);
    }
}
