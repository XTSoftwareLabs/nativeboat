use super::*;
use crate::tokenizer::serialize_tokens;

fn render(values: &[ComponentValue]) -> String {
    let mut text = String::new();
    for value in values {
        match value {
            ComponentValue::Token(token) => {
                text.push_str(&serialize_tokens(std::slice::from_ref(token)))
            }
            ComponentValue::Function(name, arguments) => {
                text.push_str(&format!("{name}({})", render(arguments)));
            }
            ComponentValue::Parens(inner) => text.push_str(&format!("({})", render(inner))),
            ComponentValue::Brackets(inner) => text.push_str(&format!("[{}]", render(inner))),
        }
    }
    text.trim().to_owned()
}

fn expanded(name: &str, value: &str) -> Option<Vec<(&'static str, String)>> {
    let list = expand(name, &parse_component_values(value))?;
    Some(
        list.into_iter()
            .map(|(id, value)| (id.name(), render(&value)))
            .collect(),
    )
}

fn pairs(items: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    items
        .iter()
        .map(|(name, value)| (*name, (*value).to_owned()))
        .collect()
}

#[test]
fn list_style_none_resolves_against_explicit_type_and_image() {
    for (source, kind, position) in [
        ("disc outside none", "disc", "outside"),
        ("none outside disc", "disc", "outside"),
        ("outside none decimal", "decimal", "outside"),
        ("inside square none", "square", "inside"),
        ("none", "none", "outside"),
        ("none none inside", "none", "inside"),
        ("none url(x.png)", "none", "outside"),
        ("url('x.png') none", "none", "outside"),
        ("url(x.png) inside", "disc", "inside"),
    ] {
        assert_eq!(
            expanded("list-style", source),
            Some(pairs(&[
                ("list-style-type", kind),
                ("list-style-position", position)
            ])),
            "{source}"
        );
    }
    for invalid in [
        "disc square",
        "inside outside",
        "inside inside",
        "none none none",
        "none none disc",
        "disc none url(x.png)",
        "none none url('x.png')",
        "url(x.png) url(y.png)",
        "url('x.png') url('y.png')",
        "disc 1px",
    ] {
        assert_eq!(expanded("list-style", invalid), None, "{invalid}");
    }
}

#[test]
fn transition_shorthand_defaults_order_and_lists() {
    assert_eq!(
        expanded("transition", "200ms"),
        Some(pairs(&[
            ("transition-property", "all"),
            ("transition-duration", "200ms"),
            ("transition-timing-function", "ease"),
            ("transition-delay", "0s")
        ]))
    );
    assert_eq!(
        expanded(
            "transition",
            "ease-in 2s color -50ms, transform cubic-bezier(0, 0, 1, 1) 120ms"
        ),
        Some(pairs(&[
            ("transition-property", "color,transform"),
            ("transition-duration", "2s,120ms"),
            (
                "transition-timing-function",
                "ease-in,cubic-bezier(0, 0, 1, 1)"
            ),
            ("transition-delay", "-50ms,0s")
        ]))
    );
    assert!(expanded("transition", "none").is_some());
    for invalid in [
        "",
        ",",
        "1s,",
        "-1s",
        "1s 2s 3s",
        "color opacity",
        "ease linear",
        "1px",
        "none, color 1s",
        "1s steps(0)",
    ] {
        assert!(expanded("transition", invalid).is_none(), "{invalid}");
    }
}

