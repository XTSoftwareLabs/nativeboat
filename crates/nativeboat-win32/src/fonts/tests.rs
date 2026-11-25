use super::*;
use shimforge::{Session, mock};

fn request(families: &[&str], size: f32) -> Font {
    Font {
        families: families.iter().map(|family| (*family).to_owned()).collect(),
        size,
        weight: 400,
        italic: false,
        features: Vec::new(),
    }
}

fn names(families: &[&str]) -> Vec<String> {
    families.iter().map(|family| (*family).to_owned()).collect()
}

#[test]
fn generic_and_installed_families() {
    // Generic families stand for the first installed font of the locale's
    // list; names Windows lacks, as Apple's system font, are skipped.
    for generic_name in ["sans-serif", "serif", "monospace", "cursive", "fantasy"] {
        let list = generic(generic_name).unwrap();
        assert_eq!(
            family(&names(&["Nope Sans", generic_name])),
            first_available(list)
        );
        assert!(list.contains(&family(&names(&[generic_name])).as_str()));
    }
    assert_eq!(generic("fantasy"), Some(&["Impact"][..]));
    assert_eq!(generic("Arial"), None);
    assert_eq!(family(&names(&["system-ui"])), system_ui());
    assert!(installed(&system_ui()));
    assert_eq!(family(&names(&["-apple-system", "Arial"])), "Arial");
    assert_eq!(family(&names(&["BlinkMacSystemFont", "Arial"])), "Arial");
    assert_eq!(family(&names(&["Arial"])), "Arial");
    let sans = first_available(generic("sans-serif").unwrap());
    assert_eq!(family(&names(&["Missing Font 123"])), sans);
    assert_eq!(family(&[]), sans);
    assert_eq!(first_available(&["Missing Font 123", "Arial"]), "Arial");
    assert_eq!(first_available(&["Missing Font 123"]), "Missing Font 123");
}

#[test]
fn the_application_locale_and_release_pick_generic_families() {
    // An application that ships only English resources gets English fonts
    // on a Chinese Windows, as in Electron.
    follow_locale("en-US");
    assert_eq!(generic("sans-serif"), Some(&["Arial"][..]));
    assert_eq!(generic("serif"), Some(&["Times New Roman"][..]));
    assert_eq!(standard(), "Times New Roman");
    // Electron 36 made Consolas the fixed font where text uses ClearType.
    follow_chromium(124);
    assert_eq!(generic("monospace"), Some(&["Courier New"][..]));
    follow_chromium(136);
    let fixed = if cleartype() {
        "Consolas"
    } else {
        "Courier New"
    };
    assert_eq!(generic("monospace"), Some(&[fixed][..]));
    assert_eq!(cleartype(), cleartype());
    follow_locale("zh-CN");
    assert_eq!(generic("monospace"), Some(&["NSimSun"][..]));
    follow_locale("zh_Hant_TW");
    assert_eq!(generic("cursive"), Some(&["DFKai-SB"][..]));
    follow_locale("ja");
    assert_eq!(
        generic("monospace"),
        Some(&["BIZ UDGothic", "MS Gothic"][..])
    );
    follow_locale("ko-KR");
    assert_eq!(generic("monospace"), Some(&["GulimChe"][..]));
    follow_locale("th");
    assert_eq!(generic("sans-serif"), Some(&["Tahoma"][..]));
    LOCALE.with(|locale| locale.set(None));
    CHROMIUM.with(|chromium| chromium.set(u32::MAX));
}

#[test]
fn font_lists_follow_the_chromium_release() {
    // Chromium 128 put Noto CJK fonts first in the default fonts and the
    // script fallbacks; Electron 30's Chromium 124 starts from the system's.
    assert_eq!(
        released_in(124, script_fallbacks(0x4E2D)),
        ["Microsoft YaHei", "SimSun"]
    );
    assert_eq!(released_in(124, script_fallbacks(0x3042))[0], "Meiryo");
    assert_eq!(
        released_in(126, script_fallbacks(0xAC00))[0],
        "Malgun Gothic"
    );
    assert_eq!(
        released_in(124, &["BIZ UDGothic", "MS Gothic"]),
        ["MS Gothic"]
    );
    assert_eq!(released_in(124, &["Arial"]), ["Arial"]);
    assert_eq!(
        released_in(128, script_fallbacks(0x4E2D))[0],
        "Noto Sans SC"
    );
    // Without a release to follow, the latest.
    assert_eq!(script_fallbacks(0x4E2D)[0], "Noto Sans SC");
}

