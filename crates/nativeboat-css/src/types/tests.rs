use super::*;
use crate::parsers::Keyword;
use crate::values::Calc;

#[test]
fn easing_progress_and_repeating_lists() {
    for curve in [
        Easing::LINEAR,
        Easing::EASE,
        Easing::EASE_IN,
        Easing::EASE_OUT,
        Easing::EASE_IN_OUT,
    ] {
        assert_eq!(curve.apply(-1.0), 0.0);
        assert_eq!(curve.apply(2.0), 1.0);
        assert!(curve.apply(0.5) > 0.0 && curve.apply(0.5) < 1.0);
        assert!(!curve.to_css().contains("cubic-bezier"));
    }
    assert!((Easing::LINEAR.apply(0.37) - 0.37).abs() < 0.00001);
    assert!((Easing::EASE.apply(0.5) - 0.8024).abs() < 0.0001);
    assert!(Easing::CubicBezier(0.2, 2.0, 0.8, 2.0).apply(0.5) > 1.0);
    for (position, first, middle) in [
        (StepPosition::Start, 0.25, 0.75),
        (StepPosition::End, 0.0, 0.5),
        (StepPosition::None, 0.0, 2.0 / 3.0),
        (StepPosition::Both, 0.2, 0.6),
    ] {
        let curve = Easing::Steps(4, position);
        assert_eq!(curve.apply(0.0), first);
        assert_eq!(curve.apply(0.5), middle);
        assert_eq!(curve.apply(1.0), 1.0);
    }
    assert_eq!(Easing::Steps(0, StepPosition::End).apply(0.5), 0.0);
    let values = List(vec![Seconds(0.2), Seconds(-0.1)]);
    assert_eq!(values.to_css(), "0.2s, -0.1s");
    assert_eq!(values.at(3), Some(&Seconds(-0.1)));
    assert_eq!(List::<Seconds>(vec![]).at(0), None);
}

macro_rules! check_keywords {
    ($($kind:ty),+ $(,)?) => {
        $(
            for value in <$kind>::ALL {
                assert_eq!(<$kind>::from_keyword(value.keyword()), Some(*value));
                assert_eq!(<$kind as Keyword>::parse_keyword(value.keyword()), Some(*value));
                assert_eq!(value.to_css(), value.keyword());
            }
            assert_eq!(<$kind>::from_keyword("bogus-keyword"), None);
            assert!(<$kind>::ALL.contains(&<$kind>::default()));
        )+
    };
}

#[test]
fn keyword_enums_round_trip() {
    check_keywords!(
        Display,
        PositionKind,
        BoxSizing,
        Float,
        FlexDirection,
        FlexWrap,
        Alignment,
        Visibility,
        Overflow,
        ScrollbarWidth,
        BorderStyle,
        FontStyle,
        TextAlign,
        TextTransform,
        WhiteSpaceCollapse,
        TextWrapMode,
        TextWrapStyle,
        TextOverflow,
        WordBreak,
        OverflowWrap,
        TextAnchor,
        SvgBaseline,
        ListStyleType,
        Cursor,
        PointerEvents,
        UserSelect,
        ObjectFit,
        Isolation,
    );
    assert_eq!(Display::from_keyword("-webkit-flex"), Some(Display::Flex));
    assert_eq!(
        Display::from_keyword("-webkit-box"),
        Some(Display::WebkitBox)
    );
    assert_eq!(
        BoxOrient::from_keyword("block-axis"),
        Some(BoxOrient::Vertical)
    );
    assert_eq!(LineClamp(Some(2)).to_css(), "2");
    assert_eq!(LineClamp(None).to_css(), "none");
    assert_eq!(Cursor::from_keyword("se-resize"), Some(Cursor::NwseResize));
}

#[test]
fn vertical_alignments_take_keywords_and_lengths() {
    for keyword in [
        "baseline",
        "top",
        "middle",
        "bottom",
        "text-top",
        "text-bottom",
        "sub",
        "super",
    ] {
        assert_eq!(
            VerticalAlign::from_keyword(keyword).unwrap().to_css(),
            keyword
        );
    }
    assert_eq!(VerticalAlign::from_keyword("bogus-keyword"), None);
    assert_eq!(VerticalAlign::default(), VerticalAlign::Baseline);
    assert_eq!(
        VerticalAlign::Length(LengthPercentage::Px(-4.0)).to_css(),
        "-4px"
    );
}

