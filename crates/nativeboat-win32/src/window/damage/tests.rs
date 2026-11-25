use super::*;
use nativeboat_css::Color;
use nativeboat_ir::{Font, NativeChild};

fn fill(x: f32) -> DisplayItem {
    DisplayItem::Fill {
        rect: Rect::new(x, 0.0, 10.0, 10.0),
        color: Color::BLACK,
        radii: Default::default(),
    }
}

fn text(x: f32, content: &str) -> DisplayItem {
    DisplayItem::Text {
        rect: Rect::new(x, 20.0, 40.0, 10.0),
        text: content.into(),
        font: Font::default(),
        color: Color::BLACK,
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
    }
}

#[test]
fn equal_lists_damage_nothing_and_changed_items_their_bounds() {
    let old = [fill(0.0), text(0.0, "a"), fill(100.0)];
    assert_eq!(between(&old, &old), Damage::None);
    // A typed character changes one text run: its line, grown for glyphs.
    let new = [fill(0.0), text(0.0, "ab"), fill(100.0)];
    assert_eq!(
        between(&old, &new),
        Damage::Areas(vec![Rect::new(-6.0, 14.0, 52.0, 22.0)])
    );
    // Moved items damage where they were and where they are.
    let moved = [fill(0.0), text(0.0, "a"), fill(200.0)];
    assert_eq!(
        between(&old, &moved),
        Damage::Areas(vec![Rect::new(99.0, -1.0, 112.0, 12.0)])
    );
    // Removed and added items alike.
    assert_eq!(
        between(&old, &old[..2]),
        Damage::Areas(vec![Rect::new(99.0, -1.0, 12.0, 12.0)])
    );
}

#[test]
fn clips_bound_their_content_and_effects_repaint_all() {
    let clip = |width: f32| DisplayItem::PushClip {
        rect: Rect::new(0.0, 0.0, width, 50.0),
        radii: Default::default(),
    };
    let old = [clip(50.0), fill(0.0), DisplayItem::PopClip];
    let new = [clip(80.0), fill(0.0), DisplayItem::PopClip];
    assert_eq!(
        between(&old, &new),
        Damage::Areas(vec![Rect::new(-1.0, -1.0, 82.0, 52.0)])
    );
    // Content moving out of a clip changes only its own bounds.
    let out = [clip(50.0), DisplayItem::PopClip, fill(0.0)];
    assert_eq!(
        between(&old, &out),
        Damage::Areas(vec![Rect::new(-1.0, -1.0, 12.0, 12.0)])
    );
    // Shadows spread by their blur.
    let shadow = DisplayItem::Shadow {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        radii: Default::default(),
        color: Color::BLACK,
        blur: 4.0,
        inset: false,
        clip: Rect::new(0.0, 0.0, 10.0, 10.0),
        clip_radii: Default::default(),
    };
    assert_eq!(
        between(&[], &[shadow]),
        Damage::Areas(vec![Rect::new(-9.0, -9.0, 28.0, 28.0)])
    );
    for effect in [
        DisplayItem::PushOpacity(0.5),
        DisplayItem::Child(NativeChild::Surface(1)),
        DisplayItem::PopTransform,
    ] {
        assert_eq!(between(&[fill(0.0)], &[fill(0.0), effect]), Damage::All);
    }
    // A change inside an open transform repaints all.
    let transformed = |x: f32| {
        [
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 5.0, 0.0]),
            fill(x),
            DisplayItem::PopTransform,
        ]
    };
    assert_eq!(between(&transformed(0.0), &transformed(1.0)), Damage::All);
    // Closed transforms before the change do not matter.
    let mut after = transformed(0.0).to_vec();
    after.push(fill(300.0));
    assert_eq!(
        between(&transformed(0.0), &after),
        Damage::Areas(vec![Rect::new(299.0, -1.0, 12.0, 12.0)])
    );
}

#[test]
fn changes_far_apart_damage_apart() {
    // A typed character and a word count far above it.
    let old = [text(0.0, "W 9"), fill(0.0), fill(100.0), text(300.0, "a")];
    let new = [text(0.0, "W 10"), fill(0.0), fill(100.0), text(300.0, "ab")];
    assert_eq!(
        between(&old, &new),
        Damage::Areas(vec![
            Rect::new(-6.0, 14.0, 52.0, 22.0),
            Rect::new(294.0, 14.0, 52.0, 22.0),
        ])
    );
    // So many changes repaint the rectangle around them.
    let old: Vec<_> = (0..6).map(|index| fill(index as f32 * 100.0)).collect();
    let new: Vec<_> = (0..6)
        .map(|index| text(index as f32 * 100.0, "x"))
        .collect();
    assert!(matches!(between(&old, &new), Damage::Areas(areas) if areas.len() == 1));
}
