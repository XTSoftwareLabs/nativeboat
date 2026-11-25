//! Inline formatting: line breaking of styled text runs.

use crate::prelude::*;

use crate::text::{Piece, TextMeasurer};
use alloc::rc::Rc;
use nativeboat_css::Color;
use nativeboat_css::LengthPercentage;
use nativeboat_css::types::{Direction, TextAlign, VerticalAlign, WhiteSpace};
use nativeboat_dom::NodeId;
use nativeboat_ir::Font;

/// An inline element whose box shows: its background and borders cover its
/// text on each line, and its margins, borders and padding take space where
/// it starts and ends.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineBox {
    /// The element.
    pub node: NodeId,
    /// Its font, whose ascent and descent make the box's content height.
    pub font: Font,
    /// Its left and right margins.
    pub margin: [f32; 2],
    /// Border and padding above and below its content.
    pub above: f32,
    /// Border and padding below its content.
    pub below: f32,
    /// How far its baseline is raised above the line's (`vertical-align`).
    pub shift: f32,
    /// Whether it runs right to left: it starts on the right, and its
    /// start gap is its right margin, border and padding.
    pub rtl: bool,
}

/// Where an inline box's start or end gap is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The box's start.
    Start,
    /// The box's end.
    End,
}

/// A styled run of inline text.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Words, spaces and breaks.
    pub pieces: Vec<Piece>,
    /// Font.
    pub font: Font,
    /// Used line height in pixels.
    pub line_height: f32,
    /// Whether the line height is `normal`: the fonts the text falls back to
    /// may then make its line taller.
    pub normal_line: bool,
    /// Text color.
    pub color: Color,
    /// Underline decoration.
    pub underline: bool,
    /// How far below the baseline underlines begin, when
    /// `text-underline-offset` sets it.
    pub underline_offset: Option<f32>,
    /// Line-through decoration.
    pub line_through: bool,
    /// Extra space after each character.
    pub letter_spacing: f32,
    /// White-space handling.
    pub white_space: WhiteSpace,
    /// Where words longer than a line break.
    pub word_breaking: WordBreaking,
    /// The text node, or the element of generated content.
    pub node: NodeId,
    /// The element that receives pointer events for the text.
    pub element: NodeId,
    /// The inline boxes the text is in, outermost first.
    pub boxes: Rc<[Rc<InlineBox>]>,
    /// The edge of the innermost box that this segment's gap is.
    pub edge: Option<Edge>,
    /// The shape a list marker's symbol is painted as.
    pub symbol: Option<Symbol>,
    /// How far the text's baseline is raised above the line's, by the
    /// `vertical-align` of the inline elements around it.
    pub shift: f32,
    /// How far apart tab stops are and the advance of a space, for text
    /// with preserved tabs.
    pub tab: (f32, f32),
    /// The bidirectional embedding level the text is at, from the
    /// `direction` and `unicode-bidi` of its block container and the
    /// inline elements around it.
    pub level: u8,
    /// Whether `bidi-override` gives every character of the text the
    /// direction of its level.
    pub overridden: bool,
}

/// The shapes of `disc`, `circle` and `square` list markers, which browsers
/// paint at a size of their own instead of drawing the font's glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// A filled circle.
    Disc,
    /// An outlined circle.
    Circle,
    /// A filled square.
    Square,
    /// A filled triangle pointing to the end of the line.
    DisclosureClosed,
    /// A filled triangle pointing down.
    DisclosureOpen,
}

/// Where words break, from `overflow-wrap` and `word-break`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreaking {
    /// Words never break.
    #[default]
    Normal,
    /// Words that do not fit on a line of their own break between any two
    /// characters; the minimum content width ignores these breaks
    /// (`overflow-wrap: break-word`).
    Overflowing,
    /// Like `Overflowing`, but the minimum content width counts the breaks
    /// (`overflow-wrap: anywhere`, `word-break: break-word`).
    Anywhere,
    /// Words break between any two characters, filling lines
    /// (`word-break: break-all`).
    All,
}

/// The inline content of a block container.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineContent {
    /// Runs in order.
    pub segments: Vec<Segment>,
    /// Alignment of lines.
    pub text_align: TextAlign,
    /// The base direction of the lines: where they start and the order of
    /// their text.
    pub direction: Direction,
    /// First-line indent in pixels.
    pub indent: f32,
    /// Height of an empty line (the container's line height).
    pub strut: f32,
    /// The container's font, whose ascent places the baseline of every line.
    pub strut_font: Font,
    /// The atomic inline-level boxes among the runs.
    pub atoms: Vec<Atom>,
    /// Lines that overflow the container end in an ellipsis
    /// (`text-overflow: ellipsis` where the container clips).
    pub ellipsis: bool,
    /// The lines shown at most, the last ending in an ellipsis when more
    /// follow (`-webkit-line-clamp`).
    pub clamp: Option<u32>,
}

/// An atomic inline-level box in inline content.
#[derive(Debug, Clone, PartialEq)]
pub struct Atom {
    /// Its layout node.
    pub node: usize,
    /// How it sits on its line.
    pub align: VerticalAlign,
}

/// The margin box of an atom as laid out: its size and the offset of its
/// baseline from its top.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AtomSize {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Baseline below the top.
    pub baseline: f32,
}