#[test]
fn fonts_are_cached() {
    let first = font(&request(&["Arial"], 12.0), 1.0, false, false);
    assert!(!first.is_null());
    assert_eq!(font(&request(&["Arial"], 12.0), 1.0, false, false), first);
    assert_ne!(font(&request(&["Arial"], 12.0), 2.0, true, true), first);
}

#[test]
fn measuring_text() {
    let measurer = GdiMeasurer::default();
    assert!(format!("{measurer:?}").contains("GdiMeasurer"));
    let arial = request(&["Arial"], 20.0);
    let width = measurer.width("Hello", &arial);
    assert!(width > 20.0 && width < 100.0, "{width}");
    assert_eq!(measurer.width("", &arial), 0.0);
    let metrics = measurer.metrics(&arial);
    assert!(
        metrics.ascent > 10.0 && metrics.descent > 2.0,
        "{metrics:?}"
    );
    // Measurements are kept, per font and scale.
    assert_eq!(measurer.width("Hello", &arial), width);
    assert_eq!(measurer.metrics(&arial), metrics);
    measurer.set_scale(2.0);
    let scaled = measurer.width("Hello", &arial);
    assert!((scaled - width).abs() < 3.0, "{scaled} {width}");
    // Changing text does not keep growing the measurements.
    for count in 0..=KEPT_WIDTHS {
        measurer.width(&count.to_string(), &arial);
    }
    let kept = measurer.measured.borrow();
    let entry = kept
        .iter()
        .find(|entry| entry.scale == 2.0 && entry.font == arial)
        .unwrap();
    assert!(entry.widths.len() < 10, "{}", entry.widths.len());
}

#[test]
fn enumeration_reports_missing_families() {
    let mut session = Session::new();
    let enumerate = mock!(
        session,
        EnumFontFamiliesExW,
        unsafe extern "system" fn(HDC, *const LOGFONTW, FONTENUMPROCW, LPARAM, u32) -> i32
    );
    enumerate.expect().once().returns(1);
    assert!(!installed("Mocked Family"));
}

#[test]
fn declared_fonts_serve_their_css_family_by_weight_and_style() {
    let regular = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    let bold = std::fs::read("C:/Windows/Fonts/arialbd.ttf").unwrap();
    assert_eq!(sfnt_family(&regular).as_deref(), Some("Arial"));
    assert!(sfnt_family(b"not a font").is_none());
    assert!(!register_font("Page Face", 400, false, b"not a font"));
    let measurer = GdiMeasurer::new();
    let before = measurer.width("Wide words", &request(&["Page Face", "serif"], 20.0));
    assert!(register_font("Page Face", 400, false, &regular));
    assert!(register_font("page face", 700, false, &bold));
    // The family list's declared family wins over later fallbacks, and
    // earlier measurements start over.
    let after = measurer.width("Wide words", &request(&["Page Face", "serif"], 20.0));
    assert_ne!(before, after);
    // Each face is a family of its own to GDI, named after the font's.
    let (regular_name, regular_weight, regular_italic) =
        face(&names(&["Missing", "PAGE FACE"]), 400, false).unwrap();
    assert!(regular_name.starts_with("Arial#"));
    assert_eq!((regular_weight, regular_italic), (400, false));
    // The closest weight is used: bolder requests use the bold face, and
    // light ones the regular face.
    assert_eq!(face(&names(&["Page Face"]), 800, false).unwrap().1, 700);
    assert_eq!(face(&names(&["Page Face"]), 300, false).unwrap().1, 400);
    // An upright face is slanted for italic text.
    assert_eq!(
        face(&names(&["Page Face"]), 500, true),
        Some((regular_name, 400, true))
    );
    assert!(face(&names(&["Other"]), 400, false).is_none());
    // CSS weight matching order.
    assert!(weight_distance(400, 500) < weight_distance(400, 300));
    assert!(weight_distance(300, 200) < weight_distance(300, 400));
    assert!(weight_distance(700, 800) < weight_distance(700, 600));
    assert_eq!(weight_distance(600, 600), 0);
}

