//! Color values.

use crate::prelude::*;

use crate::tokenizer::Token;
use crate::values::{ComponentValue, LengthContext, number, split_commas, without_whitespace};
use serde::{Deserialize, Serialize};

/// An sRGB color with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Opacity between 0 and 1.
    pub a: f32,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0.0);
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// Creates an opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 1.0 }
    }

    /// Creates a color with alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color { r, g, b, a }
    }

    /// Whether the color is fully transparent.
    pub fn is_transparent(&self) -> bool {
        self.a <= 0.0
    }

    /// Serializes the color like `getComputedStyle`.
    pub fn to_css(&self) -> String {
        if self.a >= 1.0 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            format!(
                "rgba({}, {}, {}, {})",
                self.r,
                self.g,
                self.b,
                crate::values::format_float(self.a)
            )
        }
    }

    fn from_unit(r: f32, g: f32, b: f32, a: f32) -> Color {
        let channel = |value: f32| (crate::math::round(value.clamp(0.0, 1.0) * 255.0)) as u8;
        Color {
            r: channel(r),
            g: channel(g),
            b: channel(b),
            a: a.clamp(0.0, 1.0),
        }
    }

    fn to_unit(self) -> [f32; 3] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        ]
    }
}

/// Colors that depend on the operating system theme.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemColors {
    /// `Canvas`: window background.
    pub canvas: Color,
    /// `CanvasText`: window text.
    pub canvas_text: Color,
    /// `LinkText`
    pub link_text: Color,
    /// `VisitedText`
    pub visited_text: Color,
    /// `ActiveText`
    pub active_text: Color,
    /// `ButtonFace`
    pub button_face: Color,
    /// `ButtonText`
    pub button_text: Color,
    /// `ButtonBorder`
    pub button_border: Color,
    /// `Field`: input background.
    pub field: Color,
    /// `FieldText`: input text.
    pub field_text: Color,
    /// `Highlight`: selection background.
    pub highlight: Color,
    /// `HighlightText`: selected text.
    pub highlight_text: Color,
    /// `GrayText`: disabled text.
    pub gray_text: Color,
    /// `Mark`
    pub mark: Color,
    /// `MarkText`
    pub mark_text: Color,
    /// `AccentColor`
    pub accent: Color,
    /// `AccentColorText`
    pub accent_text: Color,
}

impl SystemColors {
    /// The light theme defaults.
    pub const LIGHT: SystemColors = SystemColors {
        canvas: Color::WHITE,
        canvas_text: Color::BLACK,
        link_text: Color::rgb(0, 0, 238),
        visited_text: Color::rgb(85, 26, 139),
        active_text: Color::rgb(255, 0, 0),
        button_face: Color::rgb(240, 240, 240),
        button_text: Color::BLACK,
        button_border: Color::rgb(118, 118, 118),
        field: Color::WHITE,
        field_text: Color::BLACK,
        highlight: Color::rgb(0, 120, 215),
        highlight_text: Color::WHITE,
        gray_text: Color::rgb(109, 109, 109),
        mark: Color::rgb(255, 255, 0),
        mark_text: Color::BLACK,
        accent: Color::rgb(0, 120, 215),
        accent_text: Color::WHITE,
    };

    /// The dark theme defaults.
    pub const DARK: SystemColors = SystemColors {
        canvas: Color::rgb(18, 18, 18),
        canvas_text: Color::WHITE,
        link_text: Color::rgb(158, 158, 255),
        visited_text: Color::rgb(208, 173, 240),
        active_text: Color::rgb(255, 158, 158),
        button_face: Color::rgb(107, 107, 107),
        button_text: Color::WHITE,
        button_border: Color::rgb(107, 107, 107),
        field: Color::rgb(59, 59, 59),
        field_text: Color::WHITE,
        highlight: Color::rgb(0, 120, 215),
        highlight_text: Color::WHITE,
        gray_text: Color::rgb(141, 141, 141),
        mark: Color::rgb(102, 92, 0),
        mark_text: Color::WHITE,
        accent: Color::rgb(76, 194, 255),
        accent_text: Color::BLACK,
    };

