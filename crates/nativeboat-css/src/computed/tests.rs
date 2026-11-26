use super::*;
use crate::values::parse_component_values;

/// A valid value for every longhand.
const SAMPLES: &[(&str, &str)] = &[
    ("display", "flex"),
    ("scrollbar-width", "thin"),
    ("scrollbar-color", "red blue"),
    ("position", "absolute"),
    ("box-sizing", "border-box"),
    ("float", "left"),
    ("width", "10px"),
    ("height", "50%"),
    ("min-width", "1px"),
    ("min-height", "2px"),
    ("max-width", "3px"),
    ("max-height", "none"),
    ("margin-top", "1px"),
    ("margin-right", "auto"),
    ("margin-bottom", "-2px"),
    ("margin-left", "3%"),
    ("padding-top", "1px"),
    ("padding-right", "2px"),
    ("padding-bottom", "3px"),
    ("padding-left", "4px"),
    ("border-top-width", "thin"),
    ("border-right-width", "2px"),
    ("border-bottom-width", "thick"),
    ("border-left-width", "medium"),
    ("border-top-style", "solid"),
    ("border-right-style", "dashed"),
    ("border-bottom-style", "dotted"),
    ("border-left-style", "double"),
    ("border-top-color", "red"),
    ("border-right-color", "blue"),
    ("border-bottom-color", "green"),
    ("border-left-color", "black"),
    ("border-top-left-radius", "1px"),
    ("border-top-right-radius", "2px"),
    ("border-bottom-right-radius", "3px 4px"),
    ("border-bottom-left-radius", "50%"),
    ("top", "1px"),
    ("right", "auto"),
    ("bottom", "2px"),
    ("left", "3px"),
    ("z-index", "4"),
    ("flex-direction", "column"),
    ("flex-wrap", "wrap"),
    ("flex-grow", "1"),
    ("flex-shrink", "0"),
    ("flex-basis", "10px"),
    ("order", "2"),
    ("justify-content", "space-between"),
    ("align-content", "center"),
    ("align-items", "stretch"),
    ("align-self", "flex-end"),
    ("justify-items", "start"),
    ("justify-self", "end"),
    ("row-gap", "4px"),
    ("column-gap", "5px"),
    ("grid-template-columns", "1fr 2fr"),
    ("grid-template-rows", "auto"),
    ("grid-auto-columns", "10px"),
    ("grid-auto-rows", "min-content"),
    ("grid-auto-flow", "column"),
    ("grid-column-start", "1"),
    ("grid-column-end", "span 2"),
    ("grid-row-start", "2"),
    ("grid-row-end", "-1"),
    ("color", "red"),
    ("background-color", "white"),
    ("background-image", "url(a.png)"),
    ("background-repeat", "repeat-x"),
    ("background-size", "50% auto"),
    ("background-position", "right 10px bottom"),
    ("mask-image", "url(mask.svg)"),
    ("mask-repeat", "no-repeat"),
    ("mask-size", "contain"),
    ("mask-position", "center"),
    ("opacity", "0.5"),
    ("visibility", "hidden"),
    ("overflow-x", "auto"),
    ("overflow-y", "scroll"),
    ("cursor", "pointer"),
    ("pointer-events", "none"),
    ("isolation", "isolate"),
    ("user-select", "none"),
    ("-webkit-user-drag", "none"),
    ("resize", "vertical"),
    ("table-layout", "fixed"),
    ("caption-side", "bottom"),
    ("app-region", "drag"),
    ("font-family", "Arial"),
    ("font-size", "20px"),
    ("font-weight", "bold"),
    ("font-style", "italic"),
    ("line-height", "1.5"),
    ("text-align", "center"),
    ("text-decoration-line", "underline"),
    ("text-decoration-color", "red"),
    ("text-underline-offset", "3px"),
    ("text-transform", "uppercase"),
    ("white-space", "nowrap"),
    ("text-overflow", "ellipsis"),
    ("-webkit-box-orient", "vertical"),
    ("-webkit-line-clamp", "3"),
    ("word-break", "break-all"),
    ("overflow-wrap", "anywhere"),
    ("letter-spacing", "1px"),
    ("tab-size", "4"),
    ("container-type", "inline-size"),
    ("container-name", "sidebar main"),
    ("text-indent", "2px"),
    ("vertical-align", "middle"),
    ("list-style-type", "square"),
    ("list-style-position", "inside"),
    ("border-collapse", "collapse"),
    ("border-spacing", "1px 2px"),
    ("content", "\"x\""),
    ("counter-reset", "a 2 b"),
    ("counter-increment", "a"),
    ("counter-set", "none"),
    ("aspect-ratio", "2"),
    ("object-fit", "cover"),
    ("object-position", "left 10px top 20%"),
    ("box-shadow", "1px 1px red"),
    ("outline-width", "2px"),
    ("outline-style", "solid"),
    ("outline-color", "blue"),
    ("outline-offset", "1px"),
    ("caret-color", "red"),
    ("-webkit-text-fill-color", "red"),
    ("accent-color", "red"),
    ("scroll-margin-top", "1px"),
    ("scroll-margin-right", "2px"),
    ("scroll-margin-bottom", "3px"),
    ("scroll-margin-left", "4px"),
    ("scroll-padding-top", "5px"),
    ("scroll-padding-right", "10%"),
    ("scroll-padding-bottom", "auto"),
    ("scroll-padding-left", "7px"),
    ("clip", "rect(0, auto, 10px, 2px)"),
    ("transform", "rotate(1deg)"),
    ("translate", "1px 2px"),
    ("rotate", "1deg"),
    ("scale", "2"),
    ("transform-origin", "bottom right 3px"),
    ("filter", "blur(2px)"),
    ("backdrop-filter", "saturate(2)"),
    ("transition-property", "color, opacity"),
    ("transition-duration", "200ms, 1s"),
    ("transition-delay", "-1s, 250ms"),
    ("transition-timing-function", "ease, steps(4,start)"),
    ("direction", "rtl"),
    ("unicode-bidi", "isolate"),
    ("font-variant-numeric", "tabular-nums slashed-zero"),
    ("font-feature-settings", "\"tnum\", \"ss01\" 2"),
    ("font-kerning", "none"),
    ("animation-name", "spin, none"),
    ("animation-duration", "1s, 250ms"),
    ("animation-timing-function", "steps(4, end)"),
    ("animation-delay", "-1s"),
    ("animation-iteration-count", "infinite, 2.5"),
    ("animation-direction", "alternate-reverse"),
    ("animation-fill-mode", "both"),
    ("animation-play-state", "paused"),
    ("fill", "currentColor"),
    ("fill-opacity", "0.5"),
    ("fill-rule", "evenodd"),
    ("stroke", "red"),
    ("stroke-width", "2"),
    ("stroke-opacity", "50%"),
    ("stroke-linecap", "round"),
    ("stroke-linejoin", "bevel"),
    ("stroke-dasharray", "6, 2"),
    ("stroke-dashoffset", "-3"),
    ("marker-start", "url(#start)"),
    ("marker-mid", "none"),
    ("marker-end", "url('#end')"),
    ("text-anchor", "middle"),
    ("dominant-baseline", "central"),
    ("alignment-baseline", "hanging"),
    ("clip-path", "inset(1px 2px round 3px)"),
    ("backface-visibility", "hidden"),
];