#[test]
fn aliases() {
    let names = [
        "margin-inline-start",
        "margin-inline-end",
        "margin-block-start",
        "margin-block-end",
        "padding-inline-start",
        "padding-inline-end",
        "padding-block-start",
        "padding-block-end",
        "scroll-margin-block-start",
        "scroll-margin-block-end",
        "scroll-margin-inline-start",
        "scroll-margin-inline-end",
        "scroll-padding-block-start",
        "scroll-padding-block-end",
        "scroll-padding-inline-start",
        "scroll-padding-inline-end",
        "inset-inline-start",
        "inset-inline-end",
        "inset-block-start",
        "inset-block-end",
        "border-inline-start-width",
        "border-inline-end-width",
        "border-block-start-width",
        "border-block-end-width",
        "border-inline-start-style",
        "border-inline-end-style",
        "border-block-start-style",
        "border-block-end-style",
        "border-inline-start-color",
        "border-inline-end-color",
        "border-block-start-color",
        "border-block-end-color",
        "border-start-start-radius",
        "border-start-end-radius",
        "border-end-end-radius",
        "border-end-start-radius",
        "inline-size",
        "block-size",
        "min-inline-size",
        "min-block-size",
        "max-inline-size",
        "max-block-size",
        "word-wrap",
        "grid-column-gap",
        "grid-row-gap",
    ];
    for name in names {
        assert!(alias(name).is_some(), "{name}");
    }
    assert_eq!(alias("margin-inline-start"), Some(P::MarginLeft));
    assert_eq!(alias("nope"), None);
}

#[test]
fn vendor_prefixes() {
    assert_eq!(unprefixed("-webkit-user-select"), "user-select");
    assert_eq!(unprefixed("-webkit-backdrop-filter"), "backdrop-filter");
    assert_eq!(unprefixed("-moz-box-sizing"), "box-sizing");
    assert_eq!(unprefixed("-ms-flex"), "flex");
    assert_eq!(
        unprefixed("-webkit-margin-block-start"),
        "margin-block-start"
    );
    assert_eq!(unprefixed("-webkit-appearance"), "-webkit-appearance");
    assert_eq!(unprefixed("color"), "color");
}

#[test]
fn shorthand_names() {
    for name in [
        "margin",
        "padding",
        "inset",
        "border-width",
        "border-style",
        "border-color",
        "border-radius",
        "margin-inline",
        "margin-block",
        "padding-inline",
        "padding-block",
        "inset-inline",
        "inset-block",
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-block-start",
        "border-block-end",
        "border-inline-start",
        "border-inline-end",
        "border-inline",
        "border-block",
        "outline",
        "background",
        "mask",
        "flex",
        "flex-flow",
        "gap",
        "grid-gap",
        "place-items",
        "place-content",
        "place-self",
        "overflow",
        "font",
        "text-decoration",
        "list-style",
        "grid-column",
        "grid-row",
        "grid-area",
    ] {
        assert!(longhands_of(name).is_some(), "{name}");
    }
    assert_eq!(longhands_of("color"), None);
    assert_eq!(expand("color", &parse_component_values("red")), None);
}

#[test]
fn box_sides() {
    assert_eq!(
        expanded("margin", "1px"),
        Some(pairs(&[
            ("margin-top", "1px"),
            ("margin-right", "1px"),
            ("margin-bottom", "1px"),
            ("margin-left", "1px")
        ]))
    );
    assert_eq!(
        expanded("padding", "1px 2px"),
        Some(pairs(&[
            ("padding-top", "1px"),
            ("padding-right", "2px"),
            ("padding-bottom", "1px"),
            ("padding-left", "2px")
        ]))
    );
    assert_eq!(
        expanded("inset", "1px 2px 3px").unwrap()[3],
        ("left", "2px".to_owned())
    );
    assert_eq!(
        expanded("border-color", "a b c d").unwrap()[3],
        ("border-left-color", "d".to_owned())
    );
    assert_eq!(expanded("margin", "1px 2px 3px 4px 5px"), None);
    assert_eq!(expanded("margin", ""), None);
    assert_eq!(
        expanded("margin-inline", "1px 2px"),
        Some(pairs(&[("margin-left", "1px"), ("margin-right", "2px")]))
    );
    assert_eq!(
        expanded("gap", "3px"),
        Some(pairs(&[("row-gap", "3px"), ("column-gap", "3px")]))
    );
    assert_eq!(expanded("place-items", "a b c"), None);
    assert_eq!(
        expanded("overflow", "hidden auto"),
        Some(pairs(&[("overflow-x", "hidden"), ("overflow-y", "auto")]))
    );
}

