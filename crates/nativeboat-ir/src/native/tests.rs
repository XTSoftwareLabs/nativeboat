use super::*;

#[test]
fn native_children_restack_in_paint_order_even_with_a_single_control() {
    let mut new = tree(
        vec![control(1, ControlKind::TextBox, "a")],
        vec![scroller(10, Vec::new(), Vec::new())],
        vec![
            DisplayItem::Child(NativeChild::Surface(10)),
            DisplayItem::Child(NativeChild::Control(1)),
            fill(10.0),
        ],
    );
    let expected = vec![NativeChild::Surface(10), NativeChild::Control(1)];
    assert!(diff(&NativeTree::default(), &new).contains(&Change::Stack {
        surface: 0,
        children: expected.clone(),
    }));
    assert!(diff(&new, &new).is_empty());
    let old = new.clone();
    new.root.items.swap(0, 1);
    assert!(diff(&old, &new).contains(&Change::Stack {
        surface: 0,
        children: expected.into_iter().rev().collect(),
    }));
    let old = new.clone();
    new.root.controls[0].kind = ControlKind::Password;
    assert!(
        diff(&old, &new)
            .iter()
            .any(|change| matches!(change, Change::Stack { .. }))
    );
    let old = new.clone();
    // Recreate a surviving id after reparenting; paint-order markers stay stable.
    new.root.children.push(scroller(20, Vec::new(), Vec::new()));
    let moved = new.root.children.remove(0);
    new.root.children[0].children.push(moved);
    assert!(
        diff(&old, &new)
            .iter()
            .any(|change| matches!(change, Change::Stack { .. }))
    );
    assert_eq!(
        serde_json::from_str::<DisplayItem>(
            &serde_json::to_string(&DisplayItem::Child(NativeChild::Control(1))).unwrap()
        )
        .unwrap(),
        DisplayItem::Child(NativeChild::Control(1))
    );
}

#[test]
fn scrollbar_changes_round_trip_default_old_payloads_and_diff_independently() {
    assert!(size_of::<Surface>() < size_of::<Scrollbars>());
    assert!(size_of::<Change>() < size_of::<Scrollbars>());
    let mut json = serde_json::to_value(Surface::default()).unwrap();
    json.as_object_mut().unwrap().remove("scrollbars");
    assert_eq!(
        serde_json::from_value::<Surface>(json).unwrap(),
        Surface::default()
    );
    let paint = ScrollbarPaint {
        visible: true,
        background: Color::BLACK,
        widths: [0.0; 4],
        colors: [Color::BLACK; 4],
        borders: [BorderStyle::None; 4],
        radii: std::array::from_fn(|_| Radius::default()),
        margins: std::array::from_fn(|_| LengthPercentageAuto::zero()),
    };
    let part = ScrollbarPart(std::array::from_fn(|_| paint.clone()));
    let bars = Scrollbars {
        axes: std::array::from_fn(|_| ScrollbarAxis {
            enabled: true,
            force: false,
            thickness: Size::Auto,
            bar: part.clone(),
            track: part.clone(),
            thumb: part.clone(),
        }),
        corner: part,
        platform: None,
    };
    let old = NativeTree::default();
    let new = NativeTree {
        root: Surface {
            scrollbars: Some(Box::new(bars.clone())),
            ..Surface::default()
        },
    };
    assert_eq!(
        diff(&old, &new),
        [Change::Scrollbars {
            surface: 0,
            style: Some(Box::new(bars.clone()))
        }]
    );
    assert_eq!(
        diff(&new, &old),
        [Change::Scrollbars {
            surface: 0,
            style: None
        }]
    );
    assert!(diff(&new, &new).is_empty());
    let change = &diff(&old, &new)[0];
    assert_eq!(
        serde_json::from_str::<Change>(&serde_json::to_string(change).unwrap()).unwrap(),
        *change
    );
    let child = NativeTree {
        root: Surface {
            children: vec![Surface {
                id: 1,
                scrollbars: Some(Box::new(bars)),
                ..Surface::default()
            }],
            ..Surface::default()
        },
    };
    assert!(diff(&old, &child).iter().any(|c| matches!(
        c,
        Change::Scrollbars {
            surface: 1,
            style: Some(_)
        }
    )));
}

fn attributes<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<&'a str> {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }
}

