//! GDI fonts shared by painting, controls and text measurement.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use core::cell::{Cell, RefCell};
use nativeboat_ir::Font;
use nativeboat_layout::{FontMetrics, TextMeasurer};
use windows_sys::Win32::Foundation::{LPARAM, SIZE};
use windows_sys::Win32::Graphics::Gdi::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    family: String,
    height: i32,
    weight: u16,
    italic: bool,
    underline: bool,
    strike: bool,
    /// Smoothed in grayscale rather than with ClearType.
    grayscale: bool,
}

/// A font face a page declared with `@font-face`.
#[derive(Debug, Clone)]
struct Face {
    /// The CSS family it was declared for, lowercased.
    family: String,
    /// The weights it serves: one, or a variable font's range.
    weight: u16,
    max_weight: u16,
    italic: bool,
    /// The characters it shows, as `unicode-range` limits them: inclusive
    /// ranges of code points, all of them when empty.
    ranges: Vec<(u32, u32)>,
    /// The family name GDI knows the loaded font by, its own.
    name: String,
    /// The weight and slant the font file gives itself, which GDI matches
    /// requests against: asking for them selects the face as it is.
    own_weight: u16,
    own_italic: bool,
    /// A variable TrueType font's file and its `wght` axis's default, which
    /// it is instanced away from: GDI shows only its default instance.
    variable: Option<(Rc<[u8]>, u16)>,
    /// The instances loaded: weight and the name GDI knows each by.
    instances: Vec<(u16, String)>,
}

local! {
    static FONTS: RefCell<HashMap<Key, HFONT>> = RefCell::new(HashMap::new());
    static INSTALLED: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
    static FACES: RefCell<Vec<Face>> = RefCell::new(Vec::new());
    /// Counts the faces loaded, so measurements made before start over.
    static GENERATION: Cell<u32> = Cell::new(0);
    /// The Chromium release whose font choices text follows; the latest
    /// unless [`follow_chromium`] names another.
    static CHROMIUM: Cell<u32> = Cell::new(u32::MAX);
    /// The application's locale, which picks the generic families; the
    /// Windows display language unless [`follow_locale`] names another.
    static LOCALE: Cell<Option<Locale>> = Cell::new(None);
    static CLEARTYPE: Cell<Option<bool>> = Cell::new(None);
}

/// The family name in a TrueType or OpenType font's naming table (name 1),
/// preferring the Windows Unicode names GDI uses.
fn sfnt_family(data: &[u8]) -> Option<String> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let tables = usize::from(u16_at(4)?);
    let name = (0..tables).find_map(|index| {
        let record = 12 + 16 * index;
        (data.get(record..record + 4)? == b"name").then(|| u32_at(record + 8))?
    })? as usize;
    let count = usize::from(u16_at(name + 2)?);
    let strings = name + usize::from(u16_at(name + 4)?);
    let mut best: Option<(u8, String)> = None;
    for index in 0..count {
        let record = name + 6 + 12 * index;
        let (platform, language, id) = (u16_at(record)?, u16_at(record + 4)?, u16_at(record + 6)?);
        if id != 1 {
            continue;
        }
        let length = usize::from(u16_at(record + 8)?);
        let start = strings + usize::from(u16_at(record + 10)?);
        let bytes = data.get(start..start + length)?;
        let (rank, text) = match platform {
            3 => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                (
                    if language == 0x409 { 0 } else { 1 },
                    String::from_utf16_lossy(&units),
                )
            }
            1 => (2, bytes.iter().map(|byte| char::from(*byte)).collect()),
            _ => continue,
        };
        if best.as_ref().is_none_or(|(known, _)| rank < *known) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, text)| text).filter(|text| !text.is_empty())
}

/// Makes a TrueType or OpenType font available to this process for a CSS
/// family, weight and style, as `@font-face` declares it; returns whether
/// the font loaded.
pub fn register_font(family: &str, weight: u16, italic: bool, data: &[u8]) -> bool {
    register_font_face(family, (weight, weight), italic, &[], data)
}

/// Makes a font available for a CSS family as an `@font-face` rule declares
/// it: the weights it serves, its style, and the characters it shows as
/// inclusive code point ranges (all of them when empty); returns whether
/// the font loaded.
pub fn register_font_face(
    family: &str,
    (weight, max_weight): (u16, u16),
    italic: bool,
    ranges: &[(u32, u32)],
    data: &[u8],
) -> bool {
    let Some(original) = sfnt_family(data) else {
        return false;
    };
    // Each face is a family of its own to GDI, as each `@font-face` is a
    // face of its own to CSS: faces whose files name the same family and
    // claim the same style, as KaTeX's regular and italic ones do, would
    // otherwise stand in for each other.
    let serial = FACES.with_borrow(Vec::len);
    let suffix = format!("#{serial}");
    let kept: String = original
        .chars()
        .take(31usize.saturating_sub(suffix.len()))
        .collect();
    let name = format!("{kept}{suffix}");
    let renamed = renamed_font(data, &name);
    let (name, data) = match &renamed {
        Some(renamed) => (name, renamed.as_slice()),
        None => (original, data),
    };
    let (own_weight, own_italic) = own_style(data).unwrap_or((400, false));
    if !load(data) {
        return false;
    }
    // A face that serves a range of weights from a variable font keeps it,
    // to instance at the weights asked for.
    let variable = (max_weight > weight)
        .then(|| instance::weight_axis(data))
        .flatten()
        .map(|(_, default, _)| (Rc::from(data), libm::floorf(default + 0.5) as u16));
    FACES.with_borrow_mut(|faces| {
        faces.push(Face {
            family: family.to_lowercase(),
            weight,
            max_weight: max_weight.max(weight),
            italic,
            ranges: ranges.to_vec(),
            name,
            own_weight,
            own_italic,
            variable,
            instances: Vec::new(),
        });
    });
    GENERATION.with(|generation| generation.set(generation.get() + 1));
    true
}

/// Whether GDI knows a face, by its name as a `LOGFONTW` holds it, from a
/// font this process loaded from memory, as web fonts are.
pub(crate) fn from_memory(face: &[u16]) -> bool {
    let length = face
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(face.len());
    let name = String::from_utf16_lossy(&face[..length]);
    FACES.with_borrow(|faces| {
        faces.iter().any(|loaded| {
            loaded.name == name || loaded.instances.iter().any(|(_, known)| *known == name)
        })
    })
}

/// Loads a font into GDI for this process; returns whether it loaded.
fn load(data: &[u8]) -> bool {
    let mut count = 0u32;
    // SAFETY: the bytes are read during the call; GDI keeps its own copy.
    let handle = unsafe {
        AddFontMemResourceEx(
            data.as_ptr().cast(),
            data.len() as u32,
            core::ptr::null(),
            (&raw mut count).cast_const(),
        )
    };
    !handle.is_null() && count > 0
}

/// The name GDI knows a variable face's instance at a weight by, loading
/// the instance the first time; `None` when it cannot be made.
fn instanced(index: usize, weight: u16) -> Option<String> {
    let (data, name) = FACES.with_borrow(|faces| {
        let face = faces.get(index)?;
        if let Some((_, name)) = face.instances.iter().find(|(known, _)| *known == weight) {
            return Some((None, name.clone()));
        }
        Some((Some(face.variable.as_ref()?.0.clone()), face.name.clone()))
    })?;
    let Some(data) = data else {
        return Some(name);
    };
    let instance = instance::instance(&data, f32::from(weight))?;
    let suffix = format!(".{weight}");
    let kept: String = name
        .chars()
        .take(31usize.saturating_sub(suffix.len()))
        .collect();
    let name = format!("{kept}{suffix}");
    let renamed = renamed_font(&instance, &name)?;
    if !load(&renamed) {
        return None;
    }
    FACES.with_borrow_mut(|faces| {
        if let Some(face) = faces.get_mut(index) {
            face.instances.push((weight, name.clone()));
        }
    });
    Some(name)
}