#[test]
fn scroll_spacing_shorthands() {
    for kind in ["scroll-margin", "scroll-padding"] {
        let expanded = expanded(kind, "1px 2px 3px 4px").unwrap();
        assert_eq!(expanded.len(), 4);
        assert_eq!(expanded[0].1, "1px");
        assert_eq!(expanded[3].1, "4px");
        for axis in ["block", "inline"] {
            let name = format!("{kind}-{axis}");
            let values = super::tests::expanded(&name, "2px 3px").unwrap();
            assert_eq!(values.len(), 2);
            assert_eq!(values[1].1, "3px");
        }
    }
}

#[test]
fn radii() {
    assert_eq!(
        expanded("border-radius", "1px 2px / 3px").unwrap(),
        pairs(&[
            ("border-top-left-radius", "1px 3px"),
            ("border-top-right-radius", "2px 3px"),
            ("border-bottom-right-radius", "1px 3px"),
            ("border-bottom-left-radius", "2px 3px"),
        ])
    );
    assert_eq!(expanded("border-radius", "4px").unwrap()[0].1, "4px 4px");
    assert_eq!(expanded("border-radius", "1px / 2px / 3px"), None);
    assert_eq!(expanded("border-radius", "1px / "), None);
}

#[test]
fn borders_and_outlines() {
    let border = expanded("border", "1px solid red").unwrap();
    assert_eq!(border.len(), 12);
    assert!(border.contains(&("border-left-style", "solid".to_owned())));
    assert!(border.contains(&("border-bottom-color", "red".to_owned())));
    assert_eq!(
        expanded("border-top", "dashed").unwrap(),
        pairs(&[
            ("border-top-width", "medium"),
            ("border-top-style", "dashed"),
            ("border-top-color", "currentcolor")
        ])
    );
    let inline = expanded("border-inline", "2px").unwrap();
    assert_eq!(inline.len(), 6);
    assert_eq!(inline[3], ("border-right-width", "2px".to_owned()));
    assert_eq!(expanded("border", "1px 2px"), None);
    assert_eq!(expanded("border", "nonsense"), None);
    assert_eq!(expanded("border", ""), None);
    assert_eq!(
        expanded("outline", "2px solid").unwrap()[2],
        ("outline-color", "auto".to_owned())
    );
    assert_eq!(
        expanded("outline", "blue").unwrap()[2],
        ("outline-color", "blue".to_owned())
    );
}

#[test]
fn masks() {
    // Element Plus's checkmarks; mode, origin and clip keywords take their
    // defaults.
    assert_eq!(
        expanded(
            "mask",
            "url(check.svg) no-repeat center / contain alpha border-box"
        )
        .unwrap(),
        pairs(&[
            ("mask-image", "url(check.svg)"),
            ("mask-repeat", "no-repeat"),
            ("mask-position", "center"),
            ("mask-size", "contain"),
        ])
    );
    assert_eq!(
        expanded("mask", "none").unwrap(),
        pairs(&[
            ("mask-image", "none"),
            ("mask-repeat", "repeat"),
            ("mask-position", "0% 0%"),
            ("mask-size", "auto"),
        ])
    );
}

