use super::*;
use crate::values::parse_component_values;

const SYSTEM: SystemColors = SystemColors::LIGHT;

#[test]
fn scrollbar_colors_have_exactly_two_colors_or_auto() {
    assert_eq!(run(scrollbar_color, "auto"), Some(ScrollbarColor(None)));
    assert_eq!(ScrollbarColor(None).to_css(), "auto");
    assert_eq!(
        run(scrollbar_color, "red rgb(0, 0, 255)"),
        Some(ScrollbarColor(Some((
            Color::rgb(255, 0, 0),
            Color::rgb(0, 0, 255)
        ))))
    );
    for text in [
        "",
        "red",
        "auto red",
        "red auto",
        "red blue green",
        "red,blue",
        "1px blue",
    ] {
        assert_eq!(run(scrollbar_color, text), None, "{text}");
    }
}

#[test]
fn transition_times_and_properties() {
    assert_eq!(
        run(durations, "200ms, 0s, 2S"),
        Some(List(vec![Seconds(0.2), Seconds(0.0), Seconds(2.0)]))
    );
    assert_eq!(
        run(delays, "-100MS, 1s"),
        Some(List(vec![Seconds(-0.1), Seconds(1.0)]))
    );
    assert_eq!(
        run(transition_properties, "ALL, background-color, --Custom"),
        Some(List(vec![
            Ident("all".into()),
            Ident("background-color".into()),
            Ident("--Custom".into())
        ]))
    );
    assert_eq!(
        run(transition_properties, "NONE"),
        Some(List(vec![Ident("none".into())]))
    );
    for invalid in ["", "0", "2px", "-1s", "1s 2s", "1s,", ",1s"] {
        assert!(run(durations, invalid).is_none(), "{invalid}");
    }
    for invalid in ["", "0", "1px", "1s 2s"] {
        assert!(run(delays, invalid).is_none(), "{invalid}");
    }
    for invalid in [
        "",
        "none, color",
        "color, NONE",
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
        "1px",
        "all color",
    ] {
        assert!(run(transition_properties, invalid).is_none(), "{invalid}");
    }
}

#[test]
fn transition_easing_lists() {
    let named = [
        Easing::LINEAR,
        Easing::EASE,
        Easing::EASE_IN,
        Easing::EASE_OUT,
        Easing::EASE_IN_OUT,
    ];
    assert_eq!(
        run(easings, "linear, EASE, ease-in, ease-out, ease-in-out"),
        Some(List(named.to_vec()))
    );
    for (source, expected) in [
        ("step-start", Easing::Steps(1, StepPosition::Start)),
        ("step-end", Easing::Steps(1, StepPosition::End)),
        ("steps(4)", Easing::Steps(4, StepPosition::End)),
        ("steps(4, start)", Easing::Steps(4, StepPosition::Start)),
        ("steps(4, end)", Easing::Steps(4, StepPosition::End)),
        (
            "steps(4, jump-start)",
            Easing::Steps(4, StepPosition::Start),
        ),
        ("steps(4, jump-end)", Easing::Steps(4, StepPosition::End)),
        ("steps(4, jump-none)", Easing::Steps(4, StepPosition::None)),
        ("steps(4, jump-both)", Easing::Steps(4, StepPosition::Both)),
        (
            "cubic-bezier(.1, -2, .9, 3)",
            Easing::CubicBezier(0.1, -2.0, 0.9, 3.0),
        ),
    ] {
        assert_eq!(run(easings, source), Some(List(vec![expected])), "{source}");
        assert_eq!(run(easings, &expected.to_css()), Some(List(vec![expected])));
    }
    for invalid in [
        "",
        "1s",
        "bogus",
        "ease linear",
        "ease,",
        "spring(1)",
        "cubic-bezier(0, 1, 2)",
        "cubic-bezier(-.1, 0, 1, 1)",
        "cubic-bezier(0, 0, 1.1, 1)",
        "cubic-bezier(a, 0, 1, 1)",
        "steps(0)",
        "steps(1.5)",
        "steps(1, jump-none)",
        "steps(2, bogus)",
        "steps(2, 1)",
        "steps(2, start, end)",
    ] {
        assert!(run(easings, invalid).is_none(), "{invalid}");
    }
}

#[test]
fn rectangular_clipping() {
    assert_eq!(run(clip, "auto"), Some(None));
    let rect = ClipRect {
        top: Some(0.0),
        right: None,
        bottom: Some(20.0),
        left: Some(-10.0),
    };
    assert_eq!(run(clip, "rect(0, auto, 2em, -10px)"), Some(Some(rect)));
    assert_eq!(run(clip, "rect(0 auto 2em -10px)"), Some(Some(rect)));
    assert_eq!(Some(rect).to_css(), "rect(0px, auto, 20px, -10px)");
    assert_eq!(Option::<ClipRect>::None.to_css(), "auto");
    for invalid in [
        "",
        "auto auto",
        "none",
        "1px",
        "circle(1px)",
        "rect(0 0 0)",
        "rect(0 0 0 10%)",
        "rect(0 0 0 nope)",
    ] {
        assert_eq!(run(clip, invalid), None, "{invalid}");
    }
}

fn context() -> Context<'static> {
    Context {
        length: LengthContext {
            font_size: 10.0,
            ..LengthContext::default()
        },
        parent_font_size: 20.0,
        parent_font_weight: 400,
        color: Color::rgb(1, 2, 3),
        system: &SYSTEM,
    }
}

fn run<T>(parser: fn(&[ComponentValue], &Context) -> Option<T>, text: &str) -> Option<T> {
    parser(&parse_component_values(text), &context())
}

fn px(value: f32) -> LengthPercentage {
    LengthPercentage::Px(value)
}

#[test]
fn single_values() {
    assert!(single(&parse_component_values(" a ")).is_some());
    assert!(single(&parse_component_values("a b")).is_none());
    assert!(single(&parse_component_values("  ")).is_none());
}