#[test]
fn elements_map_to_controls() {
    let none: &[(&str, &str)] = &[];
    assert_eq!(
        control_for("button", attributes(none)),
        Some(ControlKind::Button)
    );
    assert_eq!(
        control_for("textarea", attributes(none)),
        Some(ControlKind::TextArea)
    );
    assert_eq!(
        control_for("progress", attributes(none)),
        Some(ControlKind::ProgressBar)
    );
    assert_eq!(
        control_for("select", attributes(none)),
        Some(ControlKind::ComboBox)
    );
    assert_eq!(
        control_for("select", attributes(&[("size", "1")])),
        Some(ControlKind::ComboBox)
    );
    assert_eq!(
        control_for("select", attributes(&[("size", " 4 ")])),
        Some(ControlKind::ListBox)
    );
    assert_eq!(
        control_for("select", attributes(&[("size", "x")])),
        Some(ControlKind::ComboBox)
    );
    assert_eq!(
        control_for("select", attributes(&[("multiple", "")])),
        Some(ControlKind::ListBox)
    );
    assert_eq!(
        control_for("input", attributes(none)),
        Some(ControlKind::TextBox)
    );
    let cases = [
        ("text", Some(ControlKind::TextBox)),
        ("EMAIL", Some(ControlKind::TextBox)),
        ("number", Some(ControlKind::TextBox)),
        ("hidden", None),
        ("password", Some(ControlKind::Password)),
        ("checkbox", Some(ControlKind::CheckBox)),
        ("radio", Some(ControlKind::Radio)),
        ("button", Some(ControlKind::Button)),
        ("submit", Some(ControlKind::Button)),
        ("reset", Some(ControlKind::Button)),
        ("file", Some(ControlKind::Button)),
        ("image", Some(ControlKind::Button)),
        ("range", Some(ControlKind::Slider)),
        ("date", Some(ControlKind::Date)),
        ("time", Some(ControlKind::Time)),
        ("datetime-local", Some(ControlKind::DateTime)),
        ("month", Some(ControlKind::Month)),
    ];
    for (kind, expected) in cases {
        assert_eq!(
            control_for("input", attributes(&[("type", kind)])),
            expected,
            "{kind}"
        );
    }
    assert_eq!(control_for("div", attributes(none)), None);
}

#[test]
fn rectangles() {
    let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
    assert!(rect.contains(1.0, 2.0));
    assert!(!rect.contains(4.0, 2.0));
    assert!(!rect.contains(1.0, 6.0));
    assert!(!rect.contains(0.0, 3.0));
    assert_eq!(rect.offset(1.0, 1.0), Rect::new(2.0, 3.0, 3.0, 4.0));
    assert_eq!(Font::default().size, 13.333);
}

fn control(id: u64, kind: ControlKind, text: &str) -> Control {
    Control {
        id,
        kind,
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        state: ControlState {
            text: text.to_owned(),
            visible: true,
            ..ControlState::default()
        },
    }
}

fn fill(width: f32) -> DisplayItem {
    DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, width, 1.0),
        color: Color::BLACK,
        radii: [Corner::SQUARE; 4],
    }
}

fn tree(
    root_controls: Vec<Control>,
    children: Vec<Surface>,
    items: Vec<DisplayItem>,
) -> NativeTree {
    NativeTree {
        root: Surface {
            id: 0,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            items,
            controls: root_controls,
            children,
            ..Surface::default()
        },
    }
}

fn scroller(id: u64, controls: Vec<Control>, children: Vec<Surface>) -> Surface {
    Surface {
        id,
        rect: Rect::new(0.0, 0.0, 50.0, 50.0),
        content_height: 200.0,
        controls,
        children,
        ..Surface::default()
    }
}

