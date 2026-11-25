use super::*;

#[test]
fn the_animated_properties_are_the_style_engines_animatable_ones() {
    let ids: Vec<PropertyId> = PROPERTIES.iter().map(|property| property.id).collect();
    assert_eq!(ids, ComputedStyle::ANIMATABLE);
}

fn px(value: f32) -> Length {
    Length::Px(value)
}
fn shadow(inset: bool) -> Shadow {
    Shadow {
        x: 2.0,
        y: 4.0,
        blur: 6.0,
        spread: 8.0,
        color: Color::BLACK,
        inset,
    }
}

fn endpoints(value: Value) -> (Value, Value) {
    match value {
        Value::Color(_) => (Value::Color(Color::BLACK), Value::Color(Color::WHITE)),
        Value::Number(_) => (Value::Number(0.2), Value::Number(0.8)),
        Value::Pixels(_) => (Value::Pixels(Px(2.0)), Value::Pixels(Px(10.0))),
        Value::Length(_) => (Value::Length(px(2.0)), Value::Length(px(10.0))),
        Value::Auto(_) => (
            Value::Auto(LengthPercentageAuto::Length(px(2.0))),
            Value::Auto(LengthPercentageAuto::Length(px(10.0))),
        ),
        Value::Size(_) => (
            Value::Size(Size::Length(px(2.0))),
            Value::Size(Size::Length(px(10.0))),
        ),
        Value::MaxSize(_) => (
            Value::MaxSize(MaxSize::Length(px(2.0))),
            Value::MaxSize(MaxSize::Length(px(10.0))),
        ),
        Value::Radius(_) => (
            Value::Radius(Radius {
                horizontal: px(2.0),
                vertical: px(4.0),
            }),
            Value::Radius(Radius {
                horizontal: px(10.0),
                vertical: px(12.0),
            }),
        ),
        Value::Shadows(_) => (
            Value::Shadows(Shadows(vec![])),
            Value::Shadows(Shadows(vec![shadow(false)])),
        ),
        Value::Filters(_) => (
            Value::Filters(Filters(vec![])),
            Value::Filters(Filters(vec![Filter::Blur(4.0)])),
        ),
        Value::Transform(_) => (
            Value::Transform(Transform::default()),
            Value::Transform(Transform(vec![
                TransformFunction::Translate(px(20.0), Length::Percent(0.5)),
                TransformFunction::Scale(2.0, 3.0),
                TransformFunction::Rotate(90.0),
            ])),
        ),
        Value::OptionalColor(_) => (
            Value::OptionalColor(OptionalColor(Some(Color::BLACK))),
            Value::OptionalColor(OptionalColor(Some(Color::WHITE))),
        ),
        Value::LineHeight(_) => (
            Value::LineHeight(LineHeight::Number(1.0)),
            Value::LineHeight(LineHeight::Number(2.0)),
        ),
        Value::Weight(_) => (
            Value::Weight(FontWeight(100)),
            Value::Weight(FontWeight(900)),
        ),
        Value::Visibility(_) => (
            Value::Visibility(Visibility::Visible),
            Value::Visibility(Visibility::Hidden),
        ),
        Value::Paint(_) => (
            Value::Paint(Paint::Color(Color::BLACK)),
            Value::Paint(Paint::Color(Color::WHITE)),
        ),
        Value::ZIndex(_) => (
            Value::ZIndex(ZIndex(Some(1))),
            Value::ZIndex(ZIndex(Some(4))),
        ),
        Value::Integer(_) => (Value::Integer(1), Value::Integer(5)),
        Value::BackgroundSize(_) => (
            Value::BackgroundSize(BackgroundSize::Explicit(
                LengthPercentageAuto::Length(px(2.0)),
                LengthPercentageAuto::Auto,
            )),
            Value::BackgroundSize(BackgroundSize::Explicit(
                LengthPercentageAuto::Length(px(10.0)),
                LengthPercentageAuto::Auto,
            )),
        ),
        Value::Position(_) => (
            Value::Position(BackgroundPosition {
                x: px(2.0),
                y: Length::Percent(0.0),
            }),
            Value::Position(BackgroundPosition {
                x: px(10.0),
                y: Length::Percent(1.0),
            }),
        ),
        Value::VerticalAlign(_) => (
            Value::VerticalAlign(VerticalAlign::Length(px(2.0))),
            Value::VerticalAlign(VerticalAlign::Length(px(10.0))),
        ),
        Value::Spacing(_) => (
            Value::Spacing(Spacing {
                horizontal: Px(2.0),
                vertical: Px(4.0),
            }),
            Value::Spacing(Spacing {
                horizontal: Px(10.0),
                vertical: Px(12.0),
            }),
        ),
        Value::TabSize(_) => (
            Value::TabSize(TabSize::Spaces(2.0)),
            Value::TabSize(TabSize::Spaces(8.0)),
        ),
        Value::ScrollbarColor(_) => (
            Value::ScrollbarColor(ScrollbarColor(Some((Color::BLACK, Color::WHITE)))),
            Value::ScrollbarColor(ScrollbarColor(Some((Color::WHITE, Color::BLACK)))),
        ),
        Value::AspectRatio(_) => (
            Value::AspectRatio(AspectRatio(Some(1.0))),
            Value::AspectRatio(AspectRatio(Some(4.0))),
        ),
        Value::Origin(_) => (
            Value::Origin(TransformOrigin::default()),
            Value::Origin(TransformOrigin {
                x: px(10.0),
                y: px(20.0),
                z: Px(0.0),
            }),
        ),
        Value::Clip(_) => (
            Value::Clip(Some(ClipRect {
                top: Some(0.0),
                right: Some(10.0),
                bottom: Some(10.0),
                left: None,
            })),
            Value::Clip(Some(ClipRect {
                top: Some(4.0),
                right: Some(20.0),
                bottom: Some(30.0),
                left: None,
            })),
        ),
        Value::ClipPath(_) => (
            Value::ClipPath(ClipPath::Shape(
                BasicShape::Inset([px(0.0), px(0.0), px(0.0), px(0.0)], Default::default()),
                GeometryBox::BorderBox,
            )),
            Value::ClipPath(ClipPath::Shape(
                BasicShape::Inset([px(10.0), px(0.0), px(20.0), px(0.0)], Default::default()),
                GeometryBox::BorderBox,
            )),
        ),
        Value::Image(_) => (
            Value::Image(BackgroundImage::Url("a.png".to_owned())),
            Value::Image(BackgroundImage::LinearGradient {
                angle: 180.0,
                stops: vec![],
            }),
        ),
        Value::Tracks(_) => (
            Value::Tracks(TrackList(vec![
                TrackEntry::Single(TrackSize::Length(px(100.0))),
                TrackEntry::Single(TrackSize::Fr(1.0)),
            ])),
            Value::Tracks(TrackList(vec![
                TrackEntry::Single(TrackSize::Length(px(200.0))),
                TrackEntry::Single(TrackSize::Fr(3.0)),
            ])),
        ),
    }
}

