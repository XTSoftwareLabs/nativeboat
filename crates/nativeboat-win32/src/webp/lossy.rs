//! Decodes WebP's lossy format, a VP8 key frame (RFC 6386), as libwebp
//! does: the same boolean decoding, prediction, transforms and loop filter,
//! then RGB from YUV with libwebp's "fancy" upsampling of chroma.

#[allow(unused_imports)]
use crate::prelude::*;

use super::tables::{AC_TABLE, BMODES_PROBA, COEFFS_PROBA, COEFFS_UPDATE_PROBA, DC_TABLE};

/// Prediction modes. Whole-block luma and chroma modes share the values of
/// the first four 4x4 modes.
const DC_PRED: u8 = 0;
const TM_PRED: u8 = 1;
const V_PRED: u8 = 2;
const H_PRED: u8 = 3;
const B_RD_PRED: u8 = 4;
const B_VR_PRED: u8 = 5;
const B_LD_PRED: u8 = 6;
const B_VL_PRED: u8 = 7;
const B_HD_PRED: u8 = 8;
const B_HU_PRED: u8 = 9;
/// DC prediction where the block has no row above, no column to the left,
/// or neither.
const DC_PRED_NO_TOP: u8 = 4;
const DC_PRED_NO_LEFT: u8 = 5;
const DC_PRED_NO_TOP_LEFT: u8 = 6;

/// The stride of the work area a macroblock is predicted in, with the row
/// above and the column to the left of each plane.
const BPS: usize = 32;
const Y_OFF: usize = BPS + 8;
const U_OFF: usize = Y_OFF + BPS * 16 + BPS;
const V_OFF: usize = U_OFF + 16;
const WORK_SIZE: usize = BPS * 17 + BPS * 9;

/// Each coefficient's band, which picks its probabilities; one more past the
/// last.
const BANDS: [usize; 17] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 0];
/// The coefficients of a 4x4 block in coding order.
const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
/// The offsets of a macroblock's 4x4 luma blocks in the work area.
const SCAN: [usize; 16] = [
    0,
    4,
    8,
    12,
    4 * BPS,
    4 + 4 * BPS,
    8 + 4 * BPS,
    12 + 4 * BPS,
    8 * BPS,
    4 + 8 * BPS,
    8 + 8 * BPS,
    12 + 8 * BPS,
    12 * BPS,
    4 + 12 * BPS,
    8 + 12 * BPS,
    12 + 12 * BPS,
];
/// The probabilities of the extra bits of large coefficients, by category.
const CATEGORIES: [&[u8]; 4] = [
    &[173, 148, 140],
    &[176, 155, 140, 135],
    &[180, 157, 141, 134, 130],
    &[254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129],
];

/// The boolean decoder, as libwebp's `VP8BitReader`, a byte at a time.
struct BoolReader<'a> {
    data: &'a [u8],
    position: usize,
    value: u64,
    /// The range less one.
    range: u32,
    /// The bits of `value` below the decoding window.
    bits: i32,
    /// Whether it read past the end, as zeros.
    eof: bool,
}

impl<'a> BoolReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut reader = Self {
            data,
            position: 0,
            value: 0,
            range: 255 - 1,
            bits: -8,
            eof: false,
        };
        reader.load();
        reader
    }

    fn load(&mut self) {
        if let Some(&byte) = self.data.get(self.position) {
            self.position += 1;
            self.bits += 8;
            self.value = (self.value << 8) | u64::from(byte);
        } else if !self.eof {
            self.value <<= 8;
            self.bits += 8;
            self.eof = true;
        } else {
            self.bits = 0;
        }
    }

    /// A bit that is 0 with `probability` in 256.
    fn bit(&mut self, probability: u8) -> bool {
        let mut range = self.range;
        if self.bits < 0 {
            self.load();
        }
        let position = self.bits;
        let split = (range * u32::from(probability)) >> 8;
        let value = (self.value >> position) as u32;
        let bit = value > split;
        if bit {
            range -= split;
            self.value -= u64::from(split + 1) << position;
        } else {
            range = split + 1;
        }
        let shift = 7 ^ (31 - range.leading_zeros()) as i32;
        range <<= shift;
        self.bits -= shift;
        self.range = range - 1;
        bit
    }

    fn flag(&mut self) -> bool {
        self.bit(0x80)
    }

    /// An unsigned number of `count` bits, most significant first.
    fn value(&mut self, count: u32) -> u32 {
        (0..count)
            .rev()
            .fold(0, |value, bit| value | u32::from(self.flag()) << bit)
    }

    /// A number of `count` bits and a sign.
    fn signed_value(&mut self, count: u32) -> i32 {
        let value = self.value(count) as i32;
        if self.flag() { -value } else { value }
    }

    /// `value` with the sign the next even bit gives, as libwebp's
    /// `VP8GetSigned`, which always moves the window one bit.
    fn signed(&mut self, value: i32) -> i32 {
        if self.bits < 0 {
            self.load();
        }
        let position = self.bits;
        let split = self.range >> 1;
        let current = (self.value >> position) as u32;
        let mask = (split.wrapping_sub(current) as i32) >> 31;
        self.bits -= 1;
        self.range = self.range.wrapping_add(mask as u32) | 1;
        self.value -= u64::from((split + 1) & mask as u32) << position;
        (value ^ mask) - mask
    }
}

/// The dequantization factors of a segment: for DC and AC coefficients.
#[derive(Clone, Copy, Default)]
struct Quant {
    y1: [i32; 2],
    y2: [i32; 2],
    uv: [i32; 2],
}

/// How strongly the loop filter smooths a macroblock's edges.
#[derive(Clone, Copy, Default)]
struct Strength {
    limit: i32,
    inner_level: i32,
    hev_threshold: i32,
    /// Whether the edges between its 4x4 blocks are filtered too.
    inner: bool,
}

/// A macroblock's modes and coefficients.
struct Block {
    segment: usize,
    skip: bool,
    i4x4: bool,
    modes: [u8; 16],
    uv_mode: u8,
    coeffs: [i16; 384],
    /// Two bits for each 4x4 block, the first highest: whether it has
    /// coefficients, and which transform they need.
    non_zero_y: u32,
    non_zero_uv: u32,
}

/// The bottom row of a macroblock, which predicts the one below.
#[derive(Clone, Copy)]
struct Top {
    y: [u8; 16],
    u: [u8; 8],
    v: [u8; 8],
}

/// The probabilities of coefficients by plane type, band, context and
/// branch.
type Bands = [[[[u8; 11]; 3]; 8]; 4];

fn clip(value: i32, max: i32) -> usize {
    value.clamp(0, max) as usize
}