    fn lookup(&self, name: &str) -> Option<Color> {
        Some(match name {
            "canvas" | "window" | "background" => self.canvas,
            "canvastext" | "windowtext" | "menutext" | "captiontext" | "infotext" => {
                self.canvas_text
            }
            "linktext" => self.link_text,
            "visitedtext" => self.visited_text,
            "activetext" => self.active_text,
            "buttonface" | "buttonhighlight" | "buttonshadow" | "threedface" | "menu" => {
                self.button_face
            }
            "buttontext" => self.button_text,
            "buttonborder" | "activeborder" | "inactiveborder" | "windowframe" => {
                self.button_border
            }
            "field" => self.field,
            "fieldtext" => self.field_text,
            "highlight" | "selecteditem" => self.highlight,
            "highlighttext" | "selecteditemtext" => self.highlight_text,
            "graytext" | "inactivecaptiontext" => self.gray_text,
            "mark" => self.mark,
            "marktext" => self.mark_text,
            "accentcolor" => self.accent,
            "accentcolortext" => self.accent_text,
            _ => return None,
        })
    }
}

impl Default for SystemColors {
    fn default() -> Self {
        SystemColors::LIGHT
    }
}

/// A specified color: either concrete or `currentcolor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedColor {
    /// A concrete color.
    Color(Color),
    /// `currentcolor`
    CurrentColor,
}

/// Parses a color value.
pub fn parse_color(value: &ComponentValue, system: &SystemColors) -> Option<SpecifiedColor> {
    match value {
        ComponentValue::Token(Token::Ident(name)) => {
            let name = name.to_ascii_lowercase();
            if name == "currentcolor" {
                return Some(SpecifiedColor::CurrentColor);
            }
            if name == "transparent" {
                return Some(SpecifiedColor::Color(Color::TRANSPARENT));
            }
            named_color(&name)
                .or_else(|| system.lookup(&name))
                .map(SpecifiedColor::Color)
        }
        ComponentValue::Token(Token::Hash { value, .. }) => {
            hex_color(value).map(SpecifiedColor::Color)
        }
        ComponentValue::Function(name, arguments) => {
            color_function(name, arguments, system).map(SpecifiedColor::Color)
        }
        _ => None,
    }
}

/// Parses `#rgb`, `#rgba`, `#rrggbb` and `#rrggbbaa`.
pub fn hex_color(hex: &str) -> Option<Color> {
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |index: usize| u8::from_str_radix(&hex[index..=index], 16).unwrap_or(0);
    let pair = |index: usize| u8::from_str_radix(&hex[index..index + 2], 16).unwrap_or(0);
    let alpha = |value: u8| f32::from(value) / 255.0;
    Some(match hex.len() {
        3 => Color::rgb(digit(0) * 17, digit(1) * 17, digit(2) * 17),
        4 => Color::rgba(
            digit(0) * 17,
            digit(1) * 17,
            digit(2) * 17,
            alpha(digit(3) * 17),
        ),
        6 => Color::rgb(pair(0), pair(2), pair(4)),
        8 => Color::rgba(pair(0), pair(2), pair(4), alpha(pair(6))),
        _ => return None,
    })
}

/// Splits color function arguments in legacy comma or modern space syntax.
/// Returns the channels and the optional alpha.
fn channels(arguments: &[ComponentValue]) -> Option<(Vec<ComponentValue>, Option<ComponentValue>)> {
    let comma_parts = split_commas(arguments);
    if comma_parts.len() > 1 {
        let mut values: Vec<ComponentValue> = Vec::new();
        for part in &comma_parts {
            let [value] = part.as_slice() else {
                return None;
            };
            values.push(value.clone());
        }
        let alpha = (values.len() == 4).then(|| values.pop()).flatten();
        return Some((values, alpha));
    }
    let values = without_whitespace(arguments);
    match values
        .iter()
        .position(|value| matches!(value, ComponentValue::Token(Token::Delim('/'))))
    {
        Some(slash) => {
            let [alpha] = &values[slash + 1..] else {
                return None;
            };
            Some((values[..slash].to_vec(), Some(alpha.clone())))
        }
        None => Some((values, None)),
    }
}

/// A channel as a fraction: numbers are divided by `scale`, percentages by 100.
fn fraction(value: &ComponentValue, scale: f32) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Percentage(percent)) => Some(*percent as f32 / 100.0),
        ComponentValue::Token(Token::Ident(name)) if name.eq_ignore_ascii_case("none") => Some(0.0),
        other => number(other, &LengthContext::default()).map(|value| value / scale),
    }
}

fn hue(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Dimension { value, unit }) => {
            let value = *value as f32;
            Some(match unit.to_ascii_lowercase().as_str() {
                "deg" => value,
                "rad" => value.to_degrees(),
                "grad" => value * 0.9,
                "turn" => value * 360.0,
                _ => return None,
            })
        }
        other => fraction(other, 1.0),
    }
}