#[test]
fn every_animation_property_reads_interpolates_and_writes() {
    for property in PROPERTIES {
        let mut style = ComputedStyle::default();
        let (from, to) = endpoints((property.read)(&style));
        let middle = from.interpolate(&to, 0.5).unwrap();
        (property.write)(&mut style, middle.clone());
        assert_eq!((property.read)(&style), middle, "{}", property.id.name());
        if property.id != PropertyId::Visibility {
            assert_ne!(middle, from, "{}", property.id.name());
            assert_ne!(middle, to, "{}", property.id.name());
        }
    }
}

#[test]
fn transforms_interpolate_function_by_function_or_by_matrix() {
    use TransformFunction as F;
    let list = |functions: Vec<F>| Transform(functions);
    // A spinner turns all the way round: rotations interpolate as angles.
    assert_eq!(
        transform(
            &list(vec![F::Rotate(0.0)]),
            &list(vec![F::Rotate(360.0)]),
            0.5
        ),
        list(vec![F::Rotate(180.0)])
    );
    // From `none`, and to a longer list, missing functions are identities.
    assert_eq!(
        transform(
            &Transform::default(),
            &list(vec![F::Scale(3.0, 3.0), F::Skew(10.0, 0.0)]),
            0.5
        ),
        list(vec![F::Scale(2.0, 2.0), F::Skew(5.0, 0.0)])
    );
    assert_eq!(
        transform(
            &list(vec![F::Rotate(-45.0), F::Scale(0.0, 0.0)]),
            &list(vec![F::Rotate(-45.0)]),
            0.5
        ),
        list(vec![F::Rotate(-45.0), F::Scale(0.5, 0.5)])
    );
    // Functions that do not pair up interpolate by matrix.
    let mixed = transform(
        &list(vec![F::Rotate(90.0)]),
        &list(vec![F::Scale(2.0, 2.0)]),
        0.25,
    );
    assert!(matches!(mixed.0.as_slice(), [F::Mix(_, _, progress)] if *progress == 0.25));
}