/// An atom on a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedAtom {
    /// Index of the atom.
    pub atom: usize,
    /// Index of its segment.
    pub segment: usize,
    /// Its margin box's left, from the content box's left edge.
    pub x: f32,
    /// Its margin box's top, from the content box's top edge.
    pub y: f32,
}

/// A run of text on one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    /// Index of the segment.
    pub segment: usize,
    /// Offset from the left edge of the content box.
    pub x: f32,
    /// Advance width.
    pub width: f32,
    /// The text.
    pub text: String,
    /// The resolved bidirectional level: odd for right-to-left text.
    pub level: u8,
}

/// One line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Offset from the top of the content box.
    pub top: f32,
    /// Line height.
    pub height: f32,
    /// The baseline, below the line's top.
    pub baseline: f32,
    /// Fragments in the order of the text; right-to-left text places
    /// later fragments further left.
    pub fragments: Vec<Fragment>,
    /// Atomic boxes in the order of the text.
    pub atoms: Vec<PlacedAtom>,
}

impl Line {
    /// The left and right edges of the line's text and atoms, from the
    /// left edge of the content box.
    pub fn extent(&self, sizes: &[AtomSize]) -> (f32, f32) {
        let spans = self
            .fragments
            .iter()
            .map(|fragment| (fragment.x, fragment.x + fragment.width))
            .chain(self.atoms.iter().map(|placed| {
                let width = sizes.get(placed.atom).map_or(0.0, |size| size.width);
                (placed.x, placed.x + width)
            }));
        spans
            .reduce(|(left, right), (from, to)| (left.min(from), right.max(to)))
            .unwrap_or((0.0, 0.0))
    }
}

/// How far a line-height box of text reaches above its baseline: the font's
/// ascent and half the leading.
pub fn above_baseline(measurer: &dyn TextMeasurer, font: &Font, line_height: f32) -> f32 {
    if font.size <= 0.0 {
        return line_height / 2.0;
    }
    let metrics = measurer.metrics(font);
    metrics.ascent
        + crate::text::half_leading(measurer, line_height, metrics.ascent + metrics.descent)
}

/// The room floats leave a line: from `left` across `width`, between `top`
/// and `bottom`, where the floats beside it change; `None` when they do not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// Where the band starts, from the top of the content box.
    pub top: f32,
    /// The left edge of the room, from the left of the content box.
    pub left: f32,
    /// The width of the room.
    pub width: f32,
    /// Where the band ends.
    pub bottom: Option<f32>,
}

/// The result of line breaking.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lines {
    /// The lines.
    pub lines: Vec<Line>,
    /// Whether `-webkit-line-clamp` cut lines off after the last.
    pub clamped: bool,
    /// Width of the widest line.
    pub width: f32,
    /// Total height.
    pub height: f32,
}

impl InlineContent {
    /// Whether the content has no visible text: line boxes with nothing but
    /// spaces and inline boxes without margins, borders or padding have no
    /// height, so `<div><span></span></div>` is empty.
    pub fn is_blank(&self) -> bool {
        self.segments
            .iter()
            .flat_map(|segment| segment.pieces.iter())
            .all(|piece| match piece {
                Piece::Space(_) => true,
                Piece::Gap(width) => *width == 0.0,
                _ => false,
            })
            && self
                .segments
                .iter()
                .flat_map(|segment| segment.boxes.iter())
                .all(|inline| inline.above == 0.0 && inline.below == 0.0)
            && self.atoms.is_empty()
    }
}

fn measure(measurer: &dyn TextMeasurer, segment: &Segment, text: &str) -> f32 {
    // A disclosure triangle and its space are 0.98em wide whatever the
    // font, as measured in Chromium, which paints the triangle as a shape.
    if matches!(
        segment.symbol,
        Some(Symbol::DisclosureClosed | Symbol::DisclosureOpen)
    ) {
        return segment.font.size * 0.98 * text.chars().count() as f32 / 2.0;
    }
    let spacing = segment.letter_spacing * text.chars().count() as f32;
    // Text at `font-size: 0` has no glyphs; platforms would draw the
    // smallest font instead.
    if segment.font.size <= 0.0 {
        return spacing;
    }
    measurer.width(text, &segment.font) + spacing
}

