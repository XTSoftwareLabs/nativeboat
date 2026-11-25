use super::*;
use nativeboat_css::types::{Edges, GridAutoFlow as CssFlow, TrackList, TrackSizes};

fn styled(change: impl FnOnce(&mut ComputedStyle)) -> ComputedStyle {
    let mut style = ComputedStyle::default();
    change(&mut style);
    style
}

#[test]
fn modes_and_displays() {
    assert_eq!(mode_of(CssDisplay::Flex), Mode::Flex);
    assert_eq!(mode_of(CssDisplay::InlineFlex), Mode::Flex);
    assert_eq!(mode_of(CssDisplay::Grid), Mode::Grid);
    assert_eq!(mode_of(CssDisplay::InlineGrid), Mode::Grid);
    assert_eq!(mode_of(CssDisplay::Block), Mode::Block);
    let mut calcs = CalcStore::default();
    let style = ComputedStyle::default();
    assert_eq!(
        convert(&style, Mode::Block, &mut calcs).display,
        Display::Block
    );
    assert_eq!(
        convert(&style, Mode::Flex, &mut calcs).display,
        Display::Flex
    );
    assert_eq!(
        convert(&style, Mode::Grid, &mut calcs).display,
        Display::Grid
    );
}

#[test]
fn box_properties() {
    let mut calcs = CalcStore::default();
    let style = styled(|style| {
        style.box_sizing = BoxSizing::BorderBox;
        style.float = CssFloat::Left;
        style.position = PositionKind::Absolute;
        style.width = CssSize::Length(CssLength::Px(10.0));
        style.height = CssSize::Length(CssLength::Percent(0.5));
        style.min_width = CssSize::Length(CssLength::Px(1.0));
        style.min_height = CssSize::MinContent;
        style.max_width = MaxSize::Length(CssLength::Px(100.0));
        style.max_height = MaxSize::None;
        style.margin.left = CssAuto::Auto;
        style.inset.top = CssAuto::Length(CssLength::Px(3.0));
        style.overflow_x = CssOverflow::Hidden;
        style.overflow_y = CssOverflow::Auto;
        style.aspect_ratio = nativeboat_css::types::AspectRatio(Some(2.0));
        style.border_width = Edges::all(nativeboat_css::types::Px(2.0));
    });
    let converted = convert(&style, Mode::Block, &mut calcs);
    assert_eq!(converted.box_sizing, taffy::BoxSizing::BorderBox);
    assert_eq!(converted.float, Float::Left);
    assert_eq!(converted.position, Position::Absolute);
    assert_eq!(converted.size.width, Dimension::length(10.0));
    assert_eq!(converted.size.height, Dimension::percent(0.5));
    assert_eq!(converted.min_size.width, LengthPercentageAuto::length(1.0));
    assert_eq!(converted.min_size.height, LengthPercentageAuto::auto());
    assert_eq!(
        converted.max_size.width,
        LengthPercentageAuto::length(100.0)
    );
    assert_eq!(converted.max_size.height, LengthPercentageAuto::auto());
    assert_eq!(converted.margin.left, LengthPercentageAuto::auto());
    assert_eq!(converted.inset.top, LengthPercentageAuto::length(3.0));
    // `auto` takes room for a scrollbar once layout finds it overflowing.
    assert_eq!(converted.overflow.x, Overflow::Hidden);
    assert_eq!(converted.overflow.y, Overflow::Hidden);
    assert_eq!(converted.aspect_ratio, Some(2.0));
    assert_eq!(converted.border.top, LengthPercentage::length(2.0));
    let other = styled(|style| {
        style.float = CssFloat::Right;
        style.position = PositionKind::Fixed;
        style.overflow_x = CssOverflow::Clip;
        style.overflow_y = CssOverflow::Scroll;
        style.width = CssSize::MaxContent;
    });
    let converted = convert(&other, Mode::Block, &mut calcs);
    assert_eq!(converted.float, Float::Right);
    assert_eq!(converted.position, Position::Absolute);
    assert_eq!(converted.overflow.x, Overflow::Clip);
    assert_eq!(converted.overflow.y, Overflow::Scroll);
    assert_eq!(converted.size.width, Dimension::max_content());
    for (size, dimension) in [
        (CssSize::MinContent, Dimension::min_content()),
        (CssSize::FitContent, Dimension::fit_content()),
        (CssSize::Auto, Dimension::auto()),
    ] {
        let style = styled(|style| style.height = size.clone());
        assert_eq!(
            convert(&style, Mode::Block, &mut calcs).size.height,
            dimension
        );
    }
    let sticky = styled(|style| {
        style.position = PositionKind::Sticky;
        style.inset.top = CssAuto::Length(CssLength::Px(3.0));
    });
    let relative = convert(&sticky, Mode::Block, &mut calcs);
    assert_eq!(relative.position, Position::Relative);
    assert_eq!(relative.inset.top, LengthPercentageAuto::auto());
    assert_eq!(
        convert(&ComputedStyle::default(), Mode::Block, &mut calcs).float,
        Float::None
    );
}

#[test]
fn calc_values_resolve_through_the_store() {
    let mut calcs = CalcStore::default();
    let calc = Calc::Sum(vec![Calc::Percent(1.0), Calc::Px(-20.0)]);
    let style = styled(|style| style.width = CssSize::Length(CssLength::Calc(Box::new(calc))));
    let converted = convert(&style, Mode::Block, &mut calcs);
    let raw = converted.size.width.into_raw();
    assert_eq!(CalcStore::resolve(raw.calc_value(), 200.0), 180.0);
}