#[test]
fn display_helpers() {
    let inline = [
        Display::Inline,
        Display::InlineBlock,
        Display::InlineFlex,
        Display::InlineGrid,
        Display::InlineTable,
        Display::WebkitInlineBox,
    ];
    for display in Display::ALL {
        assert_eq!(display.is_inline_level(), inline.contains(display));
    }
    assert_eq!(Display::Inline.blockified(), Display::Block);
    assert_eq!(Display::InlineBlock.blockified(), Display::Block);
    assert_eq!(Display::InlineFlex.blockified(), Display::Flex);
    assert_eq!(Display::InlineGrid.blockified(), Display::Grid);
    assert_eq!(Display::InlineTable.blockified(), Display::Table);
    assert_eq!(Display::Grid.blockified(), Display::Grid);
    assert_eq!(Display::WebkitInlineBox.blockified(), Display::WebkitBox);
}

#[test]
fn keyword_helpers() {
    assert!(Overflow::Auto.is_scroll_container());
    assert!(!Overflow::Visible.is_scroll_container());
    assert!(!Overflow::Clip.is_scroll_container());
    assert!(BorderStyle::Hidden.is_none());
    assert!(!BorderStyle::Solid.is_none());
    assert!(WhiteSpace::PreLine.collapses_spaces());
    assert!(!WhiteSpace::Pre.collapses_spaces());
    assert!(WhiteSpace::PreWrap.keeps_newlines());
    assert!(!WhiteSpace::NoWrap.keeps_newlines());
    assert!(WhiteSpace::Normal.wraps());
    assert!(!WhiteSpace::Pre.wraps());
    assert!(!WhiteSpace::new(WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::NoWrap).wraps());
    for (keyword, value) in WhiteSpace::KEYWORDS {
        assert_eq!(WhiteSpace::from_keyword(keyword), Some(value));
        assert_eq!(value.to_css(), keyword);
    }
    assert_eq!(WhiteSpace::from_keyword("wrap"), None);
    assert_eq!(
        WhiteSpace::new(WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::NoWrap).to_css(),
        "preserve-breaks nowrap"
    );
}

#[test]
fn scalar_serialization() {
    assert_eq!(1.5f32.to_css(), "1.5");
    assert_eq!(7i32.to_css(), "7");
    assert_eq!(ToCss::to_css(&LengthPercentage::Px(2.0)), "2px");
    assert_eq!(ToCss::to_css(&Color::BLACK), "rgb(0, 0, 0)");
    assert_eq!(Px(3.25).to_css(), "3.25px");
    assert_eq!(ZIndex(None).to_css(), "auto");
    assert_eq!(ZIndex(Some(-2)).to_css(), "-2");
    assert_eq!(FontWeight::default().to_css(), "400");
    assert_eq!(
        FontFamily(vec!["Segoe UI".to_owned(), "serif".to_owned()]).to_css(),
        "\"Segoe UI\", serif"
    );
    assert_eq!(FontFamily::default().0, vec!["Times New Roman".to_owned()]);
    assert_eq!(Edges::all(1).left, 1);
}

#[test]
fn size_serialization() {
    assert_eq!(Size::default().to_css(), "auto");
    assert_eq!(Size::Length(LengthPercentage::Percent(0.5)).to_css(), "50%");
    assert_eq!(Size::MinContent.to_css(), "min-content");
    assert_eq!(Size::MaxContent.to_css(), "max-content");
    assert_eq!(Size::FitContent.to_css(), "fit-content");
    assert_eq!(MaxSize::default().to_css(), "none");
    assert_eq!(MaxSize::Length(LengthPercentage::Px(5.0)).to_css(), "5px");
    assert_eq!(MaxSize::MinContent.to_css(), "min-content");
    assert_eq!(MaxSize::MaxContent.to_css(), "max-content");
    assert_eq!(MaxSize::FitContent.to_css(), "fit-content");
    assert_eq!(LengthPercentageAuto::default().to_css(), "auto");
    assert_eq!(LengthPercentageAuto::zero().to_css(), "0px");
    let round = Radius {
        horizontal: LengthPercentage::Px(2.0),
        vertical: LengthPercentage::Px(2.0),
    };
    assert_eq!(round.to_css(), "2px");
    let oval = Radius {
        horizontal: LengthPercentage::Px(2.0),
        vertical: LengthPercentage::Percent(0.1),
    };
    assert_eq!(oval.to_css(), "2px 10%");
}