/// A line's fragments cut to end in an ellipsis within `available` pixels,
/// and the ellipsis, in the font of the text it follows: text that does not
/// fit before the mark is left out, as Chromium truncates
/// `text-overflow: ellipsis` and clamped lines.
pub(crate) fn ellipsize(
    content: &InlineContent,
    fragments: &[Fragment],
    available: f32,
    measurer: &dyn TextMeasurer,
) -> (Vec<Fragment>, Option<Fragment>) {
    const MARK: &str = "\u{2026}";
    if content.direction == Direction::Rtl {
        return ellipsize_left(content, fragments, measurer);
    }
    let Some(last) = fragments
        .iter()
        .rev()
        .find(|fragment| !fragment.text.is_empty())
    else {
        return (fragments.to_vec(), None);
    };
    let mark = |segment: usize, x: f32| Fragment {
        segment,
        x,
        width: measure(measurer, &content.segments[segment], MARK),
        text: MARK.into(),
        level: 0,
    };
    let mut kept: Vec<Fragment> = Vec::new();
    for fragment in fragments {
        let segment = &content.segments[fragment.segment];
        let room = available - measure(measurer, segment, MARK);
        if fragment.x + fragment.width <= room {
            kept.push(fragment.clone());
            continue;
        }
        let mut text = String::new();
        let mut width = 0.0;
        for character in fragment.text.chars() {
            let mut longer = text.clone();
            longer.push(character);
            let longer_width = measure(measurer, segment, &longer);
            if fragment.x + longer_width > room {
                break;
            }
            text = longer;
            width = longer_width;
        }
        if !text.is_empty() {
            kept.push(Fragment {
                segment: fragment.segment,
                x: fragment.x,
                width,
                text,
                level: fragment.level,
            });
        }
        let (segment, x) = kept.last().map_or((fragment.segment, fragment.x), |kept| {
            (kept.segment, kept.x + kept.width)
        });
        return (kept, Some(mark(segment, x)));
    }
    // A clamped line that fits: the mark follows its text.
    let x = kept.last().map_or(0.0, |kept| kept.x + kept.width);
    let segment = last.segment;
    (kept, Some(mark(segment, x)))
}

/// The ellipsis of a right-to-left line, whose text ends on the left: the
/// mark is at the content box's left edge, and text left of it is cut off,
/// from the end of right-to-left text and the start of left-to-right text.
fn ellipsize_left(
    content: &InlineContent,
    fragments: &[Fragment],
    measurer: &dyn TextMeasurer,
) -> (Vec<Fragment>, Option<Fragment>) {
    const MARK: &str = "\u{2026}";
    let Some(last) = fragments
        .iter()
        .rev()
        .find(|fragment| !fragment.text.is_empty())
    else {
        return (fragments.to_vec(), None);
    };
    let mut kept: Vec<Fragment> = Vec::new();
    let mut cut = false;
    let mut leftmost: Option<&Fragment> = None;
    for fragment in fragments {
        let segment = &content.segments[fragment.segment];
        let room = measure(measurer, segment, MARK);
        if fragment.x >= room - 0.01 {
            if leftmost.is_none_or(|left| fragment.x < left.x) {
                leftmost = Some(fragment);
            }
            kept.push(fragment.clone());
            continue;
        }
        cut = true;
        let right = fragment.x + fragment.width;
        let characters: Vec<char> = fragment.text.chars().collect();
        // Left-to-right text loses its first characters, right-to-left
        // text its last.
        let rtl = fragment.level % 2 == 1;
        let mut shown = characters.len();
        let mut text = String::new();
        let mut width = 0.0;
        while shown > 0 {
            let candidate: String = if rtl {
                characters[..shown].iter().collect()
            } else {
                characters[characters.len() - shown..].iter().collect()
            };
            let candidate_width = measure(measurer, segment, &candidate);
            if right - candidate_width >= room - 0.01 {
                text = candidate;
                width = candidate_width;
                break;
            }
            shown -= 1;
        }
        if !text.is_empty() {
            kept.push(Fragment {
                segment: fragment.segment,
                x: right - width,
                width,
                text,
                level: fragment.level,
            });
        }
    }
    let left = kept.iter().map(|fragment| fragment.x).reduce(f32::min);
    let segment = leftmost.map_or(last.segment, |fragment| fragment.segment);
    let width = measure(measurer, &content.segments[segment], MARK);
    // A clamped line that fits: the mark is left of its text.
    let x = if cut {
        0.0
    } else {
        left.unwrap_or(width) - width
    };
    (
        kept,
        Some(Fragment {
            segment,
            x,
            width,
            text: MARK.into(),
            level: 1,
        }),
    )
}

struct Builder<'s> {
    lines: Vec<Line>,
    current: Vec<Fragment>,
    /// The atoms on the current line, with their segments and left edges.
    atoms: Vec<(usize, usize, f32)>,
    sizes: &'s [AtomSize],
    x: f32,
    max_width: Option<f32>,
    has_content: bool,
    break_allowed: bool,
    /// The last character of a word just placed, which decides whether the
    /// line may break before the next word.
    last_char: Option<char>,
    /// The last place on the current line where it may break, after some
    /// content: text and boxes after it move to the next line together
    /// with a following word that cannot break before itself.
    opportunity: Option<Opportunity>,
    /// The bands floats leave lines, by the top of a line.
    bands: Option<&'s dyn Fn(f32) -> Band>,
    /// The band of the current line.
    band: Band,
    /// The bands of the finished lines.
    placed: Vec<Band>,
}

/// Where a line may break: how much of the current line comes before it.
#[derive(Clone, Copy)]
struct Opportunity {
    /// Fragments wholly before it.
    fragments: usize,
    /// The text length and width of the last of them then, which later
    /// text of the same segment may have grown.
    last: Option<(usize, f32)>,
    /// Atoms before it.
    atoms: usize,
    x: f32,
}