fn alpha_of(alpha: Option<ComponentValue>) -> Option<f32> {
    match alpha {
        None => Some(1.0),
        Some(value) => fraction(&value, 1.0),
    }
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let hue = crate::math::rem_euclid(hue, 360.0) / 360.0;
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let channel = |offset: f32| {
        let t = crate::math::rem_euclid(hue + offset, 1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(1.0 / 3.0), channel(0.0), channel(-1.0 / 3.0)]
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * crate::math::pow(value, 1.0 / 2.4) - 0.055
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        crate::math::pow((value + 0.055) / 1.055, 2.4)
    }
}

/// Converts OKLab to sRGB channels.
pub fn oklab_to_srgb(lightness: f32, a: f32, b: f32) -> [f32; 3] {
    let l = crate::math::pow(lightness + 0.396_337_78 * a + 0.215_803_76 * b, 3.0);
    let m = crate::math::pow(lightness - 0.105_561_346 * a - 0.063_854_17 * b, 3.0);
    let s = crate::math::pow(lightness - 0.089_484_18 * a - 1.291_485_5 * b, 3.0);
    [
        linear_to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        linear_to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        linear_to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

/// Converts sRGB channels to OKLab.
pub fn srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(srgb_to_linear);
    let l = crate::math::cbrt(0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b);
    let m = crate::math::cbrt(0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b);
    let s = crate::math::cbrt(0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b);
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn color_function(
    name: &str,
    arguments: &[ComponentValue],
    system: &SystemColors,
) -> Option<Color> {
    if name == "color-mix" {
        return color_mix(arguments, system);
    }
    let (values, alpha) = channels(arguments)?;
    let [first, second, third] = values.as_slice() else {
        return None;
    };
    let alpha = alpha_of(alpha)?;
    let [r, g, b] = match name {
        "rgb" | "rgba" => [
            fraction(first, 255.0)?,
            fraction(second, 255.0)?,
            fraction(third, 255.0)?,
        ],
        "hsl" | "hsla" => hsl_to_rgb(
            hue(first)?,
            fraction(second, 100.0)?,
            fraction(third, 100.0)?,
        ),
        "hwb" => {
            let white = fraction(second, 100.0)?;
            let black = fraction(third, 100.0)?;
            if white + black >= 1.0 {
                let gray = white / (white + black);
                [gray, gray, gray]
            } else {
                hsl_to_rgb(hue(first)?, 1.0, 0.5)
                    .map(|channel| channel * (1.0 - white - black) + white)
            }
        }
        "oklab" => oklab_to_srgb(
            fraction(first, 1.0)?,
            oklab_axis(second)?,
            oklab_axis(third)?,
        ),
        "lab" => lab_to_srgb(
            scaled(first, 100.0)?,
            scaled(second, 125.0)?,
            scaled(third, 125.0)?,
        ),
        "lch" => {
            let [lightness, a, b] =
                from_polar([scaled(first, 100.0)?, scaled(second, 150.0)?, hue(third)?]);
            lab_to_srgb(lightness, a, b)
        }
        "oklch" => {
            let chroma = match second {
                ComponentValue::Token(Token::Percentage(percent)) => *percent as f32 / 100.0 * 0.4,
                other => fraction(other, 1.0)?,
            };
            let angle = hue(third)?.to_radians();
            oklab_to_srgb(
                fraction(first, 1.0)?,
                chroma * crate::math::cos(angle),
                chroma * crate::math::sin(angle),
            )
        }
        _ => return None,
    };
    Some(Color::from_unit(r, g, b, alpha))
}

/// A number, or a percentage of `full`, as CIE Lab and LCH components are.
fn scaled(value: &ComponentValue, full: f32) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Percentage(percent)) => Some(*percent as f32 / 100.0 * full),
        other => fraction(other, 1.0),
    }
}

fn oklab_axis(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Percentage(percent)) => Some(*percent as f32 / 100.0 * 0.4),
        other => fraction(other, 1.0),
    }
}

/// A color space `color-mix` interpolates in.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MixSpace {
    Srgb,
    SrgbLinear,
    Lab,
    Oklab,
    XyzD65,
    XyzD50,
    Hsl,
    Hwb,
    Lch,
    Oklch,
}

