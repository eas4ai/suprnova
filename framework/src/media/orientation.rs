//! The EXIF `Orientation` tag, and applying it to packed RGBA.
//!
//! Cameras store the sensor's pixels as they were read and record in the
//! tag how to turn them for display. Laravel's drivers apply it on decode
//! (Intervention's `autoOrientation`), and so do both drivers here unless
//! `IMAGE_AUTO_ORIENT` turns it off. All eight orientations are exact pixel
//! permutations (quarter turns and mirrors), so applying one never
//! resamples.

/// One of the eight EXIF orientations, by its tag value `1..=8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Orientation(u8);

impl Orientation {
    /// The tag value, if it is one EXIF defines.
    pub(crate) fn from_tag(value: u8) -> Option<Self> {
        (1..=8).contains(&value).then_some(Self(value))
    }

    /// Read the tag from TIFF-structured EXIF bytes (`II*\0` or `MM\0*`
    /// first, no `Exif\0\0` prefix), as `image` parses it. Reads in place:
    /// no allocation, and an IFD whose entries run past the data ends the
    /// search rather than erroring.
    pub(crate) fn from_tiff(tiff: &[u8]) -> Option<Self> {
        let parsed = image::metadata::Orientation::from_exif_chunk(tiff)?;
        Self::from_tag(parsed.to_exif())
    }

    /// The tag value, `1..=8`.
    pub(crate) fn tag(self) -> u8 {
        self.0
    }

    /// Whether applying the orientation leaves the pixels as they are.
    pub(crate) fn is_identity(self) -> bool {
        self.0 == 1
    }

    /// Whether applying it swaps width and height (5 to 8 turn a quarter).
    pub(crate) fn swaps_axes(self) -> bool {
        self.0 >= 5
    }

    /// The source position whose pixel lands at `(x, y)` of the oriented
    /// image, for a source of `width x height`.
    fn source_of(self, x: usize, y: usize, width: usize, height: usize) -> (usize, usize) {
        match self.0 {
            // Mirrored left to right.
            2 => (width - 1 - x, y),
            // Turned half way.
            3 => (width - 1 - x, height - 1 - y),
            // Mirrored top to bottom.
            4 => (x, height - 1 - y),
            // Transposed: mirrored about the main diagonal.
            5 => (y, x),
            // Turned a quarter clockwise to display.
            6 => (y, height - 1 - x),
            // Transversed: mirrored about the other diagonal.
            7 => (width - 1 - y, height - 1 - x),
            // Turned a quarter counter-clockwise to display.
            8 => (width - 1 - y, x),
            _ => (x, y),
        }
    }

    /// Apply the orientation to packed RGBA of `width x height`, returning
    /// the oriented pixels and their size.
    ///
    /// One new buffer of the same size: every orientation but the identity
    /// moves every pixel, and a quarter turn cannot be done in place. That
    /// buffer is the full-size copy the decode estimate counts.
    pub(crate) fn apply(self, pixels: Vec<u8>, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
        if self.is_identity() {
            return (pixels, width, height);
        }
        let (source_width, source_height) = (width as usize, height as usize);
        let (out_width, out_height) = if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        };
        let mut out = vec![0u8; pixels.len()];
        for (index, target) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (index % out_width as usize, index / out_width as usize);
            let (sx, sy) = self.source_of(x, y, source_width, source_height);
            let at = (sy * source_width + sx) * 4;
            if let Some(pixel) = pixels.get(at..at + 4) {
                target.copy_from_slice(pixel);
            }
        }
        (out, out_width, out_height)
    }
}

/// The Orientation-only EXIF the output carries when orientation was not
/// applied: a big-endian TIFF header and one IFD holding the one tag.
///
/// Built rather than copied, because the source's EXIF holds the GPS
/// position and the rest of what IMG-002 strips; a viewer that honours the
/// tag needs nothing else.
pub(crate) fn orientation_only_exif(orientation: Orientation) -> [u8; 26] {
    let mut tiff = [0u8; 26];
    // Byte order, magic, offset of the first IFD.
    tiff[..8].copy_from_slice(&[b'M', b'M', 0, 42, 0, 0, 0, 8]);
    // One entry.
    tiff[8..10].copy_from_slice(&1u16.to_be_bytes());
    // Tag 0x0112 (Orientation), type 3 (SHORT), count 1, the value padded
    // to four bytes.
    tiff[10..12].copy_from_slice(&0x0112u16.to_be_bytes());
    tiff[12..14].copy_from_slice(&3u16.to_be_bytes());
    tiff[14..18].copy_from_slice(&1u32.to_be_bytes());
    tiff[18..20].copy_from_slice(&u16::from(orientation.tag()).to_be_bytes());
    // No next IFD (tiff[22..26] stays zero).
    tiff
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 image whose every pixel is its own index.
    fn indexed(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|index| [index as u8, 0, 0, 255])
            .collect()
    }

    fn red_channel(pixels: &[u8]) -> Vec<u8> {
        pixels.as_chunks::<4>().0.iter().map(|p| p[0]).collect()
    }

    #[test]
    fn img_001_every_orientation_is_the_exact_permutation_it_names() {
        // Source, 3 wide and 2 tall:
        //   0 1 2
        //   3 4 5
        let expect: [(u8, (u32, u32), [u8; 6]); 8] = [
            (1, (3, 2), [0, 1, 2, 3, 4, 5]),
            (2, (3, 2), [2, 1, 0, 5, 4, 3]),
            (3, (3, 2), [5, 4, 3, 2, 1, 0]),
            (4, (3, 2), [3, 4, 5, 0, 1, 2]),
            (5, (2, 3), [0, 3, 1, 4, 2, 5]),
            (6, (2, 3), [3, 0, 4, 1, 5, 2]),
            (7, (2, 3), [5, 2, 4, 1, 3, 0]),
            (8, (2, 3), [2, 5, 1, 4, 0, 3]),
        ];
        for (tag, size, order) in expect {
            let orientation = Orientation::from_tag(tag).unwrap();
            let (out, width, height) = orientation.apply(indexed(3, 2), 3, 2);
            assert_eq!((width, height), size, "orientation {tag}");
            assert_eq!(red_channel(&out), order, "orientation {tag}");
        }
    }

    #[test]
    fn img_001_the_tag_is_read_from_either_byte_order() {
        let big = orientation_only_exif(Orientation::from_tag(6).unwrap());
        assert_eq!(Orientation::from_tiff(&big), Orientation::from_tag(6));
        let little = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0,
            0,
        ];
        assert_eq!(Orientation::from_tiff(&little), Orientation::from_tag(8));
        assert_eq!(Orientation::from_tiff(b"Exif\0\0MM"), None);
        assert_eq!(Orientation::from_tag(0), None);
        assert_eq!(Orientation::from_tag(9), None);
    }
}