/// The offset and length of a table of a TrueType or OpenType font, and
/// where its directory record is.
fn table(data: &[u8], tag: &[u8; 4]) -> Option<(usize, usize, usize)> {
    let tables = usize::from(u16::from_be_bytes(data.get(4..6)?.try_into().ok()?));
    (0..tables).find_map(|index| {
        let record = 12 + 16 * index;
        if data.get(record..record + 4)? != tag {
            return None;
        }
        let offset = u32::from_be_bytes(data.get(record + 8..record + 12)?.try_into().ok()?);
        let length = u32::from_be_bytes(data.get(record + 12..record + 16)?.try_into().ok()?);
        Some((record, offset as usize, length as usize))
    })
}

/// The weight and slant a font file gives itself (`OS/2` weight class and
/// italic selection bit, else `head`'s style bits).
pub(crate) fn own_style(data: &[u8]) -> Option<(u16, bool)> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    if let Some((_, os2, _)) = table(data, b"OS/2") {
        let weight = u16_at(os2 + 4)?;
        let selection = u16_at(os2 + 62)?;
        let weight = if weight == 0 {
            400
        } else {
            weight.clamp(1, 1000)
        };
        return Some((weight, selection & 1 != 0));
    }
    let (_, head, _) = table(data, b"head")?;
    let style = u16_at(head + 44)?;
    Some((if style & 1 != 0 { 700 } else { 400 }, style & 2 != 0))
}

/// The sum of a table's big-endian 32-bit words, as font checksums are.
fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, word| {
        let mut padded = [0u8; 4];
        padded[..word.len()].copy_from_slice(word);
        sum.wrapping_add(u32::from_be_bytes(padded))
    })
}

/// A copy of a TrueType or OpenType font whose naming table names its
/// family and full name `name`, its other names kept: a new table at the
/// end of the file, which the table directory points to.
fn renamed_font(data: &[u8], name: &str) -> Option<Vec<u8>> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    if !matches!(data.get(..4)?, [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let (record, old, _) = table(data, b"name")?;
    let count = usize::from(u16_at(old + 2)?);
    let strings = old + usize::from(u16_at(old + 4)?);
    // Family, full and typographic family names in each record's encoding:
    // UTF-16 for Unicode and Windows records, single bytes for Macintosh.
    let utf16: Vec<u8> = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let bytes: Vec<u8> = name
        .chars()
        .map(|character| {
            if character.is_ascii() {
                character as u8
            } else {
                b'?'
            }
        })
        .collect();
    let mut records = Vec::new();
    for index in 0..count {
        let at = old + 6 + 12 * index;
        let (platform, encoding, language, id) = (
            u16_at(at)?,
            u16_at(at + 2)?,
            u16_at(at + 4)?,
            u16_at(at + 6)?,
        );
        let length = usize::from(u16_at(at + 8)?);
        let start = strings + usize::from(u16_at(at + 10)?);
        let text = match (id, platform) {
            (1 | 4 | 16, 0 | 3) => utf16.clone(),
            (1 | 4 | 16, _) => bytes.clone(),
            _ => data.get(start..start + length)?.to_vec(),
        };
        records.push((platform, encoding, language, id, text));
    }
    let mut table_bytes = Vec::new();
    table_bytes.extend(0u16.to_be_bytes());
    table_bytes.extend((records.len() as u16).to_be_bytes());
    table_bytes.extend(((6 + 12 * records.len()) as u16).to_be_bytes());
    let mut offset = 0usize;
    for (platform, encoding, language, id, text) in &records {
        for value in [
            *platform,
            *encoding,
            *language,
            *id,
            text.len() as u16,
            offset as u16,
        ] {
            table_bytes.extend(value.to_be_bytes());
        }
        offset += text.len();
    }
    for (.., text) in &records {
        table_bytes.extend(text);
    }
    let length = table_bytes.len();
    table_bytes.resize(length.div_ceil(4) * 4, 0);
    let mut font = data.to_vec();
    font.resize(font.len().div_ceil(4) * 4, 0);
    let start = font.len();
    let sum = checksum(&table_bytes);
    font.extend(&table_bytes);
    font[record + 4..record + 8].copy_from_slice(&sum.to_be_bytes());
    font[record + 8..record + 12].copy_from_slice(&(start as u32).to_be_bytes());
    font[record + 12..record + 16].copy_from_slice(&(length as u32).to_be_bytes());
    // The whole file sums to the magic number again.
    if let Some((_, head, _)) = table(&font, b"head") {
        font.get_mut(head + 8..head + 12)?.copy_from_slice(&[0; 4]);
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
        font[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    Some(font)
}

/// How well a face's weight matches a requested weight, as CSS font
/// matching orders them: lower is better.
fn weight_distance(requested: u16, face: u16) -> u32 {
    let (requested, face) = (u32::from(requested), u32::from(face));
    let lighter = 10_000 + requested.abs_diff(face);
    let heavier = 20_000 + requested.abs_diff(face);
    match requested {
        400 if face == 500 => 1,
        500 if face == 400 => 1,
        _ if face == requested => 0,
        400..=500 if face < requested => lighter,
        400..=500 => heavier,
        0..400 if face < requested => requested - face,
        0..400 => lighter + face,
        _ if face > requested => face - requested,
        _ => lighter + (1000 - face),
    }
}

/// A face chosen for text: its name, the weight and slant to ask GDI for,
/// and the characters it shows.
type ChosenFace = (String, u16, bool, Vec<(u32, u32)>);

/// The loaded face to use for a family list, with the weight and slant to
/// ask GDI for: the face's own, or a synthesized bolder or slanted one.
fn face(families: &[String], weight: u16, italic: bool) -> Option<(String, u16, bool)> {
    faces(families, weight, italic)
        .into_iter()
        .next()
        .map(|(name, weight, italic, _)| (name, weight, italic))
}

/// The loaded faces that serve a family list's first declared family at a
/// weight and style, as GDI asks for them, with the characters each shows:
/// those that match best, last declared first as CSS tries them. Faces
/// that `unicode-range` splits a font into share the best match.
fn faces(families: &[String], weight: u16, italic: bool) -> Vec<ChosenFace> {
    // The faces chosen, with the weight and slant each is asked for, and
    // the variable ones' instance weights.
    let chosen = FACES.with_borrow(|faces| {
        families
            .iter()
            .find_map(|family| {
                let family = family.to_lowercase();
                let rank = |face: &Face| {
                    (
                        u32::from(face.italic != italic),
                        weight_distance(weight, weight.clamp(face.weight, face.max_weight)),
                    )
                };
                let best = faces
                    .iter()
                    .filter(|face| face.family == family)
                    .map(rank)
                    .min()?;
                let chosen = faces
                    .iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, face)| face.family == family && rank(face) == best)
                    .map(|(index, face)| {
                        let within = weight.clamp(face.weight, face.max_weight);
                        // A variable font is instanced at the weight within
                        // its range; other faces are used as they are,
                        // unless CSS synthesizes a bolder or slanted one.
                        let instance = face
                            .variable
                            .as_ref()
                            .filter(|(_, default)| *default != within)
                            .map(|_| within);
                        let weight = if weight >= 600 && face.max_weight < 600 {
                            weight.max(face.own_weight + 300)
                        } else if face.variable.is_some() {
                            within
                        } else {
                            face.own_weight
                        };
                        let slant = if italic && !face.italic {
                            true
                        } else {
                            face.own_italic
                        };
                        (
                            index,
                            face.name.clone(),
                            weight,
                            slant,
                            instance,
                            face.ranges.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                Some(chosen)
            })
            .unwrap_or_default()
    });
    chosen
        .into_iter()
        .map(|(index, name, weight, slant, instance, ranges)| {
            let name = instance
                .and_then(|weight| instanced(index, weight))
                .unwrap_or(name);
            (name, weight, slant, ranges)
        })
        .collect()
}

/// Whether a face shows a character, as its `unicode-range` says.
fn in_ranges(ranges: &[(u32, u32)], character: u32) -> bool {
    ranges.is_empty()
        || ranges
            .iter()
            .any(|(first, last)| (*first..=*last).contains(&character))
}

unsafe extern "system" fn found(
    _: *const LOGFONTW,
    _: *const TEXTMETRICW,
    _: u32,
    flag: LPARAM,
) -> i32 {
    // SAFETY: `installed` passes a pointer to a live bool.
    unsafe { *(flag as *mut bool) = true };
    0
}

/// Whether a font family is installed.
fn installed(family: &str) -> bool {
    if let Some(known) = INSTALLED.with_borrow(|cache| cache.get(family).copied()) {
        return known;
    }
    let mut logfont = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..LOGFONTW::default()
    };
    for (slot, unit) in logfont
        .lfFaceName
        .iter_mut()
        .zip(family.encode_utf16().take(31))
    {
        *slot = unit;
    }
    let mut exists = false;
    // SAFETY: the screen DC is released before returning; the callback only
    // writes through the pointer to `exists`, which outlives the call.
    unsafe {
        let dc = GetDC(core::ptr::null_mut());
        EnumFontFamiliesExW(
            dc,
            &logfont,
            Some(found),
            &mut exists as *mut bool as LPARAM,
            0,
        );
        ReleaseDC(core::ptr::null_mut(), dc);
    }
    INSTALLED.with_borrow_mut(|cache| cache.insert(family.to_owned(), exists));
    exists
}

/// The user's interface language, as Chromium's locale settings tell its
/// fonts apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Locale {
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
    Thai,
    Other,
}

fn locale() -> Locale {
    if let Some(locale) = LOCALE.with(Cell::get) {
        return locale;
    }
    // SAFETY: GetUserDefaultUILanguage has no preconditions.
    let language = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    match (language & 0x3FF, language >> 10) {
        (0x04, 0x01 | 0x03 | 0x05) => Locale::TraditionalChinese,
        (0x04, _) => Locale::SimplifiedChinese,
        (0x11, _) => Locale::Japanese,
        (0x12, _) => Locale::Korean,
        (0x1E, _) => Locale::Thai,
        _ => Locale::Other,
    }
}

/// Makes the generic families those of an application locale, such as
/// the `en-US` an Electron application falls back to when it ships no
/// resources for the user's language.
pub fn follow_locale(tag: &str) {
    let language = tag.split(['-', '_']).next().unwrap_or_default();
    let locale = match language.to_ascii_lowercase().as_str() {
        "zh" if tag.split(['-', '_']).skip(1).any(|part| {
            ["TW", "HK", "MO", "HANT"]
                .iter()
                .any(|traditional| part.eq_ignore_ascii_case(traditional))
        }) =>
        {
            Locale::TraditionalChinese
        }
        "zh" => Locale::SimplifiedChinese,
        "ja" => Locale::Japanese,
        "ko" => Locale::Korean,
        "th" => Locale::Thai,
        _ => Locale::Other,
    };
    LOCALE.with(|cell| cell.set(Some(locale)));
}

/// Whether Windows smooths text with ClearType, read once.
fn cleartype() -> bool {
    if let Some(known) = CLEARTYPE.with(Cell::get) {
        return known;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETFONTSMOOTHINGTYPE, SystemParametersInfoW,
    };
    const FE_FONTSMOOTHINGCLEARTYPE: u32 = 2;
    let mut kind = 0u32;
    // SAFETY: the query writes one u32 through the pointer.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETFONTSMOOTHINGTYPE,
            0,
            (&mut kind as *mut u32).cast(),
            0,
        )
    };
    let cleartype = read != 0 && kind == FE_FONTSMOOTHINGCLEARTYPE;
    CLEARTYPE.with(|cell| cell.set(Some(cleartype)));
    cleartype
}