/// Decodes a VP8 bitstream into its size and straight-alpha RGBA pixels,
/// with alpha from an ALPH chunk.
pub(super) fn decode(data: &[u8], alpha: Option<&[u8]>) -> Option<((u32, u32), Vec<u8>)> {
    let tag =
        u32::from(*data.first()?) | u32::from(*data.get(1)?) << 8 | u32::from(*data.get(2)?) << 16;
    let key_frame = tag & 1 == 0;
    let profile = (tag >> 1) & 7;
    let shown = (tag >> 4) & 1 == 1;
    let first_size = (tag >> 5) as usize;
    if !key_frame || profile > 3 || !shown || data.get(3..6)? != [0x9d, 0x01, 0x2a] {
        return None;
    }
    let width = usize::from(u16::from_le_bytes([*data.get(6)?, *data.get(7)?]) & 0x3fff);
    let height = usize::from(u16::from_le_bytes([*data.get(8)?, *data.get(9)?]) & 0x3fff);
    if width == 0 || height == 0 || !super::fits(width as u32, height as u32) {
        return None;
    }
    let rest = &data[10..];
    let mut header = BoolReader::new(rest.get(..first_size)?);
    let partitions = &rest[first_size..];

    let _color_space = header.flag();
    let _clamping = header.flag();
    // Segments, with their own quantizers and filter strengths.
    let segmented = header.flag();
    let mut update_map = false;
    let mut absolute = true;
    let mut segment_quantizer = [0_i32; 4];
    let mut segment_filter = [0_i32; 4];
    let mut segment_probabilities = [255_u8; 3];
    if segmented {
        update_map = header.flag();
        if header.flag() {
            absolute = header.flag();
            for quantizer in &mut segment_quantizer {
                *quantizer = if header.flag() {
                    header.signed_value(7)
                } else {
                    0
                };
            }
            for filter in &mut segment_filter {
                *filter = if header.flag() {
                    header.signed_value(6)
                } else {
                    0
                };
            }
        }
        if update_map {
            for probability in &mut segment_probabilities {
                *probability = if header.flag() {
                    header.value(8) as u8
                } else {
                    255
                };
            }
        }
    }
    if header.eof {
        return None;
    }
    // The loop filter.
    let simple = header.flag();
    let level = header.value(6) as i32;
    let sharpness = header.value(3) as i32;
    let mut reference_delta = 0;
    let mut mode_delta = 0;
    if header.flag() && header.flag() {
        let mut deltas = [0_i32; 8];
        for delta in &mut deltas {
            if header.flag() {
                *delta = header.signed_value(6);
            }
        }
        // Only the deltas of intra frames apply to key frames.
        reference_delta = deltas[0];
        mode_delta = deltas[4];
    }
    let filter_type = if level == 0 {
        0
    } else if simple {
        1
    } else {
        2
    };
    if header.eof {
        return None;
    }
    // The token partitions, which take macroblock rows in turn.
    let count = 1 << header.value(2);
    let sizes_length = 3 * (count - 1);
    if partitions.len() < sizes_length {
        return None;
    }
    let mut start = sizes_length;
    let mut tokens = Vec::with_capacity(count);
    for sizes in partitions[..sizes_length].chunks(3) {
        let size =
            (usize::from(sizes[0]) | usize::from(sizes[1]) << 8 | usize::from(sizes[2]) << 16)
                .min(partitions.len() - start);
        tokens.push(BoolReader::new(&partitions[start..start + size]));
        start += size;
    }
    if start >= partitions.len() {
        return None;
    }
    tokens.push(BoolReader::new(&partitions[start..]));
    // Quantizers.
    let base = header.value(7) as i32;
    let mut deltas = [0_i32; 5];
    for delta in &mut deltas {
        *delta = if header.flag() {
            header.signed_value(4)
        } else {
            0
        };
    }
    let [y1_dc, y2_dc, y2_ac, uv_dc, uv_ac] = deltas;
    let mut quants = [Quant::default(); 4];
    for (segment, quant) in quants.iter_mut().enumerate() {
        let q = if segmented {
            segment_quantizer[segment] + if absolute { 0 } else { base }
        } else {
            base
        };
        *quant = Quant {
            y1: [
                i32::from(DC_TABLE[clip(q + y1_dc, 127)]),
                i32::from(AC_TABLE[clip(q, 127)]),
            ],
            y2: [
                i32::from(DC_TABLE[clip(q + y2_dc, 127)]) * 2,
                // x * 155 / 100, exactly for the table's values.
                ((i32::from(AC_TABLE[clip(q + y2_ac, 127)]) * 101_581) >> 16).max(8),
            ],
            uv: [
                i32::from(DC_TABLE[clip(q + uv_dc, 117)]),
                i32::from(AC_TABLE[clip(q + uv_ac, 127)]),
            ],
        };
    }
    // Whether the frame updates probabilities, which a lone key frame
    // ignores.
    header.flag();
    let mut bands: Bands = COEFFS_PROBA;
    for (t, plane) in bands.iter_mut().enumerate() {
        for (b, band) in plane.iter_mut().enumerate() {
            for (c, context) in band.iter_mut().enumerate() {
                for (p, probability) in context.iter_mut().enumerate() {
                    if header.bit(COEFFS_UPDATE_PROBA[t][b][c][p]) {
                        *probability = header.value(8) as u8;
                    }
                }
            }
        }
    }
    let skip_probability = header.flag().then(|| header.value(8) as u8);

    let mut strengths = [[Strength::default(); 2]; 4];
    if filter_type > 0 {
        for (segment, strengths) in strengths.iter_mut().enumerate() {
            let base = if segmented {
                segment_filter[segment] + if absolute { 0 } else { level }
            } else {
                level
            };
            for (i4x4, strength) in strengths.iter_mut().enumerate() {
                let mut level = base + reference_delta;
                if i4x4 == 1 {
                    level += mode_delta;
                }
                let level = level.clamp(0, 63);
                if level > 0 {
                    let mut inner_level = level;
                    if sharpness > 0 {
                        inner_level >>= if sharpness > 4 { 2 } else { 1 };
                        inner_level = inner_level.min(9 - sharpness);
                    }
                    let inner_level = inner_level.max(1);
                    strength.limit = 2 * level + inner_level;
                    strength.inner_level = inner_level;
                    strength.hev_threshold = if level >= 40 {
                        2
                    } else if level >= 15 {
                        1
                    } else {
                        0
                    };
                }
                strength.inner = i4x4 == 1;
            }
        }
    }

    let mut frame = Frame::new(width, height);
    let mut top_modes = vec![DC_PRED; 4 * frame.mb_width];
    let mut top_nz = vec![(0_u8, 0_u8); frame.mb_width];
    for mb_y in 0..frame.mb_height {
        let mut left_modes = [DC_PRED; 4];
        let mut left_nz = (0_u8, 0_u8);
        let mut blocks = Vec::with_capacity(frame.mb_width);
        for modes in top_modes.chunks_mut(4) {
            let segment = if update_map {
                if header.bit(segment_probabilities[0]) {
                    2 + usize::from(header.bit(segment_probabilities[2]))
                } else {
                    usize::from(header.bit(segment_probabilities[1]))
                }
            } else {
                0
            };
            let skip = skip_probability.is_some_and(|probability| header.bit(probability));
            blocks.push(parse_modes(
                &mut header,
                modes,
                &mut left_modes,
                segment,
                skip,
            ));
        }
        if header.eof {
            return None;
        }
        let token = &mut tokens[mb_y & (count - 1)];
        for (mb_x, block) in blocks.iter_mut().enumerate() {
            let skip = if block.skip {
                left_nz.0 = 0;
                top_nz[mb_x].0 = 0;
                if !block.i4x4 {
                    left_nz.1 = 0;
                    top_nz[mb_x].1 = 0;
                }
                true
            } else {
                parse_residuals(
                    token,
                    &bands,
                    &quants[block.segment],
                    block,
                    &mut top_nz[mb_x],
                    &mut left_nz,
                )
            };
            if filter_type > 0 {
                let mut strength = strengths[block.segment][usize::from(block.i4x4)];
                strength.inner |= !skip;
                frame.strengths[mb_y * frame.mb_width + mb_x] = strength;
            }
            if token.eof {
                return None;
            }
        }
        frame.reconstruct_row(mb_y, &blocks);
        if filter_type > 0 {
            for mb_x in 0..frame.mb_width {
                frame.filter(mb_x, mb_y, filter_type == 1);
            }
        }
    }
    let alpha = match alpha {
        Some(data) => Some(super::alpha_plane(data, (width, height))?),
        None => None,
    };
    Some(((width as u32, height as u32), frame.rgba(alpha.as_deref())))
}

