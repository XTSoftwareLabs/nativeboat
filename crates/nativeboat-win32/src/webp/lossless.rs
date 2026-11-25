//! Decodes WebP's lossless format (VP8L) as libwebp does: prefix-coded
//! pixels with backward references and a color cache, under up to four
//! transforms.

#[allow(unused_imports)]
use crate::prelude::*;

/// The green codes that are literal values, before the length prefixes.
const LITERALS: usize = 256;
/// The length prefix codes after the literals.
const LENGTHS: usize = 24;
/// The distance prefix codes.
const DISTANCES: usize = 40;
/// The longest prefix code.
const MAX_LENGTH: usize = 15;
/// The bits of the fast lookup of short codes.
const FAST_BITS: u32 = 8;
/// The most bits a color cache key has.
const MAX_CACHE_BITS: u32 = 11;
/// The order the lengths of the code length code are stored in.
const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
/// Distance codes 1 to 120, the nearest pixels: `y << 4 | (8 - x)`, for
/// `y` rows up and `x` columns to the left.
const CODE_TO_PLANE: [u8; 120] = [
    0x18, 0x07, 0x17, 0x19, 0x28, 0x06, 0x27, 0x29, 0x16, 0x1a, 0x26, 0x2a, 0x38, 0x05, 0x37, 0x39,
    0x15, 0x1b, 0x36, 0x3a, 0x25, 0x2b, 0x48, 0x04, 0x47, 0x49, 0x14, 0x1c, 0x35, 0x3b, 0x46, 0x4a,
    0x24, 0x2c, 0x58, 0x45, 0x4b, 0x34, 0x3c, 0x03, 0x57, 0x59, 0x13, 0x1d, 0x56, 0x5a, 0x23, 0x2d,
    0x44, 0x4c, 0x55, 0x5b, 0x33, 0x3d, 0x68, 0x02, 0x67, 0x69, 0x12, 0x1e, 0x66, 0x6a, 0x22, 0x2e,
    0x54, 0x5c, 0x43, 0x4d, 0x65, 0x6b, 0x32, 0x3e, 0x78, 0x01, 0x77, 0x79, 0x53, 0x5d, 0x11, 0x1f,
    0x64, 0x6c, 0x42, 0x4e, 0x76, 0x7a, 0x21, 0x2f, 0x75, 0x7b, 0x31, 0x3f, 0x63, 0x6d, 0x52, 0x5e,
    0x00, 0x74, 0x7c, 0x41, 0x4f, 0x10, 0x20, 0x62, 0x6e, 0x30, 0x73, 0x7d, 0x51, 0x5f, 0x40, 0x72,
    0x7e, 0x61, 0x6f, 0x50, 0x71, 0x7f, 0x60, 0x70,
];
/// Opaque black, what the first pixel is predicted from.
const BLACK: u32 = 0xff00_0000;

/// Reads bits least significant first. Past the end it reads zeros and
/// notes the overrun, as libwebp's reader does.
pub(super) struct Bits<'a> {
    data: &'a [u8],
    /// The next bit, counted from the start.
    position: usize,
}

impl<'a> Bits<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// The next 32 bits, without taking them.
    fn peek(&self) -> u32 {
        let start = self.position / 8;
        let mut word = [0_u8; 8];
        if let Some(bytes) = self.data.get(start..start + 8) {
            word.copy_from_slice(bytes);
        } else if let Some(bytes) = self.data.get(start..) {
            word[..bytes.len()].copy_from_slice(bytes);
        }
        (u64::from_le_bytes(word) >> (self.position % 8)) as u32
    }

    fn skip(&mut self, count: u32) {
        self.position += count as usize;
    }

    /// The next `count` bits, at most 24, as a number.
    pub(super) fn read(&mut self, count: u32) -> u32 {
        let value = self.peek() & ((1 << count) - 1);
        self.skip(count);
        value
    }

    /// Whether more bits were read than there are. libwebp's reader holds 64
    /// bits, so it only notices past those.
    fn overran(&self) -> bool {
        self.position > (self.data.len() * 8).max(64)
    }
}