fn values(text: &str) -> Vec<ComponentValue> {
    parse_component_values(text)
}

fn specified(pairs: &[(PropertyId, Specified)]) -> SpecifiedValues {
    SpecifiedValues {
        values: pairs.iter().cloned().collect(),
        custom: Rc::default(),
        author_appearance: false,
        appearance_none: false,
    }
}

fn value(text: &str) -> Specified {
    Specified::Value(values(text))
}

#[test]
fn dash_arrays_and_marker_urls_compute_as_in_chromium() {
    let system = SystemColors::LIGHT;
    let context = Context {
        length: LengthContext::default(),
        parent_font_size: 16.0,
        parent_font_weight: 400,
        color: Color::BLACK,
        system: &system,
    };
    let computed = |id: PropertyId, text: &str| {
        let mut style = ComputedStyle::default();
        style
            .apply(id, &values(text), &context)
            .then(|| style.value_of(id))
    };
    let dashes = |text| computed(PropertyId::StrokeDasharray, text);
    assert_eq!(dashes("6, 2").as_deref(), Some("6px, 2px"));
    assert_eq!(dashes("6 2px").as_deref(), Some("6px, 2px"));
    assert_eq!(dashes("none").as_deref(), Some("none"));
    assert_eq!(dashes("5, -1"), None);
    assert_eq!(dashes("red"), None);
    let offset = computed(PropertyId::StrokeDashoffset, "-3");
    assert_eq!(offset.as_deref(), Some("-3px"));
    let marker = |text| computed(PropertyId::MarkerEnd, text);
    assert_eq!(marker("url(#a)").as_deref(), Some("url(\"#a\")"));
    assert_eq!(marker("url('#b')").as_deref(), Some("url(\"#b\")"));
    assert_eq!(marker("none").as_deref(), Some("none"));
    assert_eq!(marker("red"), None);
}

