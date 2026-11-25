//! Text measurement and preparation.

use crate::prelude::*;

use nativeboat_css::ComputedStyle;
use nativeboat_css::types::{FontStyle, LineHeight, TextTransform, WhiteSpace};
use nativeboat_ir::Font;

/// Vertical font metrics in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FontMetrics {
    /// Distance from the baseline to the top of the em box.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the em box.
    pub descent: f32,
}

/// Measures text with platform fonts.
pub trait TextMeasurer {
    /// The advance width of `text`.
    fn width(&self, text: &str, font: &Font) -> f32;
    /// The vertical metrics of a font.
    fn metrics(&self, font: &Font) -> FontMetrics;
    /// Measures for a scale of device pixels per CSS pixel from now on.
    fn set_scale(&self, _scale: f32) {}
    /// Changes whenever the same text may measure differently, as when the
    /// scale changes or fonts load, so earlier layouts are not reused.
    fn generation(&self) -> u64 {
        0
    }
    /// How far text reaches above and below its baseline at `line-height:
    /// normal`: the ascent and the descent, each with half the line gap, of
    /// the font and of every font the text falls back to, as Chromium unites
    /// them. `None` leaves the line 1.2 times the font size.
    fn normal_extent(&self, _text: &str, _font: &Font) -> Option<(f32, f32)> {
        None
    }
    /// The size of a device pixel in CSS pixels, to which Chromium rounds
    /// the leading above text; `0` lays text out unrounded.
    fn device_pixel(&self) -> f32 {
        0.0
    }
    /// How the browser draws scrollbars that no author CSS styles.
    fn scrollbar_look(&self) -> nativeboat_ir::scrollbar::Look {
        nativeboat_ir::scrollbar::Look::Classic
    }
    /// The Chromium release whose rendering the browser follows: the
    /// latest, unless the application's Electron ships an earlier one.
    fn chromium(&self) -> u32 {
        u32::MAX
    }
    /// The family text without a `font-family` uses: the standard font of
    /// the user's locale.
    fn standard_family(&self) -> &'static str {
        "Times New Roman"
    }
}

/// The leading above text in a line box: half of what the line height adds
/// to the font's content area, rounded down to device pixels as Chromium
/// lays lines out in them.
pub fn half_leading(measurer: &dyn TextMeasurer, line_height: f32, content: f32) -> f32 {
    let half = (line_height - content) / 2.0;
    let pixel = measurer.device_pixel();
    if pixel > 0.0 {
        // Lengths a hair under a whole number of pixels count as it.
        crate::math::floor(half / pixel + 1e-3) * pixel
    } else {
        half
    }
}

/// The height of a line of a font at `line-height: normal`: its ascent,
/// descent and line gap, as Chromium takes them from the font.
pub fn normal_line_height(measurer: &dyn TextMeasurer, font: &Font) -> f32 {
    measurer
        .normal_extent("", font)
        .map_or(font.size * 1.2, |(above, below)| above + below)
}

/// An element's used line height, `normal` from its font's metrics.
pub fn line_height(style: &ComputedStyle, measurer: &dyn TextMeasurer) -> f32 {
    if style.line_height == LineHeight::Normal {
        normal_line_height(measurer, &font_of(style))
    } else {
        style.used_line_height()
    }
}

/// The font an element's text uses.
pub fn font_of(style: &ComputedStyle) -> Font {
    Font {
        families: style.font_family.0.clone(),
        size: style.font_size.0,
        weight: style.font_weight.0,
        italic: style.font_style != FontStyle::Normal,
        features: features_of(style),
    }
}

/// The OpenType features a style asks for: those of `font-variant-numeric`
/// and `font-kerning: none`, then `font-feature-settings`, whose values win
/// for the same tag.
fn features_of(style: &ComputedStyle) -> Vec<(u32, u32)> {
    let mut features: Vec<(u32, u32)> = style
        .font_variant_numeric
        .features()
        .map(|tag| (u32::from_le_bytes(tag), 1))
        .collect();
    if style.font_kerning == nativeboat_css::types::FontKerning::None {
        features.push((u32::from_le_bytes(*b"kern"), 0));
    }
    for (tag, value) in &style.font_feature_settings.0 {
        let tag = u32::from_le_bytes(*tag);
        features.retain(|(known, _)| *known != tag);
        features.push((tag, *value));
    }
    features
}

/// Applies `text-transform`.
pub fn transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_owned(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut result = String::with_capacity(text.len());
            let mut start = true;
            for character in text.chars() {
                if start && character.is_alphabetic() {
                    result.extend(character.to_uppercase());
                } else {
                    result.push(character);
                }
                start = character.is_whitespace();
            }
            result
        }
    }
}

/// A piece of text between break opportunities.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// The space an inline box's margin, border and padding take at its
    /// start or end, in pixels.
    Gap(f32),
    /// A word or other unbreakable run.
    Word(String),
    /// Collapsible or preserved spaces.
    Space(String),
    /// A forced line break.
    Break,
    /// An atomic inline-level box, such as an inline-block or an image: an
    /// index into the content's atoms.
    Atom(usize),
}

/// Splits text into words, spaces and forced breaks according to `white-space`.
///
/// `after_space` tells whether the previous text ended with a space, so that
/// collapsible spaces are not repeated across text nodes; it is updated.
pub fn pieces(text: &str, white_space: WhiteSpace, after_space: &mut bool) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, pieces: &mut Vec<Piece>| {
        if !word.is_empty() {
            pieces.push(Piece::Word(core::mem::take(word)));
        }
    };
    for character in text.chars() {
        match character {
            '\n' if white_space.keeps_newlines() => {
                flush(&mut word, &mut pieces);
                pieces.push(Piece::Break);
                *after_space = true;
            }
            ' ' | '\t' | '\n' | '\r' | '\u{c}' if white_space.collapses_spaces() => {
                flush(&mut word, &mut pieces);
                if !*after_space {
                    pieces.push(Piece::Space(" ".to_owned()));
                    *after_space = true;
                }
            }
            // A preserved tab stays one character, which line layout
            // advances to the next tab stop.
            ' ' | '\t' => {
                flush(&mut word, &mut pieces);
                match pieces.last_mut() {
                    Some(Piece::Space(spaces)) => spaces.push(character),
                    _ => pieces.push(Piece::Space(String::from(character))),
                }
                *after_space = true;
            }
            '\r' => {}
            other => {
                word.push(other);
                *after_space = false;
            }
        }
    }
    flush(&mut word, &mut pieces);
    pieces
}

#[cfg(test)]
mod tests;
