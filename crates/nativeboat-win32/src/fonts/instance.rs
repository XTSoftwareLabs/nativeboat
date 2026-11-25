//! Static instances of variable TrueType fonts. GDI shows a variable font
//! only as its default instance, so a page's variable web font asked for at
//! another weight would be thin or synthesized bold. Chromium draws such a
//! font at the weight asked for; here the font is instanced at that weight
//! as a static font GDI loads: the `gvar` deltas for the axis coordinates
//! move each glyph's points and advance, and the variation tables go.

#[allow(unused_imports)]
use crate::prelude::*;

/// Tables that only variable fonts use, which an instance leaves out.
const VARIATION_TABLES: [&[u8; 4]; 8] = [
    b"fvar", b"gvar", b"avar", b"HVAR", b"VVAR", b"MVAR", b"STAT", b"cvar",
];

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn i16_at(data: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn f2dot14(data: &[u8], at: usize) -> Option<f32> {
    Some(f32::from(i16_at(data, at)?) / 16384.0)
}

fn fixed(data: &[u8], at: usize) -> Option<f32> {
    Some(u32_at(data, at)? as i32 as f32 / 65536.0)
}

/// The tables of a TrueType-outlined font, by tag.
fn tables(data: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    if !matches!(data.get(..4)?, [0, 1, 0, 0] | b"true") {
        return None;
    }
    let count = usize::from(u16_at(data, 4)?);
    (0..count)
        .map(|index| {
            let record = 12 + 16 * index;
            let tag: [u8; 4] = data.get(record..record + 4)?.try_into().ok()?;
            let offset = u32_at(data, record + 8)? as usize;
            let length = u32_at(data, record + 12)? as usize;
            Some((tag, data.get(offset..offset + length)?))
        })
        .collect()
}

fn find<'a>(tables: &[([u8; 4], &'a [u8])], tag: &[u8; 4]) -> Option<&'a [u8]> {
    tables
        .iter()
        .find(|(name, _)| name == tag)
        .map(|(_, bytes)| *bytes)
}

/// A variable axis: its tag, and its minimum, default and maximum values.
type Axis = ([u8; 4], f32, f32, f32);

fn axes(fvar: &[u8]) -> Option<Vec<Axis>> {
    let start = usize::from(u16_at(fvar, 4)?);
    let count = usize::from(u16_at(fvar, 8)?);
    let size = usize::from(u16_at(fvar, 10)?);
    (0..count)
        .map(|index| {
            let at = start + index * size;
            Some((
                fvar.get(at..at + 4)?.try_into().ok()?,
                fixed(fvar, at + 4)?,
                fixed(fvar, at + 8)?,
                fixed(fvar, at + 12)?,
            ))
        })
        .collect()
}

/// The weights a variable TrueType font's `wght` axis spans, with its
/// default; `None` for other fonts, which cannot be instanced here.
pub(super) fn weight_axis(data: &[u8]) -> Option<(f32, f32, f32)> {
    let tables = tables(data)?;
    find(&tables, b"gvar")?;
    find(&tables, b"glyf")?;
    axes(find(&tables, b"fvar")?)?
        .into_iter()
        .find(|(tag, ..)| tag == b"wght")
        .map(|(_, minimum, default, maximum)| (minimum, default, maximum))
}

/// The normalized coordinates of a weight: each axis's value between -1
/// and 1 around its default, through the `avar` maps, at F2Dot14
/// precision as fonts store them.
fn coordinates(axes: &[Axis], avar: Option<&[u8]>, weight: f32) -> Option<Vec<f32>> {
    let mut coordinates: Vec<f32> = axes
        .iter()
        .map(|(tag, minimum, default, maximum)| {
            let value = if tag == b"wght" {
                weight.clamp(*minimum, *maximum)
            } else {
                *default
            };
            let normalized = if value < *default && default > minimum {
                (value - default) / (default - minimum)
            } else if value > *default && maximum > default {
                (value - default) / (maximum - default)
            } else {
                0.0
            };
            libm::roundf(normalized.clamp(-1.0, 1.0) * 16384.0) / 16384.0
        })
        .collect();
    if let Some(avar) = avar {
        let mut at = 8;
        for coordinate in coordinates.iter_mut() {
            let count = usize::from(u16_at(avar, at)?);
            let maps: Vec<(f32, f32)> = (0..count)
                .map(|index| {
                    let pair = at + 2 + 4 * index;
                    Some((f2dot14(avar, pair)?, f2dot14(avar, pair + 2)?))
                })
                .collect::<Option<_>>()?;
            at += 2 + 4 * count;
            if let Some(index) = maps.iter().position(|(from, _)| *from >= *coordinate) {
                let (from, to) = maps[index];
                *coordinate = if from == *coordinate || index == 0 {
                    to
                } else {
                    let (before, mapped) = maps[index - 1];
                    mapped + (to - mapped) * (*coordinate - before) / (from - before)
                };
                *coordinate = libm::roundf(*coordinate * 16384.0) / 16384.0;
            }
        }
    }
    Some(coordinates)
}