#[test]
fn characters_a_font_lacks_fall_back_to_fonts_that_have_them() {
    let measurer = GdiMeasurer::new();
    let units: Vec<u16> = "ab 中文😀c".encode_utf16().collect();
    let font = request(&["Arial"], 16.0);
    let pieces = runs(measurer.dc, &font, 1.0, false, false, &units);
    let primary = self::font(&font, 1.0, false, false);
    // Latin text and the space stay in Arial; Han characters and the emoji
    // (one surrogate pair) use fallback fonts; the last letter returns.
    assert_eq!(pieces.first().unwrap().font, primary);
    assert_eq!(pieces.first().unwrap().range, 0..3);
    assert_eq!(pieces.last().unwrap().font, primary);
    assert_eq!(pieces.last().unwrap().range, 7..8);
    assert!(
        pieces[1..pieces.len() - 1]
            .iter()
            .all(|run| run.font != primary)
    );
    let han = pieces.iter().find(|run| run.range.start == 3).unwrap();
    assert_eq!(han.range, 3..5);
    assert!(han.direct.is_none());
    // Weights between a system family's faces use the family Windows names
    // for the weight, as DirectWrite matches it within the family.
    let (semibold, weight) = system_face("Segoe UI".to_owned(), 600);
    assert!(semibold.eq_ignore_ascii_case("Segoe UI Semibold") && weight == 600);
    let (light, weight) = system_face("Segoe UI".to_owned(), 300);
    assert!(light.eq_ignore_ascii_case("Segoe UI Light") && weight == 300);
    let (black, weight) = system_face("Segoe UI".to_owned(), 950);
    assert!(black.eq_ignore_ascii_case("Segoe UI Black") && weight == 900);
    assert_eq!(
        system_face("Segoe UI".to_owned(), 500),
        ("Segoe UI".to_owned(), 400)
    );
    // GDI is asked for the matched face's weight: the family's bold face
    // for 600, as Chromium picks Microsoft YaHei Bold, not its regular one.
    assert_eq!(
        system_face("Arial".to_owned(), 600),
        ("Arial".to_owned(), 700)
    );
    assert_eq!(
        system_face("Arial".to_owned(), 700),
        ("Arial".to_owned(), 700)
    );
    // The emoji's font is drawn in color.
    let emoji = pieces.iter().find(|run| run.range.start == 5).unwrap();
    assert_eq!(emoji.direct.as_deref(), Some("Segoe UI Emoji"));
    // Ranges cover the text without gaps.
    assert!(
        pieces
            .windows(2)
            .all(|pair| pair[0].range.end == pair[1].range.start)
    );
    // Fallback text measures as it is drawn.
    assert!(measurer.width("中文", &font) > measurer.width("ab", &font));
    assert_eq!(script_fallbacks(0x3042)[0], "Noto Sans JP");
    assert_eq!(script_fallbacks(0xAC00)[0], "Noto Sans KR");
    assert_eq!(script_fallbacks(0x1F600)[0], "Segoe UI Emoji");
    assert_eq!(script_fallbacks(0x2192)[0], "Segoe UI Symbol");
    // Text in an emoji font is drawn in color, and its sequences measure as
    // one emoji.
    let emoji_font = request(&["Segoe UI Emoji"], 20.0);
    let units: Vec<u16> = "😀".encode_utf16().collect();
    let pieces = runs(measurer.dc, &emoji_font, 1.0, false, false, &units);
    assert_eq!(pieces[0].direct.as_deref(), Some("Segoe UI Emoji"));
    let single = measurer.width("👨", &font);
    assert!(single > 0.0);
    assert!(measurer.width("👨\u{200d}👩\u{200d}👧", &font) < single * 1.5);
    // U+FE0F asks for an emoji font even where the requested font has the
    // character; the selector and a keycap stay with it.
    for text in ["a\u{2764}\u{fe0f}", "a1\u{fe0f}\u{20e3}"] {
        let units: Vec<u16> = text.encode_utf16().collect();
        let pieces = runs(measurer.dc, &font, 1.0, false, false, &units);
        assert_eq!(pieces.len(), 2, "{text}");
        assert_eq!(pieces[1].range, 1..units.len());
        assert_eq!(pieces[1].direct.as_deref(), Some("Segoe UI Emoji"));
    }
    // Before Chromium 131 the selector keeps a font that has the character.
    follow_chromium(124);
    let symbol = request(&["Segoe UI Symbol"], 20.0);
    let units: Vec<u16> = "a\u{2764}\u{fe0f}".encode_utf16().collect();
    let pieces = runs(measurer.dc, &symbol, 1.0, false, false, &units);
    follow_chromium(u32::MAX);
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].direct, None);
    // Symbols shown as text by default fall back to a symbol font, those
    // shown as emoji to an emoji font.
    assert_eq!(script_fallbacks(0x2764)[0], "Segoe UI Symbol");
    assert_eq!(script_fallbacks(0x263A)[0], "Segoe UI Symbol");
    assert_eq!(script_fallbacks(0x2705)[0], "Segoe UI Emoji");
    assert_eq!(script_fallbacks(0x2B50)[0], "Segoe UI Emoji");
    assert_eq!(script_fallbacks(0x231A)[0], "Segoe UI Emoji");
}