#[test]
fn keywords_and_display() {
    assert_eq!(
        run(keyword::<Visibility>, "HIDDEN"),
        Some(Visibility::Hidden)
    );
    assert_eq!(run(keyword::<Visibility>, "1px"), None);
    assert_eq!(run(display, "flex"), Some(Display::Flex));
    let pairs = [
        ("inline flow", Display::Inline),
        ("block flow", Display::Block),
        ("inline flow-root", Display::InlineBlock),
        ("block flow-root", Display::FlowRoot),
        ("inline flex", Display::InlineFlex),
        ("block flex", Display::Flex),
        ("inline grid", Display::InlineGrid),
        ("block grid", Display::Grid),
        ("inline table", Display::InlineTable),
        ("block table", Display::Table),
    ];
    for (text, expected) in pairs {
        assert_eq!(run(display, text), Some(expected), "{text}");
    }
    for invalid in ["run-in flex", "block ruby", "a b c", "1"] {
        assert_eq!(run(display, invalid), None, "{invalid}");
    }
}

#[test]
fn alignment_values() {
    assert_eq!(run(alignment, "center"), Some(Alignment::Center));
    assert_eq!(run(alignment, "safe flex-end"), Some(Alignment::FlexEnd));
    assert_eq!(run(alignment, "first baseline"), Some(Alignment::Baseline));
    assert_eq!(run(alignment, "legacy left"), Some(Alignment::Start));
    assert_eq!(run(alignment, "center end"), None);
    assert_eq!(run(alignment, "1px"), None);
}

#[test]
fn lengths_sizes_and_auto() {
    assert_eq!(run(length, "-5px"), Some(px(-5.0)));
    assert_eq!(run(padding, "-5px"), None);
    assert_eq!(run(padding, "2em"), Some(px(20.0)));
    assert_eq!(run(length_auto, "auto"), Some(LengthPercentageAuto::Auto));
    assert_eq!(
        run(length_auto, "10%"),
        Some(LengthPercentageAuto::Length(LengthPercentage::Percent(0.1)))
    );
    assert_eq!(run(length_auto, "x"), None);
    assert_eq!(run(size, "auto"), Some(Size::Auto));
    assert_eq!(run(size, "-webkit-fill-available"), Some(Size::Auto));
    assert_eq!(run(size, "min-content"), Some(Size::MinContent));
    assert_eq!(run(size, "max-content"), Some(Size::MaxContent));
    assert_eq!(run(size, "fit-content"), Some(Size::FitContent));
    assert_eq!(run(size, "5px"), Some(Size::Length(px(5.0))));
    assert_eq!(run(size, "wide"), None);
    assert_eq!(run(size, "-1px"), None);
    assert_eq!(run(max_size, "none"), Some(MaxSize::None));
    assert_eq!(run(max_size, "auto"), None);
    assert_eq!(run(max_size, "5px"), Some(MaxSize::Length(px(5.0))));
    assert_eq!(run(max_size, "min-content"), Some(MaxSize::MinContent));
    assert_eq!(run(max_size, "max-content"), Some(MaxSize::MaxContent));
    assert_eq!(run(max_size, "fit-content"), Some(MaxSize::FitContent));
    assert_eq!(run(max_size, ""), None);
}