/// Reads a macroblock's prediction modes, after its segment and skip flag.
fn parse_modes(
    reader: &mut BoolReader<'_>,
    top: &mut [u8],
    left: &mut [u8; 4],
    segment: usize,
    skip: bool,
) -> Block {
    let mut block = Block {
        segment,
        skip,
        i4x4: !reader.bit(145),
        modes: [DC_PRED; 16],
        uv_mode: DC_PRED,
        coeffs: [0; 384],
        non_zero_y: 0,
        non_zero_uv: 0,
    };
    if !block.i4x4 {
        let mode = if reader.bit(156) {
            if reader.bit(128) { TM_PRED } else { H_PRED }
        } else if reader.bit(163) {
            V_PRED
        } else {
            DC_PRED
        };
        block.modes[0] = mode;
        top.fill(mode);
        left.fill(mode);
    } else {
        for (y, left) in left.iter_mut().enumerate() {
            let mut mode = *left;
            for (x, top) in top.iter_mut().enumerate() {
                let p = &BMODES_PROBA[usize::from(*top)][usize::from(mode)];
                mode = if !reader.bit(p[0]) {
                    DC_PRED
                } else if !reader.bit(p[1]) {
                    TM_PRED
                } else if !reader.bit(p[2]) {
                    V_PRED
                } else if !reader.bit(p[3]) {
                    if !reader.bit(p[4]) {
                        H_PRED
                    } else if !reader.bit(p[5]) {
                        B_RD_PRED
                    } else {
                        B_VR_PRED
                    }
                } else if !reader.bit(p[6]) {
                    B_LD_PRED
                } else if !reader.bit(p[7]) {
                    B_VL_PRED
                } else if !reader.bit(p[8]) {
                    B_HD_PRED
                } else {
                    B_HU_PRED
                };
                *top = mode;
                block.modes[y * 4 + x] = mode;
            }
            *left = mode;
        }
    }
    block.uv_mode = if !reader.bit(142) {
        DC_PRED
    } else if !reader.bit(114) {
        V_PRED
    } else if reader.bit(183) {
        TM_PRED
    } else {
        H_PRED
    };
    block
}

/// A coefficient of magnitude 2 or more, after the token tree's first
/// branches.
fn large_value(reader: &mut BoolReader<'_>, p: &[u8; 11]) -> i32 {
    if !reader.bit(p[3]) {
        if !reader.bit(p[4]) {
            2
        } else {
            3 + i32::from(reader.bit(p[5]))
        }
    } else if !reader.bit(p[6]) {
        if !reader.bit(p[7]) {
            5 + i32::from(reader.bit(159))
        } else {
            7 + 2 * i32::from(reader.bit(165)) + i32::from(reader.bit(145))
        }
    } else {
        let high = usize::from(reader.bit(p[8]));
        let category = 2 * high + usize::from(reader.bit(p[9 + high]));
        let extra = CATEGORIES[category].iter().fold(0, |value, &probability| {
            2 * value + i32::from(reader.bit(probability))
        });
        extra + 3 + (8 << category)
    }
}

/// Reads a 4x4 block's coefficients from the `n`th, dequantized, into
/// `out`; returns the position after the last that is not zero.
fn coefficients(
    reader: &mut BoolReader<'_>,
    bands: &[[[u8; 11]; 3]; 8],
    context: usize,
    quant: [i32; 2],
    mut n: usize,
    out: &mut [i16],
) -> usize {
    let mut p = &bands[BANDS[n]][context];
    while n < 16 {
        if !reader.bit(p[0]) {
            return n;
        }
        while !reader.bit(p[1]) {
            n += 1;
            if n == 16 {
                return 16;
            }
            p = &bands[BANDS[n]][0];
        }
        let next = &bands[BANDS[n + 1]];
        let value = if !reader.bit(p[2]) {
            p = &next[1];
            1
        } else {
            let value = large_value(reader, p);
            p = &next[2];
            value
        };
        out[ZIGZAG[n]] = (reader.signed(value) * quant[usize::from(n > 0)]) as i16;
        n += 1;
    }
    16
}

/// Two bits for a 4x4 block's coefficients: past the first three, past the
/// first, or whether the first is not zero.
fn nz_code(codes: u32, nz: usize, dc: bool) -> u32 {
    codes << 2
        | if nz > 3 {
            3
        } else if nz > 1 {
            2
        } else {
            u32::from(dc)
        }
}