/// How a polar space's hues interpolate.
#[derive(Debug, Clone, Copy, PartialEq)]
enum HueMethod {
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// Linear sRGB to CIE XYZ (D65), and back.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_390_8, 0.357_584_33, 0.180_480_8],
    [0.212_639, 0.715_168_7, 0.072_192_32],
    [0.019_330_818, 0.119_194_78, 0.950_532_14],
];
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.240_97, -1.537_383_2, -0.498_610_76],
    [-0.969_243_65, 1.875_967_5, 0.041_555_06],
    [0.055_630_08, -0.203_976_96, 1.056_971_5],
];
/// Bradford adaptation from D65 to D50 white, and back.
const D65_TO_D50: [[f32; 3]; 3] = [
    [1.047_929_8, 0.022_946_793, -0.050_192_23],
    [0.029_627_815, 0.990_434_5, -0.017_073_825],
    [-0.009_243_058, 0.015_055_145, 0.751_874_3],
];
const D50_TO_D65: [[f32; 3]; 3] = [
    [0.955_473_4, -0.023_098_537, 0.063_259_31],
    [-0.028_369_706, 1.009_995_5, 0.021_041_399],
    [0.012_314_002, -0.020_507_697, 1.330_366],
];
/// The D50 white point CIE Lab is relative to.
const D50_WHITE: [f32; 3] = [0.964_295_7, 1.0, 0.825_104_6];
/// CIE Lab's ε and κ.
const LAB_EPSILON: f32 = 216.0 / 24389.0;
const LAB_KAPPA: f32 = 24389.0 / 27.0;