#[test]
fn object_position_is_one_position_centred_by_default() {
    let system = SystemColors::LIGHT;
    let context = Context {
        length: LengthContext::default(),
        parent_font_size: 16.0,
        parent_font_weight: 400,
        color: Color::BLACK,
        system: &system,
    };
    let mut style = ComputedStyle::default();
    assert_eq!(style.value_of(PropertyId::ObjectPosition), "50% 50%");
    let id = PropertyId::ObjectPosition;
    assert!(!style.apply(id, &values("left, right"), &context));
    assert!(style.apply(id, &values("right top"), &context));
    assert_eq!(style.value_of(id), "100% 0%");
}

#[test]
fn every_longhand_parses_copies_and_serializes() {
    assert_eq!(SAMPLES.len(), PropertyId::ALL.len());
    let system = SystemColors::LIGHT;
    let context = Context {
        length: LengthContext::default(),
        parent_font_size: 16.0,
        parent_font_weight: 400,
        color: Color::BLACK,
        system: &system,
    };
    for (name, text) in SAMPLES {
        let id = PropertyId::from_name(name).unwrap_or_else(|| panic!("unknown {name}"));
        assert_eq!(id.name(), *name);
        let mut style = ComputedStyle::default();
        assert!(style.apply(id, &values(text), &context), "{name}: {text}");
        let serialized = style.value_of(id);
        assert!(!serialized.is_empty());
        let mut copy = ComputedStyle::default();
        copy.copy_property(id, &style);
        assert_eq!(copy.value_of(id), serialized, "{name}");
        assert!(
            !style.apply(id, &values("@"), &context),
            "{name} accepted @"
        );
    }
    let inherited: Vec<&str> = PropertyId::ALL
        .iter()
        .filter(|id| id.inherited())
        .map(|id| id.name())
        .collect();
    assert!(inherited.contains(&"color"));
    assert!(!inherited.contains(&"width"));
    assert_eq!(PropertyId::from_name("nope"), None);
}

#[test]
fn inheritance_and_css_wide_keywords() {
    let environment = StyleEnvironment::default();
    let parent = ComputedStyle::compute(
        &specified(&[
            (PropertyId::Color, value("red")),
            (PropertyId::Width, value("10px")),
            (PropertyId::FontSize, value("20px")),
        ]),
        None,
        &environment,
    );
    let child = ComputedStyle::compute(&specified(&[]), Some(&parent), &environment);
    assert_eq!(child.color, Color::rgb(255, 0, 0));
    assert_eq!(child.width, Size::Auto);
    assert_eq!(child.font_size, Px(20.0));
    let child = ComputedStyle::compute(
        &specified(&[
            (PropertyId::Width, Specified::Inherit),
            (PropertyId::Color, Specified::Initial),
            (PropertyId::FontSize, Specified::Unset),
            (PropertyId::Height, Specified::Unset),
        ]),
        Some(&parent),
        &environment,
    );
    assert_eq!(child.width, Size::Length(LengthPercentage::Px(10.0)));
    assert_eq!(child.color, Color::BLACK);
    assert_eq!(child.font_size, Px(20.0));
    assert_eq!(child.height, Size::Auto);
}