#[test]
fn colors() {
    assert_eq!(run(color, "currentcolor"), Some(Color::rgb(1, 2, 3)));
    assert_eq!(run(color, "red"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(run(color, "1px"), None);
    assert_eq!(run(optional_color, "auto"), Some(OptionalColor(None)));
    assert_eq!(
        run(optional_color, "currentColor"),
        Some(OptionalColor(None))
    );
    assert_eq!(
        run(optional_color, "white"),
        Some(OptionalColor(Some(Color::WHITE)))
    );
    assert_eq!(run(optional_color, "1px"), None);
}

#[test]
fn widths_and_spacing() {
    assert_eq!(run(border_width, "thin"), Some(Px(1.0)));
    assert_eq!(run(border_width, "medium"), Some(Px(3.0)));
    assert_eq!(run(border_width, "thick"), Some(Px(5.0)));
    assert_eq!(run(border_width, "2px"), Some(Px(2.0)));
    assert_eq!(run(border_width, "wide"), None);
    assert_eq!(run(border_width, "10%"), None);
    assert_eq!(run(pixels, "3px"), Some(Px(3.0)));
    assert_eq!(run(pixels, "3%"), None);
    assert_eq!(run(spacing, "normal"), Some(Px(0.0)));
    assert_eq!(run(spacing, "1px"), Some(Px(1.0)));
    assert_eq!(run(gap, "normal"), Some(px(0.0)));
    assert_eq!(run(gap, "4px"), Some(px(4.0)));
    assert_eq!(
        run(radius, "4px"),
        Some(Radius {
            horizontal: px(4.0),
            vertical: px(4.0)
        })
    );
    assert_eq!(
        run(radius, "4px 50%"),
        Some(Radius {
            horizontal: px(4.0),
            vertical: LengthPercentage::Percent(0.5)
        })
    );
    assert_eq!(run(radius, "1px 2px 3px"), None);
    assert_eq!(run(radius, "-1px"), None);
}

#[test]
fn numbers_and_integers() {
    assert_eq!(run(plain_number, "-2"), Some(-2.0));
    assert_eq!(run(non_negative_number, "-2"), None);
    assert_eq!(run(non_negative_number, "2"), Some(2.0));
    assert_eq!(run(integer, "3"), Some(3));
    assert_eq!(run(integer, "3.5"), None);
    assert_eq!(run(z_index, "auto"), Some(ZIndex(None)));
    assert_eq!(run(z_index, "-1"), Some(ZIndex(Some(-1))));
    assert_eq!(run(z_index, "x"), None);
    assert_eq!(run(opacity, "50%"), Some(0.5));
    assert_eq!(run(opacity, "2"), Some(1.0));
    assert_eq!(run(opacity, "-1"), Some(0.0));
    assert_eq!(run(opacity, "x"), None);
}

#[test]
fn fonts() {
    let sizes = [
        ("xx-small", 9.0),
        ("x-small", 10.0),
        ("small", 13.0),
        ("medium", 16.0),
        ("large", 18.0),
        ("x-large", 24.0),
        ("xx-large", 32.0),
        ("xxx-large", 48.0),
        ("larger", 24.0),
        ("2em", 20.0),
        ("50%", 10.0),
    ];
    for (text, expected) in sizes {
        assert!(
            (run(font_size, text).unwrap().0 - expected).abs() < 1e-4,
            "{text}"
        );
    }
    assert!((run(font_size, "smaller").unwrap().0 - 20.0 / 1.2).abs() < 1e-4);
    assert_eq!(run(font_size, "huge"), None);
    assert_eq!(run(font_size, "-1px"), None);
    assert_eq!(run(font_weight, "normal"), Some(FontWeight(400)));
    assert_eq!(run(font_weight, "bold"), Some(FontWeight(700)));
    assert_eq!(run(font_weight, "550"), Some(FontWeight(550)));
    assert_eq!(run(font_weight, "0"), None);
    assert_eq!(run(font_weight, "heavy"), None);
    let weight = |parent: u16, text: &str| {
        let mut context = context();
        context.parent_font_weight = parent;
        font_weight(&parse_component_values(text), &context)
            .unwrap()
            .0
    };
    assert_eq!(weight(100, "bolder"), 400);
    assert_eq!(weight(400, "bolder"), 700);
    assert_eq!(weight(700, "bolder"), 900);
    assert_eq!(weight(400, "lighter"), 100);
    assert_eq!(weight(600, "lighter"), 400);
    assert_eq!(weight(800, "lighter"), 700);
    assert_eq!(
        run(font_family, "\"Segoe UI\", Helvetica Neue, sans-serif"),
        Some(FontFamily(vec![
            "Segoe UI".to_owned(),
            "Helvetica Neue".to_owned(),
            "sans-serif".to_owned()
        ]))
    );
    assert_eq!(run(font_family, "a,,b"), None);
    assert_eq!(run(font_family, "1px"), None);
    assert_eq!(run(line_height, "normal"), Some(LineHeight::Normal));
    assert_eq!(run(line_height, "1.5"), Some(LineHeight::Number(1.5)));
    assert_eq!(run(line_height, "-1"), None);
    assert_eq!(run(line_height, "150%"), Some(LineHeight::Px(15.0)));
    assert_eq!(run(line_height, "20px"), Some(LineHeight::Px(20.0)));
    assert_eq!(run(line_height, "x"), None);
}

#[test]
fn text_values() {
    assert_eq!(
        run(decoration_line, "underline line-through blink"),
        Some(TextDecorationLine {
            underline: true,
            overline: false,
            line_through: true
        })
    );
    assert_eq!(
        run(decoration_line, "overline"),
        Some(TextDecorationLine {
            underline: false,
            overline: true,
            line_through: false
        })
    );
    assert_eq!(
        run(decoration_line, "none"),
        Some(TextDecorationLine::default())
    );
    assert_eq!(run(decoration_line, "wavy"), None);
    assert_eq!(run(decoration_line, "1px"), None);
    assert_eq!(run(vertical_align, "middle"), Some(VerticalAlign::Middle));
    assert_eq!(
        run(vertical_align, "-2px"),
        Some(VerticalAlign::Length(LengthPercentage::Px(-2.0)))
    );
    assert_eq!(
        run(vertical_align, "50%"),
        Some(VerticalAlign::Length(LengthPercentage::Percent(0.5)))
    );
    assert_eq!(run(vertical_align, "x"), None);
    assert_eq!(run(cursor, "url(a.cur), pointer"), Some(Cursor::Pointer));
    assert_eq!(run(cursor, "url(a.cur)"), None);
    assert_eq!(run(cursor, ""), None);
}

#[test]
fn generated_content() {
    assert_eq!(run(content, "normal"), Some(Content::Normal));
    assert_eq!(run(content, "none"), Some(Content::None));
    assert_eq!(
        run(
            content,
            "\"a\" attr(title) counter(x) open-quote close-quote no-open-quote"
        ),
        Some(Content::Items(vec![
            ContentItem::Text("a".to_owned()),
            ContentItem::Attr("title".to_owned()),
            ContentItem::Counter("x".to_owned(), ListStyleType::Decimal),
            ContentItem::Text("\u{201c}".to_owned()),
            ContentItem::Text("\u{201d}".to_owned()),
            ContentItem::Text(String::new()),
        ]))
    );
    assert_eq!(
        run(
            content,
            "counters(Item, '.', lower-roman) counter(n, upper-alpha)"
        ),
        Some(Content::Items(vec![
            ContentItem::Counters("Item".to_owned(), ".".to_owned(), ListStyleType::LowerRoman),
            ContentItem::Counter("n".to_owned(), ListStyleType::UpperAlpha),
        ]))
    );
    assert_eq!(run(content, "counter(none)"), None);
    assert_eq!(run(content, "counters(n)"), None);
    assert_eq!(run(content, "counter(n, nope)"), None);
    assert_eq!(run(content, "attr()"), None);
    assert_eq!(run(content, "foo"), None);
    assert_eq!(run(content, "1px"), None);
}

#[test]
fn counter_changes() {
    assert_eq!(
        run(counter_reset, "a b -2 C"),
        Some(CounterChanges(vec![
            ("a".to_owned(), 0),
            ("b".to_owned(), -2),
            ("C".to_owned(), 0)
        ]))
    );
    assert_eq!(
        run(counter_increment, "a"),
        Some(CounterChanges(vec![("a".to_owned(), 1)]))
    );
    assert_eq!(
        run(counter_increment, "none"),
        Some(CounterChanges::default())
    );
    assert_eq!(run(counter_reset, "2"), None);
    assert_eq!(run(counter_reset, "a none"), None);
    assert_eq!(run(counter_reset, "a 1.5"), None);
    assert_eq!(CounterChanges(vec![("a".to_owned(), 1)]).to_css(), "a 1");
    assert_eq!(CounterChanges::default().to_css(), "none");
}

#[test]
fn background_images() {
    assert_eq!(run(background_image, "none"), Some(BackgroundImage::None));
    assert_eq!(
        run(background_image, "url(a.png), url(b.png)"),
        Some(BackgroundImage::Url("a.png".to_owned()))
    );
    assert_eq!(
        run(background_image, "url(\"q.png\")"),
        Some(BackgroundImage::Url("q.png".to_owned()))
    );
    assert_eq!(run(background_image, "url(\"q.png\" x)"), None);
    assert_eq!(run(background_image, "url(\"bad\n)"), None);
    let Some(BackgroundImage::LinearGradient { angle, stops }) = run(
        background_image,
        "linear-gradient(red, blue 50%, green 10px)",
    ) else {
        panic!("expected a gradient");
    };
    assert_eq!(angle, 180.0);
    assert_eq!(stops.len(), 3);
    assert_eq!(stops[1].position, Some(0.5));
    assert_eq!(stops[2].position, None);
    let angle_of = |text: &str| match run(background_image, text) {
        Some(BackgroundImage::LinearGradient { angle, .. }) => angle,
        other => panic!("{text}: {other:?}"),
    };
    assert_eq!(angle_of("linear-gradient(90deg, red, blue)"), 90.0);
    assert_eq!(angle_of("linear-gradient(0.25turn, red, blue)"), 90.0);
    assert_eq!(angle_of("linear-gradient(100grad, red, blue)"), 90.0);
    assert!((angle_of("linear-gradient(1.5708rad, red, blue)") - 90.0).abs() < 0.01);
    assert_eq!(angle_of("linear-gradient(0, red, blue)"), 0.0);
    assert_eq!(angle_of("linear-gradient(to right, red, blue)"), 90.0);
    assert_eq!(angle_of("linear-gradient(to top, red, blue)"), 0.0);
    assert_eq!(
        angle_of("linear-gradient(to bottom left, red, blue)"),
        225.0
    );
    assert_eq!(
        angle_of("repeating-linear-gradient(red, red 50%, blue)"),
        180.0
    );
    assert!(matches!(
        run(
            background_image,
            "radial-gradient(circle at center, red, blue)"
        ),
        Some(BackgroundImage::RadialGradient { .. })
    ));
    assert!(matches!(
        run(background_image, "radial-gradient(red, blue)"),
        Some(BackgroundImage::RadialGradient { .. })
    ));
    for invalid in [
        "linear-gradient(red)",
        "linear-gradient(to middle, red, blue)",
        "linear-gradient(to 1px, red, blue)",
        "linear-gradient(1foo, red, blue)",
        "linear-gradient(red, nope)",
        "linear-gradient(red x, blue)",
        "linear-gradient()",
        "conic-gradient(red, blue)",
        "radial-gradient()",
        "1px",
        "url(a) url(b)",
        "",
        "linear-gradient(in nope, red, blue)",
        "linear-gradient(to right in, red, blue)",
        "linear-gradient(in srgb longer hue, red, blue)",
        "linear-gradient(in oklab red, blue)",
    ] {
        assert_eq!(run(background_image, invalid), None, "{invalid}");
    }
}

#[test]
fn gradients_interpolate_in_the_space_they_name() {
    let stops_of = |text: &str| match run(background_image, text) {
        Some(
            BackgroundImage::LinearGradient { stops, .. }
            | BackgroundImage::RadialGradient { stops },
        ) => stops,
        other => panic!("{text}: {other:?}"),
    };
    let angle_of = |text: &str| match run(background_image, text) {
        Some(BackgroundImage::LinearGradient { angle, .. }) => angle,
        other => panic!("{text}: {other:?}"),
    };
    // The clause comes before or after the direction, as Tailwind writes
    // `to top right in oklab`.
    assert_eq!(
        angle_of("linear-gradient(to top right in oklab, red, blue)"),
        45.0
    );
    assert_eq!(
        angle_of("linear-gradient(in oklab to right, red, blue)"),
        90.0
    );
    assert_eq!(
        angle_of("linear-gradient(90deg in hsl longer hue, red, blue)"),
        90.0
    );
    assert_eq!(angle_of("linear-gradient(in oklab, red, blue)"), 180.0);
    // Painting interpolates in sRGB, so stops in between follow the space.
    let stops = stops_of("linear-gradient(in oklab, red, blue)");
    assert_eq!(stops.len(), 9);
    assert_eq!(
        (stops[0].color, stops[0].position),
        (Color::rgb(255, 0, 0), Some(0.0))
    );
    assert_eq!(
        (stops[8].color, stops[8].position),
        (Color::rgb(0, 0, 255), Some(1.0))
    );
    assert_eq!(stops[4].position, Some(0.5));
    assert_eq!(
        Some(stops[4].color),
        run(color, "color-mix(in oklab, red, blue)")
    );
    assert_eq!(stops_of("linear-gradient(in srgb, red, blue)").len(), 2);
    // Stops without positions spread out first.
    let spread = stops_of("linear-gradient(in oklab, red, white, blue 80%)");
    assert_eq!(spread.len(), 17);
    assert_eq!(spread[8].position, Some(0.4));
    assert_eq!(spread[16].position, Some(0.8));
    assert_eq!(
        stops_of("radial-gradient(circle in oklch, red, blue)").len(),
        9
    );
    assert_eq!(stops_of("radial-gradient(in oklab, red, blue)").len(), 9);
}

#[test]
fn shadows() {
    assert_eq!(run(box_shadow, "none"), Some(Shadows::default()));
    let Some(Shadows(list)) = run(box_shadow, "1px 2px red, inset 0 0 3px 4px") else {
        panic!("expected shadows");
    };
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].x, list[0].y, list[0].blur, list[0].spread),
        (1.0, 2.0, 0.0, 0.0)
    );
    assert_eq!(list[0].color, Color::rgb(255, 0, 0));
    assert!(list[1].inset);
    assert_eq!((list[1].blur, list[1].spread), (3.0, 4.0));
    assert_eq!(list[1].color, Color::rgb(1, 2, 3));
    assert_eq!(run(box_shadow, "1px 2px 3px").unwrap().0[0].blur, 3.0);
    assert_eq!(run(box_shadow, "1px red"), None);
    assert_eq!(run(box_shadow, "1px 2px nope"), None);
}