#[test]
fn text_values() {
    assert_eq!(LineHeight::Normal.resolve(10.0), 12.0);
    assert_eq!(LineHeight::Number(1.5).resolve(10.0), 15.0);
    assert_eq!(LineHeight::Px(20.0).resolve(10.0), 20.0);
    assert_eq!(LineHeight::Normal.to_css(), "normal");
    assert_eq!(LineHeight::Number(1.5).to_css(), "1.5");
    assert_eq!(LineHeight::Px(20.0).to_css(), "20px");
    let none = TextDecorationLine::default();
    assert!(none.is_none());
    assert_eq!(none.to_css(), "none");
    let all = TextDecorationLine {
        underline: true,
        overline: true,
        line_through: true,
    };
    assert!(!all.is_none());
    assert_eq!(all.to_css(), "underline overline line-through");
    assert_eq!(OptionalColor(None).to_css(), "currentcolor");
    assert_eq!(
        OptionalColor(Some(Color::WHITE)).to_css(),
        "rgb(255, 255, 255)"
    );
}

#[test]
fn image_shadow_and_transform_serialization() {
    let stops = vec![
        ColorStop {
            color: Color::BLACK,
            position: None,
        },
        ColorStop {
            color: Color::WHITE,
            position: Some(0.5),
        },
    ];
    assert_eq!(BackgroundImage::None.to_css(), "none");
    assert_eq!(
        BackgroundImage::Url("a.png".to_owned()).to_css(),
        "url(\"a.png\")"
    );
    assert_eq!(
        BackgroundImage::LinearGradient {
            angle: 90.0,
            stops: stops.clone()
        }
        .to_css(),
        "linear-gradient(90deg, rgb(0, 0, 0), rgb(255, 255, 255) 50%)"
    );
    assert_eq!(
        BackgroundImage::RadialGradient { stops }.to_css(),
        "radial-gradient(rgb(0, 0, 0), rgb(255, 255, 255) 50%)"
    );
    assert_eq!(Shadows::default().to_css(), "none");
    let shadows = Shadows(vec![
        Shadow {
            x: 1.0,
            y: 2.0,
            blur: 3.0,
            spread: 0.0,
            color: Color::BLACK,
            inset: false,
        },
        Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: Color::WHITE,
            inset: true,
        },
    ]);
    assert_eq!(
        shadows.to_css(),
        "rgb(0, 0, 0) 1px 2px 3px 0px, rgb(255, 255, 255) 0px 0px 0px 1px inset"
    );
    assert!(Transform::default().is_identity());
    assert_eq!(Filters::default().to_css(), "none");
    let filters = [
        Filter::Blur(2.0),
        Filter::Brightness(0.8),
        Filter::Contrast(1.5),
        Filter::Grayscale(0.5),
        Filter::HueRotate(90.0),
        Filter::Invert(1.0),
        Filter::Opacity(0.25),
        Filter::Saturate(2.0),
        Filter::Sepia(0.1),
        Filter::DropShadow {
            x: 16.0,
            y: 0.0,
            blur: 3.0,
            color: Color::WHITE,
        },
    ];
    assert_eq!(
        Filters(filters.to_vec()).to_css(),
        "blur(2px) brightness(0.8) contrast(1.5) grayscale(0.5) hue-rotate(90deg) invert(1) \
         opacity(0.25) saturate(2) sepia(0.1) drop-shadow(rgb(255, 255, 255) 16px 0px 3px)"
    );
    let identities: Vec<String> = filters
        .iter()
        .map(|filter| filter.identity().to_css())
        .collect();
    assert_eq!(
        identities.join(" "),
        "blur(0px) brightness(1) contrast(1) grayscale(0) hue-rotate(0deg) invert(0) \
         opacity(1) saturate(1) sepia(0) drop-shadow(rgba(0, 0, 0, 0) 0px 0px 0px)"
    );
    assert_eq!(Transform::default().to_css(), "none");
    // Browsers report the matrix of transforms that do not depend on the
    // box's size.
    let moved = Transform(vec![
        TransformFunction::Translate(LengthPercentage::Px(1.0), LengthPercentage::Px(0.0)),
        TransformFunction::Rotate(45.0),
    ]);
    assert_eq!(
        moved.to_css(),
        "matrix(0.707107, 0.707107, -0.707107, 0.707107, 1, 0)"
    );
    let centered = Transform(vec![
        TransformFunction::Translate(LengthPercentage::Percent(-0.5), LengthPercentage::Px(0.0)),
        TransformFunction::Skew(10.0, 0.0),
    ]);
    assert_eq!(centered.to_css(), "translate(-50%, 0px) skew(10deg, 0deg)");
    // The order of functions matters: a translation after a rotation
    // moves along the rotated axis.
    let turned = Transform(vec![
        TransformFunction::Rotate(90.0),
        TransformFunction::Translate(LengthPercentage::Percent(-0.5), LengthPercentage::Px(0.0)),
    ])
    .matrix(20.0, 10.0);
    assert!(
        (turned[4]).abs() < 1e-5 && (turned[5] + 10.0).abs() < 1e-5,
        "{turned:?}"
    );
}