#[test]
fn invalid_values_fall_back() {
    let environment = StyleEnvironment::default();
    let parent = ComputedStyle::compute(
        &specified(&[(PropertyId::Color, value("blue"))]),
        None,
        &environment,
    );
    let child = ComputedStyle::compute(
        &specified(&[
            (PropertyId::Color, value("nonsense")),
            (PropertyId::Width, value("nonsense")),
        ]),
        Some(&parent),
        &environment,
    );
    assert_eq!(child.color, Color::rgb(0, 0, 255));
    assert_eq!(child.width, Size::Auto);
}

#[test]
fn transform_origin_defaults_cascade_and_font_relative_offsets() {
    let environment = StyleEnvironment::default();
    let id = PropertyId::TransformOrigin;
    assert!(!id.inherited());
    assert_eq!(ComputedStyle::default().value_of(id), "50% 50%");
    let parent = ComputedStyle::compute(&specified(&[(id, value("bottom"))]), None, &environment);
    for (specified_value, expected) in [
        (Specified::Inherit, "50% 100%"),
        (Specified::Initial, "50% 50%"),
        (Specified::Unset, "50% 50%"),
        (value("top 10px"), "50% 50%"),
    ] {
        let child = ComputedStyle::compute(
            &specified(&[(id, specified_value)]),
            Some(&parent),
            &environment,
        );
        assert_eq!(child.value_of(id), expected);
    }
    assert_eq!(
        ComputedStyle::compute(&specified(&[]), Some(&parent), &environment).value_of(id),
        "50% 50%"
    );
    let style = ComputedStyle::compute(
        &specified(&[
            (PropertyId::FontSize, value("20px")),
            (id, value("calc(25% + 1em) -1em calc(1em + 1px)")),
        ]),
        None,
        &environment,
    );
    assert_eq!(style.transform_origin.x.resolve(100.0), 45.0);
    assert_eq!(style.transform_origin.y.resolve(40.0), -20.0);
    assert_eq!(style.transform_origin.z, Px(21.0));
}

#[test]
fn relative_values_use_the_right_font_size() {
    let environment = StyleEnvironment {
        root_font_size: 10.0,
        ..StyleEnvironment::default()
    };
    let parent = ComputedStyle::compute(
        &specified(&[(PropertyId::FontSize, value("20px"))]),
        None,
        &environment,
    );
    let child = ComputedStyle::compute(
        &specified(&[
            (PropertyId::FontSize, value("1.5em")),
            (PropertyId::Width, value("2em")),
            (PropertyId::Height, value("2rem")),
            (PropertyId::LineHeight, value("200%")),
        ]),
        Some(&parent),
        &environment,
    );
    assert_eq!(child.font_size, Px(30.0));
    assert_eq!(child.width, Size::Length(LengthPercentage::Px(60.0)));
    assert_eq!(child.height, Size::Length(LengthPercentage::Px(20.0)));
    assert_eq!(child.used_line_height(), 60.0);
}

#[test]
fn font_family_starts_as_the_standard_font() {
    let environment = StyleEnvironment {
        standard_font: "Microsoft YaHei",
        ..StyleEnvironment::default()
    };
    let root = ComputedStyle::compute(&specified(&[]), None, &environment);
    assert_eq!(
        root.font_family,
        FontFamily(vec!["Microsoft YaHei".to_owned()])
    );
    let styled = ComputedStyle::compute(
        &specified(&[(PropertyId::FontFamily, value("Arial"))]),
        Some(&root),
        &environment,
    );
    let inherited = ComputedStyle::compute(&specified(&[]), Some(&styled), &environment);
    assert_eq!(inherited.font_family, FontFamily(vec!["Arial".to_owned()]));
    let reset = ComputedStyle::compute(
        &specified(&[(PropertyId::FontFamily, Specified::Initial)]),
        Some(&styled),
        &environment,
    );
    assert_eq!(reset.font_family, root.font_family);
}