#[test]
fn filters() {
    assert_eq!(run(filter, "none"), Some(Filters::default()));
    let Some(Filters(list)) = run(
        filter,
        "blur(2px) blur() brightness(80%) contrast(1.5) grayscale(2) hue-rotate(0.5turn) \
         hue-rotate() invert() opacity(30%) saturate(3) sepia(150%)",
    ) else {
        panic!("expected filters");
    };
    assert_eq!(
        list,
        vec![
            Filter::Blur(2.0),
            Filter::Blur(0.0),
            Filter::Brightness(0.8),
            Filter::Contrast(1.5),
            Filter::Grayscale(1.0),
            Filter::HueRotate(180.0),
            Filter::HueRotate(0.0),
            Filter::Invert(1.0),
            Filter::Opacity(0.3),
            Filter::Saturate(3.0),
            Filter::Sepia(1.0),
        ]
    );
    let shadow = |text: &str| run(filter, text).map(|filters| filters.0);
    assert_eq!(
        shadow("drop-shadow(16px 0 currentcolor)"),
        Some(vec![Filter::DropShadow {
            x: 16.0,
            y: 0.0,
            blur: 0.0,
            color: Color::rgb(1, 2, 3),
        }])
    );
    assert_eq!(
        shadow("drop-shadow(red 1px 2px 3px)"),
        Some(vec![Filter::DropShadow {
            x: 1.0,
            y: 2.0,
            blur: 3.0,
            color: Color::rgb(255, 0, 0),
        }])
    );
    assert_eq!(
        shadow("drop-shadow(1px 2px)").unwrap()[0],
        Filter::DropShadow {
            x: 1.0,
            y: 2.0,
            blur: 0.0,
            color: Color::rgb(1, 2, 3),
        }
    );
    for invalid in [
        "",
        "blur",
        "nope(1)",
        "url(#shadow)",
        "blur(10%)",
        "blur(-1px)",
        "blur(1px 2px)",
        "brightness(-1)",
        "brightness(1 2)",
        "brightness(nope)",
        "hue-rotate(1px)",
        "hue-rotate(1deg 2deg)",
        "drop-shadow(1px)",
        "drop-shadow(1px red 2px)",
        "drop-shadow(1px 2px -3px)",
        "drop-shadow(1px 2px 10%)",
        "drop-shadow(red blue 1px 2px)",
        "drop-shadow(1px 2px nope)",
        "drop-shadow(1px 2px 3px 4px)",
    ] {
        assert_eq!(run(filter, invalid), None, "{invalid}");
    }
}