/// A canonical prefix code, as VP8L stores its codes.
struct Code {
    /// The symbol of a code with one symbol, which takes no bits.
    single: Option<u16>,
    /// By the next `FAST_BITS` bits: the symbol and the length of its code,
    /// or a length of 0 for the prefixes of longer codes.
    fast: Vec<(u16, u8)>,
    /// How many codes each length has.
    counts: [u16; MAX_LENGTH + 1],
    /// The symbols, by code.
    symbols: Vec<u16>,
}

impl Code {
    /// The code of these code lengths, as libwebp's `BuildHuffmanTable`
    /// accepts them: some code, none longer than 15 bits, and either one
    /// alone or exactly as many as fill the code space.
    fn new(lengths: &[u8]) -> Option<Self> {
        let mut counts = [0_u16; MAX_LENGTH + 1];
        for &length in lengths {
            *counts.get_mut(usize::from(length))? += 1;
        }
        if usize::from(counts[0]) == lengths.len() {
            return None;
        }
        let mut offsets = [0_usize; MAX_LENGTH + 2];
        for length in 1..=MAX_LENGTH {
            offsets[length + 1] = offsets[length] + usize::from(counts[length]);
        }
        let mut symbols = vec![0_u16; offsets[MAX_LENGTH + 1]];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length > 0 {
                symbols[offsets[usize::from(length)]] = symbol as u16;
                offsets[usize::from(length)] += 1;
            }
        }
        if symbols.len() == 1 {
            return Some(Self {
                single: Some(symbols[0]),
                fast: Vec::new(),
                counts,
                symbols,
            });
        }
        let mut open = 1_i32;
        for &count in &counts[1..] {
            open = open * 2 - i32::from(count);
            if open < 0 {
                return None;
            }
        }
        if open != 0 {
            return None;
        }
        // Codes are assigned in order of length, then symbol, and stored
        // first bit first: the lookup indexes them reversed.
        let mut fast = vec![(0_u16, 0_u8); 1 << FAST_BITS];
        let mut code = 0_u32;
        let mut index = 0;
        for length in 1..=MAX_LENGTH as u32 {
            for _ in 0..counts[length as usize] {
                if length <= FAST_BITS {
                    let reversed = code.reverse_bits() >> (32 - length);
                    for entry in (reversed as usize..fast.len()).step_by(1 << length) {
                        fast[entry] = (symbols[index], length as u8);
                    }
                }
                code += 1;
                index += 1;
            }
            code <<= 1;
        }
        Some(Self {
            single: None,
            fast,
            counts,
            symbols,
        })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> usize {
        if let Some(symbol) = self.single {
            return usize::from(symbol);
        }
        let next = bits.peek();
        let (symbol, length) = self.fast[(next & ((1 << FAST_BITS) - 1)) as usize];
        if length > 0 {
            bits.skip(u32::from(length));
            return usize::from(symbol);
        }
        let (mut code, mut first, mut index) = (0_i32, 0_i32, 0_i32);
        // A complete code always ends within its longest length.
        (1..=MAX_LENGTH)
            .find_map(|length| {
                code |= ((next >> (length - 1)) & 1) as i32;
                let count = i32::from(self.counts[length]);
                if code - first < count {
                    bits.skip(length as u32);
                    return Some(usize::from(self.symbols[(index + code - first) as usize]));
                }
                index += count;
                first = (first + count) << 1;
                code <<= 1;
                None
            })
            .unwrap_or(0)
    }
}