#[test]
fn backgrounds() {
    assert_eq!(
        expanded("background", "url(a.png) no-repeat center / cover, #fff").unwrap(),
        pairs(&[
            ("background-color", "#fff"),
            ("background-image", "url(a.png)"),
            ("background-repeat", "no-repeat"),
            ("background-position", "center"),
            ("background-size", "cover"),
        ])
    );
    assert_eq!(
        expanded("background", "red").unwrap(),
        pairs(&[
            ("background-color", "red"),
            ("background-image", "none"),
            ("background-repeat", "repeat"),
            ("background-position", "0% 0%"),
            ("background-size", "auto"),
        ])
    );
    // A position alone (Element Plus inputs) leaves the color transparent.
    assert_eq!(
        expanded("background", "0 0").unwrap()[..2],
        pairs(&[
            ("background-color", "transparent"),
            ("background-image", "none")
        ])
    );
    assert_eq!(
        expanded(
            "background",
            "url(x.png) right 4px top / 10px 20px repeat-y padding-box fixed"
        )
        .unwrap()[2..],
        pairs(&[
            ("background-repeat", "repeat-y"),
            ("background-position", "right 4px top"),
            ("background-size", "10px 20px"),
        ])
    );
    assert_eq!(expanded("background", "url(a.png) / 10px"), None);
    assert_eq!(expanded("background", "0 0 / 1px 2px 3px"), None);
}

#[test]
fn background_longhands_compute() {
    use crate::types::{BackgroundPosition, BackgroundRepeat, BackgroundSize, Repeat};
    use crate::values::LengthPercentage;
    let parse =
        |id: PropertyId, text: &str| ComputedStyle::is_valid(id, &parse_component_values(text));
    for valid in ["repeat", "no-repeat space", "repeat-x", "round"] {
        assert!(parse(P::BackgroundRepeat, valid), "{valid}");
    }
    assert!(!parse(P::BackgroundRepeat, "repeat-x repeat"));
    for valid in ["cover", "contain", "auto", "10px", "50% auto", "auto 3em"] {
        assert!(parse(P::BackgroundSize, valid), "{valid}");
    }
    assert!(!parse(P::BackgroundSize, "-1px"));
    for valid in [
        "left",
        "center",
        "top",
        "10px",
        "left top",
        "top left",
        "10px 20%",
        "center bottom",
        "right 3px top",
        "left 1px bottom 2px",
    ] {
        assert!(parse(P::BackgroundPosition, valid), "{valid}");
    }
    for invalid in ["top 10px", "left right", "top bottom", "10px left"] {
        assert!(!parse(P::BackgroundPosition, invalid), "{invalid}");
    }
    let _ = (
        BackgroundPosition::default(),
        BackgroundRepeat {
            x: Repeat::Space,
            y: Repeat::Round,
        },
        BackgroundSize::Cover,
        LengthPercentage::Px(0.0),
    );
}

#[test]
fn flex_shorthand() {
    let flex = |value: &str| {
        expanded("flex", value).map(|list| {
            list.into_iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>()
                .join(" ")
        })
    };
    assert_eq!(flex("none").as_deref(), Some("0 0 auto"));
    assert_eq!(flex("auto").as_deref(), Some("1 1 auto"));
    assert_eq!(flex("2").as_deref(), Some("2 1 0%"));
    assert_eq!(flex("2 3").as_deref(), Some("2 3 0%"));
    assert_eq!(flex("2 3 0").as_deref(), Some("2 3 0"));
    assert_eq!(flex("10px").as_deref(), Some("1 1 10px"));
    assert_eq!(flex("2 10px").as_deref(), Some("2 1 10px"));
    assert_eq!(flex("2 3 10%").as_deref(), Some("2 3 10%"));
    assert_eq!(flex("2 3 4"), None);
    assert_eq!(flex("bogus"), None);
    assert_eq!(
        expanded("flex-flow", "wrap column").unwrap(),
        pairs(&[("flex-direction", "column"), ("flex-wrap", "wrap")])
    );
    assert_eq!(expanded("flex-flow", "diagonal"), None);
}