#[test]
fn individual_transforms() {
    let css = |parser: fn(&[ComponentValue], &Context) -> Option<IndividualTransform>,
               text: &str| { run(parser, text).map(|value| value.to_css()) };
    assert_eq!(css(translate, "none").as_deref(), Some("none"));
    assert_eq!(css(translate, "2px").as_deref(), Some("2px"));
    assert_eq!(css(translate, "2px 50%").as_deref(), Some("2px 50%"));
    assert_eq!(css(translate, "2px 3px 4px").as_deref(), Some("2px 3px"));
    assert_eq!(css(rotate, "3deg").as_deref(), Some("3deg"));
    assert_eq!(css(rotate, "0.5turn").as_deref(), Some("180deg"));
    assert_eq!(css(rotate, "x 45deg").as_deref(), Some("x 45deg"));
    assert_eq!(css(rotate, "45deg z").as_deref(), Some("45deg"));
    assert_eq!(css(rotate, "1 1 0 90deg").as_deref(), Some("1 1 0 90deg"));
    assert_eq!(css(scale, "1.1").as_deref(), Some("1.1"));
    assert_eq!(css(scale, "50% 2").as_deref(), Some("0.5 2"));
    assert_eq!(css(scale, "2 3 4").as_deref(), Some("2 3"));
    for (parser, invalid) in [
        (
            translate as fn(&[ComponentValue], &Context) -> Option<IndividualTransform>,
            "1px 2px 50%",
        ),
        (translate, "red"),
        (rotate, "45"),
        (rotate, "w 45deg"),
        (scale, "1 2 3 4"),
        (scale, "1px"),
    ] {
        assert_eq!(run(parser, invalid), None, "{invalid}");
    }
}