/// Reads a macroblock's coefficients; returns whether they are all zero.
/// The `nz` pairs hold, for the macroblocks above and to the left, which
/// edge blocks had coefficients, and whether the whole-block DC had.
fn parse_residuals(
    reader: &mut BoolReader<'_>,
    bands: &Bands,
    quant: &Quant,
    block: &mut Block,
    top: &mut (u8, u8),
    left: &mut (u8, u8),
) -> bool {
    let coeffs = &mut block.coeffs;
    let mut at = 0;
    let (first, ac_type) = if block.i4x4 {
        (0, 3)
    } else {
        let mut dc = [0_i16; 16];
        let context = usize::from(top.1 + left.1);
        let nz = coefficients(reader, &bands[1], context, quant.y2, 0, &mut dc);
        top.1 = u8::from(nz > 0);
        left.1 = top.1;
        if nz > 1 {
            transform_wht(&dc, coeffs);
        } else {
            let dc0 = ((i32::from(dc[0]) + 3) >> 3) as i16;
            for luma in coeffs.chunks_mut(16).take(16) {
                luma[0] = dc0;
            }
        }
        (1, 0)
    };
    let mut tnz = u32::from(top.0 & 0x0f);
    let mut lnz = u32::from(left.0 & 0x0f);
    let mut non_zero_y = 0;
    for _ in 0..4 {
        let mut l = lnz & 1;
        let mut codes = 0;
        for _ in 0..4 {
            let context = (l + (tnz & 1)) as usize;
            let nz = coefficients(
                reader,
                &bands[ac_type],
                context,
                quant.y1,
                first,
                &mut coeffs[at..at + 16],
            );
            l = u32::from(nz > first);
            tnz = (tnz >> 1) | (l << 7);
            codes = nz_code(codes, nz, coeffs[at] != 0);
            at += 16;
        }
        tnz >>= 4;
        lnz = (lnz >> 1) | (l << 7);
        non_zero_y = (non_zero_y << 8) | codes;
    }
    let mut out_top = tnz;
    let mut out_left = lnz >> 4;
    let mut non_zero_uv = 0;
    for channel in [0, 2] {
        let mut codes = 0;
        tnz = u32::from(top.0) >> (4 + channel);
        lnz = u32::from(left.0) >> (4 + channel);
        for _ in 0..2 {
            let mut l = lnz & 1;
            for _ in 0..2 {
                let context = (l + (tnz & 1)) as usize;
                let nz = coefficients(
                    reader,
                    &bands[2],
                    context,
                    quant.uv,
                    0,
                    &mut coeffs[at..at + 16],
                );
                l = u32::from(nz > 0);
                tnz = (tnz >> 1) | (l << 3);
                codes = nz_code(codes, nz, coeffs[at] != 0);
                at += 16;
            }
            tnz >>= 2;
            lnz = (lnz >> 1) | (l << 5);
        }
        non_zero_uv |= codes << (4 * channel);
        out_top |= (tnz << 4) << channel;
        out_left |= (lnz & 0xf0) << channel;
    }
    top.0 = out_top as u8;
    left.0 = out_left as u8;
    block.non_zero_y = non_zero_y;
    block.non_zero_uv = non_zero_uv;
    non_zero_y | non_zero_uv == 0
}

/// The inverse Walsh-Hadamard transform of the whole-block DC coefficients
/// into the DC of each 4x4 block.
fn transform_wht(input: &[i16; 16], out: &mut [i16; 384]) {
    let mut tmp = [0_i32; 16];
    for i in 0..4 {
        let a0 = i32::from(input[i]) + i32::from(input[12 + i]);
        let a1 = i32::from(input[4 + i]) + i32::from(input[8 + i]);
        let a2 = i32::from(input[4 + i]) - i32::from(input[8 + i]);
        let a3 = i32::from(input[i]) - i32::from(input[12 + i]);
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3;
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        out[64 * i] = ((a0 + a1) >> 3) as i16;
        out[64 * i + 16] = ((a3 + a2) >> 3) as i16;
        out[64 * i + 32] = ((a0 - a1) >> 3) as i16;
        out[64 * i + 48] = ((a3 - a2) >> 3) as i16;
    }
}

fn mul1(a: i32) -> i32 {
    ((a * 20_091) >> 16) + a
}

fn mul2(a: i32) -> i32 {
    (a * 35_468) >> 16
}

/// Adds a residual, scaled down, to a predicted sample.
fn store(work: &mut [u8], at: usize, value: i32) {
    work[at] = (i32::from(work[at]) + (value >> 3)).clamp(0, 255) as u8;
}

/// The inverse DCT of a 4x4 block, added to its prediction.
fn transform(input: &[i16], work: &mut [u8], dst: usize) {
    let input = |i: usize| i32::from(input[i]);
    let mut tmp = [0_i32; 16];
    for i in 0..4 {
        let a = input(i) + input(8 + i);
        let b = input(i) - input(8 + i);
        let c = mul2(input(4 + i)) - mul1(input(12 + i));
        let d = mul1(input(4 + i)) + mul2(input(12 + i));
        tmp[4 * i] = a + d;
        tmp[4 * i + 1] = b + c;
        tmp[4 * i + 2] = b - c;
        tmp[4 * i + 3] = a - d;
    }
    for i in 0..4 {
        let dc = tmp[i] + 4;
        let a = dc + tmp[8 + i];
        let b = dc - tmp[8 + i];
        let c = mul2(tmp[4 + i]) - mul1(tmp[12 + i]);
        let d = mul1(tmp[4 + i]) + mul2(tmp[12 + i]);
        let row = dst + i * BPS;
        store(work, row, a + d);
        store(work, row + 1, b + c);
        store(work, row + 2, b - c);
        store(work, row + 3, a - d);
    }
}

/// The transform of a block with only its first three coefficients.
fn transform_ac3(input: &[i16], work: &mut [u8], dst: usize) {
    let a = i32::from(input[0]) + 4;
    let (c4, d4) = (mul2(i32::from(input[4])), mul1(i32::from(input[4])));
    let (c1, d1) = (mul2(i32::from(input[1])), mul1(i32::from(input[1])));
    for (y, dc) in [a + d4, a + c4, a - c4, a - d4].into_iter().enumerate() {
        let row = dst + y * BPS;
        store(work, row, dc + d1);
        store(work, row + 1, dc + c1);
        store(work, row + 2, dc - c1);
        store(work, row + 3, dc - d1);
    }
}

/// The transform of a block with only a DC coefficient.
fn transform_dc(input: &[i16], work: &mut [u8], dst: usize) {
    let dc = i32::from(input[0]) + 4;
    for y in 0..4 {
        for x in 0..4 {
            store(work, dst + y * BPS + x, dc);
        }
    }
}

/// Transforms a luma block by what its two bits of `bits`, the highest,
/// say it holds.
fn transform_luma(bits: u32, input: &[i16], work: &mut [u8], dst: usize) {
    match bits >> 30 {
        3 => transform(input, work, dst),
        2 => transform_ac3(input, work, dst),
        1 => transform_dc(input, work, dst),
        _ => {}
    }
}