impl Builder<'_> {
    /// Marks that the line may break here.
    fn allow_break(&mut self) {
        self.break_allowed = true;
        if self.has_content {
            self.opportunity = Some(Opportunity {
                fragments: self.current.len(),
                last: self
                    .current
                    .last()
                    .map(|fragment| (fragment.text.len(), fragment.width)),
                atoms: self.atoms.len(),
                x: self.x,
            });
        }
    }

    /// Ends the line at its last opportunity and moves what follows that to
    /// the next line, as browsers do when a word that cannot break before
    /// itself overflows; returns false when the line has no opportunity.
    fn break_earlier(&mut self, content: &InlineContent, measurer: &dyn TextMeasurer) -> bool {
        let Some(opportunity) = self.opportunity.take() else {
            return false;
        };
        let mut tail = self.current.split_off(opportunity.fragments);
        if let (Some((length, width)), Some(last)) = (opportunity.last, self.current.last_mut())
            && last.text.len() > length
        {
            tail.insert(
                0,
                Fragment {
                    segment: last.segment,
                    x: last.x + width,
                    width: last.width - width,
                    text: last.text.split_off(length),
                    level: last.level,
                },
            );
            last.width = width;
        }
        let atoms = self.atoms.split_off(opportunity.atoms);
        let end = self.x;
        self.finish_line(content, measurer);
        let start = self.x;
        for mut fragment in tail {
            fragment.x += start - opportunity.x;
            self.current.push(fragment);
        }
        for (atom, segment, x) in atoms {
            self.atoms.push((atom, segment, x + start - opportunity.x));
        }
        self.x = start + end - opportunity.x;
        self.has_content = !self.current.is_empty() || !self.atoms.is_empty();
        true
    }

    /// Starts a line at `top`, in the band the floats leave there.
    fn start(&mut self, top: f32) {
        if let Some(bands) = self.bands {
            self.band = bands(top);
            self.max_width = Some(self.band.width.max(0.0));
        }
    }

    /// Moves an empty line down past floats while content `width` wide does
    /// not fit beside them, as browsers move line boxes below floats.
    fn descend(&mut self, width: f32) {
        while self.current.is_empty()
            && self.atoms.is_empty()
            && let Some(bottom) = self.band.bottom
            && bottom > self.band.top
            && self
                .max_width
                .is_some_and(|max| self.x + width > max + 0.01)
        {
            self.start(bottom);
        }
    }

    fn push_atom(&mut self, segment: usize, atom: usize) {
        let width = self.sizes.get(atom).map_or(0.0, |size| size.width);
        self.atoms.push((atom, segment, self.x));
        self.x += width;
    }

    fn push(&mut self, segment: usize, text: &str, width: f32) {
        // A tab is a fragment of its own, as wide as its stop makes it.
        match self.current.last_mut() {
            Some(last)
                if last.segment == segment && !last.text.contains('\t') && !text.contains('\t') =>
            {
                last.text.push_str(text);
                last.width += width;
            }
            _ => self.current.push(Fragment {
                segment,
                x: self.x,
                width,
                text: text.to_owned(),
                level: 0,
            }),
        }
        self.x += width;
    }

    fn finish_line(&mut self, content: &InlineContent, measurer: &dyn TextMeasurer) {
        let mut fragments = core::mem::take(&mut self.current);
        while let Some(last) = fragments.last_mut() {
            let trimmed = last.text.trim_end_matches(' ');
            if trimmed.len() == last.text.len()
                || !content.segments[last.segment]
                    .white_space
                    .collapses_spaces()
            {
                break;
            }
            if trimmed.is_empty() {
                fragments.pop();
                continue;
            }
            let length = trimmed.len();
            last.text.truncate(length);
            last.width = measure(measurer, &content.segments[last.segment], &last.text);
            break;
        }
        // A fragment's text is shaped whole, so pairs kern across the words
        // and spaces it was measured in: each takes its whole width, and
        // what follows it moves by the difference.
        let mut moved: Vec<(f32, f32)> = Vec::new();
        let mut shift = 0.0;
        for fragment in &mut fragments {
            let end = fragment.x + fragment.width;
            fragment.x += shift;
            if !fragment.text.is_empty() && !fragment.text.contains('\t') {
                let whole = measure(
                    measurer,
                    &content.segments[fragment.segment],
                    &fragment.text,
                );
                shift += whole - fragment.width;
                fragment.width = whole;
            }
            moved.push((end, shift));
        }
        if shift != 0.0 {
            for (_, _, x) in &mut self.atoms {
                let before = moved.iter().rev().find(|(end, _)| *end <= *x + 0.01);
                *x += before.map_or(0.0, |(_, shift)| *shift);
            }
        }
        // Text and atoms share the baseline: the line reaches as far above
        // and below it as they do, starting from the container's strut.
        let mut above = above_baseline(measurer, &content.strut_font, content.strut);
        let mut below = content.strut - above;
        for fragment in &fragments {
            let segment = &content.segments[fragment.segment];
            let top = above_baseline(measurer, &segment.font, segment.line_height);
            above = above.max(top + segment.shift);
            below = below.max(segment.line_height - top - segment.shift);
            if segment.normal_line
                && let Some((top, bottom)) = measurer.normal_extent(&fragment.text, &segment.font)
            {
                above = above.max(top + segment.shift);
                below = below.max(bottom - segment.shift);
            }
        }
        let atoms = core::mem::take(&mut self.atoms);
        let mut raised = Vec::with_capacity(atoms.len());
        for (atom, segment, _) in &atoms {
            let size = self.sizes.get(*atom).copied().unwrap_or_default();
            let segment = &content.segments[*segment];
            let lift = match &content.atoms[*atom].align {
                VerticalAlign::Top | VerticalAlign::Bottom => None,
                align => Some(lift(align, size, segment, measurer) + segment.shift),
            };
            if let Some(lift) = lift {
                above = above.max(size.baseline + lift);
                below = below.max(size.height - size.baseline - lift);
            }
            raised.push(lift);
        }
        // Boxes aligned with the line's top or bottom may make it taller.
        for ((atom, _, _), lift) in atoms.iter().zip(&raised) {
            let size = self.sizes.get(*atom).copied().unwrap_or_default();
            if lift.is_none() && size.height > above + below {
                match content.atoms[*atom].align {
                    VerticalAlign::Bottom => above = size.height - below,
                    _ => below = size.height - above,
                }
            }
        }
        let height = above + below;
        let placed = atoms
            .iter()
            .zip(&raised)
            .map(|((atom, segment, x), lift)| {
                let size = self.sizes.get(*atom).copied().unwrap_or_default();
                let y = match (lift, &content.atoms[*atom].align) {
                    (Some(lift), _) => above - lift - size.baseline,
                    (None, VerticalAlign::Bottom) => height - size.height,
                    (None, _) => 0.0,
                };
                PlacedAtom {
                    atom: *atom,
                    segment: *segment,
                    x: *x,
                    y,
                }
            })
            .collect();
        self.lines.push(Line {
            top: 0.0,
            height,
            baseline: above,
            fragments,
            atoms: placed,
        });
        self.x = 0.0;
        self.has_content = false;
        self.opportunity = None;
        if self.bands.is_some() {
            self.placed.push(self.band);
            self.start(self.band.top + height);
        }
    }
}

