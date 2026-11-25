//! Conversion of computed styles to taffy styles.

use crate::prelude::*;

use nativeboat_css::ComputedStyle;
use nativeboat_css::types::{
    Alignment, BoxSizing, Direction, Display as CssDisplay, FlexDirection as CssFlexDirection,
    FlexWrap as CssFlexWrap, Float as CssFloat, GridLine, LengthPercentageAuto as CssAuto, MaxSize,
    Overflow as CssOverflow, PositionKind, RepeatCount, Size as CssSize, TrackEntry, TrackSize,
};
use nativeboat_css::{Calc, LengthPercentage as CssLength};
use taffy::Point;
use taffy::prelude::*;
use taffy::style::{
    AlignContent, AlignItems, Float, GridTemplateComponent, GridTemplateRepetition,
    MaxTrackSizingFunction, MinTrackSizingFunction, Overflow, RepetitionCount,
};

/// Owns `calc()` expressions referenced by taffy styles.
#[derive(Debug, Default)]
pub struct CalcStore {
    calcs: Vec<alloc::rc::Rc<Calc>>,
}

impl CalcStore {
    fn pointer(&mut self, calc: &Calc) -> *const () {
        let shared = alloc::rc::Rc::new(calc.clone());
        let pointer = alloc::rc::Rc::as_ptr(&shared).cast::<()>();
        self.calcs.push(shared);
        pointer
    }

    /// Resolves a pointer created by this store.
    pub fn resolve(pointer: *const (), basis: f32) -> f32 {
        // SAFETY: taffy only hands back pointers produced by `CalcStore::pointer`,
        // which point into reference-counted values owned by the store; the store
        // outlives every layout pass that uses its styles.
        let calc = unsafe { &*pointer.cast::<Calc>() };
        calc.resolve(basis)
    }

    fn length(&mut self, value: &CssLength) -> LengthPercentage {
        match value {
            CssLength::Px(px) => LengthPercentage::length(*px),
            CssLength::Percent(fraction) => LengthPercentage::percent(*fraction),
            CssLength::Calc(calc) => LengthPercentage::calc(self.pointer(calc)),
        }
    }

    fn length_auto(&mut self, value: &CssAuto) -> LengthPercentageAuto {
        match value {
            CssAuto::Auto => LengthPercentageAuto::auto(),
            CssAuto::Length(length) => self.length(length).into(),
        }
    }

    fn dimension(&mut self, value: &CssSize) -> Dimension {
        match value {
            CssSize::Length(length) => self.length(length).into(),
            CssSize::MinContent => Dimension::min_content(),
            CssSize::MaxContent => Dimension::max_content(),
            CssSize::FitContent => Dimension::fit_content(),
            CssSize::Auto => Dimension::auto(),
        }
    }

    fn min_dimension(&mut self, value: &CssSize) -> LengthPercentageAuto {
        match value {
            CssSize::Length(length) => self.length(length).into(),
            _ => LengthPercentageAuto::auto(),
        }
    }

    fn max_dimension(&mut self, value: &MaxSize) -> LengthPercentageAuto {
        match value {
            MaxSize::Length(length) => self.length(length).into(),
            _ => LengthPercentageAuto::auto(),
        }
    }

    fn min_track(&mut self, track: &TrackSize) -> MinTrackSizingFunction {
        match track {
            TrackSize::Length(length) => MinTrackSizingFunction::from(self.length(length)),
            TrackSize::MinContent => MinTrackSizingFunction::min_content(),
            TrackSize::MaxContent => MinTrackSizingFunction::max_content(),
            TrackSize::MinMax(minimum, _) => self.min_track(minimum),
            TrackSize::Fr(_) | TrackSize::Auto | TrackSize::FitContent(_) => {
                MinTrackSizingFunction::auto()
            }
        }
    }

    fn max_track(&mut self, track: &TrackSize) -> MaxTrackSizingFunction {
        match track {
            TrackSize::Length(length) => MaxTrackSizingFunction::from(self.length(length)),
            TrackSize::Fr(fraction) => MaxTrackSizingFunction::fr(*fraction),
            TrackSize::Auto => MaxTrackSizingFunction::auto(),
            TrackSize::MinContent => MaxTrackSizingFunction::min_content(),
            TrackSize::MaxContent => MaxTrackSizingFunction::max_content(),
            TrackSize::MinMax(_, maximum) => self.max_track(maximum),
            TrackSize::FitContent(limit) => MaxTrackSizingFunction::fit_content(self.length(limit)),
        }
    }

    pub(crate) fn track(&mut self, track: &TrackSize) -> TrackSizingFunction {
        let minimum = self.min_track(track);
        let maximum = self.max_track(track);
        minmax(minimum, maximum)
    }
}