/// Reads a prefix code over `alphabet` symbols.
fn read_code(bits: &mut Bits<'_>, alphabet: usize) -> Option<Code> {
    // Simple codes name symbols of up to 8 bits, even in smaller alphabets;
    // those past the alphabet count for nothing.
    let mut lengths = vec![0_u8; alphabet.max(256)];
    if bits.read(1) == 1 {
        let symbols = bits.read(1) + 1;
        let first_bits = if bits.read(1) == 0 { 1 } else { 8 };
        lengths[bits.read(first_bits) as usize] = 1;
        if symbols == 2 {
            lengths[bits.read(8) as usize] = 1;
        }
    } else {
        let mut code_lengths = [0_u8; 19];
        for &symbol in &CODE_LENGTH_ORDER[..bits.read(4) as usize + 4] {
            code_lengths[symbol] = bits.read(3) as u8;
        }
        read_code_lengths(bits, &code_lengths, &mut lengths[..alphabet])?;
    }
    if bits.overran() {
        return None;
    }
    Code::new(&lengths[..alphabet])
}

/// Reads the code lengths of a code, themselves prefix-coded.
fn read_code_lengths(
    bits: &mut Bits<'_>,
    code_lengths: &[u8; 19],
    lengths: &mut [u8],
) -> Option<()> {
    let code = Code::new(code_lengths)?;
    let mut max_symbol = if bits.read(1) == 1 {
        let length_bits = 2 + 2 * bits.read(3);
        let max = 2 + bits.read(length_bits) as usize;
        if max > lengths.len() {
            return None;
        }
        max
    } else {
        lengths.len()
    };
    let mut symbol = 0;
    let mut previous = 8;
    while symbol < lengths.len() {
        if max_symbol == 0 {
            break;
        }
        max_symbol -= 1;
        let length = code.decode(bits);
        if length < 16 {
            lengths[symbol] = length as u8;
            symbol += 1;
            if length != 0 {
                previous = length as u8;
            }
        } else {
            let slot = length - 16;
            let repeat = bits.read([2, 3, 7][slot]) as usize + [3, 3, 11][slot];
            if symbol + repeat > lengths.len() {
                return None;
            }
            let value = if length == 16 { previous } else { 0 };
            lengths[symbol..symbol + repeat].fill(value);
            symbol += repeat;
        }
    }
    Some(())
}

/// The five codes of a group: green with lengths and cache keys, red,
/// blue, alpha and distance.
struct Group([Code; 5]);

fn read_group(bits: &mut Bits<'_>, cache_size: usize) -> Option<Group> {
    Some(Group([
        read_code(bits, LITERALS + LENGTHS + cache_size)?,
        read_code(bits, LITERALS)?,
        read_code(bits, LITERALS)?,
        read_code(bits, LITERALS)?,
        read_code(bits, DISTANCES)?,
    ]))
}

/// `size` divided by `1 << bits`, rounded up.
fn subsampled(size: usize, bits: u32) -> usize {
    (size + (1 << bits) - 1) >> bits
}

/// A length or distance from its prefix symbol and extra bits.
fn prefix_value(symbol: usize, bits: &mut Bits<'_>) -> usize {
    if symbol < 4 {
        return symbol + 1;
    }
    let extra = (symbol as u32 - 2) >> 1;
    let offset = (2 + (symbol & 1)) << extra;
    offset + bits.read(extra) as usize + 1
}

/// The distance back, in pixels, a distance code names in rows `width`
/// wide.
fn plane_distance(width: usize, code: usize) -> usize {
    if code > CODE_TO_PLANE.len() {
        return code - CODE_TO_PLANE.len();
    }
    let plane = CODE_TO_PLANE[code - 1];
    let distance = isize::from(plane >> 4) * width as isize + 8 - isize::from(plane & 0xf);
    distance.max(1) as usize
}

/// The prefix codes of an image: one group, or one per tile of a group
/// index image.
struct Codes {
    /// The bits of the tiles' size, the tiles across and their groups.
    tiles: Option<(u32, usize, Vec<u32>)>,
    /// The groups, `None` where no tile uses one.
    groups: Vec<Option<Group>>,
}

impl Codes {
    fn group(&self, x: usize, y: usize) -> Option<&Group> {
        let index = match &self.tiles {
            Some((bits, across, image)) => image[(y >> bits) * across + (x >> bits)] as usize,
            None => 0,
        };
        self.groups.get(index)?.as_ref()
    }
}