/// How far an atom's baseline sits above the line's baseline, for the
/// `vertical-align` values that place it relative to the parent's text.
fn lift(
    align: &VerticalAlign,
    size: AtomSize,
    segment: &Segment,
    measurer: &dyn TextMeasurer,
) -> f32 {
    let font = &segment.font;
    let metrics = if font.size > 0.0 {
        measurer.metrics(font)
    } else {
        crate::text::FontMetrics {
            ascent: 0.0,
            descent: 0.0,
        }
    };
    match align {
        // The box's middle at half an x-height above the baseline; the
        // x-height is taken as half the font size.
        VerticalAlign::Middle => font.size / 4.0 + size.height / 2.0 - size.baseline,
        VerticalAlign::TextTop => metrics.ascent - size.baseline,
        VerticalAlign::TextBottom => size.height - size.baseline - metrics.descent,
        // As Chromium shifts them.
        VerticalAlign::Sub => -(font.size / 5.0 + 1.0),
        VerticalAlign::Super => font.size / 3.0 + 1.0,
        VerticalAlign::Length(LengthPercentage::Px(px)) => *px,
        VerticalAlign::Length(LengthPercentage::Percent(fraction)) => {
            fraction * segment.line_height
        }
        _ => 0.0,
    }
}

/// Preserved white space split into runs of spaces and single tabs, with
/// whether each is a tab.
fn space_runs(spaces: &str) -> Vec<(&str, bool)> {
    let mut runs = Vec::new();
    let mut start = 0;
    for (index, character) in spaces.char_indices() {
        if character == '\t' {
            if start < index {
                runs.push((&spaces[start..index], false));
            }
            runs.push((&spaces[index..index + 1], true));
            start = index + 1;
        }
    }
    if start < spaces.len() {
        runs.push((&spaces[start..], false));
    }
    runs
}

/// How far a tab at `x` advances: to the next tab stop at least half a
/// space away (CSS Text's tab stops, measured from the line's start).
fn tab_advance(x: f32, (interval, space): (f32, f32)) -> f32 {
    if interval <= 0.0 {
        return 0.0;
    }
    let mut stop = (crate::math::floor(x / interval + 1e-4) + 1.0) * interval;
    if stop - x < space / 2.0 {
        stop += interval;
    }
    stop - x
}

/// The longest start of `word` no wider than `room`, with its width.
fn fitting(measurer: &dyn TextMeasurer, segment: &Segment, word: &str, room: f32) -> (usize, f32) {
    let ends: Vec<usize> = word
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .collect();
    let (mut low, mut high) = (0, ends.len());
    let mut width = 0.0;
    // Widths grow with length: find the most characters that fit.
    while low < high {
        let middle = (low + high).div_ceil(2);
        let measured = measure(measurer, segment, &word[..ends[middle - 1]]);
        if measured <= room + 0.01 {
            low = middle;
            width = measured;
        } else {
            high = middle - 1;
        }
    }
    (if low == 0 { 0 } else { ends[low - 1] }, width)
}

/// Breaks inline content into lines no wider than `max_width` when given.
pub fn layout_lines(
    content: &InlineContent,
    max_width: Option<f32>,
    measurer: &dyn TextMeasurer,
    sizes: &[AtomSize],
) -> Lines {
    break_lines(content, max_width, false, measurer, sizes)
}