#[test]
fn font_shorthand() {
    assert_eq!(
        expanded(
            "font",
            "italic bold 12px/30px Georgia, \"Times New Roman\", serif"
        )
        .unwrap(),
        pairs(&[
            ("font-style", "italic"),
            ("font-weight", "bold"),
            ("font-size", "12px"),
            ("line-height", "30px"),
            ("font-family", "Georgia , \"Times New Roman\" , serif"),
        ])
    );
    assert_eq!(
        expanded("font", "normal small-caps 600 condensed 1em Segoe UI").unwrap(),
        pairs(&[
            ("font-style", "normal"),
            ("font-weight", "600"),
            ("font-size", "1em"),
            ("line-height", "normal"),
            ("font-family", "Segoe UI"),
        ])
    );
    assert_eq!(
        expanded("font", "menu").unwrap()[2],
        ("font-size", "12px".to_owned())
    );
    assert_eq!(expanded("font", "bold"), None);
    assert_eq!(expanded("font", "bold huge serif"), None);
    assert_eq!(expanded("font", "12px/ serif"), None);
    assert_eq!(expanded("font", "12px/"), None);
    assert_eq!(expanded("font", "12px"), None);
}

#[test]
fn decoration_and_lists() {
    assert_eq!(
        expanded("text-decoration", "underline dotted red").unwrap(),
        pairs(&[
            ("text-decoration-line", "underline"),
            ("text-decoration-color", "red")
        ])
    );
    assert_eq!(
        expanded("text-decoration", "none").unwrap(),
        pairs(&[
            ("text-decoration-line", "none"),
            ("text-decoration-color", "currentcolor")
        ])
    );
    assert_eq!(
        expanded("text-decoration", "solid").unwrap()[0],
        ("text-decoration-line", "none".to_owned())
    );
    assert_eq!(expanded("text-decoration", "1px"), None);
    assert_eq!(
        expanded("list-style", "square inside url(x.png)").unwrap(),
        pairs(&[
            ("list-style-type", "square"),
            ("list-style-position", "inside")
        ])
    );
    assert_eq!(
        expanded("list-style", "outside url(\"x.png\")").unwrap(),
        pairs(&[
            ("list-style-type", "disc"),
            ("list-style-position", "outside")
        ])
    );
    assert_eq!(
        expanded("list-style", "circle").unwrap(),
        pairs(&[
            ("list-style-type", "circle"),
            ("list-style-position", "outside")
        ])
    );
    assert_eq!(expanded("list-style", "1px"), None);
}

#[test]
fn grid_placement_shorthands() {
    assert_eq!(
        expanded("grid-column", "1 / span 2").unwrap(),
        pairs(&[("grid-column-start", "1"), ("grid-column-end", "span 2")])
    );
    assert_eq!(
        expanded("grid-row", "2").unwrap(),
        pairs(&[("grid-row-start", "2"), ("grid-row-end", "auto")])
    );
    assert_eq!(expanded("grid-row", "1 / 2 / 3"), None);
    assert_eq!(
        expanded("grid-area", "1 / 2 / 3").unwrap(),
        pairs(&[
            ("grid-row-start", "1"),
            ("grid-column-start", "2"),
            ("grid-row-end", "3"),
            ("grid-column-end", "auto"),
        ])
    );
    assert_eq!(expanded("grid-area", "1 / 2 / 3 / 4 / 5"), None);
}

/// The flow-relative border pairs set the two sides of a horizontal
/// left-to-right box, one value setting both; Chromium's older margin and
/// padding names are aliases.
#[test]
fn flow_relative_border_pairs_and_webkit_margin_names() {
    assert_eq!(
        expanded("border-inline-width", "1px 2px"),
        Some(pairs(&[
            ("border-left-width", "1px"),
            ("border-right-width", "2px")
        ]))
    );
    assert_eq!(
        expanded("border-block-color", "red"),
        Some(pairs(&[
            ("border-top-color", "red"),
            ("border-bottom-color", "red")
        ]))
    );
    assert_eq!(
        expanded("border-block-style", "solid dashed"),
        Some(pairs(&[
            ("border-top-style", "solid"),
            ("border-bottom-style", "dashed")
        ]))
    );
    assert_eq!(alias("-webkit-margin-before"), Some(P::MarginTop));
    assert_eq!(alias("-webkit-padding-end"), Some(P::PaddingRight));
}