fn transform(matrix: &[[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| row[0] * vector[0] + row[1] * vector[1] + row[2] * vector[2])
}

/// Converts sRGB channels to CIE XYZ relative to D50.
fn srgb_to_xyz_d50(rgb: [f32; 3]) -> [f32; 3] {
    transform(
        &D65_TO_D50,
        transform(&SRGB_TO_XYZ, rgb.map(srgb_to_linear)),
    )
}

/// Converts CIE XYZ relative to D50 to sRGB channels.
fn xyz_d50_to_srgb(xyz: [f32; 3]) -> [f32; 3] {
    transform(&XYZ_TO_SRGB, transform(&D50_TO_D65, xyz)).map(linear_to_srgb)
}

/// Converts sRGB channels to CIE Lab.
fn srgb_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    let xyz = srgb_to_xyz_d50(rgb);
    let [x, y, z] = [0, 1, 2].map(|index| {
        let value = xyz[index] / D50_WHITE[index];
        if value > LAB_EPSILON {
            crate::math::cbrt(value)
        } else {
            (LAB_KAPPA * value + 16.0) / 116.0
        }
    });
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// Converts CIE Lab to sRGB channels.
pub fn lab_to_srgb(lightness: f32, a: f32, b: f32) -> [f32; 3] {
    let y = (lightness + 16.0) / 116.0;
    let x = a / 500.0 + y;
    let z = y - b / 200.0;
    let cube = |value: f32| {
        let cubed = value * value * value;
        if cubed > LAB_EPSILON {
            cubed
        } else {
            (116.0 * value - 16.0) / LAB_KAPPA
        }
    };
    let y = if lightness > LAB_KAPPA * LAB_EPSILON {
        y * y * y
    } else {
        lightness / LAB_KAPPA
    };
    xyz_d50_to_srgb([
        cube(x) * D50_WHITE[0],
        y * D50_WHITE[1],
        cube(z) * D50_WHITE[2],
    ])
}

/// A rectangular color's polar form: lightness, chroma and hue in degrees.
fn to_polar([lightness, a, b]: [f32; 3]) -> [f32; 3] {
    let hue = crate::math::rem_euclid(crate::math::atan2(b, a).to_degrees(), 360.0);
    [lightness, crate::math::sqrt(a * a + b * b), hue]
}

fn from_polar([lightness, chroma, hue]: [f32; 3]) -> [f32; 3] {
    let angle = hue.to_radians();
    [
        lightness,
        chroma * crate::math::cos(angle),
        chroma * crate::math::sin(angle),
    ]
}

/// Converts sRGB channels to hue, saturation and lightness.
fn srgb_to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    if delta <= 0.0 {
        return [0.0, 0.0, lightness];
    }
    let saturation = if lightness <= 0.0 || lightness >= 1.0 {
        0.0
    } else {
        (max - lightness) / lightness.min(1.0 - lightness)
    };
    let hue = if max == r {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    [hue * 60.0, saturation, lightness]
}

impl MixSpace {
    fn named(name: &str) -> Option<MixSpace> {
        Some(match name {
            "srgb" => MixSpace::Srgb,
            "srgb-linear" => MixSpace::SrgbLinear,
            "lab" => MixSpace::Lab,
            "oklab" => MixSpace::Oklab,
            "xyz" | "xyz-d65" => MixSpace::XyzD65,
            "xyz-d50" => MixSpace::XyzD50,
            "hsl" => MixSpace::Hsl,
            "hwb" => MixSpace::Hwb,
            "lch" => MixSpace::Lch,
            "oklch" => MixSpace::Oklch,
            _ => return None,
        })
    }

    /// The component that is a hue, in a polar space.
    fn hue(self) -> Option<usize> {
        match self {
            MixSpace::Hsl | MixSpace::Hwb => Some(0),
            MixSpace::Lch | MixSpace::Oklch => Some(2),
            _ => None,
        }
    }

    fn of_srgb(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            MixSpace::Srgb => rgb,
            MixSpace::SrgbLinear => rgb.map(srgb_to_linear),
            MixSpace::Lab => srgb_to_lab(rgb),
            MixSpace::Oklab => srgb_to_oklab(rgb),
            MixSpace::XyzD65 => transform(&SRGB_TO_XYZ, rgb.map(srgb_to_linear)),
            MixSpace::XyzD50 => srgb_to_xyz_d50(rgb),
            MixSpace::Hsl => srgb_to_hsl(rgb),
            MixSpace::Hwb => {
                let [hue, ..] = srgb_to_hsl(rgb);
                let max = rgb[0].max(rgb[1]).max(rgb[2]);
                let min = rgb[0].min(rgb[1]).min(rgb[2]);
                [hue, min, 1.0 - max]
            }
            MixSpace::Lch => to_polar(srgb_to_lab(rgb)),
            MixSpace::Oklch => to_polar(srgb_to_oklab(rgb)),
        }
    }

    fn to_srgb(self, [first, second, third]: [f32; 3]) -> [f32; 3] {
        match self {
            MixSpace::Srgb => [first, second, third],
            MixSpace::SrgbLinear => [first, second, third].map(linear_to_srgb),
            MixSpace::Lab => lab_to_srgb(first, second, third),
            MixSpace::Oklab => oklab_to_srgb(first, second, third),
            MixSpace::XyzD65 => transform(&XYZ_TO_SRGB, [first, second, third]).map(linear_to_srgb),
            MixSpace::XyzD50 => xyz_d50_to_srgb([first, second, third]),
            MixSpace::Hsl => hsl_to_rgb(first, second, third),
            MixSpace::Hwb => {
                if second + third >= 1.0 {
                    let gray = second / (second + third);
                    [gray, gray, gray]
                } else {
                    hsl_to_rgb(first, 1.0, 0.5)
                        .map(|channel| channel * (1.0 - second - third) + second)
                }
            }
            MixSpace::Lch => {
                let [lightness, a, b] = from_polar([first, second, third]);
                lab_to_srgb(lightness, a, b)
            }
            MixSpace::Oklch => {
                let [lightness, a, b] = from_polar([first, second, third]);
                oklab_to_srgb(lightness, a, b)
            }
        }
    }

    /// Whether a color's hue in this space is powerless: an achromatic
    /// color's, which takes the other color's hue.
    fn powerless_hue(self, components: [f32; 3]) -> bool {
        match self {
            MixSpace::Hsl => components[1] <= 1e-5,
            MixSpace::Hwb => components[1] + components[2] >= 1.0 - 1e-5,
            MixSpace::Lch => components[1] <= 1e-3,
            MixSpace::Oklch => components[1] <= 1e-5,
            _ => false,
        }
    }
}

/// Two hues adjusted so that interpolating them goes the way `method` says.
fn fix_hues(method: HueMethod, mut first: f32, mut second: f32) -> (f32, f32) {
    let difference = second - first;
    match method {
        HueMethod::Shorter if difference > 180.0 => first += 360.0,
        HueMethod::Shorter if difference < -180.0 => second += 360.0,
        HueMethod::Longer if difference > 0.0 && difference < 180.0 => first += 360.0,
        HueMethod::Longer if difference > -180.0 && difference <= 0.0 => second += 360.0,
        HueMethod::Increasing if second < first => second += 360.0,
        HueMethod::Decreasing if first < second => first += 360.0,
        _ => {}
    }
    (first, second)
}