#[test]
fn current_color_and_border_widths() {
    let environment = StyleEnvironment::default();
    let style = ComputedStyle::compute(
        &specified(&[
            (PropertyId::Color, value("lime")),
            (PropertyId::BorderTopStyle, value("solid")),
            (PropertyId::BorderRightColor, Specified::Initial),
            (PropertyId::BackgroundColor, value("currentcolor")),
            (PropertyId::OutlineStyle, value("none")),
            (PropertyId::OutlineWidth, value("4px")),
        ]),
        None,
        &environment,
    );
    let lime = Color::rgb(0, 255, 0);
    assert_eq!(style.border_color.top, lime);
    assert_eq!(style.border_color.right, lime);
    assert_eq!(style.background_color, lime);
    assert_eq!(style.border_width.top, Px(3.0));
    assert_eq!(style.border_width.left, Px(0.0));
    assert_eq!(style.outline_width, Px(0.0));
}

#[test]
fn color_inherits_through_currentcolor() {
    let environment = StyleEnvironment {
        system: SystemColors::DARK,
        ..StyleEnvironment::default()
    };
    let root = ComputedStyle::compute(&specified(&[]), None, &environment);
    assert_eq!(root.color, Color::WHITE);
    let child = ComputedStyle::compute(
        &specified(&[(PropertyId::Color, value("currentcolor"))]),
        Some(&root),
        &environment,
    );
    assert_eq!(child.color, Color::WHITE);
    assert_eq!(
        ComputedStyle::initial(&environment).border_color.top,
        Color::WHITE
    );
}

#[test]
fn validity() {
    assert!(ComputedStyle::is_valid(PropertyId::Width, &values("1px")));
    assert!(!ComputedStyle::is_valid(PropertyId::Width, &values("red")));
}

#[test]
fn property_values() {
    let environment = StyleEnvironment::default();
    let mut custom = BTreeMap::new();
    custom.insert("--brand".to_owned(), "red".to_owned());
    let style = ComputedStyle::compute(
        &SpecifiedValues {
            values: [
                (PropertyId::MarginTop, value("1px")),
                (PropertyId::MarginRight, value("2px")),
                (PropertyId::PaddingTop, value("1px")),
                (PropertyId::PaddingBottom, value("3px")),
                (PropertyId::Top, value("1px")),
                (PropertyId::Right, value("1px")),
                (PropertyId::Bottom, value("1px")),
                (PropertyId::Left, value("1px")),
                (PropertyId::OverflowX, value("hidden")),
                (PropertyId::RowGap, value("1px")),
                (PropertyId::ColumnGap, value("2px")),
            ]
            .into_iter()
            .collect(),
            custom: Rc::new(custom),
            author_appearance: true,
            appearance_none: false,
        },
        None,
        &environment,
    );
    assert!(style.author_appearance);
    let get = |name: &str| style.property_value(name).unwrap();
    assert_eq!(get("margin"), "1px 2px 0px 0px");
    assert_eq!(get("padding"), "1px 0px 3px");
    assert_eq!(get("inset"), "1px");
    assert_eq!(get("border-width"), "0px");
    assert_eq!(get("border-style"), "none");
    assert_eq!(get("border-color"), "rgb(0, 0, 0)");
    assert_eq!(get("border-radius"), "0px");
    assert_eq!(get("overflow"), "hidden visible");
    assert_eq!(get("gap"), "1px 2px");
    assert_eq!(get("flex"), "0 1 auto");
    assert_eq!(get("flex-flow"), "row nowrap");
    assert_eq!(get("text-decoration"), "none");
    assert_eq!(get("width"), "auto");
    // Logical properties report the physical sides they map to, as
    // Chromium's computed styles do.
    assert_eq!(get("margin-block-start"), "1px");
    assert_eq!(get("margin-inline-end"), "2px");
    assert_eq!(get("padding-block-end"), "3px");
    assert_eq!(get("block-size"), "auto");
    assert_eq!(get("margin-block"), "1px 0px");
    assert_eq!(get("margin-inline"), "0px 2px");
    assert_eq!(get("padding-block"), "1px 3px");
    assert_eq!(get("padding-inline"), "0px");
    assert_eq!(get("inset-block"), "1px");
    assert_eq!(get("inset-inline"), "1px");
    assert_eq!(get("--brand"), "red");
    assert_eq!(style.property_value("--missing"), None);
    assert_eq!(style.property_value("bogus"), None);
    let plain = ComputedStyle::default();
    assert_eq!(plain.property_value("overflow").unwrap(), "visible");
    assert_eq!(plain.property_value("gap").unwrap(), "0px");
    assert_eq!(
        sides(["a".into(), "b".into(), "c".into(), "d".into()]),
        "a b c d"
    );
    assert_eq!(
        sides(["a".into(), "b".into(), "a".into(), "b".into()]),
        "a b"
    );
}