/// Transforms a chroma plane's four blocks.
fn transform_chroma(bits: u32, input: &[i16], work: &mut [u8], dst: usize) {
    if bits & 0xff == 0 {
        return;
    }
    let offsets = [0, 4, 4 * BPS, 4 * BPS + 4];
    for (block, offset) in input.chunks(16).zip(offsets) {
        if bits & 0xaa != 0 {
            transform(block, work, dst + offset);
        } else if block[0] != 0 {
            transform_dc(block, work, dst + offset);
        }
    }
}

/// The mean of three samples, weighted to the middle one.
fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u32::from(a) + 2 * u32::from(b) + u32::from(c) + 2) >> 2) as u8
}

fn avg2(a: u8, b: u8) -> u8 {
    ((u32::from(a) + u32::from(b) + 1) >> 1) as u8
}

/// Fills a `size` square with one value.
fn fill(work: &mut [u8], dst: usize, size: usize, value: u8) {
    for y in 0..size {
        work[dst + y * BPS..dst + y * BPS + size].fill(value);
    }
}

/// Predicts a square from the row above and the column to the left.
fn predict(work: &mut [u8], dst: usize, size: usize, mode: u8) {
    let top = dst - BPS;
    let shift = size.trailing_zeros();
    match mode {
        TM_PRED => {
            let top_left = i32::from(work[top - 1]);
            for y in 0..size {
                let left = i32::from(work[dst + y * BPS - 1]);
                for x in 0..size {
                    work[dst + y * BPS + x] =
                        (i32::from(work[top + x]) + left - top_left).clamp(0, 255) as u8;
                }
            }
        }
        V_PRED => {
            for y in 0..size {
                work.copy_within(top..top + size, dst + y * BPS);
            }
        }
        H_PRED => {
            for y in 0..size {
                let left = work[dst + y * BPS - 1];
                work[dst + y * BPS..dst + y * BPS + size].fill(left);
            }
        }
        DC_PRED_NO_TOP_LEFT => fill(work, dst, size, 0x80),
        _ => {
            let top_sum: u32 = (0..size).map(|x| u32::from(work[top + x])).sum();
            let left_sum: u32 = (0..size).map(|y| u32::from(work[dst + y * BPS - 1])).sum();
            let value = match mode {
                DC_PRED_NO_TOP => (left_sum + (1 << (shift - 1))) >> shift,
                DC_PRED_NO_LEFT => (top_sum + (1 << (shift - 1))) >> shift,
                _ => (top_sum + left_sum + (1 << shift)) >> (shift + 1),
            };
            fill(work, dst, size, value as u8);
        }
    }
}

/// Predicts a 4x4 luma block.
fn predict4(work: &mut [u8], dst: usize, mode: u8) {
    let top = dst - BPS;
    let at = |x: usize, y: usize| dst + x + y * BPS;
    // The row above, from the top left corner to four pixels past the
    // block, and the column to the left.
    let [x0, a, b, c, d, e, f, g, h] = core::array::from_fn(|i| work[top + i - 1]);
    let [i, j, k, l] = core::array::from_fn(|y| work[dst + y * BPS - 1]);
    let mut set = |points: &[(usize, usize)], value: u8| {
        for &(x, y) in points {
            work[at(x, y)] = value;
        }
    };
    match mode {
        DC_PRED => {
            let sum: u32 = [a, b, c, d, i, j, k, l].into_iter().map(u32::from).sum();
            fill(work, dst, 4, ((sum + 4) >> 3) as u8);
        }
        TM_PRED => predict(work, dst, 4, TM_PRED),
        V_PRED => {
            let values = [avg3(x0, a, b), avg3(a, b, c), avg3(b, c, d), avg3(c, d, e)];
            for y in 0..4 {
                work[at(0, y)..at(0, y) + 4].copy_from_slice(&values);
            }
        }
        H_PRED => {
            for (y, value) in [avg3(x0, i, j), avg3(i, j, k), avg3(j, k, l), avg3(k, l, l)]
                .into_iter()
                .enumerate()
            {
                work[at(0, y)..at(0, y) + 4].fill(value);
            }
        }
        B_RD_PRED => {
            set(&[(0, 3)], avg3(j, k, l));
            set(&[(1, 3), (0, 2)], avg3(i, j, k));
            set(&[(2, 3), (1, 2), (0, 1)], avg3(x0, i, j));
            set(&[(3, 3), (2, 2), (1, 1), (0, 0)], avg3(a, x0, i));
            set(&[(3, 2), (2, 1), (1, 0)], avg3(b, a, x0));
            set(&[(3, 1), (2, 0)], avg3(c, b, a));
            set(&[(3, 0)], avg3(d, c, b));
        }
        B_LD_PRED => {
            set(&[(0, 0)], avg3(a, b, c));
            set(&[(1, 0), (0, 1)], avg3(b, c, d));
            set(&[(2, 0), (1, 1), (0, 2)], avg3(c, d, e));
            set(&[(3, 0), (2, 1), (1, 2), (0, 3)], avg3(d, e, f));
            set(&[(3, 1), (2, 2), (1, 3)], avg3(e, f, g));
            set(&[(3, 2), (2, 3)], avg3(f, g, h));
            set(&[(3, 3)], avg3(g, h, h));
        }
        B_VR_PRED => {
            set(&[(0, 0), (1, 2)], avg2(x0, a));
            set(&[(1, 0), (2, 2)], avg2(a, b));
            set(&[(2, 0), (3, 2)], avg2(b, c));
            set(&[(3, 0)], avg2(c, d));
            set(&[(0, 3)], avg3(k, j, i));
            set(&[(0, 2)], avg3(j, i, x0));
            set(&[(0, 1), (1, 3)], avg3(i, x0, a));
            set(&[(1, 1), (2, 3)], avg3(x0, a, b));
            set(&[(2, 1), (3, 3)], avg3(a, b, c));
            set(&[(3, 1)], avg3(b, c, d));
        }
        B_VL_PRED => {
            set(&[(0, 0)], avg2(a, b));
            set(&[(1, 0), (0, 2)], avg2(b, c));
            set(&[(2, 0), (1, 2)], avg2(c, d));
            set(&[(3, 0), (2, 2)], avg2(d, e));
            set(&[(0, 1)], avg3(a, b, c));
            set(&[(1, 1), (0, 3)], avg3(b, c, d));
            set(&[(2, 1), (1, 3)], avg3(c, d, e));
            set(&[(3, 1), (2, 3)], avg3(d, e, f));
            set(&[(3, 2)], avg3(e, f, g));
            set(&[(3, 3)], avg3(f, g, h));
        }
        B_HU_PRED => {
            set(&[(0, 0)], avg2(i, j));
            set(&[(2, 0), (0, 1)], avg2(j, k));
            set(&[(2, 1), (0, 2)], avg2(k, l));
            set(&[(1, 0)], avg3(i, j, k));
            set(&[(3, 0), (1, 1)], avg3(j, k, l));
            set(&[(3, 1), (1, 2)], avg3(k, l, l));
            set(&[(3, 2), (2, 2), (0, 3), (1, 3), (2, 3), (3, 3)], l);
        }
        _ => {
            // Horizontal-down.
            set(&[(0, 0), (2, 1)], avg2(i, x0));
            set(&[(0, 1), (2, 2)], avg2(j, i));
            set(&[(0, 2), (2, 3)], avg2(k, j));
            set(&[(0, 3)], avg2(l, k));
            set(&[(3, 0)], avg3(a, b, c));
            set(&[(2, 0)], avg3(x0, a, b));
            set(&[(1, 0), (3, 1)], avg3(i, x0, a));
            set(&[(1, 1), (3, 2)], avg3(j, i, x0));
            set(&[(1, 2), (3, 3)], avg3(k, j, i));
            set(&[(1, 3)], avg3(l, k, j));
        }
    }
}