#[test]
fn text_without_shaping_is_measured_and_drawn_by_glyph() {
    let handle = font(&request(&["Arial"], 20.0), 1.0, false, false);
    // SAFETY: a screen-compatible memory DC for the test.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        SelectObject(dc, handle);
        let latin: Vec<u16> = "Sum \u{2211}".encode_utf16().collect();
        let glyphs = simple_glyphs(dc, &latin).expect("Arial has these glyphs");
        assert_eq!(glyphs.len(), latin.len());
        let mut width = 0;
        for unit in &latin {
            let mut advance = 0i32;
            GetCharWidth32W(dc, u32::from(*unit), u32::from(*unit), &mut advance);
            width += advance;
        }
        assert_eq!(extent(dc, &latin), width);
        // Shaped scripts, and glyphs the font lacks, keep GDI's text path.
        let arabic: Vec<u16> = "\u{0645}\u{0631}".encode_utf16().collect();
        assert!(simple_glyphs(dc, &arabic).is_none());
        assert!(extent(dc, &arabic) > 0);
        let missing: Vec<u16> = "\u{4E2D}".encode_utf16().collect();
        assert!(simple_glyphs(dc, &missing).is_none());
        DeleteDC(dc);
    }
}

#[test]
fn text_is_as_wide_as_its_face_says_at_its_exact_size() {
    // Chromium's widths of the same text, which it lays out at the face's
    // advances for the exact size, unhinted and unrounded.
    let measurer = GdiMeasurer::new();
    let arial = request(&["Arial"], 14.0);
    for scale in [1.0, 1.25, 1.5] {
        measurer.set_scale(scale);
        for (text, chromium) in [
            ("iiiiiiiiii", 31.11),
            ("WWWWWWWWWW", 132.15),
            ("long cell text", 77.83),
        ] {
            let width = measurer.width(text, &arial);
            assert!(
                (width - chromium).abs() < 0.1,
                "{text} at {scale}: {width} for {chromium}"
            );
        }
    }
    // Pairs kern as Chromium kerns them: 137.8 pixels unkerned.
    measurer.set_scale(1.0);
    let kerned = measurer.width("AVATAR To Ty We", &request(&["Arial"], 16.0));
    assert!((kerned - 129.525).abs() < 0.1, "{kerned}");
    // `font-kerning: none` keeps the advances, and GDI's text: the runs stay
    // in the GDI font.
    let unkerned = Font {
        features: vec![(KERN, 0)],
        ..request(&["Arial"], 16.0)
    };
    assert!(!kerns(&unkerned) && kerns(&request(&["Arial"], 16.0)));
    let width = measurer.width("AVATAR To Ty We", &unkerned);
    assert!((width - 137.8).abs() < 0.1, "{width}");
    let units: Vec<u16> = "AV".encode_utf16().collect();
    let runs = runs(measurer.dc, &unkerned, 1.0, false, false, &units);
    assert!(runs.iter().all(|run| run.direct.is_none()), "{runs:?}");
}

#[test]
fn normal_lines_take_the_metrics_of_the_fonts_their_text_uses() {
    let measurer = GdiMeasurer::new();
    // Arial at 100px: Windows ascent 1854, descent 434 and the 67 units
    // `hhea` adds, of 2048, each rounded, as Chromium measures them.
    let font = request(&["Arial"], 100.0);
    let (above, below) = measurer.normal_extent("Hx", &font).unwrap();
    assert_eq!(above + below, 91.0 + 21.0 + 3.0);
    assert_eq!(above, 91.0 + 1.5);
    // Han characters fall back to a font that reaches at least as far.
    let (top, bottom) = measurer.normal_extent("Hx中", &font).unwrap();
    assert!(top >= above && bottom >= below);
    assert_eq!(
        measurer.normal_extent("Hx", &request(&["Arial"], 0.0)),
        Some((0.0, 0.0))
    );
}

#[test]
fn faces_are_used_as_declared_whatever_styles_their_files_claim() {
    // Two files that claim the same family, as KaTeX's regular and italic
    // faces both claim to be regular: the declarations tell them apart.
    let upright = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    let slanted = std::fs::read("C:/Windows/Fonts/arialbd.ttf").unwrap();
    assert!(register_font("Declared Face", 400, false, &upright));
    assert!(register_font("Declared Face", 400, true, &slanted));
    let (regular, ..) = face(&names(&["Declared Face"]), 400, false).unwrap();
    // The italic face is asked for as its file describes itself, so GDI
    // neither picks the other face nor slants it again.
    let (italic, weight, slant) = face(&names(&["Declared Face"]), 400, true).unwrap();
    assert_ne!(regular, italic);
    assert_eq!((weight, slant), (700, false));
    // Bold text synthesizes from the upright face.
    let (_, bold, _) = face(&names(&["Declared Face"]), 700, false).unwrap();
    assert_eq!(bold, 700);
}