/// Makes text choose fonts as a Chromium release does, such as the one an
/// Electron application ships: before Chromium 128, the default fonts and
/// the fallback fonts of Chinese, Japanese and Korean were not yet Noto CJK
/// fonts.
pub fn follow_chromium(major: u32) {
    CHROMIUM.with(|chromium| chromium.set(major));
}

/// A list of families as the followed Chromium release has it.
fn released(families: &'static [&'static str]) -> &'static [&'static str] {
    released_in(CHROMIUM.with(Cell::get), families)
}

/// A list of families as a Chromium release has it: before 128, without
/// the families Chromium 128 put first.
fn released_in(major: u32, families: &'static [&'static str]) -> &'static [&'static str] {
    if major >= 128 {
        return families;
    }
    let added = families
        .iter()
        .take_while(|family| family.starts_with("Noto ") || **family == "BIZ UDGothic")
        .count();
    &families[added..]
}

/// The fonts a generic family stands for, the first installed one used, as
/// Chromium's locale settings for Windows list them.
fn generic(name: &str) -> Option<&'static [&'static str]> {
    use Locale::*;
    let locale = locale();
    Some(released(match name {
        "sans-serif" => match locale {
            SimplifiedChinese => &["Noto Sans SC", "Noto Sans CJK SC", "Microsoft YaHei"],
            Japanese => &["Noto Sans JP", "Noto Sans CJK JP", "Meiryo", "Yu Gothic"],
            Korean => &["Noto Sans KR", "Noto Sans CJK KR", "Malgun Gothic"],
            Thai => &["Tahoma"],
            _ => &["Arial"],
        },
        "serif" => match locale {
            SimplifiedChinese => &["Noto Serif SC", "Noto Serif CJK SC", "SimSun"],
            Japanese => &[
                "Noto Serif JP",
                "Noto Serif CJK JP",
                "Yu Mincho",
                "MS PMincho",
            ],
            Korean => &["Noto Serif KR", "Noto Serif CJK KR", "Batang"],
            Thai => &["Tahoma"],
            _ => &["Times New Roman"],
        },
        "monospace" => match locale {
            SimplifiedChinese => &["NSimSun"],
            Japanese => &["BIZ UDGothic", "MS Gothic"],
            Korean => &["GulimChe"],
            Thai => &["Tahoma"],
            // Electron 36 (Chromium 136) took Consolas for Courier New
            // where text is smoothed with ClearType.
            _ if CHROMIUM.with(Cell::get) >= 136 && cleartype() => &["Consolas"],
            _ => &["Courier New"],
        },
        "cursive" => match locale {
            SimplifiedChinese => &["KaiTi"],
            TraditionalChinese => &["DFKai-SB"],
            Korean => &["Gungsuh"],
            _ => &["Comic Sans MS"],
        },
        "fantasy" => &["Impact"],
        _ => return None,
    }))
}