/// DC prediction without the edges a macroblock at the frame's top or left
/// lacks.
fn edge_mode(mb_x: usize, mb_y: usize, mode: u8) -> u8 {
    match (mode, mb_x, mb_y) {
        (DC_PRED, 0, 0) => DC_PRED_NO_TOP_LEFT,
        (DC_PRED, 0, _) => DC_PRED_NO_LEFT,
        (DC_PRED, _, 0) => DC_PRED_NO_TOP,
        _ => mode,
    }
}

/// The decoded planes, in whole macroblocks.
struct Frame {
    width: usize,
    height: usize,
    mb_width: usize,
    mb_height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    /// Where macroblocks are predicted, before the loop filter.
    work: [u8; WORK_SIZE],
    tops: Vec<Top>,
    strengths: Vec<Strength>,
}

impl Frame {
    fn new(width: usize, height: usize) -> Self {
        let (mb_width, mb_height) = (width.div_ceil(16), height.div_ceil(16));
        Self {
            width,
            height,
            mb_width,
            mb_height,
            y: vec![0; mb_width * 16 * mb_height * 16],
            u: vec![0; mb_width * 8 * mb_height * 8],
            v: vec![0; mb_width * 8 * mb_height * 8],
            work: [0; WORK_SIZE],
            tops: vec![
                Top {
                    y: [0; 16],
                    u: [0; 8],
                    v: [0; 8]
                };
                mb_width
            ],
            strengths: vec![Strength::default(); mb_width * mb_height],
        }
    }

    /// Predicts a row of macroblocks, adds their residuals and stores them
    /// in the planes, as libwebp's `ReconstructRow`: predictions read the
    /// samples before the loop filter.
    fn reconstruct_row(&mut self, mb_y: usize, blocks: &[Block]) {
        let work = &mut self.work;
        // The column left of the frame is 129, the row above it 127.
        for j in 0..16 {
            work[Y_OFF + j * BPS - 1] = 129;
        }
        for j in 0..8 {
            work[U_OFF + j * BPS - 1] = 129;
            work[V_OFF + j * BPS - 1] = 129;
        }
        if mb_y > 0 {
            work[Y_OFF - BPS - 1] = 129;
            work[U_OFF - BPS - 1] = 129;
            work[V_OFF - BPS - 1] = 129;
        } else {
            work[Y_OFF - BPS - 1..Y_OFF - BPS + 20].fill(127);
            work[U_OFF - BPS - 1..U_OFF - BPS + 8].fill(127);
            work[V_OFF - BPS - 1..V_OFF - BPS + 8].fill(127);
        }
        let y_stride = self.mb_width * 16;
        let uv_stride = self.mb_width * 8;
        for (mb_x, block) in blocks.iter().enumerate() {
            // The left samples come from the macroblock before.
            if mb_x > 0 {
                for j in 0..17 {
                    let row = Y_OFF + j * BPS - BPS;
                    work.copy_within(row + 12..row + 16, row - 4);
                }
                for j in 0..9 {
                    for plane in [U_OFF, V_OFF] {
                        let row = plane + j * BPS - BPS;
                        work.copy_within(row + 4..row + 8, row - 4);
                    }
                }
            }
            let top = self.tops[mb_x];
            if mb_y > 0 {
                work[Y_OFF - BPS..Y_OFF - BPS + 16].copy_from_slice(&top.y);
                work[U_OFF - BPS..U_OFF - BPS + 8].copy_from_slice(&top.u);
                work[V_OFF - BPS..V_OFF - BPS + 8].copy_from_slice(&top.v);
            }
            if block.i4x4 {
                // The four pixels above and to the right, which the blocks of
                // the right column below the first take too.
                let top_right = Y_OFF - BPS + 16;
                if mb_y > 0 {
                    let pixels = match self.tops.get(mb_x + 1) {
                        Some(next) => [next.y[0], next.y[1], next.y[2], next.y[3]],
                        None => [top.y[15]; 4],
                    };
                    work[top_right..top_right + 4].copy_from_slice(&pixels);
                }
                for k in 1..4 {
                    work.copy_within(top_right..top_right + 4, top_right + k * 4 * BPS);
                }
                let mut bits = block.non_zero_y;
                for (n, (coeffs, offset)) in block.coeffs.chunks(16).zip(SCAN).enumerate() {
                    let dst = Y_OFF + offset;
                    predict4(work, dst, block.modes[n]);
                    transform_luma(bits, coeffs, work, dst);
                    bits <<= 2;
                }
            } else {
                predict(work, Y_OFF, 16, edge_mode(mb_x, mb_y, block.modes[0]));
                let mut bits = block.non_zero_y;
                if bits != 0 {
                    for (coeffs, offset) in block.coeffs.chunks(16).zip(SCAN) {
                        transform_luma(bits, coeffs, work, Y_OFF + offset);
                        bits <<= 2;
                    }
                }
            }
            let mode = edge_mode(mb_x, mb_y, block.uv_mode);
            predict(work, U_OFF, 8, mode);
            predict(work, V_OFF, 8, mode);
            transform_chroma(block.non_zero_uv, &block.coeffs[256..320], work, U_OFF);
            transform_chroma(block.non_zero_uv >> 8, &block.coeffs[320..384], work, V_OFF);
            if mb_y < self.mb_height - 1 {
                let top = &mut self.tops[mb_x];
                top.y
                    .copy_from_slice(&work[Y_OFF + 15 * BPS..Y_OFF + 15 * BPS + 16]);
                top.u
                    .copy_from_slice(&work[U_OFF + 7 * BPS..U_OFF + 7 * BPS + 8]);
                top.v
                    .copy_from_slice(&work[V_OFF + 7 * BPS..V_OFF + 7 * BPS + 8]);
            }
            for j in 0..16 {
                let out = (mb_y * 16 + j) * y_stride + mb_x * 16;
                self.y[out..out + 16].copy_from_slice(&work[Y_OFF + j * BPS..Y_OFF + j * BPS + 16]);
            }
            for j in 0..8 {
                let out = (mb_y * 8 + j) * uv_stride + mb_x * 8;
                self.u[out..out + 8].copy_from_slice(&work[U_OFF + j * BPS..U_OFF + j * BPS + 8]);
                self.v[out..out + 8].copy_from_slice(&work[V_OFF + j * BPS..V_OFF + j * BPS + 8]);
            }
        }
    }