fn read_codes(
    bits: &mut Bits<'_>,
    (width, height): (usize, usize),
    cache_size: usize,
    tiled: bool,
) -> Option<Codes> {
    let mut tiles = None;
    let mut used = vec![true];
    if tiled && bits.read(1) == 1 {
        let tile_bits = bits.read(3) + 2;
        let across = subsampled(width, tile_bits);
        let mut image = decode_stream(bits, (across, subsampled(height, tile_bits)), false)?;
        for group in &mut image {
            *group = (*group >> 8) & 0xffff;
        }
        used = vec![false; image.iter().max().map_or(1, |max| *max as usize + 1)];
        for &group in &image {
            used[group as usize] = true;
        }
        tiles = Some((tile_bits, across, image));
    }
    if bits.overran() {
        return None;
    }
    let mut groups = Vec::with_capacity(used.len());
    for used in used {
        let group = read_group(bits, cache_size)?;
        groups.push(used.then_some(group));
    }
    Some(Codes { tiles, groups })
}

/// Decodes the prefix-coded pixels of an image.
fn decode_pixels(
    bits: &mut Bits<'_>,
    (width, height): (usize, usize),
    codes: &Codes,
    cache_bits: u32,
) -> Option<Vec<u32>> {
    let total = width * height;
    let mut pixels = vec![0_u32; total];
    let mut cache = vec![0_u32; if cache_bits > 0 { 1 << cache_bits } else { 0 }];
    // Pixels go into the cache in order, up to before the one looked up.
    let mut cached = 0;
    let mut position = 0;
    let (mut x, mut y) = (0, 0);
    while position < total {
        let group = codes.group(x, y)?;
        let start = position;
        let code = group.0[0].decode(bits);
        if code < LITERALS {
            let red = group.0[1].decode(bits) as u32;
            let blue = group.0[2].decode(bits) as u32;
            let alpha = group.0[3].decode(bits) as u32;
            pixels[position] = alpha << 24 | red << 16 | (code as u32) << 8 | blue;
            position += 1;
        } else if code < LITERALS + LENGTHS {
            let length = prefix_value(code - LITERALS, bits);
            let symbol = group.0[4].decode(bits);
            let distance = plane_distance(width, prefix_value(symbol, bits));
            if bits.overran() || position < distance || total - position < length {
                return None;
            }
            for index in position..position + length {
                pixels[index] = pixels[index - distance];
            }
            position += length;
        } else {
            let key = code - LITERALS - LENGTHS;
            while cached < position {
                let pixel = pixels[cached];
                cache[(0x1e35_a7bd_u32.wrapping_mul(pixel) >> (32 - cache_bits)) as usize] = pixel;
                cached += 1;
            }
            pixels[position] = *cache.get(key)?;
            position += 1;
        }
        if bits.overran() {
            return None;
        }
        x += position - start;
        while x >= width {
            x -= width;
            y += 1;
        }
    }
    Some(pixels)
}

/// A transform of the image's pixels, with the width it applies at.
enum Transform {
    Predictor(usize, u32, Vec<u32>),
    CrossColor(usize, u32, Vec<u32>),
    SubtractGreen,
    /// Pixels index a palette, several to a pixel when it is small.
    ColorIndexing(usize, u32, Vec<u32>),
}