fn alignment_items(alignment: Alignment) -> Option<AlignItems> {
    Some(match alignment {
        Alignment::Normal | Alignment::Auto => return None,
        Alignment::Start => AlignItems::START,
        Alignment::End => AlignItems::END,
        Alignment::FlexStart => AlignItems::FLEX_START,
        Alignment::FlexEnd => AlignItems::FLEX_END,
        Alignment::Center => AlignItems::CENTER,
        Alignment::Baseline => AlignItems::BASELINE,
        Alignment::Stretch
        | Alignment::SpaceBetween
        | Alignment::SpaceAround
        | Alignment::SpaceEvenly => AlignItems::STRETCH,
    })
}

fn alignment_content(alignment: Alignment) -> Option<AlignContent> {
    Some(match alignment {
        Alignment::Normal | Alignment::Auto => return None,
        Alignment::Start | Alignment::Baseline => AlignContent::START,
        Alignment::End => AlignContent::END,
        Alignment::FlexStart => AlignContent::FLEX_START,
        Alignment::FlexEnd => AlignContent::FLEX_END,
        Alignment::Center => AlignContent::CENTER,
        Alignment::Stretch => AlignContent::STRETCH,
        Alignment::SpaceBetween => AlignContent::SPACE_BETWEEN,
        Alignment::SpaceAround => AlignContent::SPACE_AROUND,
        Alignment::SpaceEvenly => AlignContent::SPACE_EVENLY,
    })
}

/// `justify-content` for the row the layout engine runs: a right-to-left
/// row runs mirrored, so its `start`, on the right, is where the mirrored
/// row's flex items start or end.
fn main_alignment(style: &ComputedStyle) -> Alignment {
    let row = match style.flex_direction {
        CssFlexDirection::Row => true,
        CssFlexDirection::RowReverse => false,
        _ => return style.justify_content,
    };
    match (style.direction, style.justify_content) {
        (Direction::Rtl, Alignment::Start) if row => Alignment::FlexStart,
        (Direction::Rtl, Alignment::End) if row => Alignment::FlexEnd,
        (Direction::Rtl, Alignment::Start) => Alignment::FlexEnd,
        (Direction::Rtl, Alignment::End) => Alignment::FlexStart,
        (_, alignment) => alignment,
    }
}

fn overflow(value: CssOverflow) -> Overflow {
    match value {
        CssOverflow::Visible => Overflow::Visible,
        CssOverflow::Clip => Overflow::Clip,
        // An `auto` scroll container takes room for a scrollbar only once
        // its content overflows, which layout decides.
        CssOverflow::Hidden | CssOverflow::Auto => Overflow::Hidden,
        CssOverflow::Scroll => Overflow::Scroll,
    }
}

fn placement(line: GridLine) -> GridPlacement {
    match line {
        GridLine::Auto => GridPlacement::Auto,
        GridLine::Line(index) => GridPlacement::from_line_index(index),
        GridLine::Span(count) => GridPlacement::from_span(count),
    }
}

/// How a box lays out its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Block flow.
    Block,
    /// Flexbox.
    Flex,
    /// CSS grid.
    Grid,
}

/// The layout mode for a display value.
pub fn mode_of(display: CssDisplay) -> Mode {
    match display {
        CssDisplay::Flex
        | CssDisplay::InlineFlex
        | CssDisplay::WebkitBox
        | CssDisplay::WebkitInlineBox => Mode::Flex,
        CssDisplay::Grid | CssDisplay::InlineGrid => Mode::Grid,
        _ => Mode::Block,
    }
}

