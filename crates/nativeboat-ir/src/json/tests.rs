use super::*;
use crate::{CanvasCommand, CanvasPaint, CanvasSource, Font, PathSegment};
use serde::Deserialize;

#[test]
fn values_parse_with_escapes_and_nesting() {
    assert_eq!(
        parse(r#" {"a": [1, -2.5e1, true, false, null], "b": "x\"\\\/\n\u00e9\ud83d\ude00"} "#),
        Some(Json::Object(vec![
            (
                "a".into(),
                Json::Array(vec![
                    Json::Number(1.0),
                    Json::Number(-25.0),
                    Json::Bool(true),
                    Json::Bool(false),
                    Json::Null,
                ])
            ),
            ("b".into(), Json::String("x\"\\/\n\u{e9}\u{1f600}".into())),
        ]))
    );
    for broken in [
        "",
        "{",
        "[1,]",
        "{\"a\" 1}",
        "\"open",
        "nul",
        "[1] 2",
        "\"\\x\"",
    ] {
        assert_eq!(parse(broken), None, "{broken}");
    }
}

#[derive(Debug, Deserialize, PartialEq)]
enum Shape {
    Empty,
    Square(f32),
    Pair(u16, i32),
    Labelled { label: String, sides: Option<u8> },
}

#[derive(Debug, Deserialize, PartialEq)]
struct Drawing {
    shapes: Vec<Shape>,
    #[serde(default)]
    note: Option<String>,
}

#[test]
fn values_deserialize_as_serde_json_writes_them() {
    assert_eq!(
        from_str::<Drawing>(
            r#"{"shapes":["Empty",{"Square":2.5},{"Pair":[7,-3]},{"Labelled":{"label":"x","sides":null}}]}"#
        ),
        Some(Drawing {
            shapes: vec![
                Shape::Empty,
                Shape::Square(2.5),
                Shape::Pair(7, -3),
                Shape::Labelled {
                    label: "x".into(),
                    sides: None,
                },
            ],
            note: None,
        })
    );
    assert_eq!(from_str::<Drawing>(r#"{"shapes":[{"Square":"x"}]}"#), None);
    assert_eq!(from_str::<u16>("1.5"), None);
}

#[test]
fn canvas_commands_and_fonts_read_back_what_serde_json_writes() {
    let commands = vec![
        CanvasCommand::Fill {
            path: vec![
                PathSegment::Move(0.0, 0.0),
                PathSegment::Line(10.5, 2.0),
                PathSegment::Close,
            ],
            paint: CanvasPaint::Color(nativeboat_css::Color::rgba(1, 2, 3, 0.5)),
            rule: nativeboat_css::types::FillRule::EvenOdd,
            alpha: 1.0,
            copy: false,
        },
        CanvasCommand::Save,
        CanvasCommand::Text {
            text: "label".into(),
            x: 1.0,
            y: 2.0,
            anchor: crate::TextAnchor::Center,
            font: Font::default(),
            paint: CanvasPaint::Linear {
                start: (0.0, 0.0),
                end: (1.0, 0.0),
                stops: vec![(0.0, nativeboat_css::Color::BLACK)],
            },
            stroke: Some(1.5),
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            alpha: 1.0,
        },
        CanvasCommand::Image {
            source: CanvasSource::Url("logo.png".into()),
            from: crate::Rect::new(0.0, 0.0, 4.0, 4.0),
            to: crate::Rect::new(1.0, 1.0, 8.0, 8.0),
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            alpha: 0.5,
        },
        CanvasCommand::Restore,
    ];
    let text = serde_json::to_string(&commands).unwrap();
    assert_eq!(from_str::<Vec<CanvasCommand>>(&text), Some(commands));
    let font = Font {
        features: vec![(0x6d756e74, 1)],
        ..Font::default()
    };
    let text = serde_json::to_string(&font).unwrap();
    assert_eq!(from_str::<Font>(&text), Some(font));
}

#[test]
fn values_write_as_json_that_parses_back() {
    let tricky: String = ['a', '"', char::from(92), char::from(10), char::from(1), 'é']
        .into_iter()
        .collect();
    let value = Json::Object(vec![
        ("text".into(), Json::String(tricky)),
        (
            "list".into(),
            Json::Array(vec![Json::Number(1.5), Json::Null, Json::Bool(false)]),
        ),
        ("infinite".into(), Json::Number(f64::INFINITY)),
    ]);
    let text = value.text();
    let Some(Json::Object(members)) = parse(&text) else {
        panic!("{text}");
    };
    assert_eq!(members[..2], {
        let Json::Object(original) = &value else {
            unreachable!()
        };
        original[..2].to_vec()
    });
    assert_eq!(members[2].1, Json::Null);
}