/// Decodes an image stream: transforms, when `top` (the image itself, not
/// one of the images that describe it), then the coded pixels.
fn decode_stream(
    bits: &mut Bits<'_>,
    (width, height): (usize, usize),
    top: bool,
) -> Option<Vec<u32>> {
    let mut transforms = Vec::new();
    let mut coded_width = width;
    let mut seen = 0_u8;
    while top && bits.read(1) == 1 {
        let kind = bits.read(2);
        if seen & 1 << kind != 0 {
            return None;
        }
        seen |= 1 << kind;
        transforms.push(match kind {
            0 | 1 => {
                let tile_bits = bits.read(3) + 2;
                let size = (
                    subsampled(coded_width, tile_bits),
                    subsampled(height, tile_bits),
                );
                let data = decode_stream(bits, size, false)?;
                if kind == 0 {
                    Transform::Predictor(coded_width, tile_bits, data)
                } else {
                    Transform::CrossColor(coded_width, tile_bits, data)
                }
            }
            2 => Transform::SubtractGreen,
            _ => {
                let colors = bits.read(8) as usize + 1;
                let pack_bits = match colors {
                    17.. => 0,
                    5..=16 => 1,
                    3..=4 => 2,
                    _ => 3,
                };
                let deltas = decode_stream(bits, (colors, 1), false)?;
                // Each color is stored as its difference from the one before;
                // indexes past them are transparent black.
                let mut palette = vec![0_u32; 1 << (8 >> pack_bits)];
                let mut previous = 0;
                for (color, delta) in palette.iter_mut().zip(deltas) {
                    *color = add_pixels(delta, previous);
                    previous = *color;
                }
                let transform = Transform::ColorIndexing(coded_width, pack_bits, palette);
                coded_width = subsampled(coded_width, pack_bits);
                transform
            }
        });
    }
    let cache_bits = if bits.read(1) == 1 {
        let cache_bits = bits.read(4);
        if !(1..=MAX_CACHE_BITS).contains(&cache_bits) {
            return None;
        }
        cache_bits
    } else {
        0
    };
    let cache_size = if cache_bits > 0 { 1 << cache_bits } else { 0 };
    let codes = read_codes(bits, (coded_width, height), cache_size, top)?;
    let mut pixels = decode_pixels(bits, (coded_width, height), &codes, cache_bits)?;
    for transform in transforms.iter().rev() {
        pixels = transform.invert(pixels, height);
    }
    Some(pixels)
}

/// Adds two pixels channel by channel, wrapping.
fn add_pixels(a: u32, b: u32) -> u32 {
    let alpha_green = (a & 0xff00_ff00).wrapping_add(b & 0xff00_ff00);
    let red_blue = (a & 0x00ff_00ff).wrapping_add(b & 0x00ff_00ff);
    (alpha_green & 0xff00_ff00) | (red_blue & 0x00ff_00ff)
}

/// The mean of two pixels channel by channel, rounded down.
fn average(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xfefe_fefe) >> 1) + (a & b)
}

fn channel(pixel: u32, shift: u32) -> i32 {
    ((pixel >> shift) & 0xff) as i32
}

/// Joins four channels, alpha first, clamping each to a byte.
fn join(channels: impl Fn(u32) -> i32) -> u32 {
    [24, 16, 8, 0].into_iter().fold(0, |pixel, shift| {
        pixel | (channels(shift).clamp(0, 255) as u32) << shift
    })
}

/// The predictors of the predictor transform, by mode.
fn predict(mode: u32, left: u32, top: u32, top_left: u32, top_right: u32) -> u32 {
    match mode {
        1 => left,
        2 => top,
        3 => top_right,
        4 => top_left,
        5 => average(average(left, top_right), top),
        6 => average(left, top_left),
        7 => average(left, top),
        8 => average(top_left, top),
        9 => average(top, top_right),
        10 => average(average(left, top_left), average(top, top_right)),
        11 => {
            // Whichever of the left and top pixels is nearer the gradient's
            // prediction.
            let distance = |shift| {
                (channel(left, shift) - channel(top_left, shift)).abs()
                    - (channel(top, shift) - channel(top_left, shift)).abs()
            };
            if [24, 16, 8, 0].into_iter().map(distance).sum::<i32>() <= 0 {
                top
            } else {
                left
            }
        }
        12 => join(|shift| channel(left, shift) + channel(top, shift) - channel(top_left, shift)),
        13 => {
            let mean = average(left, top);
            join(|shift| {
                let a = channel(mean, shift);
                a + (a - channel(top_left, shift)) / 2
            })
        }
        _ => BLACK,
    }
}