/// The standard font of the user's locale, which text without a
/// `font-family` uses: Chromium's locale settings make it the sans-serif
/// font for Chinese, Japanese, Korean and Thai, else Times New Roman.
pub(crate) fn standard() -> &'static str {
    let families: &[&'static str] = match locale() {
        Locale::SimplifiedChinese | Locale::Japanese | Locale::Korean | Locale::Thai => {
            generic("sans-serif").unwrap_or(&[])
        }
        Locale::TraditionalChinese | Locale::Other => &[],
    };
    families
        .iter()
        .copied()
        .find(|family| installed(family))
        .unwrap_or("Times New Roman")
}

/// The first installed family of a list, else its first.
fn first_available(families: &[&str]) -> String {
    families
        .iter()
        .find(|family| installed(family))
        .unwrap_or(&families[0])
        .to_string()
}

/// The family of the system's message font, which `system-ui` names, as
/// "Microsoft YaHei UI" in Chinese.
fn system_ui() -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SystemParametersInfoW,
    };
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        // SAFETY: the structure is plain data, valid when zeroed.
        ..unsafe { core::mem::zeroed() }
    };
    // SAFETY: the structure is writable and its size is set.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&raw mut metrics).cast(),
            0,
        )
    };
    let name = &metrics.lfMessageFont.lfFaceName;
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    if read == 0 || length == 0 {
        return "Segoe UI".to_owned();
    }
    String::from_utf16_lossy(&name[..length])
}

/// The installed family used for a CSS `font-family` list: generic families
/// stand for the locale's fonts, and names Windows lacks are skipped, as
/// `-apple-system` is; none found leaves the locale's sans-serif font.
pub(crate) fn family(families: &[String]) -> String {
    for family in families {
        let name = family.to_ascii_lowercase();
        if name == "system-ui" {
            return system_ui();
        }
        if let Some(generic) = generic(&name) {
            return first_available(generic);
        }
        if installed(family) {
            return family.clone();
        }
    }
    first_available(generic("sans-serif").unwrap_or(&["Arial"]))
}

/// The weights GDI names as families of their own, as "Segoe UI Semibold"
/// or "Noto Sans SC Medium" are, and the weights they stand for.
const WEIGHT_NAMES: &[(&str, u16)] = &[
    ("Thin", 100),
    ("ExtraLight", 200),
    ("UltraLight", 200),
    ("Light", 300),
    ("SemiLight", 350),
    ("Medium", 500),
    ("SemiBold", 600),
    ("DemiBold", 600),
    ("ExtraBold", 800),
    ("UltraBold", 800),
    ("Black", 900),
    ("Heavy", 900),
];

local! {
    /// The weights installed families have under families of their own.
    static WEIGHTS: RefCell<HashMap<String, Vec<(String, u16)>>> = RefCell::new(HashMap::new());
}

/// The installed face of a system family for a weight, with the weight to
/// ask GDI for: a family named for the weight, as DirectWrite matches
/// weights within a family, or the family itself.
fn system_face(family: String, weight: u16) -> (String, u16) {
    if weight == 400 || weight == 700 {
        return (family, weight);
    }
    let named = WEIGHTS.with_borrow_mut(|weights| {
        weights
            .entry(family.clone())
            .or_insert_with(|| {
                WEIGHT_NAMES
                    .iter()
                    .map(|(name, weight)| (format!("{family} {name}"), *weight))
                    .filter(|(name, _)| installed(name))
                    .collect()
            })
            .clone()
    });
    // The family itself has its regular and bold faces.
    let best = [(family.clone(), 400), (family.clone(), 700)]
        .into_iter()
        .chain(named)
        .min_by_key(|(_, face)| weight_distance(weight, *face));
    // The matched face's weight, which GDI then picks within the family, as
    // its bold face for 600; CSS synthesizes bold from a lighter face.
    match best {
        Some((name, face)) => {
            let weight = if weight >= 600 && face < 600 {
                weight
            } else {
                face
            };
            (name, weight)
        }
        None => (family, weight),
    }
}

/// A GDI font for a font request at a scale, cached for the thread.
pub(crate) fn font(font: &Font, scale: f32, underline: bool, strike: bool) -> HFONT {
    styled(font, scale, (underline, strike), false)
}

/// The GDI font a native control draws its text with, smoothed in
/// grayscale where the page's text is.
pub(crate) fn control_font(font: &Font, scale: f32, grayscale: bool) -> HFONT {
    styled(font, scale, (false, false), grayscale)
}

fn styled(font: &Font, scale: f32, (underline, strike): (bool, bool), grayscale: bool) -> HFONT {
    let (family, weight, italic) =
        face(&font.families, font.weight, font.italic).unwrap_or_else(|| {
            let (family, weight) = system_face(family(&font.families), font.weight);
            (family, weight, font.italic)
        });
    handle(Key {
        family,
        height: (font.size * scale).round().max(1.0) as i32,
        weight,
        italic,
        underline,
        strike,
        grayscale,
    })
}

/// The GDI font for a family, size and style, cached for the thread.
fn handle(key: Key) -> HFONT {
    if let Some(handle) = FONTS.with_borrow(|fonts| fonts.get(&key).copied()) {
        return handle;
    }
    let mut logfont = LOGFONTW {
        lfHeight: -key.height,
        lfWeight: i32::from(key.weight),
        lfItalic: u8::from(key.italic),
        lfUnderline: u8::from(key.underline),
        lfStrikeOut: u8::from(key.strike),
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: if key.grayscale {
            ANTIALIASED_QUALITY
        } else {
            CLEARTYPE_QUALITY
        },
        ..LOGFONTW::default()
    };
    for (slot, unit) in logfont
        .lfFaceName
        .iter_mut()
        .zip(wide(&key.family).into_iter().take(31))
    {
        *slot = unit;
    }
    // SAFETY: the LOGFONTW is fully initialised.
    let handle = unsafe { CreateFontIndirectW(&logfont) };
    // Text in it is measured now and painted later, when DirectWrite may
    // draw it from a face made meanwhile.
    crate::dwrite::prepare(&logfont, from_memory(&logfont.lfFaceName));
    FONTS.with_borrow_mut(|fonts| fonts.insert(key, handle));
    handle
}

/// The height of a line of text in a font, its ascent and descent, in CSS
/// pixels at a scale.
pub(crate) fn cell_height(font: &Font, scale: f32) -> f32 {
    let handle = self::font(font, scale, false, false);
    let mut metrics = TEXTMETRICW::default();
    // SAFETY: a memory DC measures the live font and is deleted here.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let previous = SelectObject(dc, handle);
        GetTextMetricsW(dc, &mut metrics);
        SelectObject(dc, previous);
        DeleteDC(dc);
    }
    metrics.tmHeight as f32 / scale
}

/// A font's size in device pixels, as its GDI fonts are made.
pub(crate) fn pixel_size(font: &Font, scale: f32) -> f32 {
    (font.size * scale).round().max(1.0)
}

/// The system fonts that show emoji.
const EMOJI_FONTS: &[&str] = &["Segoe UI Emoji", "Segoe UI Symbol"];

/// Whether a unit joins the character before it into one glyph: a zero
/// width joiner, a variation selector or a combining keycap.
fn is_joiner(unit: u16) -> bool {
    matches!(unit, 0x200D | 0xFE00..=0xFE0F | 0x20E3)
}

/// The fallback fonts of the character starting at a unit: a character
/// asking for emoji presentation (U+FE0F after it) wants an emoji font.
fn fallbacks_at(units: &[u16], index: usize) -> &'static [&'static str] {
    let character = character_at(units, index);
    let next = index + if character > 0xFFFF { 2 } else { 1 };
    if units.get(next) == Some(&0xFE0F) {
        EMOJI_FONTS
    } else {
        script_fallbacks(character)
    }
}

