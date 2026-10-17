//! The first frame of a GIF, decoded by the framework.
//!
//! The pipeline uses a GIF's first frame and nothing else. A GIF decoder from
//! a codec crate keeps reading the frame's LZW codes after the frame is
//! complete: an encoder may write extra codes, so a decoder reads to the
//! End-of-Information code and discards what lands past the frame. Each of
//! those codes still costs a walk of its dictionary entry, up to 4096 steps,
//! so a few kilobytes of trailing codes cost billions of steps for pixels
//! nobody keeps. Decoding here stops reading the moment the frame is
//! complete, and writes each pixel straight onto the screen-sized canvas,
//! with no index buffer and no copy of the compressed data.
//!
//! The format is the GIF89a specification's: variable-width LZW codes packed
//! least-significant bit first into length-prefixed data sub-blocks, the code
//! width growing by one bit when the dictionary fills the current width, and
//! up to 4096 entries. The frame is placed at its offset on the logical
//! screen, which is otherwise transparent, as browsers show it.

use crate::error::FrameworkError;
use crate::media::sniff::GifFrame;

use super::Canvas;

/// The largest LZW dictionary GIF allows: twelve-bit codes.
const MAX_CODES: usize = 4096;

/// A dictionary entry with no prefix: one of the single-byte literal codes.
const NO_PREFIX: u16 = u16::MAX;

/// What the decoder allocates besides the canvas: the dictionary, as a
/// prefix code (`u16`), a final byte, a length (`u16`) and a first byte per
/// entry.
pub(super) const DICTIONARY_BYTES: u64 = (MAX_CODES * (2 + 1 + 2 + 1)) as u64;

fn malformed(detail: &str) -> FrameworkError {
    FrameworkError::param(format!("image decode failed: image/gif: {detail}"))
}

/// Bits from a run of GIF data sub-blocks, least-significant bit first.
struct SubBlockBits<'a> {
    bytes: &'a [u8],
    /// Next byte to read: a length byte when `left` is zero, data otherwise.
    pos: usize,
    /// Data bytes left in the current sub-block.
    left: usize,
    /// The zero-length terminator was read.
    ended: bool,
    bits: u32,
    count: u32,
}

impl<'a> SubBlockBits<'a> {
    fn new(bytes: &'a [u8], start: usize) -> Self {
        Self {
            bytes,
            pos: start,
            left: 0,
            ended: false,
            bits: 0,
            count: 0,
        }
    }

    fn next_byte(&mut self) -> Option<u8> {
        while self.left == 0 {
            if self.ended {
                return None;
            }
            let length = *self.bytes.get(self.pos)?;
            self.pos += 1;
            if length == 0 {
                self.ended = true;
                return None;
            }
            self.left = usize::from(length);
        }
        let byte = *self.bytes.get(self.pos)?;
        self.pos += 1;
        self.left -= 1;
        Some(byte)
    }

    /// The next `width`-bit code, or `None` when the data runs out.
    fn read(&mut self, width: u32) -> Option<u16> {
        while self.count < width {
            let byte = self.next_byte()?;
            self.bits |= u32::from(byte) << self.count;
            self.count += 8;
        }
        let code = (self.bits & ((1 << width) - 1)) as u16;
        self.bits >>= width;
        self.count -= width;
        Some(code)
    }
}

/// The screen row a stored row lands on in the four-pass interlaced order:
/// every eighth row from 0, every eighth from 4, every fourth from 2, then
/// every second from 1.
fn interlaced_row(stored: usize, height: usize) -> usize {
    let first = height.div_ceil(8);
    if stored < first {
        return stored * 8;
    }
    let stored = stored - first;
    let second = (height + 3) / 8;
    if stored < second {
        return 4 + stored * 8;
    }
    let stored = stored - second;
    let third = (height + 1) / 4;
    if stored < third {
        return 2 + stored * 4;
    }
    1 + (stored - third) * 2
}

/// Where the frame's pixels land: the canvas, the color table, and the
/// frame's placement.
struct Placement<'a> {
    pixels: Vec<u8>,
    palette: &'a [u8],
    transparent: Option<u8>,
    screen_width: usize,
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    interlaced: bool,
}

impl Placement<'_> {
    /// Paint the pixel at `index` in the frame's stored order.
    fn put(&mut self, index: usize, value: u8) -> Result<(), FrameworkError> {
        if Some(value) == self.transparent {
            return Ok(());
        }
        let color = self
            .palette
            .get(usize::from(value) * 3..usize::from(value) * 3 + 3)
            .ok_or_else(|| malformed("a pixel indexes past the color table"))?;
        let stored_row = index / self.width;
        let row = if self.interlaced {
            interlaced_row(stored_row, self.height)
        } else {
            stored_row
        };
        let at = ((self.top + row) * self.screen_width + self.left + index % self.width) * 4;
        self.pixels[at..at + 4].copy_from_slice(&[color[0], color[1], color[2], u8::MAX]);
        Ok(())
    }
}

