use super::*;

fn first(text: &str) -> ComponentValue {
    without_whitespace(&parse_component_values(text))
        .into_iter()
        .next()
        .unwrap()
}

fn lp(text: &str) -> Option<LengthPercentage> {
    length_percentage(&first(text), &LengthContext::default())
}

fn px(text: &str) -> f32 {
    lp(text).unwrap().resolve(0.0)
}

#[test]
fn component_values_nest() {
    let values = parse_component_values("f(a (b) [c]) (x) [y");
    assert_eq!(
        values,
        vec![
            ComponentValue::Function(
                "f".to_owned(),
                vec![
                    ComponentValue::Token(Token::Ident("a".to_owned())),
                    ComponentValue::Token(Token::Whitespace),
                    ComponentValue::Parens(vec![ComponentValue::Token(Token::Ident(
                        "b".to_owned()
                    ))]),
                    ComponentValue::Token(Token::Whitespace),
                    ComponentValue::Brackets(vec![ComponentValue::Token(Token::Ident(
                        "c".to_owned()
                    ))]),
                ]
            ),
            ComponentValue::Token(Token::Whitespace),
            ComponentValue::Parens(vec![ComponentValue::Token(Token::Ident("x".to_owned()))]),
            ComponentValue::Token(Token::Whitespace),
            ComponentValue::Brackets(vec![ComponentValue::Token(Token::Ident("y".to_owned()))]),
        ]
    );
}

#[test]
fn component_value_helpers() {
    let ident = first("Auto");
    assert_eq!(ident.ident().as_deref(), Some("auto"));
    assert!(ident.is_ident("AUTO"));
    assert!(!first("1").is_ident("auto"));
    assert_eq!(first("1").ident(), None);
    assert_eq!(first("2.5").number(), Some(2.5));
    assert_eq!(ident.number(), None);
    let parts = split_commas(&parse_component_values("a b, c"));
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].len(), 2);
}

#[test]
fn absolute_and_relative_units() {
    let context = LengthContext {
        font_size: 20.0,
        root_font_size: 10.0,
        viewport_width: 1000.0,
        viewport_height: 500.0,
        container: None,
    };
    let cases = [
        ("px", 1.0),
        ("em", 20.0),
        ("rem", 10.0),
        ("ex", 10.0),
        ("ch", 10.0),
        ("lh", 24.0),
        ("vw", 10.0),
        ("svw", 10.0),
        ("vh", 5.0),
        ("dvh", 5.0),
        ("vmin", 5.0),
        ("vmax", 10.0),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("Q", 96.0 / 101.6),
    ];
    for (unit, expected) in cases {
        let value = context.to_px(1.0, unit).unwrap();
        assert!((value - expected).abs() < 1e-4, "{unit}");
    }
    // Container query units use the viewport without a container.
    assert_eq!(context.to_px(10.0, "cqw"), Some(100.0));
    let inside = LengthContext {
        container: Some((300.0, 200.0)),
        ..context
    };
    for (unit, expected) in [
        ("cqw", 30.0),
        ("cqi", 30.0),
        ("cqh", 20.0),
        ("cqb", 20.0),
        ("cqmin", 20.0),
        ("cqmax", 30.0),
    ] {
        assert_eq!(inside.to_px(10.0, unit), Some(expected), "{unit}");
    }
    assert_eq!(context.to_px(1.0, "furlong"), None);
    assert_eq!(LengthContext::default().font_size, 16.0);
}

#[test]
fn lengths_and_percentages() {
    assert_eq!(lp("10px"), Some(LengthPercentage::Px(10.0)));
    assert_eq!(lp("50%"), Some(LengthPercentage::Percent(0.5)));
    assert_eq!(lp("0"), Some(LengthPercentage::Px(0.0)));
    assert_eq!(lp("3"), None);
    assert_eq!(lp("3foo"), None);
    assert_eq!(lp("auto"), None);
    assert_eq!(lp("rgb(1,2,3)"), None);
    assert_eq!(LengthPercentage::default(), LengthPercentage::Px(0.0));
}