/// Breaks inline content into lines; `min_content` tells that the lines
/// measure the minimum content width, which breaks only some words count.
/// Places a word, or a part of one between break opportunities: on the next
/// line when it overflows and the line may break before it, split between
/// lines as `overflow-wrap` and `word-break` allow.
fn place_word(
    builder: &mut Builder<'_>,
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    (index, segment): (usize, &Segment),
    word: &str,
    width: f32,
    min_content: bool,
) {
    let overflows = |builder: &Builder| {
        builder
            .max_width
            .is_some_and(|max| builder.x + width > max + 0.01)
    };
    let breaking = segment.word_breaking;
    if overflows(builder)
        && builder.has_content
        && builder.break_allowed
        && breaking != WordBreaking::All
    {
        builder.finish_line(content, measurer);
    }
    let splits = match breaking {
        WordBreaking::Normal => false,
        WordBreaking::Overflowing => !min_content,
        WordBreaking::Anywhere | WordBreaking::All => true,
    };
    if splits && let Some(max) = builder.max_width.filter(|_| overflows(builder)) {
        // The word fills lines as far as it fits, with at least one
        // character on each.
        let mut rest = word;
        while !rest.is_empty() {
            let (mut end, mut used) = fitting(measurer, segment, rest, max - builder.x);
            if end == 0 && builder.has_content {
                builder.finish_line(content, measurer);
                continue;
            }
            if end == 0 {
                end = rest.chars().next().map_or(0, char::len_utf8);
                used = measure(measurer, segment, &rest[..end]);
            }
            builder.push(index, &rest[..end], used);
            builder.has_content = true;
            rest = &rest[end..];
            if !rest.is_empty() {
                builder.finish_line(content, measurer);
            }
        }
    } else {
        builder.push(index, word, width);
    }
    builder.has_content = true;
}

/// Places a word that overflows its line, breaking it where Unicode's line
/// breaking rules allow (`starts`, ascending byte offsets): each line takes
/// the longest part that fits up to an opportunity.
fn place_breaking(
    builder: &mut Builder<'_>,
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    (index, segment): (usize, &Segment),
    (word, width, starts): (&str, f32, &[usize]),
    min_content: bool,
) {
    let mut start = 0;
    while start < word.len() {
        let rest = &word[start..];
        let rest_width = if start == 0 {
            width
        } else {
            measure(measurer, segment, rest)
        };
        let room = builder
            .max_width
            .map_or(f32::INFINITY, |max| max - builder.x);
        if rest_width <= room + 0.01 {
            place_word(
                builder,
                content,
                measurer,
                (index, segment),
                rest,
                rest_width,
                min_content,
            );
            return;
        }
        let (fitting_end, _) = fitting(measurer, segment, rest, room);
        let end = starts
            .iter()
            .rev()
            .copied()
            .find(|&end| end > start && end - start <= fitting_end);
        let end = match end {
            Some(end) => end,
            // Nothing fits before an opportunity: the rest goes on the next
            // line, or overflows a line of its own up to its next one.
            None if builder.has_content && builder.break_allowed => {
                builder.finish_line(content, measurer);
                continue;
            }
            None if start == 0 && builder.break_earlier(content, measurer) => continue,
            None => starts
                .iter()
                .copied()
                .find(|&end| end > start)
                .unwrap_or(word.len()),
        };
        let part = &word[start..end];
        let part_width = measure(measurer, segment, part);
        place_word(
            builder,
            content,
            measurer,
            (index, segment),
            part,
            part_width,
            min_content,
        );
        builder.allow_break();
        start = end;
    }
}

pub(crate) fn break_lines(
    content: &InlineContent,
    max_width: Option<f32>,
    min_content: bool,
    measurer: &dyn TextMeasurer,
    sizes: &[AtomSize],
) -> Lines {
    break_lines_in(content, max_width, min_content, measurer, sizes, None)
}

/// Breaks inline content into lines in the bands floats leave them: each
/// line takes the band at its top, and moves below floats when its first
/// word or box does not fit beside them.
pub(crate) fn break_lines_beside(
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    sizes: &[AtomSize],
    bands: &dyn Fn(f32) -> Band,
) -> Lines {
    break_lines_in(content, None, false, measurer, sizes, Some(bands))
}