/// A color space and hue method colors interpolate with, as `color-mix` and
/// gradients name them: `in <space> [<hue-method> hue]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interpolation {
    space: MixSpace,
    method: HueMethod,
}

impl Interpolation {
    /// Parses `in <space> [<hue-method> hue]` without whitespace.
    pub fn parse(values: &[ComponentValue]) -> Option<Interpolation> {
        match values {
            [keyword, space] if keyword.is_ident("in") => Some(Interpolation {
                space: MixSpace::named(&space.ident()?)?,
                method: HueMethod::Shorter,
            }),
            [keyword, space, method, hue] if keyword.is_ident("in") && hue.is_ident("hue") => {
                let space = MixSpace::named(&space.ident()?)?;
                space.hue()?;
                let method = match method.ident()?.as_str() {
                    "shorter" => HueMethod::Shorter,
                    "longer" => HueMethod::Longer,
                    "increasing" => HueMethod::Increasing,
                    "decreasing" => HueMethod::Decreasing,
                    _ => return None,
                };
                Some(Interpolation { space, method })
            }
            _ => None,
        }
    }

    /// Whether colors interpolate in sRGB, as painting does.
    pub fn is_srgb(self) -> bool {
        self.space == MixSpace::Srgb
    }

    /// The color `progress` of the way from `from` to `to`, interpolated
    /// with premultiplied alpha as CSS Color 4 says.
    pub fn between(self, from: Color, to: Color, progress: f32) -> Color {
        let (left_weight, right_weight) = (1.0 - progress, progress);
        let space = self.space;
        let alpha = from.a * left_weight + to.a * right_weight;
        let mut first = space.of_srgb(from.to_unit());
        let mut second = space.of_srgb(to.to_unit());
        if let Some(hue) = space.hue() {
            // A powerless hue takes the other color's.
            match (space.powerless_hue(first), space.powerless_hue(second)) {
                (true, false) => first[hue] = second[hue],
                (false, true) => second[hue] = first[hue],
                _ => {}
            }
            (first[hue], second[hue]) = fix_hues(self.method, first[hue], second[hue]);
        }
        // Components other than hue interpolate premultiplied by alpha, so a
        // transparent color adds no color of its own.
        let mixed = [0, 1, 2].map(|index| {
            if Some(index) == space.hue() || alpha <= 0.0 {
                first[index] * left_weight + second[index] * right_weight
            } else {
                (first[index] * from.a * left_weight + second[index] * to.a * right_weight) / alpha
            }
        });
        let rgb = space.to_srgb(mixed);
        Color::from_unit(rgb[0], rgb[1], rgb[2], alpha)
    }
}

/// `color-mix(in <space> [<hue-method> hue], <color> [<percentage>], <color> [<percentage>])`.
fn color_mix(arguments: &[ComponentValue], system: &SystemColors) -> Option<Color> {
    let parts = split_commas(arguments);
    let [interpolation, first, second] = parts.as_slice() else {
        return None;
    };
    let interpolation = Interpolation::parse(interpolation)?;
    let component = |part: &[ComponentValue]| -> Option<(Color, Option<f32>)> {
        let (color, percent) = match part {
            [color] => (color, None),
            [color, ComponentValue::Token(Token::Percentage(percent))]
            | [ComponentValue::Token(Token::Percentage(percent)), color] => {
                (color, Some(*percent as f32 / 100.0))
            }
            _ => return None,
        };
        match parse_color(color, system)? {
            SpecifiedColor::Color(color) => Some((color, percent)),
            SpecifiedColor::CurrentColor => None,
        }
    };
    let (left, left_weight) = component(first)?;
    let (right, right_weight) = component(second)?;
    let (left_weight, right_weight) = match (left_weight, right_weight) {
        (None, None) => (0.5, 0.5),
        (Some(left), None) => (left, 1.0 - left),
        (None, Some(right)) => (1.0 - right, right),
        (Some(left), Some(right)) => (left, right),
    };
    let total = left_weight + right_weight;
    if total <= 0.0 {
        return None;
    }
    let mut mixed = interpolation.between(left, right, right_weight / total);
    // Weights that add up to less than 100% make the mix that transparent.
    mixed.a = (mixed.a * total.min(1.0)).clamp(0.0, 1.0);
    Some(mixed)
}

/// Looks up a CSS named color.
pub fn named_color(name: &str) -> Option<Color> {
    NAMED_COLORS
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()
        .map(|index| {
            let value = NAMED_COLORS[index].1;
            Color::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
        })
}

/// CSS named colors, sorted by name.
const NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests;