/// Converts a computed style for a box with the given layout mode.
pub fn convert(style: &ComputedStyle, mode: Mode, calcs: &mut CalcStore) -> Style {
    let display = match mode {
        Mode::Block => Display::Block,
        Mode::Flex => Display::Flex,
        Mode::Grid => Display::Grid,
    };
    let sides = |calcs: &mut CalcStore, edges: [&CssLength; 4]| Rect {
        top: calcs.length(edges[0]),
        right: calcs.length(edges[1]),
        bottom: calcs.length(edges[2]),
        left: calcs.length(edges[3]),
    };
    let border = Rect {
        top: LengthPercentage::length(style.border_width.top.0),
        right: LengthPercentage::length(style.border_width.right.0),
        bottom: LengthPercentage::length(style.border_width.bottom.0),
        left: LengthPercentage::length(style.border_width.left.0),
    };
    let tracks = |calcs: &mut CalcStore,
                  list: &nativeboat_css::types::TrackList|
     -> Vec<GridTemplateComponent<String>> {
        list.0
            .iter()
            .map(|entry| match entry {
                TrackEntry::Single(track) => GridTemplateComponent::Single(calcs.track(track)),
                TrackEntry::Repeat(count, tracks) => {
                    GridTemplateComponent::Repeat(GridTemplateRepetition {
                        count: match count {
                            RepeatCount::Count(count) => RepetitionCount::Count(*count),
                            RepeatCount::AutoFill => RepetitionCount::AutoFill,
                            RepeatCount::AutoFit => RepetitionCount::AutoFit,
                        },
                        tracks: tracks.iter().map(|track| calcs.track(track)).collect(),
                        line_names: Vec::new(),
                    })
                }
            })
            .collect()
    };
    Style {
        display,
        box_sizing: match style.box_sizing {
            BoxSizing::ContentBox => taffy::BoxSizing::ContentBox,
            BoxSizing::BorderBox => taffy::BoxSizing::BorderBox,
        },
        overflow: Point {
            x: overflow(style.overflow_x),
            y: overflow(style.overflow_y),
        },
        scrollbar_width: 0.0,
        float: match style.float {
            CssFloat::None => Float::None,
            CssFloat::Left => Float::Left,
            CssFloat::Right => Float::Right,
        },
        position: match style.position {
            PositionKind::Absolute | PositionKind::Fixed => Position::Absolute,
            _ => Position::Relative,
        },
        inset: if style.position == PositionKind::Sticky {
            Rect {
                top: LengthPercentageAuto::auto(),
                right: LengthPercentageAuto::auto(),
                bottom: LengthPercentageAuto::auto(),
                left: LengthPercentageAuto::auto(),
            }
        } else {
            Rect {
                top: calcs.length_auto(&style.inset.top),
                right: calcs.length_auto(&style.inset.right),
                bottom: calcs.length_auto(&style.inset.bottom),
                left: calcs.length_auto(&style.inset.left),
            }
        },
        size: Size {
            width: calcs.dimension(&style.width),
            height: calcs.dimension(&style.height),
        },
        min_size: Size {
            width: calcs.min_dimension(&style.min_width),
            height: calcs.min_dimension(&style.min_height),
        },
        max_size: Size {
            width: calcs.max_dimension(&style.max_width),
            height: calcs.max_dimension(&style.max_height),
        },
        aspect_ratio: style.aspect_ratio.0,
        margin: Rect {
            top: calcs.length_auto(&style.margin.top),
            right: calcs.length_auto(&style.margin.right),
            bottom: calcs.length_auto(&style.margin.bottom),
            left: calcs.length_auto(&style.margin.left),
        },
        padding: sides(
            calcs,
            [
                &style.padding.top,
                &style.padding.right,
                &style.padding.bottom,
                &style.padding.left,
            ],
        ),
        border,
        align_items: alignment_items(style.align_items),
        align_self: alignment_items(style.align_self),
        justify_items: alignment_items(style.justify_items),
        justify_self: alignment_items(style.justify_self),
        align_content: alignment_content(style.align_content),
        justify_content: alignment_content(main_alignment(style)),
        gap: Size {
            width: calcs.length(&style.column_gap),
            height: calcs.length(&style.row_gap),
        },
        // Right to left, a row runs from the right: as a reversed row does
        // left to right.
        flex_direction: match (style.flex_direction, style.direction == Direction::Rtl) {
            (CssFlexDirection::Row, false) | (CssFlexDirection::RowReverse, true) => {
                FlexDirection::Row
            }
            (CssFlexDirection::RowReverse, false) | (CssFlexDirection::Row, true) => {
                FlexDirection::RowReverse
            }
            (CssFlexDirection::Column, _) => FlexDirection::Column,
            (CssFlexDirection::ColumnReverse, _) => FlexDirection::ColumnReverse,
        },
        flex_wrap: match style.flex_wrap {
            CssFlexWrap::NoWrap => FlexWrap::NoWrap,
            CssFlexWrap::Wrap => FlexWrap::Wrap,
            CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
        },
        flex_basis: calcs.dimension(&style.flex_basis),
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        grid_template_rows: tracks(calcs, &style.grid_template_rows),
        grid_template_columns: tracks(calcs, &style.grid_template_columns),
        grid_auto_rows: style
            .grid_auto_rows
            .0
            .iter()
            .map(|track| calcs.track(track))
            .collect(),
        grid_auto_columns: style
            .grid_auto_columns
            .0
            .iter()
            .map(|track| calcs.track(track))
            .collect(),
        grid_auto_flow: match (style.grid_auto_flow.column, style.grid_auto_flow.dense) {
            (false, false) => GridAutoFlow::Row,
            (false, true) => GridAutoFlow::RowDense,
            (true, false) => GridAutoFlow::Column,
            (true, true) => GridAutoFlow::ColumnDense,
        },
        grid_row: Line {
            start: placement(style.grid_row_start),
            end: placement(style.grid_row_end),
        },
        grid_column: Line {
            start: placement(style.grid_column_start),
            end: placement(style.grid_column_end),
        },
        ..Style::default()
    }
}

#[cfg(test)]
mod tests;