    /// Smooths a macroblock's edges with the loop filter.
    fn filter(&mut self, mb_x: usize, mb_y: usize, simple: bool) {
        let strength = self.strengths[mb_y * self.mb_width + mb_x];
        if strength.limit == 0 {
            return;
        }
        let (limit, inner, hev) = (strength.limit, strength.inner_level, strength.hev_threshold);
        let y_stride = self.mb_width * 16;
        let at = mb_y * 16 * y_stride + mb_x * 16;
        if simple {
            if mb_x > 0 {
                simple_filter(&mut self.y, at, 1, y_stride, limit + 4);
            }
            if strength.inner {
                for k in 1..4 {
                    simple_filter(&mut self.y, at + 4 * k, 1, y_stride, limit);
                }
            }
            if mb_y > 0 {
                simple_filter(&mut self.y, at, y_stride, 1, limit + 4);
            }
            if strength.inner {
                for k in 1..4 {
                    simple_filter(&mut self.y, at + 4 * k * y_stride, y_stride, 1, limit);
                }
            }
            return;
        }
        let uv_stride = self.mb_width * 8;
        let uv_at = mb_y * 8 * uv_stride + mb_x * 8;
        let thresholds = |limit| (limit, inner, hev);
        if mb_x > 0 {
            filter_edge(
                &mut self.y,
                at,
                1,
                y_stride,
                16,
                thresholds(limit + 4),
                true,
            );
            filter_edge(
                &mut self.u,
                uv_at,
                1,
                uv_stride,
                8,
                thresholds(limit + 4),
                true,
            );
            filter_edge(
                &mut self.v,
                uv_at,
                1,
                uv_stride,
                8,
                thresholds(limit + 4),
                true,
            );
        }
        if strength.inner {
            for k in 1..4 {
                filter_edge(
                    &mut self.y,
                    at + 4 * k,
                    1,
                    y_stride,
                    16,
                    thresholds(limit),
                    false,
                );
            }
            filter_edge(
                &mut self.u,
                uv_at + 4,
                1,
                uv_stride,
                8,
                thresholds(limit),
                false,
            );
            filter_edge(
                &mut self.v,
                uv_at + 4,
                1,
                uv_stride,
                8,
                thresholds(limit),
                false,
            );
        }
        if mb_y > 0 {
            filter_edge(
                &mut self.y,
                at,
                y_stride,
                1,
                16,
                thresholds(limit + 4),
                true,
            );
            filter_edge(
                &mut self.u,
                uv_at,
                uv_stride,
                1,
                8,
                thresholds(limit + 4),
                true,
            );
            filter_edge(
                &mut self.v,
                uv_at,
                uv_stride,
                1,
                8,
                thresholds(limit + 4),
                true,
            );
        }
        if strength.inner {
            for k in 1..4 {
                filter_edge(
                    &mut self.y,
                    at + 4 * k * y_stride,
                    y_stride,
                    1,
                    16,
                    thresholds(limit),
                    false,
                );
            }
            filter_edge(
                &mut self.u,
                uv_at + 4 * uv_stride,
                uv_stride,
                1,
                8,
                thresholds(limit),
                false,
            );
            filter_edge(
                &mut self.v,
                uv_at + 4 * uv_stride,
                uv_stride,
                1,
                8,
                thresholds(limit),
                false,
            );
        }
    }

    /// The frame's pixels as RGBA, chroma upsampled as libwebp's
    /// `EmitFancyRGB`: each output row pairs with the chroma rows around it.
    fn rgba(&self, alpha: Option<&[u8]>) -> Vec<u8> {
        let (width, height) = (self.width, self.height);
        let y_stride = self.mb_width * 16;
        let uv_stride = self.mb_width * 8;
        let mut out = vec![0_u8; width * height * 4];
        let luma = |row: usize| &self.y[row * y_stride..row * y_stride + width];
        let chroma = |row: usize| (&self.u[row * uv_stride..], &self.v[row * uv_stride..]);
        upsample(
            &mut out,
            width,
            (0, None),
            (luma(0), None),
            chroma(0),
            chroma(0),
        );
        let mut row = 1;
        while row + 1 < height {
            let (top, bottom) = ((row - 1) / 2, row.div_ceil(2));
            upsample(
                &mut out,
                width,
                (row, Some(row + 1)),
                (luma(row), Some(luma(row + 1))),
                chroma(top),
                chroma(bottom),
            );
            row += 2;
        }
        if height.is_multiple_of(2) {
            let last = (height - 1) / 2;
            upsample(
                &mut out,
                width,
                (height - 1, None),
                (luma(height - 1), None),
                chroma(last),
                chroma(last),
            );
        }
        for (index, pixel) in out.chunks_mut(4).enumerate() {
            pixel[3] = alpha.map_or(255, |alpha| alpha[index]);
        }
        out
    }
}

/// libwebp's `MultHi`: 8 bits of fraction.
fn mult_hi(value: u8, coefficient: i32) -> i32 {
    (i32::from(value) * coefficient) >> 8
}

/// Drops the 6 bits of fraction, clamping.
fn clip8(value: i32) -> u8 {
    if value & !16383 == 0 {
        (value >> 6) as u8
    } else if value < 0 {
        0
    } else {
        255
    }
}

/// An RGB pixel from YUV, as libwebp's BT.601 conversion.
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let luma = mult_hi(y, 19_077);
    [
        clip8(luma + mult_hi(v, 26_149) - 14_234),
        clip8(luma - mult_hi(u, 6_419) - mult_hi(v, 13_320) + 8_708),
        clip8(luma + mult_hi(u, 33_050) - 17_685),
    ]
}