/// The system fonts that show characters of a script the requested fonts
/// lack, as Chromium falls back on Windows: Han characters in the fonts of
/// the user's language, then kana, Hangul, emoji and other symbols.
fn script_fallbacks(unit: u32) -> &'static [&'static str] {
    const CHINESE: &[&str] = &[
        "Noto Sans SC",
        "Noto Sans CJK SC",
        "Microsoft YaHei",
        "SimSun",
    ];
    const TRADITIONAL: &[&str] = &[
        "Noto Sans TC",
        "Noto Sans CJK TC",
        "Microsoft JhengHei",
        "PMingLiU",
    ];
    const JAPANESE: &[&str] = &[
        "Noto Sans JP",
        "Noto Sans CJK JP",
        "Meiryo",
        "Yu Gothic",
        "MS PGothic",
        "Microsoft YaHei",
    ];
    const KOREAN: &[&str] = &["Noto Sans KR", "Noto Sans CJK KR", "Malgun Gothic", "Gulim"];
    released(match unit {
        0x3040..=0x30FF | 0x31F0..=0x31FF => JAPANESE,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => KOREAN,
        0x2E80..=0x2FFF
        | 0x3000..=0x303F
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF
        | 0x2_0000..=0x3_FFFF => match locale() {
            Locale::TraditionalChinese => TRADITIONAL,
            Locale::Japanese => JAPANESE,
            Locale::Korean => KOREAN,
            _ => CHINESE,
        },
        // Beyond the basic plane only emoji fonts are trusted; see `runs`.
        0x1_F000..=0x1_FAFF | 0xFE0F => EMOJI_FONTS,
        _ if emoji_presentation(unit) => EMOJI_FONTS,
        // Symbols shown as text by default, such as U+2764 HEAVY BLACK
        // HEART, in a symbol font, as Chromium's system fallback picks.
        _ => &["Segoe UI Symbol", "Segoe UI", "Arial Unicode MS"],
    })
}

/// Whether a character of the basic plane is shown as an emoji by default
/// (Unicode's `Emoji_Presentation`), rather than as text.
fn emoji_presentation(unit: u32) -> bool {
    matches!(
        unit,
        0x231A..=0x231B
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
    )
}

/// The characters of UTF-16 text a font has glyphs for, unit by unit; a
/// surrogate pair counts as the font has its character.
fn coverage(dc: HDC, handle: HFONT, units: &[u16]) -> Vec<bool> {
    const GGI_MARK_NONEXISTING_GLYPHS: u32 = 1;
    let mut glyphs = alloc::vec![0u16; units.len()];
    // SAFETY: the DC and font are live; the buffers have the text's length.
    unsafe {
        SelectObject(dc, handle);
        GetGlyphIndicesW(
            dc,
            units.as_ptr(),
            units.len() as i32,
            glyphs.as_mut_ptr(),
            GGI_MARK_NONEXISTING_GLYPHS,
        );
    }
    let mut covered: Vec<bool> = glyphs.iter().map(|glyph| *glyph != 0xFFFF).collect();
    for (index, unit) in units.iter().enumerate() {
        // White space and controls never fall back; GDI cannot check the
        // characters of surrogate pairs, so those need a fallback font.
        if *unit <= 0x20 || (0xD800..0xE000).contains(unit) {
            covered[index] = *unit <= 0x20;
        }
    }
    covered
}

/// Whether text with a unit needs shaping by Uniscribe: complex scripts,
/// combining marks, joiners, bidirectional controls and surrogates.
fn needs_shaping(unit: u16) -> bool {
    matches!(
        unit,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x08FF
            | 0x0900..=0x0DFF
            | 0x0E00..=0x0FFF
            | 0x1000..=0x109F
            | 0x1100..=0x11FF
            | 0x1780..=0x18AF
            | 0x1A00..=0x1CFF
            | 0x1DC0..=0x1DFF
            | 0x200C..=0x200F
            | 0x202A..=0x202E
            | 0x2066..=0x2069
            | 0x20D0..=0x20FF
            | 0xA800..=0xABFF
            | 0xD800..=0xDFFF
            | 0xFB1D..=0xFDFF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFE70..=0xFEFF
    )
}

/// The glyphs of text in the DC's font, when it needs no shaping and the
/// font has them all. GDI's text functions substitute other fonts for some
/// symbols the font has, such as KaTeX's sums; drawing and measuring the
/// glyphs keeps them in the font chosen for them.
pub(crate) fn simple_glyphs(dc: HDC, units: &[u16]) -> Option<Vec<u16>> {
    const GGI_MARK_NONEXISTING_GLYPHS: u32 = 1;
    if units.iter().any(|unit| needs_shaping(*unit)) {
        return None;
    }
    let mut glyphs = alloc::vec![0u16; units.len()];
    // SAFETY: the DC is live; the buffers have the text's length.
    unsafe {
        GetGlyphIndicesW(
            dc,
            units.as_ptr(),
            units.len() as i32,
            glyphs.as_mut_ptr(),
            GGI_MARK_NONEXISTING_GLYPHS,
        );
    }
    (!glyphs.contains(&0xFFFF)).then_some(glyphs)
}

/// A face's horizontal metrics from its `head`, `hhea` and `hmtx` tables:
/// its units per em and its glyphs' advances in those units, and its pair
/// kerning.
struct Design {
    em: f32,
    advances: Vec<u16>,
    kerning: kerning::Kerning,
    /// Ascent, descent and line gap in font units, as DirectWrite reports
    /// them: the typographic metrics when `OS/2` says to use them, else the
    /// Windows ascent and descent with what `hhea` adds as the gap.
    line: Option<(f32, f32, f32)>,
    /// The `gasp` table's version and its ranges: the largest size in
    /// pixels per em each covers, and how to rasterize there.
    gasp: Option<(u16, Vec<(u16, u16)>)>,
    /// Whether the face holds TrueType instructions (`maxp`).
    hinted: bool,
}

/// A face as GDI selected it: its family name, weight and slant.
type FaceKey = (Vec<u16>, i32, u8);

local! {
    /// Design metrics by face; `None` for faces without the tables.
    static DESIGNS: RefCell<HashMap<FaceKey, Option<Rc<Design>>>> =
        RefCell::new(HashMap::new());
}

/// A table of the DC's font, or as much of it as `length` asks for.
fn font_table(dc: HDC, tag: &[u8; 4], length: Option<u32>) -> Option<Vec<u8>> {
    let tag = u32::from_le_bytes(*tag);
    let size = match length {
        Some(length) => length,
        // SAFETY: the DC is live; a null buffer asks for the table's size.
        None => unsafe { GetFontData(dc, tag, 0, core::ptr::null_mut(), 0) },
    };
    if size == 0 || size == GDI_ERROR as u32 {
        return None;
    }
    let mut data = alloc::vec![0u8; size as usize];
    // SAFETY: the buffer holds `size` bytes.
    let read = unsafe { GetFontData(dc, tag, 0, data.as_mut_ptr().cast(), size) };
    (read == size).then_some(data)
}

