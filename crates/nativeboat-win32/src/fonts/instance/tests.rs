use super::*;
use crate::fonts::{GdiMeasurer, faces, register_font_face};
use nativeboat_ir::Font;
use nativeboat_layout::TextMeasurer;
use windows_sys::Win32::Graphics::Gdi::*;

fn bahnschrift() -> Vec<u8> {
    std::fs::read("C:/Windows/Fonts/bahnschrift.ttf").unwrap()
}

fn request(family: &str, weight: u16) -> Font {
    Font {
        families: vec![family.to_owned()],
        size: 40.0,
        weight,
        ..Font::default()
    }
}

#[test]
fn variable_truetype_fonts_report_their_weight_axis() {
    // Bahnschrift, Windows' variable font, spans weights 300 to 700.
    assert_eq!(weight_axis(&bahnschrift()), Some((300.0, 400.0, 700.0)));
    let arial = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    assert_eq!(weight_axis(&arial), None);
    assert_eq!(weight_axis(b"not a font"), None);
}

#[test]
fn instances_match_the_named_instances_gdi_draws() {
    // GDI draws an installed variable font's named instances itself: an
    // instance made here has the same advances at the same weights.
    let font = bahnschrift();
    let measurer = GdiMeasurer::new();
    let text = "Hamburgefonstiv WAVE 0123";
    for (weight, suffix, system) in [
        (300, "Light", ("Bahnschrift Light", 300)),
        (700, "Bold", ("Bahnschrift", 700)),
    ] {
        let instance = instance(&font, f32::from(weight)).unwrap();
        // Static now: no variation tables, and the weight GDI matches.
        let tags: Vec<[u8; 4]> = tables(&instance)
            .unwrap()
            .iter()
            .map(|(tag, _)| *tag)
            .collect();
        assert!(!tags.contains(b"fvar") && !tags.contains(b"gvar"));
        assert_eq!(crate::fonts::own_style(&instance), Some((weight, false)));
        assert_eq!(crate::fonts::checksum(&instance), 0xB1B0_AFBA);
        let family = format!("Instance {suffix}");
        assert!(crate::register_font(&family, weight, false, &instance));
        let ours = measurer.width(text, &request(&family, weight));
        let system = measurer.width(text, &request(system.0, system.1));
        assert_eq!(ours, system, "{suffix}");
    }
    // Different weights have different advances, and the default weight
    // keeps the font's own.
    let regular = instance(&font, 400.0).unwrap();
    assert!(crate::register_font(
        "Instance Regular",
        400,
        false,
        &regular
    ));
    assert_eq!(
        measurer.width(text, &request("Instance Regular", 400)),
        measurer.width(text, &request("Bahnschrift", 400))
    );
}

#[test]
fn variable_faces_are_instanced_at_the_weights_asked_for() {
    let font = bahnschrift();
    assert!(register_font_face(
        "Variable Face",
        (300, 700),
        false,
        &[],
        &font
    ));
    // The default weight is the font as it is; others are instances, made
    // once, which GDI asks for at their own weight.
    let regular = faces(&[String::from("Variable Face")], 400, false);
    assert_eq!(regular[0].1, 400);
    let bold = faces(&[String::from("Variable Face")], 700, false);
    assert!(bold[0].0.ends_with(".700"), "{}", bold[0].0);
    assert_eq!(bold[0].1, 700);
    assert_eq!(
        faces(&[String::from("Variable Face")], 700, false)[0].0,
        bold[0].0
    );
    // Weights beyond the range take its end.
    assert!(
        faces(&[String::from("Variable Face")], 900, false)[0]
            .0
            .ends_with(".700")
    );
    // GDI draws the instances: their stems are as wide as those of the
    // named instances Windows draws, where synthesized bold would widen
    // the default's.
    let (face, weight, light) = drawn(&request("Variable Face", 300));
    assert!(face.ends_with(".300") && weight == 300, "{face} {weight}");
    assert_eq!(light, drawn(&request("Bahnschrift Light", 300)).2);
    let (face, weight, heavy) = drawn(&request("Variable Face", 700));
    assert!(face.ends_with(".700") && weight == 700, "{face} {weight}");
    assert_eq!(heavy, drawn(&request("Bahnschrift", 700)).2);
    assert!(light < heavy, "{light} {heavy}");
}

/// The face GDI selects for a request, its weight, and the width of the
/// ink of an `l`, which is its stem.
fn drawn(font: &Font) -> (String, i32, u32) {
    let handle = crate::fonts::font(font, 1.0, false, false);
    let one = FIXED { fract: 0, value: 1 };
    let identity = MAT2 {
        eM11: one,
        eM12: FIXED::default(),
        eM21: FIXED::default(),
        eM22: one,
    };
    let mut name = [0u16; 64];
    let mut metrics = TEXTMETRICW::default();
    let mut glyph = GLYPHMETRICS::default();
    // SAFETY: a memory DC measures the font and is deleted here; every
    // buffer is writable and as long as GDI is told.
    let length = unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let previous = SelectObject(dc, handle);
        let length = GetTextFaceW(dc, name.len() as i32, name.as_mut_ptr());
        GetTextMetricsW(dc, &mut metrics);
        GetGlyphOutlineW(
            dc,
            u32::from('l'),
            GGO_METRICS,
            &mut glyph,
            0,
            core::ptr::null_mut(),
            &identity,
        );
        SelectObject(dc, previous);
        DeleteDC(dc);
        length
    };
    let face =
        String::from_utf16_lossy(&name[..usize::try_from(length).unwrap_or(1).saturating_sub(1)]);
    (face, metrics.tmWeight, glyph.gmBlackBoxX)
}

#[test]
fn malformed_fonts_are_not_instanced() {
    let font = bahnschrift();
    assert!(instance(&font[..font.len() / 2], 700.0).is_none());
    assert!(instance(b"\0\x01\0\0", 700.0).is_none());
    let arial = std::fs::read("C:/Windows/Fonts/arial.ttf").unwrap();
    assert!(instance(&arial, 700.0).is_none());
}