#[test]
fn alignment_values() {
    let items = [
        (Alignment::Normal, None),
        (Alignment::Auto, None),
        (Alignment::Start, Some(AlignItems::START)),
        (Alignment::End, Some(AlignItems::END)),
        (Alignment::FlexStart, Some(AlignItems::FLEX_START)),
        (Alignment::FlexEnd, Some(AlignItems::FLEX_END)),
        (Alignment::Center, Some(AlignItems::CENTER)),
        (Alignment::Baseline, Some(AlignItems::BASELINE)),
        (Alignment::Stretch, Some(AlignItems::STRETCH)),
        (Alignment::SpaceBetween, Some(AlignItems::STRETCH)),
    ];
    for (alignment, expected) in items {
        assert_eq!(alignment_items(alignment), expected, "{alignment:?}");
    }
    let content = [
        (Alignment::Normal, None),
        (Alignment::Start, Some(AlignContent::START)),
        (Alignment::Baseline, Some(AlignContent::START)),
        (Alignment::End, Some(AlignContent::END)),
        (Alignment::FlexStart, Some(AlignContent::FLEX_START)),
        (Alignment::FlexEnd, Some(AlignContent::FLEX_END)),
        (Alignment::Center, Some(AlignContent::CENTER)),
        (Alignment::Stretch, Some(AlignContent::STRETCH)),
        (Alignment::SpaceBetween, Some(AlignContent::SPACE_BETWEEN)),
        (Alignment::SpaceAround, Some(AlignContent::SPACE_AROUND)),
        (Alignment::SpaceEvenly, Some(AlignContent::SPACE_EVENLY)),
    ];
    for (alignment, expected) in content {
        assert_eq!(alignment_content(alignment), expected, "{alignment:?}");
    }
}

#[test]
fn flex_properties() {
    let mut calcs = CalcStore::default();
    for (direction, expected) in [
        (CssFlexDirection::Row, FlexDirection::Row),
        (CssFlexDirection::RowReverse, FlexDirection::RowReverse),
        (CssFlexDirection::Column, FlexDirection::Column),
        (
            CssFlexDirection::ColumnReverse,
            FlexDirection::ColumnReverse,
        ),
    ] {
        let style = styled(|style| style.flex_direction = direction);
        assert_eq!(
            convert(&style, Mode::Flex, &mut calcs).flex_direction,
            expected
        );
    }
    for (wrap, expected) in [
        (CssFlexWrap::NoWrap, FlexWrap::NoWrap),
        (CssFlexWrap::Wrap, FlexWrap::Wrap),
        (CssFlexWrap::WrapReverse, FlexWrap::WrapReverse),
    ] {
        let style = styled(|style| style.flex_wrap = wrap);
        assert_eq!(convert(&style, Mode::Flex, &mut calcs).flex_wrap, expected);
    }
    let style = styled(|style| {
        style.flex_grow = 2.0;
        style.flex_basis = CssSize::Length(CssLength::Percent(0.0));
        style.row_gap = CssLength::Px(4.0);
    });
    let converted = convert(&style, Mode::Flex, &mut calcs);
    assert_eq!(converted.flex_grow, 2.0);
    assert_eq!(converted.flex_basis, Dimension::percent(0.0));
    assert_eq!(converted.gap.height, LengthPercentage::length(4.0));
}

#[test]
fn grid_properties() {
    let mut calcs = CalcStore::default();
    let style = styled(|style| {
        style.grid_template_columns = TrackList(vec![
            TrackEntry::Single(TrackSize::Length(CssLength::Px(100.0))),
            TrackEntry::Repeat(RepeatCount::Count(2), vec![TrackSize::Fr(1.0)]),
            TrackEntry::Repeat(
                RepeatCount::AutoFill,
                vec![TrackSize::MinMax(
                    Box::new(TrackSize::Length(CssLength::Px(50.0))),
                    Box::new(TrackSize::Fr(1.0)),
                )],
            ),
            TrackEntry::Repeat(RepeatCount::AutoFit, vec![TrackSize::Auto]),
            TrackEntry::Single(TrackSize::MinContent),
            TrackEntry::Single(TrackSize::MaxContent),
            TrackEntry::Single(TrackSize::FitContent(CssLength::Px(10.0))),
        ]);
        style.grid_auto_rows = TrackSizes(vec![TrackSize::MinContent]);
        style.grid_row_start = GridLine::Line(1);
        style.grid_row_end = GridLine::Span(2);
    });
    let converted = convert(&style, Mode::Grid, &mut calcs);
    assert_eq!(converted.grid_template_columns.len(), 7);
    assert!(matches!(
        &converted.grid_template_columns[1],
        GridTemplateComponent::Repeat(repetition) if repetition.count == RepetitionCount::Count(2)
    ));
    assert!(matches!(
        &converted.grid_template_columns[2],
        GridTemplateComponent::Repeat(repetition) if repetition.count == RepetitionCount::AutoFill
    ));
    assert!(matches!(
        &converted.grid_template_columns[3],
        GridTemplateComponent::Repeat(repetition) if repetition.count == RepetitionCount::AutoFit
    ));
    assert_eq!(converted.grid_auto_rows.len(), 1);
    assert_eq!(converted.grid_row.start, GridPlacement::from_line_index(1));
    assert_eq!(converted.grid_row.end, GridPlacement::from_span(2));
    assert_eq!(converted.grid_column.start, GridPlacement::Auto);
    for (column, dense, expected) in [
        (false, false, GridAutoFlow::Row),
        (false, true, GridAutoFlow::RowDense),
        (true, false, GridAutoFlow::Column),
        (true, true, GridAutoFlow::ColumnDense),
    ] {
        let style = styled(|style| style.grid_auto_flow = CssFlow { column, dense });
        assert_eq!(
            convert(&style, Mode::Grid, &mut calcs).grid_auto_flow,
            expected
        );
    }
}