/// The design metrics of the DC's font.
fn design(dc: HDC) -> Option<Rc<Design>> {
    let mut logfont = LOGFONTW::default();
    // SAFETY: reads the selected font's description into a LOGFONTW.
    unsafe {
        let font = GetCurrentObject(dc, OBJ_FONT as u32);
        GetObjectW(
            font,
            size_of::<LOGFONTW>() as i32,
            (&raw mut logfont).cast(),
        );
    }
    let key = (
        logfont.lfFaceName.to_vec(),
        logfont.lfWeight,
        logfont.lfItalic,
    );
    if let Some(known) = DESIGNS.with_borrow(|designs| designs.get(&key).cloned()) {
        return known;
    }
    let read = || {
        let word = |data: &[u8], at: usize| {
            data.get(at..at + 2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        };
        let head = font_table(dc, b"head", Some(54))?;
        let hhea = font_table(dc, b"hhea", Some(36))?;
        let hmtx = font_table(dc, b"hmtx", None)?;
        let em = word(&head, 18).filter(|em| *em > 0)?;
        let count = usize::from(word(&hhea, 34)?).min(hmtx.len() / 4);
        let advances: Vec<u16> = (0..count)
            .filter_map(|index| word(&hmtx, index * 4))
            .collect();
        let signed = |data: &[u8], at: usize| word(data, at).map(|word| f32::from(word as i16));
        let line = font_table(dc, b"OS/2", None).and_then(|os2| {
            if word(&os2, 62)? & 0x80 != 0 {
                return Some((signed(&os2, 68)?, -signed(&os2, 70)?, signed(&os2, 72)?));
            }
            let (ascent, descent) = (f32::from(word(&os2, 74)?), f32::from(word(&os2, 76)?));
            let height = signed(&hhea, 4)? - signed(&hhea, 6)? + signed(&hhea, 8)?;
            Some((ascent, descent, (height - ascent - descent).max(0.0)))
        });
        let gasp = font_table(dc, b"gasp", None).and_then(|gasp| {
            let version = word(&gasp, 0).filter(|version| *version <= 1)?;
            let count = usize::from(word(&gasp, 2)?);
            let ranges = (0..count)
                .map(|index| Some((word(&gasp, 4 + index * 4)?, word(&gasp, 6 + index * 4)?)))
                .collect::<Option<Vec<_>>>()?;
            Some((version, ranges))
        });
        let hinted = font_table(dc, b"maxp", Some(32)).is_some_and(|maxp| {
            maxp[..4] == [0, 1, 0, 0] && word(&maxp, 26).is_some_and(|size| size != 0)
        });
        (!advances.is_empty()).then(|| {
            Rc::new(Design {
                em: f32::from(em),
                line,
                gasp,
                hinted,
                advances,
                kerning: kerning::Kerning::parse(
                    font_table(dc, b"GPOS", None).as_deref(),
                    font_table(dc, b"kern", None).as_deref(),
                ),
            })
        })
    };
    let design = read();
    DESIGNS.with_borrow_mut(|designs| designs.insert(key, design.clone()));
    design
}

/// How Skia has DirectWrite smooth the DC's font at `size` pixels per em
/// (`SkScalerContext_DW`): the size it rasterizes at, and whether it smooths
/// vertically as well. A `gasp` table from version 1 on says, for the size;
/// otherwise large text and faces without instructions are smoothed both
/// ways, and small hinted text at the GDI size only across.
pub(crate) fn smoothing(dc: HDC, size: f32) -> (f32, bool) {
    design(dc).map_or((size, true), |design| {
        smoothing_for(design.gasp.as_ref(), design.hinted, size)
    })
}

/// How Skia smooths a face with a `gasp` table and instructions, or
/// without, at `size` pixels per em: see [`smoothing`].
fn smoothing_for(gasp: Option<&(u16, Vec<(u16, u16)>)>, hinted: bool, size: f32) -> (f32, bool) {
    // GDI sizes are kept to sixty-fourths of a pixel.
    let gdi = (size * 64.0).round() / 64.0;
    let ppem = gdi.round() as i32;
    let range = gasp.and_then(|(version, ranges)| {
        let mut below = -1;
        ranges.iter().find_map(|(max, flags)| {
            let found = (below < ppem && ppem <= i32::from(*max)).then_some((*version, *flags));
            below = i32::from(*max);
            found
        })
    });
    match range {
        Some((version, flags)) if version >= 1 => (size, flags & GASP_SYMMETRIC_SMOOTHING != 0),
        _ if size > 20.0 || !hinted => (size, true),
        _ => (gdi, false),
    }
}

/// The `gasp` flag for smoothing in both directions.
const GASP_SYMMETRIC_SMOOTHING: u16 = 0x0008;

/// How far a line of the DC's font at `size` pixels per em reaches above and
/// below the baseline at `line-height: normal`, in device pixels: its ascent
/// and descent, each rounded and with half the rounded line gap, as Chromium
/// measures them.
/// The ascent and descent DirectWrite reports for the DC's font at `size`
/// pixels per em, each rounded to whole pixels, as Chromium's content area
/// is: the typographic metrics of a face that says to use them, not GDI's.
pub(crate) fn content_metrics(dc: HDC, size: f32) -> Option<(f32, f32)> {
    let design = design(dc)?;
    let em = size / design.em;
    let (ascent, descent, _) = design.line?;
    Some(((ascent * em).round(), (descent * em).round()))
}

fn line_extent(dc: HDC, size: f32) -> (f32, f32) {
    if let Some((ascent, descent, gap)) = design(dc).and_then(|design| {
        let scale = size / design.em;
        design
            .line
            .map(|(ascent, descent, gap)| (ascent * scale, descent * scale, gap * scale))
    }) {
        let gap = gap.round() / 2.0;
        return (ascent.round() + gap, descent.round() + gap);
    }
    let mut metrics = TEXTMETRICW::default();
    // SAFETY: `metrics` is writable and the DC has a font.
    unsafe { GetTextMetricsW(dc, &mut metrics) };
    let gap = metrics.tmExternalLeading as f32 / 2.0;
    (
        metrics.tmAscent as f32 + gap,
        metrics.tmDescent as f32 + gap,
    )
}

/// The advances in device pixels of glyphs of the DC's font at `size`
/// pixels per em: the face's own, unhinted, as Chromium lays text out,
/// rather than GDI's, rounded to whole pixels at the font's whole-pixel
/// size, with pairs kerned when `kern` is set. `None` for faces without the
/// tables.
pub(crate) fn advances(dc: HDC, glyphs: &[u16], size: f32, kern: bool) -> Option<Vec<f32>> {
    let design = design(dc)?;
    let last = *design.advances.last()?;
    let scale = size / design.em;
    let mut advances: Vec<f32> = glyphs
        .iter()
        .map(|glyph| {
            // Glyphs past the table's metrics share its last advance.
            f32::from(
                design
                    .advances
                    .get(usize::from(*glyph))
                    .copied()
                    .unwrap_or(last),
            ) * scale
        })
        .collect();
    // Pairs kern, as Chromium's shaper kerns them by default.
    if kern && !design.kerning.is_empty() {
        for (index, pair) in glyphs.windows(2).enumerate() {
            advances[index] += design.kerning.pair(pair[0], pair[1]) as f32 * scale;
        }
    }
    Some(advances)
}

/// The width in device pixels of text in the DC's font at `size` pixels
/// per em: from the face's advances when it needs no shaping, else GDI's.
pub(crate) fn width(dc: HDC, units: &[u16], size: f32, kern: bool) -> f32 {
    simple_glyphs(dc, units)
        .and_then(|glyphs| advances(dc, &glyphs, size, kern))
        .map_or_else(
            || extent(dc, units) as f32,
            |advances| advances.iter().sum(),
        )
}

/// The thickness of the DC's font's underline in device pixels, from its
/// outline metrics; `None` for fonts without them.
pub(crate) fn underline_thickness(dc: HDC) -> Option<i32> {
    // SAFETY: a null buffer asks for the size the metrics take.
    let size = unsafe { GetOutlineTextMetricsW(dc, 0, core::ptr::null_mut()) };
    if (size as usize) < size_of::<OUTLINETEXTMETRICW>() {
        return None;
    }
    // The metrics are followed by their strings; u32s keep them aligned.
    let mut buffer = vec![0_u32; (size as usize).div_ceil(4)];
    let metrics = buffer.as_mut_ptr().cast::<OUTLINETEXTMETRICW>();
    // SAFETY: the buffer holds `size` bytes, aligned for the metrics.
    if unsafe { GetOutlineTextMetricsW(dc, size, metrics) } == 0 {
        return None;
    }
    // SAFETY: GDI filled the metrics.
    Some(unsafe { (*metrics).otmsUnderscoreSize as i32 })
}

/// The width in device pixels of text in the DC's font.
pub(crate) fn extent(dc: HDC, units: &[u16]) -> i32 {
    let mut size = SIZE::default();
    // SAFETY: the DC and font are live; the buffers match their lengths and
    // `size` is writable.
    unsafe {
        match simple_glyphs(dc, units) {
            Some(mut glyphs) => {
                GetTextExtentPointI(dc, glyphs.as_mut_ptr(), glyphs.len() as i32, &mut size)
            }
            None => GetTextExtentPoint32W(dc, units.as_ptr(), units.len() as i32, &mut size),
        };
    }
    size.cx
}

/// The character starting at a unit of UTF-16 text.
fn character_at(units: &[u16], index: usize) -> u32 {
    char::decode_utf16(units[index..].iter().copied())
        .next()
        .and_then(Result::ok)
        .map_or(u32::from(units[index]), u32::from)
}

/// A piece of text and the GDI font that shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub font: HFONT,
    pub range: core::ops::Range<usize>,
    /// The family DirectWrite lays the piece out and draws it in: an emoji
    /// font, whose color glyphs GDI cannot draw, or the requested font when
    /// the text asks for OpenType features GDI does not apply.
    pub direct: Option<String>,
}