#[test]
fn calc_expressions() {
    assert_eq!(px("calc(10px + 2em - 4px)"), 38.0);
    assert_eq!(px("-webkit-calc(2 * 3px)"), 6.0);
    assert_eq!(px("calc(3px * 2 / 4)"), 1.5);
    assert_eq!(px("calc((1px + 2px) * 2)"), 6.0);
    assert_eq!(px("calc(10px / 2)"), 5.0);
    assert_eq!(px("calc(2 * 3 * 1px)"), 6.0);
    assert_eq!(px("calc(6 / 2 * 1px)"), 3.0);
    assert_eq!(lp("calc(0)"), Some(LengthPercentage::Px(0.0)));
    assert_eq!(lp("calc(2)"), None);
    let mixed = lp("calc(100% - 20px)").unwrap();
    assert!(matches!(mixed, LengthPercentage::Calc(_)));
    assert_eq!(mixed.resolve(200.0), 180.0);
    let scaled = lp("calc(-1 * (50% + 2px))").unwrap();
    assert_eq!(scaled.resolve(100.0), -52.0);
    // Math constants; infinity is the largest length, as Chromium clamps
    // it, so Tailwind's `rounded-full` is fully round.
    assert_eq!(px("calc(infinity * 1px)"), 33_554_432.0);
    assert_eq!(px("calc(-INFINITY * 1px)"), -33_554_432.0);
    assert!((px("calc(pi * 1px)") - core::f32::consts::PI).abs() < 1e-6);
    assert!((px("calc(e * 1px)") - core::f32::consts::E).abs() < 1e-6);
    for invalid in [
        "calc()",
        "calc(1px +)",
        "calc(- 1px)",
        "calc(1px 2px)",
        "calc(1px + 2)",
        "calc(2 / 3px)",
        "calc(1px * 2px)",
        "calc(2px *)",
        "calc(1px + foo)",
        "calc(1px + 1q2)",
        "calc(1px , 2px)",
        "calc(3 + 1foo)",
    ] {
        assert_eq!(lp(invalid), None, "{invalid}");
    }
}

#[test]
fn min_max_and_clamp() {
    assert_eq!(px("min(10px, 4px, 8px)"), 4.0);
    assert_eq!(px("max(10px, 4px)"), 10.0);
    let mixed = lp("min(100%, 600px)").unwrap();
    assert_eq!(mixed.resolve(400.0), 400.0);
    assert_eq!(mixed.resolve(1000.0), 600.0);
    let maximum = lp("max(50%, 0)").unwrap();
    assert_eq!(maximum.resolve(-10.0), 0.0);
    let clamped = lp("clamp(10px, 50%, 100px)").unwrap();
    assert_eq!(clamped.resolve(10.0), 10.0);
    assert_eq!(clamped.resolve(100.0), 50.0);
    assert_eq!(clamped.resolve(1000.0), 100.0);
    assert_eq!(px("calc(min(2, 3) * 1px)"), 2.0);
    assert_eq!(px("calc(max(2, 3) * 1px)"), 3.0);
    for invalid in [
        "min()",
        "min(1, 2px)",
        "min(2px, 1)",
        "clamp(1px, 2px)",
        "clamp(1, 2, 3)",
        "max(1px, foo)",
        "min(1)",
        "unknown(1px)",
    ] {
        assert_eq!(lp(invalid), None, "{invalid}");
    }
}

#[test]
fn non_negative_lengths() {
    let context = LengthContext::default();
    assert_eq!(non_negative(&first("-1px"), &context), None);
    assert_eq!(non_negative(&first("-1%"), &context), None);
    assert_eq!(non_negative(&first("x"), &context), None);
    assert_eq!(
        non_negative(&first("2px"), &context),
        Some(LengthPercentage::Px(2.0))
    );
    assert!(non_negative(&first("calc(50% - 1px)"), &context).is_some());
}

#[test]
fn numbers() {
    let context = LengthContext::default();
    assert_eq!(number(&first("1.5"), &context), Some(1.5));
    assert_eq!(number(&first("calc(1 + 2)"), &context), Some(3.0));
    assert_eq!(number(&first("calc(1px + 2px)"), &context), None);
    assert_eq!(number(&first("calc(1 + x)"), &context), None);
    assert_eq!(number(&first("px"), &context), None);
}

#[test]
fn calc_trees_resolve_and_serialize() {
    let tree = Calc::Sum(vec![
        Calc::Px(1.0),
        Calc::Product(Box::new(Calc::Percent(0.5)), 2.0),
        Calc::Min(vec![Calc::Px(3.0), Calc::Px(5.0)]),
        Calc::Max(vec![Calc::Px(3.0), Calc::Px(5.0)]),
    ]);
    assert_eq!(tree.resolve(10.0), 19.0);
    assert!(tree.has_percent());
    assert!(!Calc::Product(Box::new(Calc::Px(1.0)), 2.0).has_percent());
    assert!(!Calc::Min(vec![Calc::Px(1.0)]).has_percent());
    assert_eq!(LengthPercentage::Px(1.5).to_css(), "1.5px");
    assert_eq!(LengthPercentage::Percent(0.25).to_css(), "25%");
    assert_eq!(
        LengthPercentage::Calc(Box::new(Calc::Sum(vec![Calc::Px(4.0), Calc::Percent(0.5)])))
            .to_css(),
        "calc((4px + 50%))"
    );
    let serialized = LengthPercentage::Calc(Box::new(tree.clone())).to_css();
    let restored = length_percentage(&first(&serialized), &LengthContext::default()).unwrap();
    assert_eq!(restored.resolve(10.0), tree.resolve(10.0));
    assert_eq!(restored.resolve(100.0), tree.resolve(100.0));
    assert_eq!(format_float(1.23456), "1.235");
    assert_eq!(format_float(2.0), "2");
}