/// How much a tuple variation applies at the coordinates.
fn scalar(coordinates: &[f32], peak: &[f32], region: Option<(&[f32], &[f32])>) -> f32 {
    let mut scalar = 1.0;
    for (axis, coordinate) in coordinates.iter().enumerate() {
        let peak = peak[axis];
        if peak == 0.0 || *coordinate == peak {
            continue;
        }
        let (start, end) = match region {
            Some((starts, ends)) => (starts[axis], ends[axis]),
            None => (peak.min(0.0), peak.max(0.0)),
        };
        if *coordinate < start || *coordinate > end || *coordinate == 0.0 {
            return 0.0;
        }
        scalar *= if *coordinate < peak {
            (coordinate - start) / (peak - start)
        } else {
            (end - coordinate) / (end - peak)
        };
    }
    scalar
}

/// Packed point numbers: `None` for all points.
fn packed_points(data: &[u8], mut at: usize) -> Option<(Option<Vec<usize>>, usize)> {
    let first = *data.get(at)?;
    at += 1;
    if first == 0 {
        return Some((None, at));
    }
    let count = if first & 0x80 != 0 {
        let low = *data.get(at)?;
        at += 1;
        usize::from(first & 0x7F) << 8 | usize::from(low)
    } else {
        usize::from(first)
    };
    let mut points = Vec::with_capacity(count);
    let mut last = 0usize;
    while points.len() < count {
        let control = *data.get(at)?;
        at += 1;
        let words = control & 0x80 != 0;
        for _ in 0..=usize::from(control & 0x7F) {
            if points.len() == count {
                break;
            }
            let step = if words {
                at += 2;
                usize::from(u16_at(data, at - 2)?)
            } else {
                at += 1;
                usize::from(*data.get(at - 1)?)
            };
            last += step;
            points.push(last);
        }
    }
    Some((Some(points), at))
}

/// `count` packed deltas.
fn packed_deltas(data: &[u8], mut at: usize, count: usize) -> Option<(Vec<f32>, usize)> {
    let mut deltas = Vec::with_capacity(count);
    while deltas.len() < count {
        let control = *data.get(at)?;
        at += 1;
        for _ in 0..=usize::from(control & 0x3F) {
            if deltas.len() == count {
                break;
            }
            let delta = match control & 0xC0 {
                0x80 => 0.0,
                0x40 => {
                    at += 2;
                    f32::from(i16_at(data, at - 2)?)
                }
                0xC0 => {
                    at += 4;
                    u32_at(data, at - 4)? as i32 as f32
                }
                _ => {
                    at += 1;
                    f32::from(*data.get(at - 1)? as i8)
                }
            };
            deltas.push(delta);
        }
    }
    Some((deltas, at))
}

/// A glyph's outline as stored.
enum Outline<'a> {
    Empty,
    Simple {
        ends: Vec<usize>,
        instructions: &'a [u8],
        flags: Vec<u8>,
        points: Vec<(f32, f32)>,
    },
    Composite {
        components: Vec<Component<'a>>,
        instructions: &'a [u8],
    },
}

/// A component of a composite glyph.
struct Component<'a> {
    flags: u16,
    glyph: u16,
    /// Its offset, or the point numbers it is placed by.
    arguments: (f32, f32),
    /// The scale or matrix bytes that follow, as stored.
    transform: &'a [u8],
}

const ARGS_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY: u16 = 0x0002;
const HAS_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAS_XY_SCALE: u16 = 0x0040;
const HAS_MATRIX: u16 = 0x0080;
const HAS_INSTRUCTIONS: u16 = 0x0100;