#[test]
fn transforms() {
    assert_eq!(run(transform, "none"), Some(Transform::default()));
    use TransformFunction::*;
    let t = run(
        transform,
        "translate(10px, 50%) scale(2) rotate(0.5turn) skew(10deg)",
    )
    .unwrap();
    assert_eq!(
        t.0,
        [
            Translate(px(10.0), LengthPercentage::Percent(0.5)),
            Scale(2.0, 2.0),
            Rotate(180.0),
            Skew(10.0, 0.0)
        ]
    );
    let t = run(
        transform,
        "translateX(1px) translateY(2px) scaleX(3) scaleY(4) rotateZ(10deg) skewX(1deg) skewY(2deg)",
    )
    .unwrap();
    assert_eq!(
        t.0,
        [
            Translate(px(1.0), px(0.0)),
            Translate(px(0.0), px(2.0)),
            Scale(3.0, 1.0),
            Scale(1.0, 4.0),
            Rotate(10.0),
            Skew(1.0, 0.0),
            Skew(0.0, 2.0)
        ]
    );
    // 3D functions keep what shows without perspective.
    let t = run(
        transform,
        "translate3d(1px, 2px, 3px) scale3d(2,3,4) translateZ(5px) scaleZ(2) perspective(100px)",
    )
    .unwrap();
    assert_eq!(t.0, [Translate(px(1.0), px(2.0)), Scale(2.0, 3.0)]);
    let t = run(transform, "matrix(2, 0, 0, 3, 7, 8)").unwrap();
    assert_eq!(t.0, [Matrix([2.0, 0.0, 0.0, 3.0, 7.0, 8.0])]);
    let t = run(transform, "matrix3d(2,0,0,0, 0,3,0,0, 0,0,1,0, 7,8,0,1)").unwrap();
    assert_eq!(t.matrix(0.0, 0.0), [2.0, 0.0, 0.0, 3.0, 7.0, 8.0]);
    let close = |a: [f32; 6], b: [f32; 6]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
    let flipped = run(transform, "rotateY(180deg)").unwrap().matrix(0.0, 0.0);
    assert!(
        close(flipped, [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        "{flipped:?}"
    );
    let tilted = run(transform, "rotateX(60deg)").unwrap().matrix(0.0, 0.0);
    assert!(close(tilted, [1.0, 0.0, 0.0, 0.5, 0.0, 0.0]), "{tilted:?}");
    let about_z = run(transform, "rotate3d(0, 0, 2, 90deg)")
        .unwrap()
        .matrix(0.0, 0.0);
    assert!(
        close(about_z, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
        "{about_z:?}"
    );
    let about_x = run(transform, "rotate3d(1, 0, 0, 60deg)")
        .unwrap()
        .matrix(0.0, 0.0);
    assert!(close(about_x, tilted), "{about_x:?}");
    // A half turn about an axis in the screen shows the back; a flip in
    // the screen does not.
    let back = |text: &str| run(transform, text).unwrap().faces_back(10.0, 10.0);
    assert!(back("rotateY(180deg)"));
    assert!(back("rotateX(120deg) scale(2)"));
    assert!(!back("rotateY(60deg)"));
    assert!(!back("scaleX(-1)"));
    assert!(!back("rotateY(180deg) rotateX(180deg) rotate(180deg)"));
    assert!(!back("none"));
    for invalid in [
        "matrix(1, 2)",
        "wobble(1)",
        "translate(x)",
        "scale(x)",
        "rotate(1px)",
        "rotate()",
        "translate()",
        "scale()",
        "scale(1, 2, 3)",
        "translate3d(1px, 2px)",
        "skew(1deg, 2deg, 3deg)",
        "matrix3d(1, 0)",
        "rotate3d(1, 0, 0)",
        "1px",
    ] {
        assert_eq!(run(transform, invalid), None, "{invalid}");
    }
}

#[test]
fn transform_origins_parse_offsets_keyword_order_and_length_only_depth() {
    for (text, expected) in [
        ("center", "50% 50%"),
        ("left", "0% 50%"),
        ("right", "100% 50%"),
        ("top", "50% 0%"),
        ("bottom", "50% 100%"),
        ("25%", "25% 50%"),
        ("-3px", "-3px 50%"),
        ("left top", "0% 0%"),
        ("top left", "0% 0%"),
        ("bottom right", "100% 100%"),
        ("right bottom", "100% 100%"),
        ("center right", "100% 50%"),
        ("bottom center", "50% 100%"),
        ("center center", "50% 50%"),
        ("10px bottom", "10px 100%"),
        ("right 10px", "100% 10px"),
        ("20px 30%", "20px 30%"),
        ("120% -25%", "120% -25%"),
        ("0 0 0", "0px 0px"),
        ("left bottom 3px", "0% 100% 3px"),
        ("bottom right -1em", "100% 100% -10px"),
        ("center center calc(2px + 3px)", "50% 50% 5px"),
        ("center 2em", "50% 20px"),
        ("calc(25% + 10px) min(3px, 8px)", "calc((25% + 10px)) 3px"),
    ] {
        assert_eq!(
            run(transform_origin, text).unwrap().to_css(),
            expected,
            "{text}"
        );
    }
    for invalid in [
        "",
        "none",
        "auto",
        "left left",
        "top bottom",
        "top 10px",
        "10px left",
        "bottom 25%",
        "left center bottom",
        "1 2",
        "1px 2px 3px 4px",
        "center center 10%",
        "center center calc(1px + 0%)",
        "center center nope",
        "top nope",
        "left,top",
        "center center 1deg",
    ] {
        assert_eq!(run(transform_origin, invalid), None, "{invalid}");
    }
}

#[test]
fn aspect_ratios() {
    assert_eq!(run(aspect_ratio, "auto"), Some(AspectRatio(None)));
    assert_eq!(run(aspect_ratio, "2"), Some(AspectRatio(Some(2.0))));
    assert_eq!(run(aspect_ratio, "16 / 8"), Some(AspectRatio(Some(2.0))));
    assert_eq!(run(aspect_ratio, "auto 4/2"), Some(AspectRatio(Some(2.0))));
    assert_eq!(run(aspect_ratio, "1 / 0"), None);
    assert_eq!(run(aspect_ratio, "0"), None);
    assert_eq!(run(aspect_ratio, "1 2 3"), None);
    assert_eq!(run(aspect_ratio, "x"), None);
}

#[test]
fn grid_tracks() {
    let list = run(
        track_list,
        "[full] 100px repeat(2, [col] 1fr auto) minmax(10px, max-content) fit-content(20%) min-content",
    )
    .unwrap();
    assert_eq!(list.0.len(), 5);
    assert_eq!(
        list.0[1],
        TrackEntry::Repeat(
            RepeatCount::Count(2),
            vec![TrackSize::Fr(1.0), TrackSize::Auto]
        )
    );
    assert_eq!(run(track_list, "none"), Some(TrackList::default()));
    assert_eq!(
        run(track_list, "repeat(auto-fill, minmax(100px, 1fr))")
            .unwrap()
            .0[0],
        TrackEntry::Repeat(
            RepeatCount::AutoFill,
            vec![TrackSize::MinMax(
                Box::new(TrackSize::Length(px(100.0))),
                Box::new(TrackSize::Fr(1.0))
            )]
        )
    );
    assert!(matches!(
        run(track_list, "repeat(auto-fit, 10px)").unwrap().0[0],
        TrackEntry::Repeat(RepeatCount::AutoFit, _)
    ));
    for invalid in [
        "repeat(0, 1fr)",
        "repeat(x, 1fr)",
        "repeat(2)",
        "repeat(2,)",
        "repeat(2, nope)",
        "-1fr",
        "minmax(1px)",
        "minmax(a, 1px)",
        "minmax(1px, b)",
        "fit-content(x)",
        "stretch",
        "[a]",
        "x",
    ] {
        assert_eq!(run(track_list, invalid), None, "{invalid}");
    }
    assert_eq!(
        run(track_sizes, "min-content 1fr"),
        Some(TrackSizes(vec![TrackSize::MinContent, TrackSize::Fr(1.0)]))
    );
    assert_eq!(run(track_sizes, ""), None);
    assert_eq!(run(track_sizes, "x"), None);
}

#[test]
fn grid_placement() {
    assert_eq!(run(grid_line, "auto"), Some(GridLine::Auto));
    assert_eq!(run(grid_line, "2"), Some(GridLine::Line(2)));
    assert_eq!(run(grid_line, "-1"), Some(GridLine::Line(-1)));
    assert_eq!(run(grid_line, "span 3"), Some(GridLine::Span(3)));
    assert_eq!(run(grid_line, "3 span"), Some(GridLine::Span(3)));
    assert_eq!(run(grid_line, "span"), Some(GridLine::Span(1)));
    assert_eq!(
        run(grid_line, "header"),
        Some(GridLine::Named("header".to_owned()))
    );
    for invalid in ["0", "span -1", "name 1", "auto 1", "1.5", ""] {
        assert_eq!(run(grid_line, invalid), None, "{invalid}");
    }
    assert_eq!(run(grid_auto_flow, "row"), Some(GridAutoFlow::default()));
    assert_eq!(
        run(grid_auto_flow, "column dense"),
        Some(GridAutoFlow {
            column: true,
            dense: true
        })
    );
    assert_eq!(
        run(grid_auto_flow, "dense"),
        Some(GridAutoFlow {
            column: false,
            dense: true
        })
    );
    assert_eq!(run(grid_auto_flow, "diagonal"), None);
    assert_eq!(run(grid_auto_flow, ""), None);
    assert_eq!(run(grid_auto_flow, "1"), None);
}

#[test]
fn grid_template_areas() {
    let areas = run(
        super::grid_template_areas,
        "'head head' 'side main' '. main'",
    )
    .unwrap();
    assert_eq!((areas.rows, areas.columns), (3, 2));
    let lines = |name: &str| {
        let area = areas.areas.iter().find(|area| area.name == name).unwrap();
        (
            area.row_start,
            area.row_end,
            area.column_start,
            area.column_end,
        )
    };
    assert_eq!(lines("head"), (1, 2, 1, 3));
    assert_eq!(lines("side"), (2, 3, 1, 2));
    assert_eq!(lines("main"), (2, 4, 2, 3));
    assert_eq!(areas.to_css(), "\"head head\" \"side main\" \". main\"");
    assert_eq!(
        run(super::grid_template_areas, "none"),
        Some(GridTemplateAreas::default())
    );
    // Rows of different lengths, areas that are not rectangles, and other
    // characters make the value invalid.
    for invalid in ["'a b' 'c'", "'a b' 'b a'", "'a a' 'a .'", "'a!'", "''", "a"] {
        assert_eq!(run(super::grid_template_areas, invalid), None, "{invalid}");
    }
}

#[test]
fn table_spacing() {
    assert_eq!(
        run(border_spacing, "3px"),
        Some(Spacing {
            horizontal: Px(3.0),
            vertical: Px(3.0)
        })
    );
    assert_eq!(
        run(border_spacing, "1px 0"),
        Some(Spacing {
            horizontal: Px(1.0),
            vertical: Px(0.0)
        })
    );
    for invalid in ["10%", "-1px", "1px 2px 3px", "auto"] {
        assert_eq!(run(border_spacing, invalid), None, "{invalid}");
    }
    assert_eq!(Spacing::default().to_css(), "0px");
}

#[test]
fn svg_paints_and_stroke_widths() {
    let blue = Color::rgb(0, 0, 255);
    assert_eq!(run(paint, "none"), Some(Paint::None));
    assert_eq!(run(paint, "blue"), Some(Paint::Color(blue)));
    assert_eq!(run(paint, "currentColor"), Some(Paint::CurrentColor));
    assert_eq!(run(paint, "url(#g)"), Some(Paint::None));
    assert_eq!(run(paint, "url(#g) blue"), Some(Paint::Color(blue)));
    assert_eq!(run(paint, "url('#g') none"), Some(Paint::None));
    assert_eq!(run(paint, "url(#g) red blue"), None);
    assert_eq!(run(paint, "red blue"), None);
    assert_eq!(run(paint, "bogus"), None);
    assert_eq!(run(paint, ""), None);
    assert_eq!(run(stroke_width, "2"), Some(Px(2.0)));
    assert_eq!(run(stroke_width, "1.5px"), Some(Px(1.5)));
    assert_eq!(run(stroke_width, "1em"), Some(Px(10.0)));
    assert_eq!(run(stroke_width, "-1"), None);
    assert_eq!(run(stroke_width, "10%"), None);
    assert_eq!(run(stroke_width, "x"), None);
    assert_eq!(Paint::None.resolve(blue), None);
    assert_eq!(Paint::Color(blue).resolve(Color::BLACK), Some(blue));
    assert_eq!(Paint::CurrentColor.resolve(blue), Some(blue));
    assert_eq!(
        [Paint::None, Paint::Color(blue), Paint::CurrentColor].map(|paint| paint.to_css()),
        ["none".to_owned(), blue.to_css(), "currentcolor".to_owned()]
    );
}

#[test]
fn numeric_variants_and_feature_settings() {
    let context = Context {
        length: crate::values::LengthContext::default(),
        parent_font_size: 16.0,
        parent_font_weight: 400,
        color: Color::BLACK,
        system: &SystemColors::LIGHT,
    };
    let numeric = |text: &str| font_variant_numeric(&parse_component_values(text), &context);
    assert_eq!(numeric("normal"), Some(FontVariantNumeric::default()));
    let chosen = numeric("slashed-zero tabular-nums").unwrap();
    assert_eq!(chosen.0, vec!["tabular-nums", "slashed-zero"]);
    assert_eq!(
        chosen.features().collect::<Vec<_>>(),
        vec![*b"tnum", *b"zero"]
    );
    assert_eq!(numeric("tabular-nums proportional-nums"), None);
    assert_eq!(numeric("normal tabular-nums"), None);
    assert_eq!(numeric("bold"), None);
    let settings = |text: &str| font_feature_settings(&parse_component_values(text), &context);
    assert_eq!(settings("normal"), Some(FontFeatureSettings::default()));
    assert_eq!(
        settings("\"tnum\", \"ss01\" 2, \"liga\" off, \"kern\" on")
            .unwrap()
            .0,
        vec![(*b"tnum", 1), (*b"ss01", 2), (*b"liga", 0), (*b"kern", 1)]
    );
    assert_eq!(settings("\"toolong\""), None);
    assert_eq!(settings("tnum"), None);
    assert_eq!(settings("\"tnum\" -1"), None);
}

#[test]
fn clip_paths_parse_shapes_boxes_and_references() {
    let middle = BackgroundPosition {
        x: LengthPercentage::Percent(0.5),
        y: LengthPercentage::Percent(0.5),
    };
    assert_eq!(run(clip_path, "none"), Some(ClipPath::None));
    assert_eq!(
        run(clip_path, "inset(50%)"),
        Some(ClipPath::Shape(
            BasicShape::Inset(
                core::array::from_fn(|_| LengthPercentage::Percent(0.5)),
                Default::default()
            ),
            GeometryBox::BorderBox
        ))
    );
    let Some(ClipPath::Shape(BasicShape::Inset(offsets, radii), GeometryBox::BorderBox)) =
        run(clip_path, "inset(1px 2px 3px round 4px / 1em)")
    else {
        panic!("inset with corners");
    };
    assert_eq!(offsets, [px(1.0), px(2.0), px(3.0), px(2.0)]);
    assert!(
        radii
            .iter()
            .all(|radius| radius.horizontal == px(4.0) && radius.vertical == px(10.0))
    );
    assert_eq!(
        run(clip_path, "circle() content-box"),
        Some(ClipPath::Shape(
            BasicShape::Circle(ShapeRadius::ClosestSide, middle.clone()),
            GeometryBox::ContentBox
        ))
    );
    assert_eq!(
        run(clip_path, "padding-box circle(25% at left top)"),
        Some(ClipPath::Shape(
            BasicShape::Circle(
                ShapeRadius::Length(LengthPercentage::Percent(0.25)),
                BackgroundPosition {
                    x: LengthPercentage::Percent(0.0),
                    y: LengthPercentage::Percent(0.0),
                }
            ),
            GeometryBox::PaddingBox
        ))
    );
    assert_eq!(
        run(clip_path, "ellipse(closest-side farthest-side)"),
        Some(ClipPath::Shape(
            BasicShape::Ellipse(ShapeRadius::ClosestSide, ShapeRadius::FarthestSide, middle),
            GeometryBox::BorderBox
        ))
    );
    assert_eq!(
        run(clip_path, "polygon(evenodd, 0 0, 100% 0, 50% 1em)"),
        Some(ClipPath::Shape(
            BasicShape::Polygon(
                true,
                vec![
                    (px(0.0), px(0.0)),
                    (LengthPercentage::Percent(1.0), px(0.0)),
                    (LengthPercentage::Percent(0.5), px(10.0)),
                ]
            ),
            GeometryBox::BorderBox
        ))
    );
    assert_eq!(
        run(clip_path, "path(evenodd, 'M0 0 L10 0 Z')"),
        Some(ClipPath::Shape(
            BasicShape::Path(true, "M0 0 L10 0 Z".into()),
            GeometryBox::BorderBox
        ))
    );
    assert_eq!(
        ClipPath::Shape(
            BasicShape::Path(false, "M0 0".into()),
            GeometryBox::BorderBox
        )
        .to_css(),
        "path(\"M0 0\")"
    );
    assert_eq!(
        run(clip_path, "margin-box"),
        Some(ClipPath::Box(GeometryBox::MarginBox))
    );
    assert_eq!(
        run(clip_path, "url(#shape)"),
        Some(ClipPath::Url("#shape".into()))
    );
    for invalid in [
        "",
        "inset()",
        "inset(1px round)",
        "circle(1px 2px)",
        "circle(-1px)",
        "ellipse(1px)",
        "polygon()",
        "polygon(0 0, 1px)",
        "path()",
        "path(0 0)",
        "path(round, 'M0 0')",
        "padding-box content-box",
        "circle() circle()",
        "nope(1px)",
    ] {
        assert_eq!(run(clip_path, invalid), None, "{invalid}");
    }
}