#[test]
fn borders_outlines_and_spacing_snap_to_device_pixels() {
    let compute = |scale: f64, width: &str, spacing: &str, offset: &str| {
        let mut environment = StyleEnvironment::default();
        environment.media.resolution = scale;
        let style = ComputedStyle::compute(
            &specified(&[
                (PropertyId::BorderTopStyle, value("solid")),
                (PropertyId::BorderTopWidth, value(width)),
                (PropertyId::OutlineStyle, value("solid")),
                (PropertyId::OutlineWidth, value(width)),
                (PropertyId::OutlineOffset, value(offset)),
                (PropertyId::BorderSpacing, value(spacing)),
            ]),
            None,
            &environment,
        );
        (
            style.border_width.top.0,
            style.outline_width.0,
            style.outline_offset.0,
            style.border_spacing.horizontal.0,
        )
    };
    // As Chromium at 125%: widths round down to device pixels but keep one.
    assert_eq!(compute(1.25, "1px", "2px", "1.5px"), (0.8, 0.8, 0.8, 1.6));
    assert_eq!(
        compute(1.25, "0.5px", "0.5px", "-1.5px"),
        (0.8, 0.8, -0.8, 0.0)
    );
    assert_eq!(
        compute(1.25, "medium", "1.6px", "0px"),
        (2.4, 2.4, 0.0, 1.6)
    );
    assert_eq!(
        compute(1.0, "0.5px", "2.5px", "1.5px"),
        (1.0, 1.0, 1.0, 2.0)
    );
    // An unset style width stays zero.
    let style = ComputedStyle::compute(&specified(&[]), None, &StyleEnvironment::default());
    assert_eq!(style.border_width.top.0, 0.0);
}

#[test]
fn accent_colors_are_auto_a_color_or_the_current_color() {
    let system = SystemColors::LIGHT;
    let context = Context {
        length: LengthContext::default(),
        parent_font_size: 16.0,
        parent_font_weight: 400,
        color: Color::BLACK,
        system: &system,
    };
    let mut style = ComputedStyle::default();
    assert_eq!(style.value_of(PropertyId::AccentColor), "auto");
    assert_eq!(style.accent_color.resolve(Color::BLACK), None);
    assert!(style.apply(PropertyId::AccentColor, &values("currentcolor"), &context));
    let green = Color::rgb(0, 128, 0);
    assert_eq!(style.accent_color.resolve(green), Some(green));
    assert!(style.apply(PropertyId::AccentColor, &values("#54abde"), &context));
    assert_eq!(
        style.accent_color.resolve(green),
        Some(Color::rgb(0x54, 0xAB, 0xDE))
    );
    assert!(style.apply(PropertyId::AccentColor, &values("auto"), &context));
    assert_eq!(style.accent_color, AccentColor::Auto);
    assert!(PropertyId::AccentColor.inherited());
}