#[test]
fn initial_trees_create_everything() {
    let new = tree(
        vec![
            control(1, ControlKind::Button, "Save"),
            control(2, ControlKind::TextBox, ""),
        ],
        vec![scroller(
            10,
            vec![control(3, ControlKind::CheckBox, "")],
            vec![scroller(11, Vec::new(), Vec::new())],
        )],
        vec![fill(1.0)],
    );
    let changes = diff(&NativeTree::default(), &new);
    let kinds: Vec<String> = changes
        .iter()
        .map(|change| match change {
            Change::Scrollbars { surface, .. } => format!("scrollbars {surface}"),
            Change::SurfaceView { surface, .. } => format!("view {surface}"),
            Change::AddSurface { id, parent } => format!("add-surface {id} in {parent}"),
            Change::UpdateSurface { id, .. } => format!("update-surface {id}"),
            Change::RemoveSurface { id } => format!("remove-surface {id}"),
            Change::Paint { surface, .. } => format!("paint {surface}"),
            Change::AddControl { surface, control } => format!("add {} in {surface}", control.id),
            Change::UpdateControl { control } => format!("update {}", control.id),
            Change::RemoveControl { id } => format!("remove {id}"),
            Change::Order { surface, controls } => format!("order {surface} {controls:?}"),
            Change::Stack { surface, children } => format!("stack {surface} {children:?}"),
            Change::Select { id, start, end } => format!("select {id} {start}..{end}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "add-surface 10 in 0",
            "add-surface 11 in 10",
            "update-surface 0",
            "update-surface 10",
            "update-surface 11",
            "add 1 in 0",
            "add 2 in 0",
            "add 3 in 10",
            "order 0 [1, 2]",
            "paint 0",
            "paint 10",
            "paint 11",
        ]
    );
}

#[test]
fn identical_trees_change_nothing() {
    let build = || {
        tree(
            vec![
                control(1, ControlKind::Button, "Save"),
                control(2, ControlKind::TextBox, ""),
            ],
            vec![scroller(10, Vec::new(), Vec::new())],
            vec![fill(1.0)],
        )
    };
    assert!(diff(&build(), &build()).is_empty());
}

#[test]
fn updates_moves_and_removals() {
    let old = tree(
        vec![
            control(1, ControlKind::Button, "Save"),
            control(2, ControlKind::TextBox, ""),
            control(3, ControlKind::Button, "Gone"),
            control(4, ControlKind::Button, "Morph"),
        ],
        vec![
            scroller(10, vec![control(5, ControlKind::Radio, "")], Vec::new()),
            scroller(12, vec![control(6, ControlKind::Radio, "")], Vec::new()),
            scroller(13, Vec::new(), vec![scroller(14, Vec::new(), Vec::new())]),
        ],
        vec![fill(1.0)],
    );
    let mut moved = scroller(10, Vec::new(), Vec::new());
    moved.scroll_y = 5.0;
    let new = tree(
        vec![
            control(2, ControlKind::TextBox, "typed"),
            control(1, ControlKind::Button, "Save"),
            control(4, ControlKind::CheckBox, "Morph"),
            control(5, ControlKind::Radio, ""),
        ],
        vec![
            moved,
            scroller(14, vec![control(6, ControlKind::Radio, "")], Vec::new()),
        ],
        vec![fill(2.0)],
    );
    let changes = diff(&old, &new);
    assert_eq!(
        changes,
        vec![
            Change::RemoveControl { id: 3 },
            Change::RemoveControl { id: 4 },
            Change::RemoveControl { id: 5 },
            Change::RemoveControl { id: 6 },
            Change::RemoveSurface { id: 14 },
            Change::RemoveSurface { id: 13 },
            Change::RemoveSurface { id: 12 },
            Change::AddSurface { id: 14, parent: 0 },
            Change::UpdateSurface {
                id: 10,
                rect: Rect::new(0.0, 0.0, 50.0, 50.0),
                content_width: 0.0,
                content_height: 200.0,
                scroll_x: 0.0,
                scroll_y: 5.0,
            },
            Change::UpdateSurface {
                id: 14,
                rect: Rect::new(0.0, 0.0, 50.0, 50.0),
                content_width: 0.0,
                content_height: 200.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            Change::UpdateControl {
                control: control(2, ControlKind::TextBox, "typed")
            },
            Change::AddControl {
                surface: 0,
                control: control(4, ControlKind::CheckBox, "Morph")
            },
            Change::AddControl {
                surface: 0,
                control: control(5, ControlKind::Radio, "")
            },
            Change::AddControl {
                surface: 14,
                control: control(6, ControlKind::Radio, "")
            },
            Change::Order {
                surface: 0,
                controls: vec![2, 1, 4, 5]
            },
            Change::Paint {
                surface: 0,
                items: vec![fill(2.0)]
            },
            Change::Paint {
                surface: 14,
                items: Vec::new()
            },
        ]
    );
}

#[test]
fn reordering_alone_restacks() {
    let old = tree(
        vec![
            control(1, ControlKind::Button, "a"),
            control(2, ControlKind::Button, "b"),
        ],
        Vec::new(),
        Vec::new(),
    );
    let new = tree(
        vec![
            control(2, ControlKind::Button, "b"),
            control(1, ControlKind::Button, "a"),
        ],
        Vec::new(),
        Vec::new(),
    );
    assert_eq!(
        diff(&old, &new),
        vec![Change::Order {
            surface: 0,
            controls: vec![2, 1]
        }]
    );
}

#[test]
fn trees_serialize() {
    let new = tree(
        vec![control(1, ControlKind::Slider, "")],
        Vec::new(),
        vec![
            fill(1.0),
            DisplayItem::Gradient {
                rect: Rect::default(),
                angle: 90.0,
                stops: vec![(Color::BLACK, 0.0), (Color::WHITE, 1.0)],
                radii: [Corner::round(1.0); 4],
            },
            DisplayItem::Border {
                rect: Rect::default(),
                widths: [1.0; 4],
                colors: [Color::BLACK; 4],
                styles: [BorderStyle::Solid; 4],
                radii: [Corner::SQUARE; 4],
            },
            DisplayItem::Text {
                rect: Rect::default(),
                text: "hi".to_owned(),
                font: Font::default(),
                color: Color::BLACK,
                underline: true,
                line_through: false,
                letter_spacing: 0.0,
                rtl: false,
                grayscale: false,
                underline_offset: None,
                decorations: Vec::new(),
            },
            DisplayItem::Image {
                rect: Rect::default(),
                url: "/a.png".to_owned(),
            },
            DisplayItem::PushClip {
                rect: Rect::default(),
                radii: [Corner::SQUARE; 4],
            },
            DisplayItem::PopClip,
            DisplayItem::PushOpacity(0.5),
            DisplayItem::PopOpacity,
            DisplayItem::Caret {
                rect: Rect::new(1.0, 2.0, 1.0, 16.0),
                color: Color::BLACK,
            },
        ],
    );
    let json = serde_json::to_string(&new).unwrap();
    assert_eq!(serde_json::from_str::<NativeTree>(&json).unwrap(), new);
}