fn break_lines_in(
    content: &InlineContent,
    max_width: Option<f32>,
    min_content: bool,
    measurer: &dyn TextMeasurer,
    sizes: &[AtomSize],
    bands: Option<&dyn Fn(f32) -> Band>,
) -> Lines {
    let mut builder = Builder {
        lines: Vec::new(),
        current: Vec::new(),
        atoms: Vec::new(),
        sizes,
        x: content.indent,
        max_width,
        has_content: false,
        break_allowed: false,
        last_char: None,
        opportunity: None,
        bands,
        band: Band {
            top: 0.0,
            left: 0.0,
            width: max_width.unwrap_or(f32::INFINITY),
            bottom: None,
        },
        placed: Vec::new(),
    };
    builder.start(0.0);
    for (index, segment) in content.segments.iter().enumerate() {
        for piece in &segment.pieces {
            match piece {
                Piece::Break => {
                    builder.finish_line(content, measurer);
                    builder.last_char = None;
                }
                Piece::Atom(atom) => {
                    let width = sizes.get(*atom).map_or(0.0, |size| size.width);
                    builder.descend(width);
                    let wraps = segment.white_space.wraps();
                    let overflows = builder
                        .max_width
                        .is_some_and(|max| builder.x + width > max + 0.01);
                    // Lines may break on either side of an atomic box.
                    if overflows && builder.has_content && (builder.break_allowed || wraps) {
                        builder.finish_line(content, measurer);
                    }
                    builder.push_atom(index, *atom);
                    builder.has_content = true;
                    builder.break_allowed = false;
                    if wraps {
                        builder.allow_break();
                    }
                    builder.last_char = None;
                }
                Piece::Gap(width) => {
                    let overflows = builder
                        .max_width
                        .is_some_and(|max| builder.x + width > max + 0.01);
                    if overflows && builder.has_content && builder.break_allowed {
                        builder.finish_line(content, measurer);
                    }
                    builder.push(index, "", *width);
                    // Spaces after an empty box at the start still collapse.
                    builder.has_content |= *width != 0.0;
                    builder.break_allowed = false;
                    builder.last_char = None;
                }
                Piece::Space(spaces) => {
                    let leading = !builder.has_content && segment.white_space.collapses_spaces();
                    if !leading {
                        for (spaces, tab) in space_runs(spaces) {
                            let width = if tab {
                                tab_advance(builder.x, segment.tab)
                            } else {
                                measure(measurer, segment, spaces)
                            };
                            builder.push(index, spaces, width);
                        }
                        builder.break_allowed = false;
                        if segment.white_space.wraps() {
                            builder.allow_break();
                        }
                    }
                    builder.last_char = None;
                }
                Piece::Word(word) => {
                    let wraps = segment.white_space.wraps();
                    let width = measure(measurer, segment, word);
                    builder.descend(width);
                    let fits = builder
                        .max_width
                        .is_none_or(|max| builder.x + width <= max + 0.01);
                    // `word-break: break-all` breaks anywhere already.
                    if fits || !wraps || segment.word_breaking == WordBreaking::All {
                        place_word(
                            &mut builder,
                            content,
                            measurer,
                            (index, segment),
                            word,
                            width,
                            min_content,
                        );
                    } else {
                        let starts = crate::line_break::opportunities(builder.last_char, word);
                        if starts.first() == Some(&0) && builder.has_content {
                            builder.allow_break();
                        }
                        place_breaking(
                            &mut builder,
                            content,
                            measurer,
                            (index, segment),
                            (word, width, &starts),
                            min_content,
                        );
                    }
                    builder.has_content = true;
                    builder.break_allowed = false;
                    builder.last_char = word.chars().next_back();
                }
            }
        }
    }
    if builder.has_content
        || !builder.current.is_empty()
        || !builder.atoms.is_empty()
        || builder.lines.is_empty()
    {
        builder.finish_line(content, measurer);
    }
    let placed = builder.placed;
    let mut lines = builder.lines;
    let mut clamped = false;
    if let Some(shown) = content.clamp.filter(|_| !min_content)
        && lines.len() > shown as usize
    {
        lines.truncate(shown as usize);
        clamped = true;
    }
    let rtl = content.direction == Direction::Rtl;
    if !min_content && is_bidirectional(content) {
        for (index, line) in lines.iter_mut().enumerate() {
            let indent = if index == 0 { content.indent } else { 0.0 };
            reorder(line, content, measurer, sizes, indent);
        }
    }
    let mut top = 0.0;
    let mut width: f32 = 0.0;
    for (index, line) in lines.iter_mut().enumerate() {
        // Lines beside floats start in their bands.
        let band = placed.get(index);
        line.top = band.map_or(top, |band| band.top);
        top = line.top + line.height;
        let mut line_width = line.extent(sizes).1;
        // The first line of right-to-left text is indented on the right.
        if rtl && index == 0 {
            line_width += content.indent;
        }
        let left = band.map_or(0.0, |band| band.left);
        width = width.max(left + line_width);
        let room = band.map_or(max_width, |band| Some(band.width));
        // Lines start on the left or, right to left, on the right; a line
        // too long for the box overflows its end.
        let offset = left
            + room.map_or(0.0, |max| {
                let free = max - line_width;
                let align = match content.text_align {
                    TextAlign::Start | TextAlign::Justify if rtl => TextAlign::Right,
                    TextAlign::End if rtl => TextAlign::Left,
                    TextAlign::End => TextAlign::Right,
                    align => align,
                };
                match align {
                    _ if free < 0.0 => {
                        if rtl {
                            free
                        } else {
                            0.0
                        }
                    }
                    TextAlign::Right => free,
                    TextAlign::Center => free / 2.0,
                    _ => 0.0,
                }
            });
        for fragment in &mut line.fragments {
            fragment.x += offset;
        }
        for placed in &mut line.atoms {
            placed.x += offset;
            placed.y += line.top;
        }
    }
    Lines {
        lines,
        clamped,
        width,
        height: top,
    }
}

/// Whether inline content needs bidirectional reordering: it runs right to
/// left, embeds text in another direction, or has right-to-left text.
fn is_bidirectional(content: &InlineContent) -> bool {
    content.direction == Direction::Rtl
        || content.segments.iter().any(|segment| {
            segment.level != 0
                || segment.pieces.iter().any(|piece| match piece {
                    Piece::Word(text) | Piece::Space(text) => crate::bidi::has_right(text),
                    _ => false,
                })
        })
}