fn outline(glyph: &[u8]) -> Option<Outline<'_>> {
    if glyph.is_empty() {
        return Some(Outline::Empty);
    }
    let contours = i16_at(glyph, 0)?;
    if contours < 0 {
        let mut components = Vec::new();
        let mut at = 10;
        loop {
            let flags = u16_at(glyph, at)?;
            let index = u16_at(glyph, at + 2)?;
            at += 4;
            let arguments = if flags & ARGS_ARE_WORDS != 0 {
                at += 4;
                if flags & ARGS_ARE_XY != 0 {
                    (
                        f32::from(i16_at(glyph, at - 4)?),
                        f32::from(i16_at(glyph, at - 2)?),
                    )
                } else {
                    (
                        f32::from(u16_at(glyph, at - 4)?),
                        f32::from(u16_at(glyph, at - 2)?),
                    )
                }
            } else {
                at += 2;
                if flags & ARGS_ARE_XY != 0 {
                    (
                        f32::from(*glyph.get(at - 2)? as i8),
                        f32::from(*glyph.get(at - 1)? as i8),
                    )
                } else {
                    (
                        f32::from(*glyph.get(at - 2)?),
                        f32::from(*glyph.get(at - 1)?),
                    )
                }
            };
            let size = if flags & HAS_SCALE != 0 {
                2
            } else if flags & HAS_XY_SCALE != 0 {
                4
            } else if flags & HAS_MATRIX != 0 {
                8
            } else {
                0
            };
            components.push(Component {
                flags,
                glyph: index,
                arguments,
                transform: glyph.get(at..at + size)?,
            });
            at += size;
            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }
        let instructions = if components
            .iter()
            .any(|component| component.flags & HAS_INSTRUCTIONS != 0)
        {
            let length = usize::from(u16_at(glyph, at)?);
            glyph.get(at + 2..at + 2 + length)?
        } else {
            &[]
        };
        return Some(Outline::Composite {
            components,
            instructions,
        });
    }
    let contours = contours as usize;
    let ends: Vec<usize> = (0..contours)
        .map(|index| u16_at(glyph, 10 + 2 * index).map(usize::from))
        .collect::<Option<_>>()?;
    let count = ends.last().map_or(0, |end| end + 1);
    let mut at = 10 + 2 * contours;
    let length = usize::from(u16_at(glyph, at)?);
    let instructions = glyph.get(at + 2..at + 2 + length)?;
    at += 2 + length;
    let mut flags = Vec::with_capacity(count);
    while flags.len() < count {
        let flag = *glyph.get(at)?;
        at += 1;
        flags.push(flag);
        if flag & 0x08 != 0 {
            let repeats = *glyph.get(at)?;
            at += 1;
            for _ in 0..repeats {
                flags.push(flag);
            }
        }
    }
    flags.truncate(count);
    let mut read = |short: u8, same: u8| -> Option<Vec<f32>> {
        let mut value = 0.0;
        flags
            .iter()
            .map(|flag| {
                if flag & short != 0 {
                    let step = f32::from(*glyph.get(at)?);
                    at += 1;
                    value += if flag & same != 0 { step } else { -step };
                } else if flag & same == 0 {
                    value += f32::from(i16_at(glyph, at)?);
                    at += 2;
                }
                Some(value)
            })
            .collect()
    };
    let xs = read(0x02, 0x10)?;
    let ys = read(0x04, 0x20)?;
    Some(Outline::Simple {
        ends,
        instructions,
        flags,
        points: xs.into_iter().zip(ys).collect(),
    })
}

/// Fills in the deltas of points a tuple left out from those of their
/// neighbors along each contour, as the font format says.
fn interpolate(
    ends: &[usize],
    original: &[(f32, f32)],
    deltas: &mut [(f32, f32)],
    touched: &[bool],
) {
    let mut start = 0;
    for &end in ends {
        let contour: Vec<usize> = (start..=end).filter(|point| touched[*point]).collect();
        if !contour.is_empty() {
            for point in start..=end {
                if touched[point] {
                    continue;
                }
                // The touched points before and after, around the contour.
                let before = contour
                    .iter()
                    .rev()
                    .find(|touched| **touched < point)
                    .or(contour.last())
                    .copied()
                    .unwrap_or(point);
                let after = contour
                    .iter()
                    .find(|touched| **touched > point)
                    .or(contour.first())
                    .copied()
                    .unwrap_or(point);
                let along = |coordinate: fn(&(f32, f32)) -> f32| {
                    let (mut x1, mut x2) =
                        (coordinate(&original[before]), coordinate(&original[after]));
                    let (mut d1, mut d2) =
                        (coordinate(&deltas[before]), coordinate(&deltas[after]));
                    if x1 == x2 {
                        return if d1 == d2 { d1 } else { 0.0 };
                    }
                    if x1 > x2 {
                        core::mem::swap(&mut x1, &mut x2);
                        core::mem::swap(&mut d1, &mut d2);
                    }
                    let x = coordinate(&original[point]);
                    if x <= x1 {
                        d1
                    } else if x >= x2 {
                        d2
                    } else {
                        d1 + (x - x1) * (d2 - d1) / (x2 - x1)
                    }
                };
                deltas[point] = (along(|point| point.0), along(|point| point.1));
            }
        }
        start = end + 1;
    }
}