#[test]
fn unicode_ranges_split_text_between_a_familys_faces() {
    // A web font split into subsets, as @fontsource packages are: each
    // face shows only its own characters, the last declared tried first.
    let arial = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    let courier = std::fs::read("C:/Windows/Fonts/cour.ttf").unwrap();
    assert!(register_font_face(
        "Split Face",
        (100, 900),
        false,
        &[(0x41, 0x5A)],
        &courier
    ));
    assert!(register_font_face(
        "Split Face",
        (100, 900),
        false,
        &[(0x61, 0x7A)],
        &arial
    ));
    // A static font serving a weight range is drawn as it is: CSS
    // synthesizes bold only for faces whose range stays below 600.
    let declared = faces(&names(&["Split Face"]), 700, false);
    assert_eq!(declared.len(), 2);
    assert!(declared[0].0.starts_with("Arial#"));
    assert_eq!(
        (declared[0].1, declared[0].3.as_slice()),
        (400, &[(0x61, 0x7A)][..])
    );
    assert_eq!(faces(&names(&["Split Face"]), 950, false)[0].1, 400);
    let measurer = GdiMeasurer::new();
    let font = request(&["Split Face", "serif"], 20.0);
    let units: Vec<u16> = "abCD ef".encode_utf16().collect();
    let pieces = runs(measurer.dc, &font, 1.0, false, false, &units);
    let ranges: Vec<_> = pieces.iter().map(|piece| piece.range.clone()).collect();
    assert_eq!(ranges, [0..2, 2..4, 4..7]);
    assert_ne!(pieces[0].font, pieces[1].font);
    assert_eq!(pieces[0].font, pieces[2].font);
    assert!(in_ranges(&[], 0x10_FFFF));
    assert!(!in_ranges(&[(1, 2)], 3));
}

#[test]
fn renamed_fonts_keep_a_valid_naming_table_and_checksum() {
    let font = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    let renamed = renamed_font(&font, "Unique#7").unwrap();
    assert_eq!(sfnt_family(&renamed).as_deref(), Some("Unique#7"));
    assert_eq!(own_style(&renamed), own_style(&font));
    assert_eq!(own_style(&font), Some((400, false)));
    assert_eq!(checksum(&renamed), 0xB1B0_AFBA);
    let (_, name, length) = table(&renamed, b"name").unwrap();
    assert_eq!(name % 4, 0);
    assert!(name + length <= renamed.len());
    assert!(renamed_font(b"not a font", "x").is_none());
}

#[test]
fn grayscale_glyphs_are_smoothed_as_skia_has_directwrite_smooth_them() {
    // A `gasp` table from version 1 on says, for the size, whether to
    // smooth vertically too.
    let gasp = (1, vec![(8, 0x000A), (16, 0x0007), (0xFFFF, 0x000F)]);
    assert_eq!(smoothing_for(Some(&gasp), true, 12.0), (12.0, false));
    assert_eq!(smoothing_for(Some(&gasp), true, 7.6), (7.6, true));
    assert_eq!(smoothing_for(Some(&gasp), true, 30.0), (30.0, true));
    // Without one, or with version 0, small hinted text is smoothed across
    // only, at its GDI size, and the rest both ways.
    let old = (0, vec![(0xFFFF, 0x0002)]);
    for gasp in [None, Some(&old)] {
        assert_eq!(smoothing_for(gasp, true, 13.33), (13.328125, false));
        assert_eq!(smoothing_for(gasp, true, 24.0), (24.0, true));
        assert_eq!(smoothing_for(gasp, false, 13.0), (13.0, true));
    }
    // Faces as GDI selects them have their tables read.
    let handle = font(&request(&["Arial"], 13.0), 1.0, false, false);
    // SAFETY: a screen-compatible memory DC for the test.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        SelectObject(dc, handle);
        assert_eq!(smoothing(dc, 13.0).0, 13.0);
        DeleteDC(dc);
    }
    // Without them, it is smoothed both ways.
    // SAFETY: a memory DC with no font selected but the stock one.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        SelectObject(dc, GetStockObject(SYSTEM_FONT));
        assert_eq!(smoothing(dc, 13.0), (13.0, true));
        DeleteDC(dc);
    }
}