#[test]
fn unmatched_transforms_interpolate_by_decomposed_matrices() {
    let close = |a: [f32; 6], b: [f32; 6]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
    let rotation = |degrees: f32| TransformFunction::Rotate(degrees).matrix(0.0, 0.0);
    // Rotations take the short way around.
    let middle = interpolate_matrices(rotation(10.0), rotation(350.0), 0.5);
    assert!(close(middle, IDENTITY), "{middle:?}");
    let quarter = interpolate_matrices(IDENTITY, rotation(90.0), 0.5);
    assert!(close(quarter, rotation(45.0)), "{quarter:?}");
    // Scales, skews and translations mix.
    let from = [2.0, 0.0, 0.0, 2.0, 10.0, 0.0];
    let to = [4.0, 0.0, 0.0, 4.0, 20.0, 4.0];
    assert!(close(
        interpolate_matrices(from, to, 0.5),
        [3.0, 0.0, 0.0, 3.0, 15.0, 2.0]
    ));
    let skew = TransformFunction::Skew(30.0, 0.0).matrix(0.0, 0.0);
    assert!(close(interpolate_matrices(skew, skew, 0.3), skew));
    // A flip in each axis is a half turn.
    let flipped = interpolate_matrices(
        [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        1.0,
    );
    assert!(
        close(flipped, [1.0, 0.0, 0.0, -1.0, 0.0, 0.0]),
        "{flipped:?}"
    );
    // A mix resolves once the box's size is known.
    let mix = TransformFunction::Mix(
        Box::new(Transform(vec![TransformFunction::Translate(
            LengthPercentage::Percent(1.0),
            LengthPercentage::Px(0.0),
        )])),
        Box::new(Transform(vec![TransformFunction::Rotate(90.0)])),
        0.0,
    );
    assert!(close(
        mix.matrix(30.0, 0.0),
        [1.0, 0.0, 0.0, 1.0, 30.0, 0.0]
    ));
}

#[test]
fn misc_serialization() {
    assert_eq!(AspectRatio(None).to_css(), "auto");
    assert_eq!(AspectRatio(Some(1.5)).to_css(), "1.5 / 1");
    assert_eq!(Content::Normal.to_css(), "normal");
    assert_eq!(Content::None.to_css(), "none");
    assert_eq!(
        Content::Items(vec![
            ContentItem::Text("a".to_owned()),
            ContentItem::Attr("title".to_owned())
        ])
        .to_css(),
        "\"a\" attr(title)"
    );
}

#[test]
fn grid_serialization() {
    let tracks = TrackList(vec![
        TrackEntry::Single(TrackSize::Length(LengthPercentage::Px(10.0))),
        TrackEntry::Repeat(
            RepeatCount::Count(2),
            vec![TrackSize::Fr(1.0), TrackSize::Auto],
        ),
        TrackEntry::Repeat(RepeatCount::AutoFill, vec![TrackSize::MinContent]),
        TrackEntry::Repeat(RepeatCount::AutoFit, vec![TrackSize::MaxContent]),
        TrackEntry::Single(TrackSize::MinMax(
            Box::new(TrackSize::Length(LengthPercentage::Px(100.0))),
            Box::new(TrackSize::Fr(1.0)),
        )),
        TrackEntry::Single(TrackSize::FitContent(LengthPercentage::Percent(0.5))),
    ]);
    assert_eq!(
        tracks.to_css(),
        "10px repeat(2, 1fr auto) repeat(auto-fill, min-content) repeat(auto-fit, max-content) minmax(100px, 1fr) fit-content(50%)"
    );
    assert_eq!(TrackList::default().to_css(), "none");
    assert_eq!(TrackSizes::default().to_css(), "auto");
    assert_eq!(GridLine::Auto.to_css(), "auto");
    assert_eq!(GridLine::Line(-1).to_css(), "-1");
    assert_eq!(GridLine::Span(2).to_css(), "span 2");
    assert_eq!(GridAutoFlow::default().to_css(), "row");
    assert_eq!(
        GridAutoFlow {
            column: true,
            dense: true
        }
        .to_css(),
        "column dense"
    );
    let _ = Calc::Px(0.0);
}
