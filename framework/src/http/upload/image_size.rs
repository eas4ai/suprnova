//! Reading an image's width and height from its header as an upload
//! streams in.
//!
//! An image states its size in a header, but not always near its start. A
//! JPEG's start of frame follows its metadata segments (a camera's EXIF
//! block and thumbnail, an ICC profile, XMP), which can run to megabytes,
//! and an AVIF or HEIC file's `ispe` property sits in a `meta` box that may
//! follow other boxes. Laravel's `dimensions` rule reads the stored file
//! with `getimagesize`, which seeks to the header wherever it is.
//!
//! [`ImageSizeProbe`] does that work on the stream: the extractor feeds it
//! every chunk of a file part, and it walks JPEG segments and ISO base media
//! boxes by their declared lengths, passing over their payloads without
//! keeping them. It holds a 16-byte field whatever the size of the file, and
//! stops reading once it has the size. The formats whose size sits in their
//! first bytes (PNG, GIF, BMP, WebP) are read from the sniff buffer.

use super::validators::accepted_image_type;

/// The longest header field the probe reads at once: a box header with a
/// 64-bit size.
const FIELD_BYTES: usize = 16;

/// The containers an AVIF or HEIC file nests its sizes in, outermost
/// first: the `ispe` properties are in `meta`, then `iprp`, then `ipco`.
const BOX_PATH: [&[u8; 4]; 3] = [b"meta", b"iprp", b"ipco"];

/// Reads an image's width and height from a part's chunks as they arrive.
///
/// Feed it each chunk in order with [`feed`](Self::feed), then ask
/// [`size`](Self::size) with the part's sniff buffer once the part ends.
pub(super) struct ImageSizeProbe {
    state: State,
    /// Bytes to pass over before the next field starts.
    skip: u64,
    /// The field being read.
    field: [u8; FIELD_BYTES],
    /// How many bytes of `field` are read.
    filled: usize,
    /// How many bytes the field takes.
    want: usize,
    /// The body length of the box whose header was read last.
    body: u64,
    /// How many of [`BOX_PATH`]'s containers the walk is inside.
    depth: usize,
    /// The bytes still unread in each container entered, by depth.
    open: [u64; BOX_PATH.len()],
    /// The largest `ispe` size by area, width then height.
    largest: Option<(u32, u32)>,
    /// The last `irot` angle, in quarter turns anticlockwise.
    quarter_turns: u8,
}

/// Where the probe is in the part.
#[derive(Clone, Copy)]
enum State {
    /// The first two bytes, which tell a JPEG from the rest.
    Start,
    /// The first eight bytes of a file that is not a JPEG: an ISO base
    /// media file opens with its `ftyp` box header.
    Head,
    /// A JPEG between segments, looking for the `0xFF` that opens a marker.
    /// Bytes before it are passed over, as `getimagesize` passes them.
    JpegMarker,
    /// The JPEG marker code after `0xFF`; another `0xFF` is fill.
    JpegCode,
    /// A JPEG segment's two-byte length, which counts itself.
    JpegLength,
    /// A start of frame's length, precision, height and width.
    JpegFrame,
    /// A box header: a 32-bit size and a four-byte type.
    BoxHeader,
    /// A box header whose 32-bit size is 1: a 64-bit size follows.
    BoxLargeSize,
    /// The version and flags that open the `meta` box's body.
    MetaFlags,
    /// An `ispe` body: version and flags, width, height.
    Ispe,
    /// An `irot` body: the angle in its low two bits.
    Irot,
    /// The size is read.
    Found(u32, u32),
    /// The walk is over without a size, or the format is one read from the
    /// sniff buffer.
    Stopped,
}

impl ImageSizeProbe {
    pub(super) fn new() -> Self {
        Self {
            state: State::Start,
            skip: 0,
            field: [0; FIELD_BYTES],
            filled: 0,
            want: 2,
            body: 0,
            depth: 0,
            open: [0; BOX_PATH.len()],
            largest: None,
            quarter_turns: 0,
        }
    }