impl Transform {
    /// Undoes the transform on pixels `height` rows high.
    fn invert(&self, input: Vec<u32>, height: usize) -> Vec<u32> {
        match self {
            Self::Predictor(width, bits, modes) => {
                let width = *width;
                let mut output = input;
                let across = subsampled(width, *bits);
                // The first row is predicted from the left, after a black
                // first pixel; the first column from above.
                output[0] = add_pixels(output[0], BLACK);
                for x in 1..width {
                    output[x] = add_pixels(output[x], output[x - 1]);
                }
                for y in 1..height {
                    let row = y * width;
                    output[row] = add_pixels(output[row], output[row - width]);
                    let tiles = &modes[(y >> bits) * across..];
                    for x in 1..width {
                        let above = row - width + x;
                        // The rightmost pixel's top right is the row's first.
                        let prediction = predict(
                            (tiles[x >> bits] >> 8) & 0xf,
                            output[row + x - 1],
                            output[above],
                            output[above - 1],
                            output[above + 1],
                        );
                        output[row + x] = add_pixels(output[row + x], prediction);
                    }
                }
                output
            }
            Self::CrossColor(width, bits, multipliers) => {
                let width = *width;
                let mut output = input;
                let across = subsampled(width, *bits);
                let delta = |multiplier: u32, color: u32| {
                    (i32::from(multiplier as u8 as i8) * i32::from(color as u8 as i8)) >> 5
                };
                for (index, pixel) in output.iter_mut().enumerate() {
                    let (x, y) = (index % width, index / width);
                    let m = multipliers[(y >> bits) * across + (x >> bits)];
                    let green = (*pixel >> 8) & 0xff;
                    let red = (((*pixel >> 16) & 0xff) as i32 + delta(m, green)) & 0xff;
                    let blue = ((*pixel & 0xff) as i32
                        + delta(m >> 8, green)
                        + delta(m >> 16, red as u32))
                        & 0xff;
                    *pixel = (*pixel & 0xff00_ff00) | (red as u32) << 16 | blue as u32;
                }
                output
            }
            Self::SubtractGreen => {
                let mut output = input;
                for pixel in &mut output {
                    let green = (*pixel >> 8) & 0xff;
                    let red_blue = ((*pixel & 0x00ff_00ff) + (green << 16 | green)) & 0x00ff_00ff;
                    *pixel = (*pixel & 0xff00_ff00) | red_blue;
                }
                output
            }
            Self::ColorIndexing(width, bits, palette) => {
                let width = *width;
                let packed_width = subsampled(width, *bits);
                let index_bits = 8_u32 >> bits;
                let mut output = vec![0_u32; width * height];
                for y in 0..height {
                    let packed = &input[y * packed_width..(y + 1) * packed_width];
                    for x in 0..width {
                        let byte = (packed[x >> bits] >> 8) & 0xff;
                        let index = (byte >> ((x & ((1 << bits) - 1)) as u32 * index_bits))
                            & ((1 << index_bits) - 1);
                        output[y * width + x] = palette[index as usize];
                    }
                }
                output
            }
        }
    }
}

/// Decodes a VP8L bitstream into its size and straight-alpha RGBA pixels.
pub(super) fn decode(data: &[u8]) -> Option<((u32, u32), Vec<u8>)> {
    let mut bits = Bits::new(data);
    if bits.read(8) != 0x2f {
        return None;
    }
    let width = bits.read(14) + 1;
    let height = bits.read(14) + 1;
    let _alpha = bits.read(1);
    if bits.read(3) != 0 || !super::fits(width, height) {
        return None;
    }
    let pixels = decode_stream(&mut bits, (width as usize, height as usize), true)?;
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        rgba.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            pixel as u8,
            (pixel >> 24) as u8,
        ]);
    }
    Some(((width, height), rgba))
}

/// Decodes an alpha plane stored as a VP8L image stream with no header, its
/// values in the green channel.
pub(super) fn decode_alpha(data: &[u8], size: (usize, usize)) -> Option<Vec<u8>> {
    let pixels = decode_stream(&mut Bits::new(data), size, true)?;
    Some(pixels.into_iter().map(|pixel| (pixel >> 8) as u8).collect())
}