#[test]
fn lengths_preserve_percentages_and_calc_terms() {
    assert_eq!(
        length(&Length::Percent(0.2), &Length::Percent(0.8), 0.5),
        Length::Percent(0.5)
    );
    let mixed = length(&px(20.0), &Length::Percent(0.5), 0.5);
    assert_eq!(mixed.resolve(100.0), 35.0);
    assert_eq!(mixed.resolve(200.0), 60.0);
    let calculated = Length::Calc(Box::new(Calc::Sum(vec![
        Calc::Px(10.0),
        Calc::Percent(0.1),
    ])));
    assert_eq!(length(&calculated, &mixed, 0.5).resolve(100.0), 27.5);
}

#[test]
fn colors_use_premultiplied_alpha_and_clip_overshoots() {
    assert_eq!(
        color(Color::rgba(255, 0, 0, 0.0), Color::rgb(0, 0, 255), 0.5),
        Color::rgba(0, 0, 255, 0.5)
    );
    assert_eq!(
        color(Color::TRANSPARENT, Color::TRANSPARENT, 0.5),
        Color::TRANSPARENT
    );
    assert_eq!(color(Color::BLACK, Color::WHITE, 2.0), Color::WHITE);
}

#[test]
fn shadow_lists_pad_with_transparent_shadows_and_reject_inset_changes() {
    let empty = Shadows(vec![]);
    let end = Shadows(vec![shadow(true), shadow(false)]);
    let middle = shadows(&empty, &end, 0.5).unwrap();
    assert_eq!(middle.0.len(), 2);
    assert_eq!(
        (
            middle.0[0].x,
            middle.0[0].blur,
            middle.0[0].color.a,
            middle.0[0].inset
        ),
        (1.0, 3.0, 0.5, true)
    );
    assert_eq!(shadows(&end, &empty, 0.5), Some(middle));
    assert!(
        shadows(
            &Shadows(vec![shadow(false)]),
            &Shadows(vec![shadow(true)]),
            0.5
        )
        .is_none()
    );
}

#[test]
fn incompatible_values_do_not_transition() {
    assert!(
        Value::Size(Size::Auto)
            .interpolate(&Value::Size(Size::Length(px(1.0))), 0.5)
            .is_none()
    );
    assert!(
        Value::Auto(LengthPercentageAuto::Auto)
            .interpolate(&Value::Auto(LengthPercentageAuto::Length(px(1.0))), 0.5)
            .is_none()
    );
    assert!(
        Value::Visibility(Visibility::Hidden)
            .interpolate(&Value::Visibility(Visibility::Collapse), 0.5)
            .is_none()
    );
    let from = Value::Visibility(Visibility::Hidden);
    let to = Value::Visibility(Visibility::Visible);
    assert_eq!(from.interpolate(&to, 0.0), Some(from.clone()));
    assert_eq!(from.interpolate(&to, 0.1), Some(to.clone()));
    assert_eq!(to.interpolate(&from, 1.0), Some(from));
    assert_eq!(
        Value::LineHeight(LineHeight::Px(10.0))
            .interpolate(&Value::LineHeight(LineHeight::Px(20.0)), 0.5),
        Some(Value::LineHeight(LineHeight::Px(15.0)))
    );
}