/// Places a line's text and atoms in visual order: their characters'
/// bidirectional levels are resolved, fragments split where the level
/// changes, and the pieces reversed by level. The line was laid out left to
/// right after an indent; the gap before each fragment or atom (inline box
/// margins, borders and padding) stays on its start side, and a
/// right-to-left line's indent moves to its right.
fn reorder(
    line: &mut Line,
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    sizes: &[AtomSize],
    indent: f32,
) {
    use crate::bidi::{Class, class_runs, resolve, visual_order};
    // The fragments and atoms in the order of the text.
    enum Part {
        Text(usize),
        Atom(usize),
    }
    let mut parts: Vec<(f32, Part)> = line
        .fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| (fragment.x, Part::Text(index)))
        .chain(
            line.atoms
                .iter()
                .enumerate()
                .map(|(index, placed)| (placed.x, Part::Atom(index))),
        )
        .collect();
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The gap before each part, from the end of the one before it.
    let mut end = indent;
    let gaps: Vec<f32> = parts
        .iter()
        .map(|(x, kind)| {
            let width = match kind {
                Part::Text(index) => line.fragments[*index].width,
                Part::Atom(index) => sizes
                    .get(line.atoms[*index].atom)
                    .map_or(0.0, |size| size.width),
            };
            let gap = (x - end).max(0.0);
            end = x + width;
            gap
        })
        .collect();
    // Each part's runs of characters of one class, as byte ranges.
    let mut units: Vec<(u8, Class)> = Vec::new();
    let mut ranges: Vec<(usize, core::ops::Range<usize>)> = Vec::new();
    for (part, (_, kind)) in parts.iter().enumerate() {
        match kind {
            Part::Text(index) => {
                let fragment = &line.fragments[*index];
                let segment = &content.segments[fragment.segment];
                let runs = if segment.overridden || fragment.text.is_empty() {
                    let class = match (segment.overridden, segment.level % 2 == 1) {
                        (false, _) => Class::Neutral,
                        (true, true) => Class::Right,
                        (true, false) => Class::Left,
                    };
                    vec![(0..fragment.text.len(), class)]
                } else {
                    class_runs(&fragment.text)
                };
                for (range, class) in runs {
                    units.push((segment.level, class));
                    ranges.push((part, range));
                }
            }
            Part::Atom(index) => {
                let segment = &content.segments[line.atoms[*index].segment];
                units.push((segment.level, Class::Neutral));
                ranges.push((part, 0..0));
            }
        }
    }
    let paragraph = u8::from(content.direction == Direction::Rtl);
    let levels = resolve(&units, paragraph);
    // A left-to-right line of left-to-right text stays as it was laid out.
    if levels.iter().all(|level| *level == 0) {
        return;
    }
    // Pieces: a part's runs at one level.
    struct Run {
        part: usize,
        range: core::ops::Range<usize>,
        level: u8,
    }
    let mut pieces: Vec<Run> = Vec::new();
    for ((part, range), level) in ranges.into_iter().zip(levels) {
        match pieces.last_mut() {
            Some(last) if last.part == part && last.level == level => last.range.end = range.end,
            _ => pieces.push(Run { part, range, level }),
        }
    }
    let mut fragments: Vec<Fragment> = Vec::new();
    // For each piece, its fragment or atom, its width and the gap on its
    // start side, which only a part's first piece has.
    let mut placed: Vec<(Option<usize>, Option<usize>, f32, f32)> = Vec::new();
    for (number, piece) in pieces.iter().enumerate() {
        let first = number == 0 || pieces[number - 1].part != piece.part;
        let gap = if first { gaps[piece.part] } else { 0.0 };
        match &parts[piece.part].1 {
            Part::Text(index) => {
                let fragment = &line.fragments[*index];
                let whole = piece.range.start == 0 && piece.range.end == fragment.text.len();
                let text = fragment.text[piece.range.clone()].to_owned();
                let width = if whole {
                    fragment.width
                } else {
                    measure(measurer, &content.segments[fragment.segment], &text)
                };
                fragments.push(Fragment {
                    segment: fragment.segment,
                    x: 0.0,
                    width,
                    text,
                    level: piece.level,
                });
                placed.push((Some(fragments.len() - 1), None, width, gap));
            }
            Part::Atom(index) => {
                let atom = line.atoms[*index].atom;
                let width = sizes.get(atom).map_or(0.0, |size| size.width);
                placed.push((None, Some(*index), width, gap));
            }
        }
    }
    let levels: Vec<u8> = pieces.iter().map(|piece| piece.level).collect();
    let mut x = if paragraph == 1 { 0.0 } else { indent };
    for index in visual_order(&levels) {
        let (fragment, atom, width, gap) = placed[index];
        // Right-to-left pieces start on their right.
        let rtl = levels[index] % 2 == 1;
        if !rtl {
            x += gap;
        }
        if let Some(fragment) = fragment {
            fragments[fragment].x = x;
        }
        if let Some(atom) = atom {
            line.atoms[atom].x = x;
        }
        x += width;
        if rtl {
            x += gap;
        }
    }
    line.fragments = fragments;
}

#[cfg(test)]
mod tests;