/// Converts one or two rows to RGB, as libwebp's fancy upsampler: each
/// chroma sample counts 9/16 for its own pixels, 3/16 for the pixels beside
/// and below it, and 1/16 diagonally.
fn upsample(
    out: &mut [u8],
    width: usize,
    (top_row, bottom_row): (usize, Option<usize>),
    (top_y, bottom_y): (&[u8], Option<&[u8]>),
    (top_u, top_v): (&[u8], &[u8]),
    (cur_u, cur_v): (&[u8], &[u8]),
) {
    let load = |u: u8, v: u8| u32::from(u) | u32::from(v) << 16;
    let mut put = |row: usize, x: usize, y: u8, uv: u32| {
        let at = (row * width + x) * 4;
        out[at..at + 3].copy_from_slice(&yuv_to_rgb(y, (uv & 0xff) as u8, (uv >> 16) as u8));
    };
    let mut top_left = load(top_u[0], top_v[0]);
    let mut left = load(cur_u[0], cur_v[0]);
    put(
        top_row,
        0,
        top_y[0],
        (3 * top_left + left + 0x0002_0002) >> 2,
    );
    if let (Some(row), Some(y)) = (bottom_row, bottom_y) {
        put(row, 0, y[0], (3 * left + top_left + 0x0002_0002) >> 2);
    }
    for x in 1..=(width - 1) >> 1 {
        let top = load(top_u[x], top_v[x]);
        let current = load(cur_u[x], cur_v[x]);
        let sum = top_left + top + left + current + 0x0008_0008;
        let diagonal_12 = (sum + 2 * (top + left)) >> 3;
        let diagonal_03 = (sum + 2 * (top_left + current)) >> 3;
        put(
            top_row,
            2 * x - 1,
            top_y[2 * x - 1],
            (diagonal_12 + top_left) >> 1,
        );
        put(top_row, 2 * x, top_y[2 * x], (diagonal_03 + top) >> 1);
        if let (Some(row), Some(y)) = (bottom_row, bottom_y) {
            put(row, 2 * x - 1, y[2 * x - 1], (diagonal_03 + left) >> 1);
            put(row, 2 * x, y[2 * x], (diagonal_12 + current) >> 1);
        }
        top_left = top;
        left = current;
    }
    if width.is_multiple_of(2) {
        put(
            top_row,
            width - 1,
            top_y[width - 1],
            (3 * top_left + left + 0x0002_0002) >> 2,
        );
        if let (Some(row), Some(y)) = (bottom_row, bottom_y) {
            put(
                row,
                width - 1,
                y[width - 1],
                (3 * left + top_left + 0x0002_0002) >> 2,
            );
        }
    }
}

/// Whether the samples `step` apart across an edge differ little enough to
/// smooth.
fn needs_filter(p: &[u8], at: usize, step: usize, threshold: i32) -> bool {
    let sample = |offset: isize| i32::from(p[(at as isize + offset * step as isize) as usize]);
    4 * (sample(-1) - sample(0)).abs() + (sample(-2) - sample(1)).abs() <= threshold
}

/// Smooths the two samples nearest an edge.
fn filter2(p: &mut [u8], at: usize, step: usize) {
    let (p1, p0, q0, q1) = (
        i32::from(p[at - 2 * step]),
        i32::from(p[at - step]),
        i32::from(p[at]),
        i32::from(p[at + step]),
    );
    let a = 3 * (q0 - p0) + (p1 - q1).clamp(-128, 127);
    let a1 = ((a + 4) >> 3).clamp(-16, 15);
    let a2 = ((a + 3) >> 3).clamp(-16, 15);
    p[at - step] = (p0 + a2).clamp(0, 255) as u8;
    p[at] = (q0 - a1).clamp(0, 255) as u8;
}

/// Smooths four samples across an inner edge.
fn filter4(p: &mut [u8], at: usize, step: usize) {
    let (p1, p0, q0, q1) = (
        i32::from(p[at - 2 * step]),
        i32::from(p[at - step]),
        i32::from(p[at]),
        i32::from(p[at + step]),
    );
    let a = 3 * (q0 - p0);
    let a1 = ((a + 4) >> 3).clamp(-16, 15);
    let a2 = ((a + 3) >> 3).clamp(-16, 15);
    let a3 = (a1 + 1) >> 1;
    p[at - 2 * step] = (p1 + a3).clamp(0, 255) as u8;
    p[at - step] = (p0 + a2).clamp(0, 255) as u8;
    p[at] = (q0 - a1).clamp(0, 255) as u8;
    p[at + step] = (q1 - a3).clamp(0, 255) as u8;
}

/// Smooths six samples across a macroblock edge.
fn filter6(p: &mut [u8], at: usize, step: usize) {
    let sample = |offset: isize| i32::from(p[(at as isize + offset * step as isize) as usize]);
    let (p2, p1, p0, q0, q1, q2) = (
        sample(-3),
        sample(-2),
        sample(-1),
        sample(0),
        sample(1),
        sample(2),
    );
    let a = (3 * (q0 - p0) + (p1 - q1).clamp(-128, 127)).clamp(-128, 127);
    let a1 = (27 * a + 63) >> 7;
    let a2 = (18 * a + 63) >> 7;
    let a3 = (9 * a + 63) >> 7;
    p[at - 3 * step] = (p2 + a3).clamp(0, 255) as u8;
    p[at - 2 * step] = (p1 + a2).clamp(0, 255) as u8;
    p[at - step] = (p0 + a1).clamp(0, 255) as u8;
    p[at] = (q0 - a1).clamp(0, 255) as u8;
    p[at + step] = (q1 - a2).clamp(0, 255) as u8;
    p[at + 2 * step] = (q2 - a3).clamp(0, 255) as u8;
}

/// The simple filter along 16 samples of an edge: `step` across it, `along`
/// between the samples.
fn simple_filter(p: &mut [u8], at: usize, step: usize, along: usize, limit: i32) {
    for i in 0..16 {
        let at = at + i * along;
        if needs_filter(p, at, step, 2 * limit + 1) {
            filter2(p, at, step);
        }
    }
}

/// The normal filter along `size` samples of an edge: six samples smoothed
/// at macroblock edges, four inside, two where the edge is a real one.
fn filter_edge(
    p: &mut [u8],
    at: usize,
    step: usize,
    along: usize,
    size: usize,
    (limit, inner, hev): (i32, i32, i32),
    macroblock: bool,
) {
    for i in 0..size {
        let at = at + i * along;
        let sample = |offset: isize| i32::from(p[(at as isize + offset * step as isize) as usize]);
        let edge = 4 * (sample(-1) - sample(0)).abs() + (sample(-2) - sample(1)).abs();
        let smooth = [(-4, -3), (-3, -2), (-2, -1), (3, 2), (2, 1), (1, 0)]
            .into_iter()
            .all(|(a, b)| (sample(a) - sample(b)).abs() <= inner);
        if edge > 2 * limit + 1 || !smooth {
            continue;
        }
        let high_variance =
            (sample(-2) - sample(-1)).abs() > hev || (sample(1) - sample(0)).abs() > hev;
        if high_variance {
            filter2(p, at, step);
        } else if macroblock {
            filter6(p, at, step);
        } else {
            filter4(p, at, step);
        }
    }
}