#[test]
fn filter_lists_interpolate_matching_functions_and_pad_with_identities() {
    let shadow = |x: f32, blur: f32, color: Color| Filter::DropShadow {
        x,
        y: x,
        blur,
        color,
    };
    let from = Filters(vec![
        Filter::Blur(2.0),
        Filter::Brightness(1.0),
        Filter::Contrast(1.0),
        Filter::Saturate(1.0),
        Filter::Grayscale(0.0),
        Filter::Invert(0.0),
        Filter::Opacity(1.0),
        Filter::Sepia(0.0),
        Filter::HueRotate(0.0),
        shadow(0.0, 0.0, Color::TRANSPARENT),
    ]);
    let to = Filters(vec![
        Filter::Blur(4.0),
        Filter::Brightness(0.0),
        Filter::Contrast(2.0),
        Filter::Saturate(3.0),
        Filter::Grayscale(1.0),
        Filter::Invert(1.0),
        Filter::Opacity(0.0),
        Filter::Sepia(1.0),
        Filter::HueRotate(90.0),
        shadow(4.0, 2.0, Color::BLACK),
    ]);
    assert_eq!(
        filters(&from, &to, 0.5),
        Some(Filters(vec![
            Filter::Blur(3.0),
            Filter::Brightness(0.5),
            Filter::Contrast(1.5),
            Filter::Saturate(2.0),
            Filter::Grayscale(0.5),
            Filter::Invert(0.5),
            Filter::Opacity(0.5),
            Filter::Sepia(0.5),
            Filter::HueRotate(45.0),
            shadow(2.0, 1.0, Color::rgba(0, 0, 0, 0.5)),
        ]))
    );
    // Overshooting easing keeps amounts in range.
    let overshoot = filters(&from, &to, 1.5).unwrap();
    assert_eq!(overshoot.0[1], Filter::Brightness(0.0));
    assert_eq!(overshoot.0[4], Filter::Grayscale(1.0));
    // `none` and shorter lists continue with functions that change nothing.
    let none = Filters(vec![]);
    let short = Filters(vec![Filter::Blur(4.0)]);
    let long = Filters(vec![Filter::Blur(2.0), Filter::Invert(1.0)]);
    assert_eq!(
        filters(&none, &short, 0.25),
        Some(Filters(vec![Filter::Blur(1.0)]))
    );
    assert_eq!(
        filters(&long, &short, 0.5),
        Some(Filters(vec![Filter::Blur(3.0), Filter::Invert(0.5)]))
    );
    // Lists whose functions differ do not transition.
    let other = Filters(vec![Filter::Sepia(1.0)]);
    assert_eq!(filters(&short, &other, 0.5), None);
    assert!(
        Value::Filters(short)
            .interpolate(&Value::Filters(other), 0.5)
            .is_none()
    );
}

#[test]
fn ratios_shapes_and_grid_templates_interpolate_like_chromium() {
    // Ratios by their logarithms: halfway from 1/1 to 4/1 is 2/1.
    let ratio = Value::AspectRatio(AspectRatio(Some(1.0)))
        .interpolate(&Value::AspectRatio(AspectRatio(Some(4.0))), 0.5);
    let Some(Value::AspectRatio(AspectRatio(Some(ratio)))) = ratio else {
        panic!("{ratio:?}");
    };
    assert!((ratio - 2.0).abs() < 1e-4, "{ratio}");
    assert_eq!(
        Value::Integer(1).interpolate(&Value::Integer(2), 0.4),
        Some(Value::Integer(1))
    );
    // Different shapes, track counts and keywords change discretely.
    let circle = Value::ClipPath(ClipPath::Shape(
        BasicShape::Circle(ShapeRadius::ClosestSide, BackgroundPosition::default()),
        GeometryBox::BorderBox,
    ));
    let inset = Value::ClipPath(ClipPath::Shape(
        BasicShape::Inset([px(0.0), px(0.0), px(0.0), px(0.0)], Default::default()),
        GeometryBox::BorderBox,
    ));
    assert!(circle.interpolate(&inset, 0.5).is_none());
    assert_eq!(circle.interpolate(&circle, 0.5), Some(circle.clone()));
    let one = Value::Tracks(TrackList(vec![TrackEntry::Single(TrackSize::Fr(1.0))]));
    let two = Value::Tracks(TrackList(vec![
        TrackEntry::Single(TrackSize::Fr(1.0)),
        TrackEntry::Single(TrackSize::Fr(1.0)),
    ]));
    assert!(one.interpolate(&two, 0.5).is_none());
    let auto = Value::Tracks(TrackList(vec![TrackEntry::Single(TrackSize::Auto)]));
    assert!(one.interpolate(&auto, 0.5).is_none());
    assert!(
        Value::BackgroundSize(BackgroundSize::Cover)
            .interpolate(&Value::BackgroundSize(BackgroundSize::Contain), 0.5)
            .is_none()
    );
    assert!(
        Value::Clip(None)
            .interpolate(&Value::Clip(Some(ClipRect::default())), 0.5)
            .is_none()
    );
}

#[test]
fn images_cross_fade_and_none_changes_at_once() {
    let a = BackgroundImage::Url("a.png".to_owned());
    let b = BackgroundImage::Url("b.png".to_owned());
    assert_eq!(
        Value::Image(a.clone()).interpolate(&Value::Image(b.clone()), 0.25),
        Some(Value::Image(BackgroundImage::CrossFade {
            from: Box::new(a.clone()),
            to: Box::new(b.clone()),
            progress: 0.25,
        }))
    );
    assert_eq!(
        Value::Image(a.clone()).interpolate(&Value::Image(b.clone()), 1.0),
        Some(Value::Image(b))
    );
    assert!(
        Value::Image(a)
            .interpolate(&Value::Image(BackgroundImage::None), 0.5)
            .is_none()
    );
}