    /// Read the next chunk of the part. Once the size is read, or the walk
    /// is over, a chunk costs nothing.
    pub(super) fn feed(&mut self, mut chunk: &[u8]) {
        while !chunk.is_empty() && !matches!(self.state, State::Found(..) | State::Stopped) {
            if self.skip > 0 {
                let passed =
                    usize::try_from(self.skip).map_or(chunk.len(), |skip| skip.min(chunk.len()));
                self.skip -= passed as u64;
                chunk = &chunk[passed..];
            } else if matches!(self.state, State::JpegMarker) {
                match chunk.iter().position(|byte| *byte == 0xFF) {
                    Some(at) => {
                        chunk = &chunk[at + 1..];
                        self.expect(State::JpegCode, 1);
                    }
                    None => return,
                }
            } else {
                let read = (self.want - self.filled).min(chunk.len());
                self.field[self.filled..self.filled + read].copy_from_slice(&chunk[..read]);
                self.filled += read;
                chunk = &chunk[read..];
                if self.filled == self.want {
                    self.step();
                }
            }
        }
    }

    /// The width and height the part states, given its sniff buffer: from
    /// the walk for a JPEG, an AVIF or a HEIC file, else from the header in
    /// `sniff`. `None` for a type `ImageFile` refuses, for a size the part
    /// ends before, and for a zero side, which no limit can be checked
    /// against.
    pub(super) fn size(&self, sniff: &[u8]) -> Option<(u32, u32)> {
        let (width, height) = match accepted_image_type(sniff)? {
            "image/jpeg" | "image/avif" | "image/heic" | "image/heif" => match self.state {
                State::Found(width, height) => (width, height),
                _ => return None,
            },
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

    /// Read `want` bytes into the field next, in `state`.
    fn expect(&mut self, state: State, want: usize) {
        self.state = state;
        self.want = want;
        self.filled = 0;
    }

    /// Act on the field just read.
    fn step(&mut self) {
        let field = self.field;
        match self.state {
            State::Start if field[..2] == [0xFF, 0xD8] => self.expect(State::JpegMarker, 1),
            State::Start => {
                // Keep the two bytes read and read on to the first box's type.
                self.state = State::Head;
                self.want = 8;
            }
            State::Head if &field[4..8] == b"ftyp" => self.box_header(&field),
            State::JpegCode => match field[0] {
                0xFF => self.expect(State::JpegCode, 1),
                // The start of frame markers. C4, C8 and CC share the range
                // but are a Huffman table, an extension and an arithmetic
                // coding table.
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                    self.expect(State::JpegFrame, 7);
                }
                // The scan's image data, or the end of the image, before
                // any frame: the file states no size.
                0xDA | 0xD9 => self.state = State::Stopped,
                // Markers that stand alone, without a length.
                0x01 | 0xD0..=0xD8 => self.expect(State::JpegMarker, 1),
                _ => self.expect(State::JpegLength, 2),
            },
            State::JpegLength => match u16::from_be_bytes([field[0], field[1]]) {
                0 | 1 => self.state = State::Stopped,
                length => {
                    self.skip = u64::from(length) - 2;
                    self.expect(State::JpegMarker, 1);
                }
            },
            State::JpegFrame => {
                let height = u16::from_be_bytes([field[3], field[4]]);
                let width = u16::from_be_bytes([field[5], field[6]]);
                self.state = State::Found(u32::from(width), u32::from(height));
            }
            State::BoxHeader => self.box_header(&field),
            State::BoxLargeSize => {
                let mut size = [0; 8];
                size.copy_from_slice(&field[8..16]);
                self.open_box(u64::from_be_bytes(size), 16, &field);
            }
            State::MetaFlags => self.enter(self.body - 4),
            State::Ispe => {
                let width = u32::from_be_bytes([field[4], field[5], field[6], field[7]]);
                let height = u32::from_be_bytes([field[8], field[9], field[10], field[11]]);
                let area = |(width, height): (u32, u32)| u64::from(width) * u64::from(height);
                if area((width, height)) > self.largest.map_or(0, area) {
                    self.largest = Some((width, height));
                }
                self.skip = self.body - 12;
                self.next_box();
            }
            State::Irot => {
                self.quarter_turns = field[0] & 0x03;
                self.skip = self.body - 1;
                self.next_box();
            }
            State::Head | State::JpegMarker | State::Found(..) | State::Stopped => {
                self.state = State::Stopped;
            }
        }
    }

    /// Act on a box header's first eight bytes, its 32-bit size and type.
    fn box_header(&mut self, field: &[u8; FIELD_BYTES]) {
        match u32::from_be_bytes([field[0], field[1], field[2], field[3]]) {
            // A 64-bit size follows the type.
            1 => {
                self.state = State::BoxLargeSize;
                self.want = 16;
            }
            // The box runs to the end of its container, or of the file.
            0 => {
                let rest = if self.depth == 0 {
                    u64::MAX
                } else {
                    self.open[self.depth - 1]
                };
                self.open_box(rest, 8, field);
            }
            size => self.open_box(u64::from(size), 8, field),
        }
    }

    /// Act on a box of `size` bytes, `header` of them its header, whose
    /// type is `field[4..8]`: enter it when it is the next container on
    /// [`BOX_PATH`], read it when it is an `ispe` or `irot` property inside
    /// the last, and pass over it otherwise.
    fn open_box(&mut self, size: u64, header: u64, field: &[u8; FIELD_BYTES]) {
        if size < header {
            self.state = State::Stopped;
            return;
        }
        if self.depth > 0 {
            let container = &mut self.open[self.depth - 1];
            if size > *container {
                self.state = State::Stopped;
                return;
            }
            *container -= size;
        }
        self.body = size - header;
        let kind = &field[4..8];
        match BOX_PATH.get(self.depth) {
            // `meta` is a full box: its children follow a version and flags.
            Some(next) if kind == *next && self.depth == 0 => {
                if self.body < 4 {
                    self.state = State::Stopped;
                } else {
                    self.expect(State::MetaFlags, 4);
                }
            }
            Some(next) if kind == *next => self.enter(self.body),
            None if kind == b"ispe" && self.body >= 12 => self.expect(State::Ispe, 12),
            None if kind == b"irot" && self.body >= 1 => self.expect(State::Irot, 1),
            _ => {
                self.skip = self.body;
                self.next_box();
            }
        }
    }

    /// Step inside the container whose body has `body` bytes.
    fn enter(&mut self, body: u64) {
        self.open[self.depth] = body;
        self.depth += 1;
        self.next_box();
    }

    /// Read the next box header, unless the innermost container is read to
    /// its end. The end of `ipco` ends the walk with the largest size it
    /// stated; the end of `meta` or `iprp` before the next container on
    /// the path ends it with none.
    fn next_box(&mut self) {
        if self.depth == 0 || self.open[self.depth - 1] > 0 {
            self.expect(State::BoxHeader, 8);
            return;
        }
        self.state = match self.largest {
            Some((width, height)) if self.depth == BOX_PATH.len() => {
                // A quarter or three-quarter turn swaps the sides, as
                // `imagesize` reads the angle.
                if self.quarter_turns % 2 == 1 {
                    State::Found(height, width)
                } else {
                    State::Found(width, height)
                }
            }
            _ => State::Stopped,
        };
    }
}

/// The width and height of an image whose bytes are all in `bytes`, read
/// as the extractor reads a part that holds them.
pub(super) fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut probe = ImageSizeProbe::new();
    probe.feed(bytes);
    probe.size(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What the extractor reads from `bytes` delivered in chunks of
    /// `chunk` bytes, with the first 16 KiB as the sniff buffer.
    fn streamed(bytes: &[u8], chunk: usize) -> Option<(u32, u32)> {
        let mut probe = ImageSizeProbe::new();
        for piece in bytes.chunks(chunk) {
            probe.feed(piece);
        }
        probe.size(&bytes[..bytes.len().min(16 * 1024)])
    }

    /// The size read from `bytes` whole, one byte at a time, and in chunks
    /// that split every field, which must all agree.
    fn read(bytes: &[u8]) -> Option<(u32, u32)> {
        let whole = image_dimensions(bytes);
        for chunk in [1, 3, 7, 4096] {
            assert_eq!(streamed(bytes, chunk), whole, "chunks of {chunk} bytes");
        }
        whole
    }

    /// A JPEG segment: marker, length, then `payload` bytes of `fill`.
    fn segment(marker: u8, payload: usize, fill: u8) -> Vec<u8> {
        let mut bytes = vec![0xFF, marker];
        bytes.extend(u16::try_from(payload + 2).expect("a segment").to_be_bytes());
        bytes.resize(bytes.len() + payload, fill);
        bytes
    }

    /// A baseline start of frame of `width` by `height`.
    fn frame(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xC0, 0x00, 0x11, 0x08];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    fn jpeg(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        for part in parts {
            bytes.extend(part);
        }
        bytes.extend([0xFF, 0xD9]);
        bytes
    }

    #[test]
    fn a_jpeg_frame_is_read_after_any_amount_of_metadata() {
        // Three full APP segments of 0xFF filler, which must not read as
        // markers, then an ICC profile and a quantisation table.
        let file = jpeg(&[
            segment(0xE1, 65_533, 0xFF),
            segment(0xE1, 65_533, 0xC0),
            segment(0xE1, 65_533, 0xFF),
            segment(0xE2, 3_000, 0),
            segment(0xDB, 67, 1),
            frame(4032, 3024),
        ]);
        assert!(file.len() > 190 * 1024);
        assert_eq!(read(&file), Some((4032, 3024)));
    }

    #[test]
    fn a_jpeg_frame_is_read_past_fill_and_stray_bytes() {
        let mut file = vec![0xFF, 0xD8];
        file.extend(segment(0xE0, 14, 0));
        // Fill bytes before a marker code, a stray byte between segments
        // and a standalone restart marker, which `getimagesize` all passes.
        file.extend([0xFF, 0xFF, 0xFF]);
        file.extend(&segment(0xFE, 5, b'x')[1..]);
        file.extend([0x00, 0xFF, 0xD3]);
        file.extend(frame(640, 480));
        assert_eq!(read(&file), Some((640, 480)));
        // Every start of frame marker, and none of the three that share
        // its range.
        for marker in [0xC1, 0xC2, 0xC3, 0xC5, 0xC7, 0xC9, 0xCB, 0xCD, 0xCF] {
            let mut sof = frame(20, 10);
            sof[1] = marker;
            assert_eq!(read(&jpeg(&[sof])), Some((20, 10)), "{marker:#X}");
        }
        for marker in [0xC4, 0xC8, 0xCC] {
            let mut table = frame(20, 10);
            table[1] = marker;
            assert_eq!(
                read(&jpeg(&[table, frame(30, 40)])),
                Some((30, 40)),
                "{marker:#X}"
            );
        }
    }

    #[test]
    fn a_jpeg_states_no_size_without_a_frame_before_its_scan() {
        // Image data, then the end, before any frame.
        let scan = jpeg(&[
            segment(0xE1, 20 * 1024, 0),
            segment(0xDA, 12, 0),
            frame(1, 1),
        ]);
        assert_eq!(read(&scan), None);
        assert_eq!(read(&jpeg(&[segment(0xE1, 20 * 1024, 0)])), None);
        // A length that cannot count itself.
        let mut short = jpeg(&[segment(0xE1, 4, 0), frame(1, 1)]);
        short[4..6].copy_from_slice(&[0, 1]);
        assert_eq!(read(&short), None);
        // A file that ends inside its metadata or its frame.
        let whole = jpeg(&[segment(0xE1, 30 * 1024, 0), frame(10, 10)]);
        assert_eq!(read(&whole[..20 * 1024]), None);
        assert_eq!(read(&whole[..whole.len() - 16]), None);
        // A zero side.
        assert_eq!(read(&jpeg(&[frame(0, 10)])), None);
    }

    /// An ISO box of `kind` around `body`, a full box when `full`.
    fn iso_box(kind: &[u8; 4], body: &[u8], full: bool) -> Vec<u8> {
        let header = if full { 12 } else { 8 };
        let mut bytes = u32::try_from(header + body.len())
            .expect("a box")
            .to_be_bytes()
            .to_vec();
        bytes.extend(kind);
        if full {
            bytes.extend([0, 0, 0, 0]);
        }
        bytes.extend(body);
        bytes
    }

    fn ispe(width: u32, height: u32) -> Vec<u8> {
        let mut body = width.to_be_bytes().to_vec();
        body.extend(height.to_be_bytes());
        iso_box(b"ispe", &body, true)
    }

    /// A HEIC file: its `ftyp` box, the `boxes` before `meta`, and a `meta`
    /// box holding a handler, `before` ahead of `iprp`, and `properties`
    /// in `ipco`.
    fn heic(boxes: &[Vec<u8>], properties: &[Vec<u8>]) -> Vec<u8> {
        let mut file = iso_box(b"ftyp", b"heic\0\0\0\0mif1heic", false);
        for each in boxes {
            file.extend(each);
        }
        let ipco = iso_box(b"ipco", &properties.concat(), false);
        let iprp = iso_box(b"iprp", &ipco, false);
        let mut meta_body = iso_box(b"hdlr", &[0; 21], true);
        meta_body.extend(iprp);
        file.extend(iso_box(b"meta", &meta_body, true));
        file.extend(iso_box(b"mdat", &[0; 64], false));
        file
    }

    #[test]
    fn a_heic_size_is_read_after_any_leading_boxes() {
        let file = heic(
            &[iso_box(b"free", &vec![0; 40 * 1024], false)],
            &[
                iso_box(b"hvcC", &[1; 23], false),
                ispe(512, 512),
                ispe(4032, 3024),
                ispe(256, 192),
            ],
        );
        assert_eq!(read(&file), Some((4032, 3024)), "the largest by area");
        // A 64-bit box size.
        let mut large = 1u32.to_be_bytes().to_vec();
        large.extend(b"free");
        large.extend(32u64.to_be_bytes());
        large.extend([0; 16]);
        assert_eq!(read(&heic(&[large], &[ispe(64, 48)])), Some((64, 48)));
    }

    #[test]
    fn a_heic_quarter_turn_swaps_its_sides() {
        for (angle, size) in [(0, (64, 48)), (1, (48, 64)), (2, (64, 48)), (3, (48, 64))] {
            let file = heic(&[], &[ispe(64, 48), iso_box(b"irot", &[angle], false)]);
            assert_eq!(read(&file), Some(size), "{angle}");
        }
    }

    #[test]
    fn a_heic_states_no_size_without_its_properties() {
        assert_eq!(read(&heic(&[], &[iso_box(b"hvcC", &[1; 23], false)])), None);
        // `meta` without `iprp`.
        let mut bare = iso_box(b"ftyp", b"heic\0\0\0\0mif1heic", false);
        bare.extend(iso_box(b"meta", &iso_box(b"hdlr", &[0; 21], true), true));
        assert_eq!(read(&bare), None);
        // A box that claims more than its container holds.
        let mut overflowing = heic(&[], &[ispe(64, 48)]);
        let ispe_at = overflowing.len() - 72 - 20;
        overflowing[ispe_at..ispe_at + 4].copy_from_slice(&64u32.to_be_bytes());
        assert_eq!(read(&overflowing), None);
        // A file that ends before its `meta` box.
        let whole = heic(
            &[iso_box(b"free", &vec![0; 20 * 1024], false)],
            &[ispe(64, 48)],
        );
        assert_eq!(read(&whole[..18 * 1024]), None);
    }

    #[test]
    fn the_formats_whose_size_leads_are_read_from_the_sniff() {
        let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        png.extend(300u32.to_be_bytes());
        png.extend(200u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(read(&png), Some((300, 200)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend(30u16.to_le_bytes());
        gif.extend(20u16.to_le_bytes());
        gif.extend([0xF7, 0, 0]);
        assert_eq!(read(&gif), Some((30, 20)));
        assert_eq!(read(b"%PDF-1.7\n"), None);
        assert_eq!(read(b""), None);
    }
}