/// Decode the frame `walk` found and place it on a `screen_width x
/// screen_height` canvas.
///
/// `walk` comes from [`sniff::gif_first_frame`](crate::media::sniff), and the
/// caller has checked that the frame fits the screen.
pub(super) fn decode_first_frame(
    bytes: &[u8],
    walk: &GifFrame,
    screen_width: u32,
    screen_height: u32,
) -> Result<Canvas, FrameworkError> {
    let (palette_at, entries) = walk
        .palette
        .ok_or_else(|| malformed("the first frame has no color table"))?;
    let palette = bytes
        .get(palette_at..palette_at + entries * 3)
        .ok_or_else(|| malformed("the color table runs past the end of the stream"))?;
    let (screen_width, screen_height) = (screen_width as usize, screen_height as usize);
    let mut placement = Placement {
        pixels: vec![0u8; screen_width * screen_height * 4],
        palette,
        transparent: walk.transparent,
        screen_width,
        left: usize::from(walk.left),
        top: usize::from(walk.top),
        width: usize::from(walk.width),
        height: usize::from(walk.height),
        interlaced: walk.interlaced,
    };
    let expected = placement.width * placement.height;
    if expected > 0 {
        decode_lzw(bytes, walk.data, expected, &mut placement)?;
    }
    Canvas::packed(screen_width as u32, screen_height as u32, placement.pixels)
}

/// Decode LZW codes until `expected` pixels are placed.
fn decode_lzw(
    bytes: &[u8],
    data: usize,
    expected: usize,
    placement: &mut Placement<'_>,
) -> Result<(), FrameworkError> {
    let minimum = u32::from(*bytes.get(data).ok_or_else(|| malformed("no LZW data"))?);
    if !(2..=8).contains(&minimum) {
        return Err(malformed(&format!(
            "LZW minimum code size {minimum} is outside 2 to 8"
        )));
    }
    let clear = 1u16 << minimum;
    let end_of_information = clear + 1;
    let mut prefix = vec![NO_PREFIX; MAX_CODES];
    let mut suffix = vec![0u8; MAX_CODES];
    let mut length = vec![0u16; MAX_CODES];
    let mut first = vec![0u8; MAX_CODES];
    for literal in 0..clear {
        suffix[usize::from(literal)] = literal as u8;
        first[usize::from(literal)] = literal as u8;
        length[usize::from(literal)] = 1;
    }

    let mut bits = SubBlockBits::new(bytes, data + 1);
    let mut next = clear + 2;
    let mut width = minimum + 1;
    let mut previous: Option<u16> = None;
    let mut emitted = 0usize;
    while emitted < expected {
        let code = bits
            .read(width)
            .ok_or_else(|| malformed("the LZW data ends before the frame is complete"))?;
        if code == clear {
            next = clear + 2;
            width = minimum + 1;
            previous = None;
            continue;
        }
        if code == end_of_information {
            return Err(malformed("the LZW data ends before the frame is complete"));
        }
        // The entry's last byte and the code its earlier bytes come from. A
        // code one past the dictionary is the entry being defined: the
        // previous entry plus its own first byte.
        let (entry_length, head, last, chain) = if code < next {
            let entry = usize::from(code);
            (
                usize::from(length[entry]),
                first[entry],
                suffix[entry],
                prefix[entry],
            )
        } else if code == next {
            let earlier = previous.ok_or_else(|| malformed("an LZW code refers to no entry"))?;
            let entry = usize::from(earlier);
            (
                usize::from(length[entry]) + 1,
                first[entry],
                first[entry],
                earlier,
            )
        } else {
            return Err(malformed("an LZW code is past the dictionary"));
        };

        // Write the entry back to front, keeping only what lands inside the
        // frame.
        let mut position = emitted + entry_length - 1;
        if position < expected {
            placement.put(position, last)?;
        }
        let mut walk = chain;
        while walk != NO_PREFIX {
            position -= 1;
            if position < expected {
                placement.put(position, suffix[usize::from(walk)])?;
            }
            walk = prefix[usize::from(walk)];
        }
        emitted += entry_length;

        if let Some(earlier) = previous
            && usize::from(next) < MAX_CODES
        {
            let entry = usize::from(next);
            prefix[entry] = earlier;
            suffix[entry] = head;
            length[entry] = length[usize::from(earlier)] + 1;
            first[entry] = first[usize::from(earlier)];
            next += 1;
            if usize::from(next) == 1 << width && width < 12 {
                width += 1;
            }
        }
        previous = Some(code);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interlaced_rows_follow_the_four_passes() {
        let order: Vec<usize> = (0..10).map(|stored| interlaced_row(stored, 10)).collect();
        assert_eq!(order, [0, 8, 4, 2, 6, 1, 3, 5, 7, 9]);
        let short: Vec<usize> = (0..3).map(|stored| interlaced_row(stored, 3)).collect();
        assert_eq!(short, [0, 2, 1]);
    }
}