/// The OpenType tag of pair kerning.
const KERN: u32 = u32::from_le_bytes(*b"kern");

/// Whether text in a font kerns its glyph pairs: unless `font-kerning: none`
/// or `font-feature-settings` turns `kern` off.
pub(crate) fn kerns(font: &Font) -> bool {
    !font.features.contains(&(KERN, 0))
}

/// Whether a family is an emoji font, as "Segoe UI Emoji" is.
fn is_emoji(family: &str) -> bool {
    family.to_ascii_lowercase().contains("emoji")
}

/// Pieces of UTF-16 text and the GDI fonts that show them: the requested
/// font where it has glyphs, else the first of the remaining CSS families
/// and then the script's system fonts that has them. The pieces keep the
/// requested size, weight and style.
pub(crate) fn runs(
    dc: HDC,
    font: &Font,
    scale: f32,
    underline: bool,
    strike: bool,
    units: &[u16],
) -> Vec<Run> {
    let primary = self::font(font, scale, underline, strike);
    let primary_family = face(&font.families, font.weight, font.italic)
        .map_or_else(|| family(&font.families), |(name, _, _)| name);
    // Text asking for OpenType features is drawn by DirectWrite in the
    // requested font. Kerning on or off GDI's text applies itself.
    let featured = font
        .features
        .iter()
        .any(|(tag, _)| *tag != KERN)
        .then(|| primary_family.clone());
    let mut emoji: Vec<(HFONT, String)> = Vec::new();
    if is_emoji(&primary_family) {
        emoji.push((primary, primary_family));
    }
    // The faces of the first family: one, or the parts `unicode-range`
    // splits a web font into, each showing its own characters.
    let declared = faces(&font.families, font.weight, font.italic);
    let ranges = declared
        .first()
        .map(|(.., ranges)| ranges.clone())
        .unwrap_or_default();
    let mut owner: Vec<Option<HFONT>> = coverage(dc, primary, units)
        .into_iter()
        .enumerate()
        .map(|(index, covered)| {
            (covered && (units[index] <= 0x20 || in_ranges(&ranges, character_at(units, index))))
                .then_some(primary)
        })
        .collect();
    for (name, weight, italic, ranges) in declared.into_iter().skip(1) {
        if owner.iter().all(Option::is_some) {
            break;
        }
        let sibling = handle(Key {
            family: name,
            height: (font.size * scale).round().max(1.0) as i32,
            weight,
            italic,
            underline,
            strike,
            grayscale: false,
        });
        for (index, covered) in coverage(dc, sibling, units).into_iter().enumerate() {
            if owner[index].is_none() && covered && in_ranges(&ranges, character_at(units, index)) {
                owner[index] = Some(sibling);
            }
        }
    }
    // Characters asking for emoji presentation use an emoji font, from
    // Chromium 131 on even when the requested font has them.
    if emoji.is_empty() && CHROMIUM.with(Cell::get) >= 131 {
        for index in 0..units.len().saturating_sub(1) {
            if units[index + 1] == 0xFE0F {
                owner[index] = None;
            }
        }
    }
    let mut index = 0;
    while index < units.len() {
        if owner[index].is_some() {
            index += 1;
            continue;
        }
        // One pass per script: its characters share fallback fonts.
        let character = character_at(units, index);
        let script = fallbacks_at(units, index);
        let missing_end = (index..units.len())
            .find(|position| {
                owner[*position].is_some()
                    || (!(0xDC00..0xE000).contains(&units[*position])
                        && !core::ptr::eq(fallbacks_at(units, *position), script))
            })
            .unwrap_or(units.len());
        // GDI cannot tell whether a font has a character beyond the basic
        // plane, such as an emoji: only emoji fonts are trusted with them.
        let supplementary = character > 0xFFFF;
        let families = font
            .families
            .iter()
            .skip(1)
            .map(String::as_str)
            .filter(|family| !supplementary || family.to_ascii_lowercase().contains("emoji"))
            .chain(script.iter().copied());
        for family in families {
            let candidate = Font {
                families: alloc::vec![family.to_owned()],
                ..font.clone()
            };
            if face(&candidate.families, font.weight, font.italic).is_none() && !installed(family) {
                continue;
            }
            let handle = self::font(&candidate, scale, underline, strike);
            if is_emoji(family) && !emoji.iter().any(|(known, _)| *known == handle) {
                emoji.push((handle, family.to_owned()));
            }
            let piece = &units[index..missing_end];
            let covered = coverage(dc, handle, piece);
            for (offset, covers) in covered.iter().enumerate() {
                let unit = piece[offset];
                let pair = (0xD800..0xE000).contains(&unit);
                if owner[index + offset].is_none() && (*covers || pair) {
                    owner[index + offset] = Some(handle);
                }
            }
            if owner[index].is_some() {
                break;
            }
        }
        // A character no font has shows the requested font's missing glyph.
        if owner[index].is_none() {
            owner[index] = Some(primary);
        }
        // The low half of a pair goes with its high half.
        if (0xD800..0xDC00).contains(&units[index]) && index + 1 < units.len() {
            owner[index + 1] = owner[index];
            index += 1;
        }
        index += 1;
    }
    // Joiners stay in the font of what they join, so emoji sequences such as
    // a family or a keycap shape as one emoji.
    for index in 1..units.len() {
        if is_joiner(units[index]) {
            owner[index] = owner[index - 1];
        }
    }
    let mut runs: Vec<Run> = Vec::new();
    for (position, handle) in owner.into_iter().enumerate() {
        let handle = handle.unwrap_or(primary);
        match runs.last_mut() {
            Some(last) if last.font == handle => last.range.end = position + 1,
            _ => runs.push(Run {
                font: handle,
                range: position..position + 1,
                direct: emoji
                    .iter()
                    .find(|(known, _)| *known == handle)
                    .map(|(_, family)| family.clone())
                    .or_else(|| featured.clone().filter(|_| handle == primary)),
            }),
        }
    }
    runs
}