/// A glyph instanced: its outline data and horizontal metrics.
struct Instanced {
    data: Vec<u8>,
    advance: u16,
    /// Its bounding box, when it has an outline.
    bounds: Option<[i16; 4]>,
}

fn bounds_of(points: impl Iterator<Item = (f32, f32)>) -> Option<[i16; 4]> {
    let mut bounds: Option<[f32; 4]> = None;
    for (x, y) in points {
        bounds = Some(match bounds {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
    bounds.map(|bounds| bounds.map(|value| libm::floorf(value + 0.5) as i16))
}

/// The font at a weight on its `wght` axis, its other axes at their
/// defaults, as a static TrueType font; `None` when it is not a variable
/// TrueType font this can read.
pub(super) fn instance(data: &[u8], weight: f32) -> Option<Vec<u8>> {
    let tables = tables(data)?;
    let (head, hhea, maxp) = (
        find(&tables, b"head")?,
        find(&tables, b"hhea")?,
        find(&tables, b"maxp")?,
    );
    let (glyf, loca, hmtx, gvar) = (
        find(&tables, b"glyf")?,
        find(&tables, b"loca")?,
        find(&tables, b"hmtx")?,
        find(&tables, b"gvar")?,
    );
    let axes = axes(find(&tables, b"fvar")?)?;
    let coordinates = coordinates(&axes, find(&tables, b"avar"), weight)?;
    let glyphs = usize::from(u16_at(maxp, 4)?);
    let long_offsets = i16_at(head, 50)? == 1;
    let offset_of = |glyph: usize| -> Option<usize> {
        if long_offsets {
            u32_at(loca, 4 * glyph).map(|offset| offset as usize)
        } else {
            u16_at(loca, 2 * glyph).map(|offset| 2 * usize::from(offset))
        }
    };
    let metrics = usize::from(u16_at(hhea, 34)?).max(1);
    let horizontal = |glyph: usize| -> Option<(f32, f32)> {
        let advance = u16_at(hmtx, 4 * glyph.min(metrics - 1))?;
        let bearing = if glyph < metrics {
            i16_at(hmtx, 4 * glyph + 2)?
        } else {
            i16_at(hmtx, 4 * metrics + 2 * (glyph - metrics))?
        };
        Some((f32::from(advance), f32::from(bearing)))
    };
    // gvar's header, shared tuples and per-glyph variation data.
    let axis_count = usize::from(u16_at(gvar, 4)?);
    if axis_count != axes.len() {
        return None;
    }
    let shared_count = usize::from(u16_at(gvar, 6)?);
    let shared_start = u32_at(gvar, 8)? as usize;
    let shared: Vec<Vec<f32>> = (0..shared_count)
        .map(|index| {
            (0..axis_count)
                .map(|axis| f2dot14(gvar, shared_start + 2 * (index * axis_count + axis)))
                .collect()
        })
        .collect::<Option<_>>()?;
    let long_variations = u16_at(gvar, 14)? & 1 != 0;
    let variations_start = u32_at(gvar, 16)? as usize;
    let variation_of = |glyph: usize| -> Option<usize> {
        if long_variations {
            u32_at(gvar, 20 + 4 * glyph).map(|offset| offset as usize)
        } else {
            u16_at(gvar, 20 + 2 * glyph).map(|offset| 2 * usize::from(offset))
        }
    };
    let read_tuple = |at: usize| -> Option<Vec<f32>> {
        (0..axis_count)
            .map(|axis| f2dot14(gvar, at + 2 * axis))
            .collect()
    };

    // Each glyph's points moved by its deltas, then encoded again.
    let mut instanced: Vec<Instanced> = Vec::with_capacity(glyphs);
    let mut composites = Vec::new();
    for glyph in 0..glyphs {
        let range = offset_of(glyph)?..offset_of(glyph + 1)?;
        let outline = outline(glyf.get(range)?)?;
        let (advance, bearing) = horizontal(glyph)?;
        let x_min = match &outline {
            Outline::Empty => 0.0,
            _ => f32::from(i16_at(glyf, offset_of(glyph)? + 2)?),
        };
        let (mut original, ends): (Vec<(f32, f32)>, Vec<usize>) = match &outline {
            Outline::Empty => (Vec::new(), Vec::new()),
            Outline::Simple { points, ends, .. } => (points.clone(), ends.clone()),
            Outline::Composite { components, .. } => (
                components
                    .iter()
                    .map(|component| component.arguments)
                    .collect(),
                Vec::new(),
            ),
        };
        let outline_points = original.len();
        // The phantom points: the origin and the advance.
        let origin = x_min - bearing;
        original.extend([
            (origin, 0.0),
            (origin + advance, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
        ]);
        let total = original.len();
        let mut deltas = alloc::vec![(0.0f32, 0.0f32); total];
        let (start, end) = (variation_of(glyph)?, variation_of(glyph + 1)?);
        if end > start {
            let base = variations_start + start;
            let raw = u16_at(gvar, base)?;
            let mut header = base + 4;
            let mut serial = base + usize::from(u16_at(gvar, base + 2)?);
            let shared_points = if raw & 0x8000 != 0 {
                let (points, next) = packed_points(gvar, serial)?;
                serial = next;
                points
            } else {
                None
            };
            for _ in 0..usize::from(raw & 0x0FFF) {
                let size = usize::from(u16_at(gvar, header)?);
                let index = u16_at(gvar, header + 2)?;
                header += 4;
                let peak = if index & 0x8000 != 0 {
                    let peak = read_tuple(header)?;
                    header += 2 * axis_count;
                    peak
                } else {
                    shared.get(usize::from(index & 0x0FFF))?.clone()
                };
                let region = if index & 0x4000 != 0 {
                    let region = (read_tuple(header)?, read_tuple(header + 2 * axis_count)?);
                    header += 4 * axis_count;
                    Some(region)
                } else {
                    None
                };
                let data_start = serial;
                serial += size;
                let factor = scalar(
                    &coordinates,
                    &peak,
                    region
                        .as_ref()
                        .map(|(starts, ends)| (starts.as_slice(), ends.as_slice())),
                );
                if factor == 0.0 {
                    continue;
                }
                let (points, at) = if index & 0x2000 != 0 {
                    packed_points(gvar, data_start)?
                } else {
                    (shared_points.clone(), data_start)
                };
                let count = points.as_ref().map_or(total, Vec::len);
                let (xs, next) = packed_deltas(gvar, at, count)?;
                let (ys, _) = packed_deltas(gvar, next, count)?;
                match points {
                    None => {
                        for (point, delta) in deltas.iter_mut().enumerate() {
                            delta.0 += factor * xs[point];
                            delta.1 += factor * ys[point];
                        }
                    }
                    Some(points) => {
                        let mut tuple = alloc::vec![(0.0f32, 0.0f32); total];
                        let mut touched = alloc::vec![false; total];
                        for (index, point) in points.iter().enumerate() {
                            if let Some(delta) = tuple.get_mut(*point) {
                                *delta = (xs[index], ys[index]);
                                touched[*point] = true;
                            }
                        }
                        if matches!(outline, Outline::Simple { .. }) {
                            interpolate(&ends, &original, &mut tuple, &touched);
                        }
                        for (delta, change) in deltas.iter_mut().zip(tuple) {
                            delta.0 += factor * change.0;
                            delta.1 += factor * change.1;
                        }
                    }
                }
            }
        }
        let moved: Vec<(f32, f32)> = original
            .iter()
            .zip(&deltas)
            .map(|(point, delta)| (point.0 + delta.0, point.1 + delta.1))
            .collect();
        // The origin stays at zero, as `head` may promise.
        let shift = libm::floorf(moved[outline_points].0 + 0.5);
        let advance = libm::floorf(moved[outline_points + 1].0 - moved[outline_points].0 + 0.5)
            .max(0.0) as u16;
        let points: Vec<(f32, f32)> = moved[..outline_points]
            .iter()
            .map(|(x, y)| (libm::floorf(x - shift + 0.5), libm::floorf(*y + 0.5)))
            .collect();
        let glyph_data = match outline {
            Outline::Empty => Instanced {
                data: Vec::new(),
                advance,
                bounds: None,
            },
            Outline::Simple {
                ends,
                instructions,
                flags,
                ..
            } => {
                let bounds = bounds_of(points.iter().copied());
                let mut bytes = Vec::new();
                bytes.extend((ends.len() as i16).to_be_bytes());
                for value in bounds.unwrap_or_default() {
                    bytes.extend(value.to_be_bytes());
                }
                for end in &ends {
                    bytes.extend((*end as u16).to_be_bytes());
                }
                bytes.extend((instructions.len() as u16).to_be_bytes());
                bytes.extend(instructions);
                // Every coordinate as a word: no short or repeated forms.
                bytes.extend(flags.iter().map(|flag| flag & 0x41));
                for axis in 0..2 {
                    let mut previous = 0.0;
                    for point in &points {
                        let value = if axis == 0 { point.0 } else { point.1 };
                        bytes.extend(((value - previous) as i16).to_be_bytes());
                        previous = value;
                    }
                }
                Instanced {
                    data: bytes,
                    advance,
                    bounds,
                }
            }
            Outline::Composite {
                components,
                instructions,
            } => {
                let mut bytes = alloc::vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
                let mut placed = Vec::new();
                for (index, component) in components.iter().enumerate() {
                    let flags = (component.flags | ARGS_ARE_WORDS)
                        & if index + 1 < components.len() {
                            u16::MAX
                        } else {
                            !MORE_COMPONENTS
                        };
                    bytes.extend(flags.to_be_bytes());
                    bytes.extend(component.glyph.to_be_bytes());
                    let arguments = if component.flags & ARGS_ARE_XY != 0 {
                        points[index]
                    } else {
                        component.arguments
                    };
                    bytes.extend((arguments.0 as i16).to_be_bytes());
                    bytes.extend((arguments.1 as i16).to_be_bytes());
                    bytes.extend(component.transform);
                    placed.push((
                        usize::from(component.glyph),
                        (component.flags & ARGS_ARE_XY != 0).then_some(arguments),
                        component.flags,
                        component.transform,
                    ));
                }
                if !instructions.is_empty() {
                    bytes.extend((instructions.len() as u16).to_be_bytes());
                    bytes.extend(instructions);
                }
                composites.push((glyph, placed));
                Instanced {
                    data: bytes,
                    advance,
                    bounds: None,
                }
            }
        };
        instanced.push(glyph_data);
    }
    // Composite glyphs' bounds, from their components' once those have
    // theirs: components may be composites in turn.
    for _ in 0..4 {
        for (glyph, placed) in &composites {
            let mut corners = Vec::new();
            for (component, offset, flags, transform) in placed {
                let Some([x0, y0, x1, y1]) = instanced
                    .get(*component)
                    .and_then(|component| component.bounds)
                else {
                    continue;
                };
                let scale = |at: usize| f2dot14(transform, at).unwrap_or(1.0);
                let [a, b, c, d] = if flags & HAS_SCALE != 0 {
                    [scale(0), 0.0, 0.0, scale(0)]
                } else if flags & HAS_XY_SCALE != 0 {
                    [scale(0), 0.0, 0.0, scale(2)]
                } else if flags & HAS_MATRIX != 0 {
                    [scale(0), scale(2), scale(4), scale(6)]
                } else {
                    [1.0, 0.0, 0.0, 1.0]
                };
                let (dx, dy) = offset.unwrap_or((0.0, 0.0));
                for (x, y) in [(x0, y0), (x0, y1), (x1, y0), (x1, y1)] {
                    let (x, y) = (f32::from(x), f32::from(y));
                    corners.push((a * x + c * y + dx, b * x + d * y + dy));
                }
            }
            let bounds = bounds_of(corners.into_iter());
            let entry = &mut instanced[*glyph];
            entry.bounds = bounds;
            for (slot, value) in bounds.unwrap_or_default().iter().enumerate() {
                entry.data[2 + 2 * slot..4 + 2 * slot].copy_from_slice(&value.to_be_bytes());
            }
        }
    }

    // The new glyf, loca and hmtx, and the headers that describe them.
    let mut glyf_bytes = Vec::new();
    let mut loca_bytes = Vec::new();
    let mut hmtx_bytes = Vec::new();
    let mut extremes: Option<[i16; 4]> = None;
    let (mut widest, mut least_left, mut least_right, mut furthest) =
        (0u16, i16::MAX, i16::MAX, i16::MIN);
    for glyph in &instanced {
        loca_bytes.extend((glyf_bytes.len() as u32).to_be_bytes());
        glyf_bytes.extend(&glyph.data);
        glyf_bytes.resize(glyf_bytes.len().div_ceil(4) * 4, 0);
        let bearing = glyph.bounds.map_or(0, |bounds| bounds[0]);
        hmtx_bytes.extend(glyph.advance.to_be_bytes());
        hmtx_bytes.extend(bearing.to_be_bytes());
        widest = widest.max(glyph.advance);
        if let Some([x0, y0, x1, y1]) = glyph.bounds {
            least_left = least_left.min(x0);
            least_right = least_right.min((i32::from(glyph.advance) - i32::from(x1)) as i16);
            furthest = furthest.max(x1);
            extremes = Some(match extremes {
                None => [x0, y0, x1, y1],
                Some([a, b, c, d]) => [a.min(x0), b.min(y0), c.max(x1), d.max(y1)],
            });
        }
    }
    loca_bytes.extend((glyf_bytes.len() as u32).to_be_bytes());
    let mut head_bytes = head.to_vec();
    for (slot, value) in extremes.unwrap_or_default().iter().enumerate() {
        head_bytes
            .get_mut(36 + 2 * slot..38 + 2 * slot)?
            .copy_from_slice(&value.to_be_bytes());
    }
    head_bytes
        .get_mut(50..52)?
        .copy_from_slice(&1i16.to_be_bytes());
    head_bytes.get_mut(8..12)?.copy_from_slice(&[0; 4]);
    let mut hhea_bytes = hhea.to_vec();
    hhea_bytes
        .get_mut(10..12)?
        .copy_from_slice(&widest.to_be_bytes());
    if extremes.is_some() {
        hhea_bytes
            .get_mut(12..14)?
            .copy_from_slice(&least_left.to_be_bytes());
        hhea_bytes
            .get_mut(14..16)?
            .copy_from_slice(&least_right.to_be_bytes());
        hhea_bytes
            .get_mut(16..18)?
            .copy_from_slice(&furthest.to_be_bytes());
    }
    hhea_bytes
        .get_mut(34..36)?
        .copy_from_slice(&(glyphs as u16).to_be_bytes());

    let mut result: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for (tag, bytes) in &tables {
        if VARIATION_TABLES.contains(&tag) {
            continue;
        }
        let bytes = match tag {
            b"glyf" => glyf_bytes.clone(),
            b"loca" => loca_bytes.clone(),
            b"hmtx" => hmtx_bytes.clone(),
            b"head" => head_bytes.clone(),
            b"hhea" => hhea_bytes.clone(),
            b"OS/2" => {
                // The weight the instance is, as GDI matches it.
                let mut os2 = bytes.to_vec();
                if let Some(class) = os2.get_mut(4..6) {
                    class.copy_from_slice(&(libm::floorf(weight + 0.5) as u16).to_be_bytes());
                }
                os2
            }
            _ => bytes.to_vec(),
        };
        result.push((*tag, bytes));
    }
    Some(assemble(result))
}

/// A font file of tables, with its directory and checksums.
fn assemble(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let count = tables.len() as u16;
    let power = 1u16 << (15 - count.max(1).leading_zeros());
    let mut font = Vec::new();
    font.extend([0u8, 1, 0, 0]);
    font.extend(count.to_be_bytes());
    font.extend((power * 16).to_be_bytes());
    font.extend((power.trailing_zeros() as u16).to_be_bytes());
    font.extend((count * 16 - power * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, bytes) in &tables {
        font.extend(tag);
        font.extend(super::checksum(bytes).to_be_bytes());
        font.extend((offset as u32).to_be_bytes());
        font.extend((bytes.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(offset);
        }
        offset += bytes.len().div_ceil(4) * 4;
    }
    for (_, bytes) in &tables {
        font.extend(bytes);
        font.resize(font.len().div_ceil(4) * 4, 0);
    }
    if let Some(head) = head_at {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(super::checksum(&font));
        font[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    font
}

#[cfg(test)]
mod tests;