/// How many widths of one font a measurer keeps.
const KEPT_WIDTHS: usize = 20_000;

/// The measurements of one font at one scale.
#[derive(Debug)]
struct Measured {
    font: Font,
    scale: f32,
    widths: HashMap<String, f32>,
    metrics: Option<FontMetrics>,
    /// Extents at `line-height: normal` of text with other characters than
    /// ASCII, by text, and of the font alone.
    extents: HashMap<String, (f32, f32)>,
    extent: Option<(f32, f32)>,
}

/// Measures text with the GDI fonts used for painting.
#[derive(Debug)]
pub struct GdiMeasurer {
    dc: HDC,
    scale: Cell<f32>,
    /// Earlier measurements: layout measures the same words again and
    /// again, at every pass.
    measured: RefCell<Vec<Measured>>,
    /// The loaded faces the measurements were made with.
    generation: Cell<u32>,
}

impl GdiMeasurer {
    /// A measurer for 96 DPI.
    pub fn new() -> GdiMeasurer {
        GdiMeasurer {
            // SAFETY: a memory DC compatible with the screen needs no window.
            dc: unsafe { CreateCompatibleDC(core::ptr::null_mut()) },
            scale: Cell::new(1.0),
            measured: RefCell::default(),
            generation: Cell::new(GENERATION.with(Cell::get)),
        }
    }

    /// Works with the measurements of a font at the current scale.
    fn cached<T>(&self, font: &Font, work: impl FnOnce(&mut Measured) -> T) -> T {
        let scale = self.scale.get();
        let mut measured = self.measured.borrow_mut();
        // Fonts loaded since may replace the ones measured.
        let generation = GENERATION.with(Cell::get);
        if self.generation.get() != generation {
            measured.clear();
            self.generation.set(generation);
        }
        let index = match measured
            .iter()
            .position(|entry| entry.scale == scale && entry.font == *font)
        {
            Some(index) => index,
            None => {
                measured.push(Measured {
                    font: font.clone(),
                    scale,
                    widths: HashMap::new(),
                    metrics: None,
                    extents: HashMap::new(),
                    extent: None,
                });
                measured.len() - 1
            }
        };
        work(&mut measured[index])
    }

    /// Measures for a window scale so that measurements match painting.
    pub fn set_scale(&self, scale: f32) {
        self.scale.set(scale);
    }

    fn select(&self, request: &Font) {
        let handle = font(request, self.scale.get(), false, false);
        // SAFETY: the DC and the cached font are live.
        unsafe { SelectObject(self.dc, handle) };
    }
}

impl Default for GdiMeasurer {
    fn default() -> Self {
        GdiMeasurer::new()
    }
}

impl Drop for GdiMeasurer {
    fn drop(&mut self) {
        // SAFETY: the DC was created by this measurer.
        unsafe { DeleteDC(self.dc) };
    }
}

impl TextMeasurer for GdiMeasurer {
    fn set_scale(&self, scale: f32) {
        GdiMeasurer::set_scale(self, scale);
    }

    fn device_pixel(&self) -> f32 {
        1.0 / self.scale.get()
    }

    fn scrollbar_look(&self) -> nativeboat_ir::scrollbar::Look {
        nativeboat_ir::scrollbar::Look::of_chromium(CHROMIUM.with(Cell::get))
    }

    fn chromium(&self) -> u32 {
        CHROMIUM.with(Cell::get)
    }

    fn standard_family(&self) -> &'static str {
        standard()
    }

    fn generation(&self) -> u64 {
        (u64::from(GENERATION.with(Cell::get)) << 32) | u64::from(self.scale.get().to_bits())
    }

    fn width(&self, text: &str, font: &Font) -> f32 {
        if let Some(width) = self.cached(font, |entry| entry.widths.get(text).copied()) {
            return width;
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut pixels = 0.0;
        let scale = self.scale.get();
        // Text is laid out at its exact size, which GDI draws at whole
        // pixels.
        let size = font.size * scale;
        for run in runs(self.dc, font, scale, false, false, &units) {
            let piece = &units[run.range];
            // Emoji and text with OpenType features are laid out as
            // DirectWrite draws them.
            if let Some(width) = run.direct.and_then(|family| {
                crate::dwrite::width(
                    piece,
                    &family,
                    pixel_size(font, scale),
                    font.weight,
                    font.italic,
                    &font.features,
                )
            }) {
                pixels += width;
                continue;
            }
            // SAFETY: the DC and font are live.
            unsafe { SelectObject(self.dc, run.font) };
            pixels += self::width(self.dc, piece, size, kerns(font));
        }
        let width = pixels / self.scale.get();
        self.cached(font, |entry| {
            // Text that keeps changing, such as a clock, starts over.
            if entry.widths.len() >= KEPT_WIDTHS {
                entry.widths.clear();
            }
            entry.widths.insert(text.to_owned(), width);
        });
        width
    }

    fn normal_extent(&self, text: &str, font: &Font) -> Option<(f32, f32)> {
        if font.size <= 0.0 {
            return Some((0.0, 0.0));
        }
        // ASCII text is in the font itself, which may well have it all.
        let ascii = text.is_ascii();
        if let Some(extent) = self.cached(font, |entry| {
            if ascii {
                entry.extent
            } else {
                entry.extents.get(text).copied()
            }
        }) {
            return Some(extent);
        }
        let scale = self.scale.get();
        let size = font.size * scale;
        let primary = self::font(font, scale, false, false);
        let extent = |handle: HFONT| {
            // SAFETY: the DC and the cached font are live.
            unsafe { SelectObject(self.dc, handle) };
            line_extent(self.dc, size)
        };
        let (mut above, mut below) = extent(primary);
        if !ascii {
            let units: Vec<u16> = text.encode_utf16().collect();
            let mut seen = alloc::vec![primary];
            for run in runs(self.dc, font, scale, false, false, &units) {
                if !seen.contains(&run.font) {
                    seen.push(run.font);
                    let (top, bottom) = extent(run.font);
                    above = above.max(top);
                    below = below.max(bottom);
                }
            }
        }
        let extent = (above / scale, below / scale);
        self.cached(font, |entry| {
            if ascii {
                entry.extent = Some(extent);
            } else {
                if entry.extents.len() >= KEPT_WIDTHS {
                    entry.extents.clear();
                }
                entry.extents.insert(text.to_owned(), extent);
            }
        });
        Some(extent)
    }

    fn metrics(&self, font: &Font) -> FontMetrics {
        if let Some(metrics) = self.cached(font, |entry| entry.metrics) {
            return metrics;
        }
        self.select(font);
        let scale = self.scale.get();
        // The ascent and descent DirectWrite reports, each rounded at the
        // device size, as Chromium's content area is: the typographic
        // metrics of a face that says to use them, not GDI's.
        let metrics = match content_metrics(self.dc, font.size * scale) {
            Some((ascent, descent)) => FontMetrics {
                ascent: ascent / scale,
                descent: descent / scale,
            },
            None => {
                let mut metrics = TEXTMETRICW::default();
                // SAFETY: `metrics` is writable.
                unsafe { GetTextMetricsW(self.dc, &mut metrics) };
                FontMetrics {
                    ascent: metrics.tmAscent as f32 / scale,
                    descent: metrics.tmDescent as f32 / scale,
                }
            }
        };
        self.cached(font, |entry| entry.metrics = Some(metrics));
        metrics
    }
}

mod kerning;

mod instance;

#[cfg(test)]
mod tests;
