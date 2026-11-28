use super::*;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, TextMeasurer};
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};
use nativeboat_ir::{Corner, Font};

#[test]
fn native_child_markers_follow_positioned_paint_order_and_decorations() {
    let page = render(
        "<div id=scroll><input id=field></div><div id=panel>Find</div>",
        "body{margin:0} #scroll{height:100px;overflow:auto} #field{appearance:none;width:80px;height:20px} #panel{position:absolute;top:0;right:0;width:120px;height:30px;background:red}",
    );
    let items = &page.scene.tree.root.items;
    let marker = items
        .iter()
        .position(|item| {
            *item == DisplayItem::Child(NativeChild::Surface(page.id("scroll").index()))
        })
        .unwrap();
    assert!(items[marker + 1..].iter().any(|item| matches!(item,
        DisplayItem::Text { text, .. } if text == "Find")));
    let child = &page.scene.tree.root.children[0];
    assert!(
        child
            .items
            .contains(&DisplayItem::Child(NativeChild::Control(
                page.id("field").index()
            )))
    );
    assert_eq!(page.hit(390.0, 10.0).as_deref(), Some("panel"));
}

#[test]
fn scaled_boxes_expand_and_shrink_client_rectangles_and_pointer_regions() {
    let html = "<div id=box></div>";
    let base = "body{margin:0} #box{position:absolute;left:100px;top:80px;width:100px;height:40px}";
    let page = render(html, &format!("{base} #box{{transform:scale(1.05)}}"));
    let rect = page.scene.client_rect(page.id("box")).unwrap();
    for (value, expected) in [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .zip([97.5, 79.0, 105.0, 42.0])
    {
        assert!((value - expected).abs() < 0.001, "{rect:?}");
    }
    assert_eq!(page.hit(98.0, 100.0).as_deref(), Some("box"));
    assert_ne!(page.hit(97.0, 100.0).as_deref(), Some("box"));
    let page = render(html, &format!("{base} #box{{transform:scale(.5)}}"));
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(125.0, 90.0, 50.0, 20.0))
    );
    assert_ne!(page.hit(110.0, 95.0).as_deref(), Some("box"));
    assert_eq!(page.hit(130.0, 95.0).as_deref(), Some("box"));
}

#[test]
fn authored_transform_origins_resolve_against_the_border_box() {
    let html = "<div id=box></div>";
    let base = "body{margin:0} #box{position:absolute;left:100px;top:80px;width:100px;height:40px;transform:scale(2)}";
    for (origin, expected) in [
        ("left top", Rect::new(100.0, 80.0, 200.0, 80.0)),
        ("bottom right", Rect::new(0.0, 40.0, 200.0, 80.0)),
        ("left", Rect::new(100.0, 60.0, 200.0, 80.0)),
        ("bottom", Rect::new(50.0, 40.0, 200.0, 80.0)),
        ("20px 10px", Rect::new(80.0, 70.0, 200.0, 80.0)),
        ("25% 75%", Rect::new(75.0, 50.0, 200.0, 80.0)),
        ("calc(25% + 10px) -5px", Rect::new(65.0, 85.0, 200.0, 80.0)),
    ] {
        let page = render(html, &format!("{base} #box{{transform-origin:{origin}}}"));
        assert_eq!(
            page.scene.client_rect(page.id("box")),
            Some(expected),
            "{origin}"
        );
        assert_eq!(page.rect("box"), Rect::new(100.0, 80.0, 100.0, 40.0));
        assert_eq!(
            page.hit(expected.x + 1.0, expected.y + 1.0).as_deref(),
            Some("box")
        );
    }
    let page = render(
        html,
        &format!("{base} #box{{border:5px solid red;transform-origin:right bottom}}"),
    );
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(-10.0, 30.0, 220.0, 100.0))
    );
    let page = render(
        html,
        &format!("{base} #box{{transform:translate(7px,9px) scale(2);transform-origin:left top}}"),
    );
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(107.0, 89.0, 200.0, 80.0))
    );
}

#[test]
fn noncenter_rotation_nested_origins_and_scroll_clips_stay_in_sync() {
    let page = render(
        "<div id=box></div>",
        "body{margin:0} #box{position:absolute;left:100px;top:80px;width:100px;height:40px;transform:rotate(90deg);transform-origin:bottom right}",
    );
    let actual = page.scene.client_rect(page.id("box")).unwrap();
    for (value, expected) in [actual.x, actual.y, actual.width, actual.height]
        .into_iter()
        .zip([200.0, 20.0, 40.0, 100.0])
    {
        assert!((value - expected).abs() < 0.001, "{actual:?}");
    }
    assert_eq!(page.hit(220.0, 50.0).as_deref(), Some("box"));
    let page = render(
        "<div id=parent><div id=child></div></div>",
        "body{margin:0} #parent{position:absolute;left:100px;top:100px;width:100px;height:100px;transform:rotate(90deg);transform-origin:left top} #child{position:absolute;left:10px;top:20px;width:20px;height:10px;transform:scale(2);transform-origin:bottom right}",
    );
    let actual = page.scene.client_rect(page.id("child")).unwrap();
    for (value, expected) in [actual.x, actual.y, actual.width, actual.height]
        .into_iter()
        .zip([70.0, 90.0, 20.0, 40.0])
    {
        assert!((value - expected).abs() < 0.001, "{actual:?}");
    }
    assert_eq!(page.hit(75.0, 105.0).as_deref(), Some("child"));
    let css = "body{margin:0} #outer{position:absolute;left:30px;top:40px;width:200px;height:100px;overflow:auto} #box{width:100px;height:40px;transform:scale(2);transform-origin:bottom} #tail{height:500px}";
    let mut page = render(
        "<div id=outer><div id=box></div><div id=tail></div></div>",
        css,
    );
    page.dom.set_scroll(page.id("outer"), 0.0, 20.0);
    page = page_with(page.dom, css);
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(-20.0, -20.0, 200.0, 80.0))
    );
    assert_eq!(page.hit(35.0, 45.0).as_deref(), Some("box"));
    assert_ne!(page.hit(25.0, 45.0).as_deref(), Some("box"));
    assert_ne!(page.hit(35.0, 35.0).as_deref(), Some("box"));
}

#[test]
fn native_toggle_tick_rotates_about_its_bottom_border_box_origin() {
    let page = render(
        "<input id=box type=checkbox checked>",
        "body{margin:0} #box{appearance:none;position:absolute;left:30px;top:30px;width:12px;height:12px;margin:0} #box::after{content:'';position:absolute;left:4px;top:1px;width:8px;height:4px;border:2px solid white;border-top:none;border-right:none;transform:rotate(-45deg);transform-origin:bottom}",
    );
    let matrix = page
        .root()
        .items
        .iter()
        .find_map(|item| match item {
            DisplayItem::PushTransform(matrix) => Some(*matrix),
            _ => None,
        })
        .unwrap();
    // The 8x4 content box has a 2px left/bottom border. Its native bottom
    // midpoint is (39,37), and rotation must leave that exact point fixed.
    let pivot = (39.0, 37.0);
    let actual = (
        matrix[0] * pivot.0 + matrix[2] * pivot.1 + matrix[4],
        matrix[1] * pivot.0 + matrix[3] * pivot.1 + matrix[5],
    );
    assert!(
        (actual.0 - pivot.0).abs() < 0.001 && (actual.1 - pivot.1).abs() < 0.001,
        "{matrix:?}: {actual:?}"
    );
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(30.0, 30.0, 12.0, 12.0))
    );
    assert_eq!(page.hit(42.2, 33.4).as_deref(), Some("box"));
}

#[test]
fn rotated_regions_exclude_empty_corners_of_their_axis_aligned_bounds() {
    let page = render(
        "<div id=box></div>",
        "body{margin:0} #box{position:absolute;left:100px;top:100px;width:40px;height:40px;transform:rotate(45deg)}",
    );
    let rect = page.scene.client_rect(page.id("box")).unwrap();
    assert!(
        (rect.x - 91.71573).abs() < 0.001 && (rect.y - 91.71573).abs() < 0.001,
        "{rect:?}"
    );
    assert!(
        (rect.width - 56.56854).abs() < 0.001 && (rect.height - 56.56854).abs() < 0.001,
        "{rect:?}"
    );
    assert_eq!(page.hit(120.0, 95.0).as_deref(), Some("box"));
    assert_ne!(page.hit(92.0, 92.0).as_deref(), Some("box"));
}

#[test]
fn nested_transforms_compose_before_client_bounds_and_hit_testing() {
    let page = render(
        "<div id=parent><div id=child></div></div>",
        "body{margin:0} #parent{position:absolute;left:100px;top:100px;width:100px;height:100px;transform:rotate(90deg)} #child{position:absolute;left:10px;top:20px;width:20px;height:10px;transform:translateX(5px) scale(2)}",
    );
    let rect = page.scene.client_rect(page.id("child")).unwrap();
    for (value, expected) in [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .zip([165.0, 105.0, 20.0, 40.0])
    {
        assert!((value - expected).abs() < 0.001, "{rect:?}");
    }
    assert_eq!(page.hit(167.0, 107.0).as_deref(), Some("child"));
    assert_ne!(page.hit(186.0, 107.0).as_deref(), Some("child"));
}

#[test]
fn negative_singular_and_tiny_scales_have_precise_pointer_regions() {
    let html = "<div id=box></div>";
    let base = "body{margin:0} #box{position:absolute;left:100px;top:100px;width:40px;height:20px}";
    let page = render(html, &format!("{base} #box{{transform:scaleX(-2)}}"));
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(80.0, 100.0, 80.0, 20.0))
    );
    assert_eq!(page.hit(82.0, 110.0).as_deref(), Some("box"));
    let page = render(html, &format!("{base} #box{{transform:scaleX(0)}}"));
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(120.0, 100.0, 0.0, 20.0))
    );
    assert_ne!(page.hit(120.0, 110.0).as_deref(), Some("box"));
    let tiny = Geometry {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        transform: [1e-30, 0.0, 0.0, 1e-30, 0.0, 0.0],
    };
    assert!(tiny.contains(5e-30, 5e-30));
    assert!(!tiny.contains(15e-30, 5e-30));
    let invalid = Geometry {
        transform: [f32::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0],
        ..tiny
    };
    assert!(!invalid.contains(1.0, 1.0));
    let empty = Geometry {
        rect: Rect::new(0.0, 0.0, 0.0, 10.0),
        transform: IDENTITY,
    };
    assert!(!empty.contains(0.0, 5.0));
}

#[test]
fn clips_keep_their_own_transform_when_children_transform_again() {
    let page = render(
        "<div id=parent><div id=child></div></div>",
        "body{margin:0} #parent{position:absolute;left:100px;top:100px;width:100px;height:100px;overflow:clip;transform:rotate(45deg)} #child{width:100px;height:100px;transform:scale(2)}",
    );
    assert_eq!(page.hit(150.0, 100.0).as_deref(), Some("child"));
    // Inside the parent's bounding rectangle and child's transformed area,
    // but outside the parent's diamond-shaped clip.
    assert_ne!(page.hit(85.0, 85.0).as_deref(), Some("child"));
    let page = render(
        "<div id=parent><div id=child></div></div>",
        "body{margin:0} #parent{position:absolute;left:100px;top:100px;width:100px;height:100px;clip:rect(0px,50px,100px,0px);transform:rotate(90deg)} #child{width:100px;height:100px}",
    );
    assert_eq!(page.hit(150.0, 120.0).as_deref(), Some("child"));
    assert_ne!(page.hit(150.0, 180.0).as_deref(), Some("child"));
    let pushes: Vec<_> = page
        .root()
        .items
        .iter()
        .filter(|item| {
            matches!(
                item,
                DisplayItem::PushTransform(_) | DisplayItem::PushClip { .. }
            )
        })
        .collect();
    assert!(matches!(pushes[0], DisplayItem::PushTransform(_)));
    assert!(matches!(pushes[1], DisplayItem::PushClip { .. }));
    let pops: Vec<_> = page
        .root()
        .items
        .iter()
        .filter(|item| matches!(item, DisplayItem::PopTransform | DisplayItem::PopClip))
        .collect();
    assert!(matches!(pops[0], DisplayItem::PopClip));
    assert!(matches!(pops[1], DisplayItem::PopTransform));
}

#[test]
fn transformed_inline_text_and_scrolled_boxes_use_window_coordinates() {
    let css = "body{margin:0} #box{position:absolute;left:100px;top:100px;width:100px;height:40px;transform:scale(2)} #tail{height:1000px}";
    let mut page = render(
        "<div id=box><span id=text>word</span></div><div id=tail></div>",
        css,
    );
    let root = page.dom.document_element().unwrap();
    page.dom.set_scroll(root, 0.0, 50.0);
    page = page_with(page.dom, css);
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(50.0, 30.0, 200.0, 80.0))
    );
    let text = page.scene.client_rect(page.id("text")).unwrap();
    assert_eq!(text.x, 50.0);
    assert_eq!(text.width, 64.0);
    assert_eq!(
        page.hit(text.x + 1.0, text.y + 1.0).as_deref(),
        Some("text")
    );
    assert_eq!(page.hit(-1.0, text.y + 1.0), None);
    assert_eq!(page.hit(10.0, -1.0), None);
    assert_eq!(page.hit(400.0, 50.0), None);
    assert_eq!(page.hit(50.0, 300.0), None);
    // A child surface has its own content origin and scroll offset.
    let css = "body{margin:0} #outer{position:absolute;left:30px;top:40px;width:200px;height:100px;overflow:auto} #box{width:100px;height:40px;transform:scale(2)} #tail{height:500px}";
    let mut page = render(
        "<div id=outer><div id=box></div><div id=tail></div></div>",
        css,
    );
    page.dom.set_scroll(page.id("outer"), 0.0, 20.0);
    page = page_with(page.dom, css);
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(-20.0, 0.0, 200.0, 80.0))
    );
    assert_eq!(page.hit(35.0, 45.0).as_deref(), Some("box"));
    assert_ne!(page.hit(25.0, 45.0).as_deref(), Some("box"));
    assert_ne!(page.hit(35.0, 35.0).as_deref(), Some("box"));
}

#[test]
fn sticky_margins_bound_movement_without_offsetting_the_scrollport_inset() {
    let css = "body{margin:0} #outer{width:200px;height:100px;overflow:auto} #parent{width:600px;height:600px;display:flow-root} #sticky{position:sticky;top:10px;left:5px;width:100px;height:30px;margin:20px 50px 40px 30px;background:red}";
    let html = "<div id=outer><div id=parent><div id=sticky><input id=field></div></div></div>";
    let mut page = render(html, css);
    let outer = page.id("outer");
    let sticky = page.id("sticky");
    assert_eq!(page.scene.client_rect(sticky).unwrap().x, 30.0);
    assert_eq!(page.scene.client_rect(sticky).unwrap().y, 20.0);
    page.dom.set_scroll(outer, 200.0, 200.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(sticky).unwrap().x, 5.0);
    assert_eq!(page.scene.client_rect(sticky).unwrap().y, 10.0);
    page.dom.set_scroll(outer, 400.0, 500.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(sticky).unwrap().x, 5.0);
    assert_eq!(page.scene.client_rect(sticky).unwrap().y, 10.0);
    page.dom.set_scroll(outer, 400.0, 550.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(sticky).unwrap().y, 10.0);
    // Root scrolling can carry the containing block fully past the viewport.
    let css = "body{margin:0} #parent{height:600px;display:flow-root} #sticky{position:sticky;top:10px;width:100px;height:30px;margin-bottom:40px} #tail{height:1000px}";
    let mut page = render(
        "<div id=parent><div id=sticky></div></div><div id=tail></div>",
        css,
    );
    let root = page.dom.document_element().unwrap();
    page.dom.set_scroll(root, 0.0, 590.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().y, -60.0);
    let css = "body{margin:0} #parent{width:600px;height:50px} #sticky{position:sticky;left:5px;width:100px;height:20px;margin-left:10%;margin-right:50px} #tail{width:1000px;height:20px}";
    let mut page = render(
        "<div id=parent><span><div id=sticky></div></span></div><div id=tail></div>",
        css,
    );
    let root = page.dom.document_element().unwrap();
    page.dom.set_scroll(root, 590.0, 0.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().x, -140.0);
}

#[test]
fn sticky_boxes_follow_scroll_and_stop_at_the_containing_block() {
    let css = "body{margin:0} #parent{height:600px} #sticky{position:sticky;top:10px;width:100px;height:30px;background:red} #tail{height:1000px}";
    let html = "<div id=parent><div id=sticky><button id=control>C</button></div></div><div id=tail></div>";
    let page = render(html, css);
    let mut dom = page.dom;
    let root = dom.document_element().unwrap();
    dom.set_scroll(root, 0.0, 200.0);
    let page = page_with(dom, css);
    let id = page.id("sticky");
    assert_eq!(page.scene.client_rect(id).unwrap().y, 10.0);
    assert_eq!(page.hit(90.0, 15.0), Some("sticky".into()));
    let mut dom = page.dom;
    dom.set_scroll(root, 0.0, 590.0);
    let page = page_with(dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().y, -20.0);
}

#[test]
fn sticky_containment_uses_negative_and_resolved_auto_margins() {
    let css = "body{margin:0} #parent{height:600px} #sticky{position:sticky;top:10px;width:100px;height:30px;margin-bottom:-40px} #tail{height:1000px}";
    let mut page = render(
        "<div id=parent><div id=sticky></div></div><div id=tail></div>",
        css,
    );
    let root = page.dom.document_element().unwrap();
    page.dom.set_scroll(root, 0.0, 620.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().y, -10.0);
    let css = "body{margin:0} #parent{width:600px;display:flex} #sticky{position:sticky;left:5px;width:100px;height:30px;margin-left:auto;margin-right:auto;flex-shrink:0} #tail{width:1000px;height:20px}";
    let mut page = render(
        "<div id=parent><div id=sticky></div></div><div id=tail></div>",
        css,
    );
    let root = page.dom.document_element().unwrap();
    page.dom.set_scroll(root, 590.0, 0.0);
    page = page_with(page.dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().x, -340.0);
}

#[test]
fn nested_scrollports_position_sticky_content_and_native_controls() {
    let css = "body{margin:0} #outer{width:200px;height:100px;overflow:auto;border:2px solid} #content{height:500px} #sticky{position:sticky;top:5px;height:30px;background:red}";
    let page = render(
        "<div id=outer><div id=content><div id=sticky><input id=field></div></div></div>",
        css,
    );
    let mut dom = page.dom;
    let outer = dom.get_element_by_id(Dom::DOCUMENT, "outer").unwrap();
    dom.set_scroll(outer, 0.0, 200.0);
    let page = page_with(dom, css);
    assert_eq!(page.scene.client_rect(page.id("sticky")).unwrap().y, 7.0);
    assert!(page.scene.client_rect(page.id("field")).unwrap().y >= 7.0);
    assert_eq!(page.root().children[0].controls.len(), 1);
}

#[test]
fn sticky_offsets_cover_both_edges_auto_and_oversized_boxes() {
    assert_eq!(
        sticky_offset(20.0, 40.0, 100.0, 300.0, 0.0, 600.0, None, None),
        0.0
    );
    assert_eq!(
        sticky_offset(20.0, 40.0, 100.0, 300.0, 0.0, 600.0, Some(10.0), None),
        90.0
    );
    assert_eq!(
        sticky_offset(350.0, 40.0, 0.0, 300.0, 0.0, 600.0, None, Some(10.0)),
        -100.0
    );
    assert_eq!(
        sticky_offset(
            20.0,
            400.0,
            100.0,
            300.0,
            0.0,
            1000.0,
            Some(10.0),
            Some(10.0)
        ),
        90.0
    );
    let page = render(
        "<div style='height:500px'><div id=box style='position:sticky;left:10%;right:0;bottom:0;width:30px;height:40px'></div></div>",
        "body{margin:0}",
    );
    assert_eq!(page.scene.client_rect(page.id("box")).unwrap().x, 40.0);
}

#[test]
fn absolute_clip_crops_paint_and_pointer_targets_including_descendants() {
    let page = render(
        "<div id=clip><div id=child></div></div><div id=normal></div>",
        "body{margin:0} #clip{position:absolute;left:20px;top:20px;width:100px;height:100px;clip:rect(10px,60px,70px,20px);background:red} #child{width:100px;height:100px} #normal{clip:rect(0,0,0,0);width:10px;height:10px;background:blue}",
    );
    assert_eq!(page.hit(45.0, 40.0), Some("child".into()));
    assert_ne!(page.hit(25.0, 40.0), Some("child".into()));
    assert_ne!(page.hit(90.0, 40.0), Some("clip".into()));
    assert_eq!(page.hit(5.0, 5.0), Some("normal".into()));
    assert!(page.root().items.contains(&DisplayItem::PushClip {
        rect: Rect::new(40.0, 30.0, 40.0, 60.0),
        radii: [Corner::SQUARE; 4]
    }));
    let empty = render(
        "<span id=hidden>screen reader text</span>",
        "#hidden{position:absolute;clip:rect(0,0,0,0)}",
    );
    assert_ne!(empty.hit(9.0, 9.0), Some("hidden".into()));
    let auto = render(
        "<div id=box></div>",
        "#box{position:fixed;width:20px;height:20px;clip:rect(auto,auto,auto,auto)}",
    );
    assert!(
        auto.root()
            .items
            .iter()
            .any(|item| matches!(item, DisplayItem::PushClip { .. }))
    );
}

#[test]
fn native_controls_receive_clips_in_their_own_coordinates() {
    let page = render(
        "<div id=parent><input id=field></div>",
        "body{margin:0} #parent{position:absolute;left:20px;top:20px;width:100px;height:100px;clip:rect(10px,60px,70px,20px)} input{width:100px;height:100px;box-sizing:border-box}",
    );
    assert_eq!(
        page.root().controls[0].state.clip,
        Some(Rect::new(20.0, 10.0, 40.0, 60.0))
    );
    assert_eq!(page.hit(45.0, 40.0), Some("field".into()));
    assert_ne!(page.hit(25.0, 40.0), Some("field".into()));
    let page = render("<input>", "input{position:absolute;clip:rect(0,0,0,0)}");
    let clip = page.root().controls[0].state.clip.unwrap();
    assert_eq!((clip.width, clip.height), (0.0, 0.0));
    let page = render(
        "<div style='overflow:hidden;width:40px;height:10px'><input style='width:100px;height:30px'></div>",
        "body{margin:0}",
    );
    assert_eq!(
        page.root().controls[0].state.clip,
        Some(Rect::new(0.0, 0.0, 40.0, 10.0))
    );
}

struct Mono;

impl TextMeasurer for Mono {
    fn width(&self, text: &str, font: &Font) -> f32 {
        text.chars().count() as f32 * font.size * 0.5
    }

    fn metrics(&self, font: &Font) -> FontMetrics {
        FontMetrics {
            ascent: font.size * 0.8,
            descent: font.size * 0.2,
        }
    }
}

/// Measures as [`Mono`], for device pixels of a size in CSS pixels.
struct Pixels(f32);

impl TextMeasurer for Pixels {
    fn width(&self, text: &str, font: &Font) -> f32 {
        Mono.width(text, font)
    }

    fn metrics(&self, font: &Font) -> FontMetrics {
        Mono.metrics(font)
    }

    fn device_pixel(&self) -> f32 {
        self.0
    }
}

/// Measures as [`Mono`], following a Chromium release.
struct Release(u32);

impl TextMeasurer for Release {
    fn width(&self, text: &str, font: &Font) -> f32 {
        Mono.width(text, font)
    }

    fn metrics(&self, font: &Font) -> FontMetrics {
        Mono.metrics(font)
    }

    fn chromium(&self) -> u32 {
        self.0
    }
}

struct Page {
    dom: Dom,
    layout: LayoutResult,
    scene: Scene,
}

impl Page {
    fn id(&self, id: &str) -> NodeId {
        self.dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
    }

    fn rect(&self, id: &str) -> Rect {
        self.layout.boxes[&self.id(id)].rect
    }

    fn root(&self) -> &Surface {
        &self.scene.tree.root
    }

    fn hit(&self, x: f32, y: f32) -> Option<String> {
        self.scene.hit_test(x, y).map(|node| {
            self.dom.element_id(node).map_or_else(
                || self.dom.local_name(node).unwrap().to_owned(),
                str::to_owned,
            )
        })
    }
}

fn render(html: &str, css: &str) -> Page {
    page_with(
        Dom::from_document(&nativeboat_html::parse_document(html)),
        css,
    )
}

fn page_with(dom: Dom, css: &str) -> Page {
    page_in(dom, css, u32::MAX)
}

/// A page as a Chromium release paints it.
fn page_in(dom: Dom, css: &str, chromium: u32) -> Page {
    page_measured(dom, css, chromium, &Mono)
}

/// A page laid out with a measurer, as a Chromium release paints it.
fn page_measured(dom: Dom, css: &str, chromium: u32, measurer: &dyn TextMeasurer) -> Page {
    let (author, _) = parse_stylesheet(css);
    let sheets = [
        SheetRef {
            sheet: user_agent_stylesheet(),
            origin: Origin::UserAgent,
        },
        SheetRef {
            sheet: &author,
            origin: Origin::Author,
        },
    ];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let options = LayoutOptions {
        viewport: (400.0, 300.0),
        measurer,
        image_size: &|_| Some((40.0, 20.0)),
    };
    let layout = layout(&dom, &styles, &options);
    let release = Release(chromium);
    let highlight =
        (chromium != u32::MAX).then(|| Highlight::new(&dom, &styles, &sheets, &release));
    let scene = scene_with(&dom, &styles, &layout, (400.0, 300.0), highlight.as_ref());
    Page { dom, layout, scene }
}

fn texts(items: &[DisplayItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn fills(items: &[DisplayItem]) -> Vec<(Rect, Color)> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { rect, color, .. } => Some((*rect, *color)),
            _ => None,
        })
        .collect()
}

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 128, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

#[test]
fn filters_and_backdrop_filters_wrap_the_element() {
    use nativeboat_css::types::Filter;
    let page = render(
        "<div id=f style='filter:brightness(80%);height:10px;background:red'>\
           <div style='position:relative;z-index:-1;height:5px;background:blue'></div></div>\
         <div id=b style='backdrop-filter:blur(5px);border-radius:4px;height:10px;opacity:0.5'></div>\
         <div style='visibility:hidden;backdrop-filter:blur(5px);height:10px'></div>\
         <div style='filter:none;height:10px;background:green'></div>",
        "",
    );
    let items = &page.root().items;
    let start = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushFilter(_)))
        .unwrap();
    assert_eq!(
        items[start],
        DisplayItem::PushFilter(vec![Filter::Brightness(0.8)])
    );
    let end = items
        .iter()
        .position(|item| *item == DisplayItem::PopFilter)
        .unwrap();
    let colors: Vec<_> = fills(&items[start..end])
        .into_iter()
        .map(|(_, color)| color)
        .collect();
    assert_eq!(colors, vec![RED, BLUE], "the group holds negative layers");
    let backdrops: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            DisplayItem::Backdrop {
                rect,
                radii,
                filters,
            } => Some((index, *rect, *radii, filters.clone())),
            _ => None,
        })
        .collect();
    let b = page.rect("b");
    assert_eq!(backdrops.len(), 1, "hidden elements show no backdrop");
    let (index, rect, radii, filters) = backdrops[0].clone();
    assert_eq!(
        (rect, radii, filters),
        (b, [Corner::round(4.0); 4], vec![Filter::Blur(5.0)])
    );
    assert_eq!(items[index - 1], DisplayItem::PushOpacity(0.5));
    assert_eq!(
        items
            .iter()
            .filter(|item| matches!(item, DisplayItem::PushFilter(_)))
            .count(),
        1
    );
}

#[test]
fn canvas_text_and_hits() {
    let page = render(
        "<div id=a style='height:20px;background:red'>Hi <span id=s>there</span></div>",
        "body { background: blue }",
    );
    let root = page.root();
    assert_eq!(root.id, 0);
    assert_eq!(root.rect, Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!((root.content_width, root.content_height), (400.0, 300.0));
    assert_eq!(
        fills(&root.items),
        vec![
            (Rect::new(0.0, 0.0, 400.0, 300.0), BLUE),
            (page.rect("a"), RED)
        ]
    );
    assert_eq!(texts(&root.items), vec!["Hi ", "there"]);
    let span = page.scene.client_rect(page.id("s")).unwrap();
    assert_eq!(span.x, 32.0);
    assert_eq!(span.width, 40.0);
    assert_eq!(page.hit(span.x + 1.0, span.y + 1.0).as_deref(), Some("s"));
    assert_eq!(page.hit(10.0, span.y + 1.0).as_deref(), Some("a"));
    assert_eq!(page.hit(390.0, 27.0).as_deref(), Some("a"));
    assert_eq!(page.hit(395.0, 2.0).as_deref(), Some("html"));
    assert_eq!(page.hit(395.0, 299.0), None);
    assert_eq!(page.scene.client_rect(page.id("a")), Some(page.rect("a")));
}

#[test]
fn videos_paint_their_poster_or_first_frame_contained() {
    let page = render(
        "<video id=v src=a.mp4></video><video id=p poster=p.png src=a.mp4></video>\
         <video id=f src=a.mp4 style='object-fit:fill'></video><video id=e></video>",
        "video { display: block; width: 80px; height: 80px }",
    );
    let images: Vec<(Rect, String)> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Image { rect, url } => Some((*rect, url.clone())),
            _ => None,
        })
        .collect();
    // The 40 by 20 picture fits the box as `object-fit: contain`, the user
    // agent's default for videos, fits it; a video with no media paints
    // nothing.
    let at = |id: &str, y: f32, height: f32| {
        let rect = page.rect(id);
        Rect::new(rect.x, rect.y + y, 80.0, height)
    };
    assert_eq!(
        images,
        [
            (
                at("v", 20.0, 40.0),
                "nativeboat-video-frame:a.mp4".to_owned()
            ),
            (at("p", 20.0, 40.0), "p.png".to_owned()),
            (
                at("f", 0.0, 80.0),
                "nativeboat-video-frame:a.mp4".to_owned()
            ),
        ]
    );
}

#[test]
fn root_background_wins_over_body() {
    let page = render(
        "<p>x</p>",
        "html { background: green } body { background: red }",
    );
    let colors: Vec<Color> = fills(&page.root().items)
        .into_iter()
        .map(|(_, color)| color)
        .collect();
    assert_eq!(colors, vec![GREEN, RED]);
    let plain = page_with(Dom::new(), "");
    assert_eq!(
        plain.root().items,
        vec![DisplayItem::Fill {
            rect: Rect::new(0.0, 0.0, 400.0, 300.0),
            color: Color::WHITE,
            radii: [Corner::SQUARE; 4]
        }]
    );
    assert_eq!(plain.hit(1.0, 1.0), None);
    let hidden = render("<p>x</p>", "html { display: none }");
    assert_eq!(hidden.root().items.len(), 1);
}

#[test]
fn backgrounds_fill_the_box_they_are_clipped_to() {
    let page = render(
        "<div id=p></div><div id=c></div><div id=t>text</div>",
        "div { height: 10px; border: 2px solid blue; padding: 3px }
         #p { background: red padding-box }
         #c { background-color: green; background-clip: content-box }
         #t { background: red; -webkit-background-clip: text; color: transparent }",
    );
    let items = &page.root().items;
    let boxes = |id: &str| page.layout.boxes[&page.id(id)];
    assert!(items.contains(&DisplayItem::Fill {
        rect: boxes("p").padding_box(),
        color: RED,
        radii: [Corner::SQUARE; 4]
    }));
    assert!(items.contains(&DisplayItem::Fill {
        rect: boxes("c").content_box(),
        color: GREEN,
        radii: [Corner::SQUARE; 4]
    }));
    // Backgrounds clipped to text are not painted as boxes.
    let t = page.rect("t");
    assert!(!items.iter().any(|item| matches!(
        item,
        DisplayItem::Fill { rect, color, .. } if *rect == t && *color == RED
    )));
}

#[test]
fn backgrounds_borders_and_images() {
    let page = render(
        "<div id=r></div><div id=k></div><div id=g></div><div id=q></div><div id=u></div>\
         <img id=i src=a.png style='padding:2px'><img id=n>",
        "div { height: 10px }
         #r { background: red; border-radius: 50%; border: 2px solid blue }
         #k { background: green; border-radius: 8px }
         #g { background-image: linear-gradient(red, green 50%, blue, red) }
         #q { background-image: radial-gradient(red, blue) }
         #u { background-image: url(bg.png); padding: 1px }
         #i { outline: 3px dashed red; outline-offset: 1px }
         #n { outline: 3px none red }",
    );
    let items = &page.root().items;
    let r = page.rect("r");
    // 50% of a wide box: each corner is half the width across and half
    // the height down.
    let radius = [Corner {
        x: r.width / 2.0,
        y: 7.0,
    }; 4];
    assert!(items.contains(&DisplayItem::Fill {
        rect: r,
        color: RED,
        radii: radius
    }));
    assert!(items.contains(&DisplayItem::Border {
        rect: r,
        widths: [2.0; 4],
        colors: [BLUE; 4],
        styles: [BorderStyle::Solid; 4],
        radii: radius
    }));
    assert!(items.contains(&DisplayItem::Fill {
        rect: page.rect("k"),
        color: GREEN,
        radii: [Corner::round(5.0); 4]
    }));
    assert!(items.contains(&DisplayItem::Gradient {
        rect: page.rect("g"),
        angle: 180.0,
        stops: vec![(RED, 0.0), (GREEN, 0.5), (BLUE, 0.75), (RED, 1.0)],
        radii: [Corner::SQUARE; 4]
    }));
    assert!(items.contains(&DisplayItem::Gradient {
        rect: page.rect("q"),
        angle: 180.0,
        stops: vec![(RED, 0.0), (BLUE, 1.0)],
        radii: [Corner::SQUARE; 4]
    }));
    // The 40 by 20 image repeats from the padding box's corner.
    let u = page.layout.boxes[&page.id("u")].padding_box();
    assert!(items.contains(&DisplayItem::Image {
        rect: Rect::new(u.x, u.y, 40.0, 20.0),
        url: "bg.png".to_owned()
    }));
    let i = page.layout.boxes[&page.id("i")];
    assert!(items.contains(&DisplayItem::Image {
        rect: i.content_box(),
        url: "a.png".to_owned()
    }));
    let grow = Rect::new(
        i.rect.x - 4.0,
        i.rect.y - 4.0,
        i.rect.width + 8.0,
        i.rect.height + 8.0,
    );
    assert!(items.contains(&DisplayItem::Border {
        rect: grow,
        widths: [3.0; 4],
        colors: [RED; 4],
        styles: [BorderStyle::Dashed; 4],
        radii: [Corner::SQUARE; 4]
    }));
    let images = items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Image { .. }))
        .count();
    // Ten tiles across the 386 pixel wide box, and the image.
    assert_eq!(images, 11);
    let borders = items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Border { .. }))
        .count();
    assert_eq!(borders, 2);
}

#[test]
fn visibility_and_pointer_events() {
    let page = render(
        "<div id=h style='visibility:hidden;height:20px;background:red'>\
           <span id=v style='visibility:visible'>v</span> <span id=x>x</span></div>\
         <div id=n style='pointer-events:none;height:20px;background:blue'>\
           <span id=p style='pointer-events:auto'>p</span> <span id=q>q</span></div>",
        "",
    );
    let items = &page.root().items;
    assert_eq!(fills(items).len(), 2);
    assert_eq!(texts(items), vec!["v", "p", " ", "q"]);
    let x = page.scene.client_rect(page.id("x")).unwrap();
    assert_eq!(page.hit(x.x + 1.0, x.y + 1.0).as_deref(), Some("body"));
    let v = page.scene.client_rect(page.id("v")).unwrap();
    assert_eq!(page.hit(v.x + 1.0, v.y + 1.0).as_deref(), Some("v"));
    let p = page.scene.client_rect(page.id("p")).unwrap();
    assert_eq!(page.hit(p.x + 1.0, p.y + 1.0).as_deref(), Some("p"));
    let q = page.scene.client_rect(page.id("q")).unwrap();
    assert_eq!(page.hit(q.x + 1.0, q.y + 1.0).as_deref(), Some("body"));
}

#[test]
fn inline_rects_cover_their_text() {
    let page = render(
        "<p id=p style='width:40px'><a id=a><b id=b>aa bb</b> <i>cc</i></a></p>\
         <div style='display:none'><span id=gone>x</span></div>\
         <div style='display:contents'><div id=inner>in</div></div>",
        "",
    );
    let a = page.scene.client_rect(page.id("a")).unwrap();
    let b = page.scene.client_rect(page.id("b")).unwrap();
    assert_eq!(b.width, 40.0);
    assert!(a.height > b.height);
    assert_eq!((a.x, a.y), (b.x, b.y));
    assert!(page.scene.client_rect(page.id("p")).is_some());
    assert_eq!(page.scene.client_rect(page.id("gone")), None);
    assert_eq!(
        page.scene.client_rect(page.id("inner")),
        Some(page.rect("inner"))
    );
    assert!(texts(&page.root().items).contains(&"in"));
}

fn control(page: &Page, id: &str) -> Control {
    let node = page.id(id).index();
    page.root()
        .controls
        .iter()
        .find(|control| control.id == node)
        .unwrap()
        .clone()
}

#[test]
fn text_fills_in_its_fill_color() {
    let page = render(
        "<p style='color:red; -webkit-text-fill-color: blue'>filled</p>         <input id=i placeholder=Name style='color:red; -webkit-text-fill-color: green'>",
        "#i::placeholder { -webkit-text-fill-color: blue }",
    );
    let colors: Vec<Color> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { text, color, .. } if text == "filled" => Some(*color),
            _ => None,
        })
        .collect();
    assert_eq!(colors, [BLUE]);
    let input = control(&page, "i");
    assert_eq!(
        (input.state.color, input.state.placeholder_color),
        (GREEN, BLUE)
    );
}

#[test]
fn transparent_toggles_show_the_page_beneath_them() {
    let page = render(
        "<label style='position:relative; display:inline-block; width:52px; height:28px'>         <input id=switch type=checkbox style='position:absolute; inset:0; margin:0; opacity:0'>         <span style='display:block; height:28px; background:blue'></span></label>         <div style='opacity:0'><input id=faded type=radio></div><input id=plain type=checkbox>",
        "",
    );
    assert!(control(&page, "switch").state.bare);
    assert!(control(&page, "faded").state.bare);
    // The page paints toggles as Chromium does, but none made transparent.
    assert!(control(&page, "plain").state.bare);
    let borders = page
        .root()
        .items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Border { .. }))
        .count();
    assert_eq!(borders, 1);
}

#[test]
fn toggles_paint_as_chromium_draws_them_in_their_accent() {
    let page = render(
        "<input id=plain type=checkbox><input id=ticked type=checkbox checked><input id=radio type=radio checked><input id=styled type=checkbox checked><input id=own type=checkbox checked>",
        "body{margin:0} input{margin:0} #ticked{width:20px;height:20px;accent-color:#54abde} #styled{accent-color:currentcolor;color:rgb(0,128,0)} #own{appearance:none}",
    );
    let fills: Vec<_> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { rect, color, .. } => Some((*rect, *color)),
            _ => None,
        })
        .collect();
    let ticked = page.rect("ticked");
    assert!(
        fills.contains(&(ticked, Color::rgb(0x54, 0xAB, 0xDE))),
        "{fills:?}"
    );
    assert!(
        fills
            .iter()
            .any(|(_, color)| *color == Color::rgb(0, 128, 0))
    );
    // The radio button's dot takes the default accent.
    assert!(
        fills
            .iter()
            .any(|(_, color)| *color == Color::rgb(0x00, 0x75, 0xFF))
    );
    let marks: Vec<_> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Path {
                stroke: Some(stroke),
                ..
            } => Some(stroke.color),
            _ => None,
        })
        .collect();
    // The light accent gets a dark mark; the authored box none.
    assert_eq!(
        marks,
        [Color::rgb(0x3B, 0x3B, 0x3B), Color::rgb(255, 255, 255)]
    );
    for id in ["plain", "ticked", "radio", "styled", "own"] {
        assert!(control(&page, id).state.bare, "{id}");
    }
}

#[test]
fn static_generated_boxes_paint_their_backgrounds_in_the_flow() {
    let page = render(
        "<div id=circle></div><p id=next>after</p>",
        "#circle { display: flex; width: 20px; height: 20px; background: blue }          #circle::after { content: ''; width: 8px; height: 8px; background: red }          #next::before { content: 'x'; display: block; background: green }",
    );
    let circle = page.rect("circle");
    let fills = fills(&page.root().items);
    assert!(
        fills.contains(&(Rect::new(circle.x, circle.y, 8.0, 8.0), RED)),
        "{fills:?}"
    );
    // The dot paints over its circle, the block before the paragraph, after
    // the page's background.
    let order: Vec<Color> = fills.iter().map(|(_, color)| *color).collect();
    assert_eq!(order[order.len() - 3..], [BLUE, RED, GREEN]);
    assert_eq!(texts(&page.root().items), ["x", "after"]);
}

#[test]
fn underlines_carry_their_offset() {
    let page = render(
        "<p style='text-decoration: underline; text-underline-offset: 3px'>offset</p><p style='text-decoration: underline'>font</p>",
        "",
    );
    let offsets: Vec<(String, Option<f32>)> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text {
                text,
                underline_offset,
                ..
            } => Some((text.clone(), *underline_offset)),
            _ => None,
        })
        .collect();
    assert_eq!(
        offsets,
        [("offset".to_owned(), Some(3.0)), ("font".to_owned(), None)]
    );
}

#[test]
fn controls_carry_their_state() {
    let page = render(
        "<input id=t value=hello placeholder=Name maxlength=' 8 ' readonly style='color:red'>\
         <input id=w type=password>\
         <textarea id=ta>notes</textarea>\
         <input id=c type=checkbox checked>\
         <input id=r type=radio disabled>\
         <select id=s><option>a</option><option selected> b  c </option></select>\
         <select id=m multiple><option selected>x</option><option>y</option><option selected>z</option></select>\
         <input id=g type=range>\
         <input id=g2 type=range min=10 max=5 value=40>\
         <progress id=pr value=3 max=2></progress>\
         <progress id=p0></progress>\
         <fieldset disabled><button id=b> Go  now </button></fieldset>\
         <input id=hid type=submit style='visibility:hidden'>",
        "",
    );
    let text = control(&page, "t");
    assert_eq!(text.kind, ControlKind::TextBox);
    assert_eq!(text.rect, page.rect("t"));
    assert_eq!(text.state.text, "hello");
    assert_eq!(text.state.placeholder, "Name");
    assert_eq!(text.state.max_length, Some(8));
    assert!(text.state.read_only);
    assert!(!text.state.disabled);
    assert!(text.state.visible);
    assert_eq!(text.state.color, RED);
    assert_eq!(text.state.font.size, 13.333);
    assert_eq!(control(&page, "w").kind, ControlKind::Password);
    assert_eq!(control(&page, "ta").state.text, "notes");
    let checkbox = control(&page, "c");
    assert!(checkbox.state.checked);
    assert!(checkbox.state.text.is_empty());
    assert!(control(&page, "r").state.disabled);
    let select = control(&page, "s");
    assert_eq!(select.kind, ControlKind::ComboBox);
    assert_eq!(select.state.options, vec!["a", "b c"]);
    assert_eq!(select.state.selected, vec![1]);
    let multiple = control(&page, "m");
    assert_eq!(multiple.kind, ControlKind::ListBox);
    assert_eq!(multiple.state.selected, vec![0, 2]);
    assert!(multiple.state.multiple);
    assert!(!select.state.multiple);
    let range = control(&page, "g");
    assert_eq!(
        (range.state.value, range.state.minimum, range.state.maximum),
        (50.0, 0.0, 100.0)
    );
    let clamped = control(&page, "g2");
    assert_eq!(
        (
            clamped.state.value,
            clamped.state.minimum,
            clamped.state.maximum
        ),
        (10.0, 10.0, 10.0)
    );
    let progress = control(&page, "pr");
    assert_eq!((progress.state.value, progress.state.maximum), (2.0, 2.0));
    let empty = control(&page, "p0");
    assert_eq!((empty.state.value, empty.state.maximum), (0.0, 1.0));
    let button = control(&page, "b");
    assert_eq!(button.state.text, "Go now");
    assert!(button.state.disabled);
    let hidden = control(&page, "hid");
    assert!(!hidden.state.visible);
    assert_eq!(hidden.state.text, "Submit");
    assert!(texts(&page.root().items).is_empty());
}

#[test]
fn styled_fields_and_selects() {
    let page = render(
        "<input id=t class=s value=x><textarea id=ta class=s></textarea>\
         <input id=d class=s type=date value=2024-03-15><input id=n type=time>\
         <input id=tiny class=s style='height:4px'><input id=c class=s type=checkbox>\
         <select id=sel class=s><option>one</option></select>\
         <select id=hid class=s style='visibility:hidden'><option>two</option></select>\
         <select id=list class=s size=3><option>three</option></select>",
        ".s { border: 2px solid red; padding: 3px; height: 30px; font-size: 10px; line-height: 10px; \
              box-sizing: border-box; color: blue }",
    );
    // Styled text fields edit inside the box the page draws, in its content
    // box, where the platform centers their line; native ones keep their own
    // frame.
    let text = control(&page, "t");
    assert!(text.state.bare);
    let outer = page.rect("t");
    assert_eq!(
        text.rect,
        Rect::new(outer.x + 5.0, outer.y + 5.0, outer.width - 10.0, 20.0)
    );
    let area = control(&page, "ta");
    assert!(area.state.bare);
    let outer = page.rect("ta");
    assert_eq!(
        area.rect,
        Rect::new(outer.x + 5.0, outer.y + 5.0, outer.width - 10.0, 20.0)
    );
    let date = control(&page, "d");
    assert_eq!(date.kind, ControlKind::Date);
    assert_eq!(date.state.text, "2024-03-15");
    assert!(date.state.bare);
    // A styled date picker gets its line in the middle of its content box.
    let outer = page.rect("d");
    assert_eq!(
        date.rect,
        Rect::new(outer.x + 5.0, outer.y + 10.0, outer.width - 10.0, 10.0)
    );
    let time = control(&page, "n");
    assert_eq!(time.kind, ControlKind::Time);
    assert!(!time.state.bare);
    assert_eq!(time.rect, page.rect("n"));
    // A line taller than the content box fills it.
    let tiny = control(&page, "tiny");
    assert_eq!(tiny.rect.height, 0.0);
    assert!(control(&page, "c").state.bare);
    assert_eq!(control(&page, "c").rect, page.rect("c"));
    // A styled select is drawn by the page, with its arrow.
    assert!(
        page.root()
            .controls
            .iter()
            .all(|control| control.id != page.id("sel").index())
    );
    assert!(texts(&page.root().items).contains(&"one"));
    // The paths inside the select, not the text area's resize grip.
    let select = page.rect("sel");
    let inside = |segments: &[PathSegment]| match segments.first() {
        Some(PathSegment::Move(x, y)) => {
            (select.x..select.x + select.width).contains(x)
                && (select.y..select.y + select.height).contains(y)
        }
        _ => false,
    };
    let arrows: Vec<(Vec<PathSegment>, Color)> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Path {
                segments,
                stroke: Some(stroke),
                ..
            } if inside(segments) => Some((segments.clone(), stroke.color)),
            _ => None,
        })
        .collect();
    // Centered in the arrow's box inside the 2 pixel border.
    let (x, y) = (select.x + select.width - 2.0 - 8.5, select.y + 15.0);
    assert_eq!(
        arrows,
        vec![(
            vec![
                PathSegment::Move(x - 4.0, y - 2.0),
                PathSegment::Line(x, y + 2.0),
                PathSegment::Line(x + 4.0, y - 2.0),
            ],
            BLUE
        )]
    );
    // List boxes stay native.
    assert_eq!(control(&page, "list").kind, ControlKind::ListBox);
}

fn paint_order(page: &Page) -> Vec<Color> {
    fills(&page.root().items)
        .into_iter()
        .map(|(_, color)| color)
        .collect()
}

#[test]
fn stacking_order() {
    let page = render(
        "<div id=c style='position:relative;height:50px;background:white'>\
           <div id=a style='position:absolute;z-index:2;width:30px;height:30px;background:red'></div>\
           <div id=b style='position:absolute;z-index:1;width:30px;height:30px;background:green'></div>\
           <div id=n style='position:absolute;z-index:-1;width:30px;height:30px;background:blue'></div>\
           <div id=f style='height:40px;background:black'></div>\
         </div>",
        "",
    );
    // `c` makes no stacking context, so its negative layer belongs to the
    // root's and paints beneath `c`'s own background.
    assert_eq!(
        paint_order(&page),
        vec![Color::WHITE, BLUE, Color::WHITE, Color::BLACK, GREEN, RED]
    );
    let corner = page.rect("a");
    assert_eq!(
        page.hit(corner.x + 1.0, corner.y + 1.0).as_deref(),
        Some("a")
    );
    let f = page.rect("f");
    assert_eq!(page.hit(f.x + 35.0, f.y + 1.0).as_deref(), Some("f"));
}

#[test]
fn opacity_and_transforms() {
    let page = render(
        "<div id=o style='opacity:0.5;height:10px;background:red'></div>\
         <div id=t style='transform:translate(10px, 50%);height:10px;background:blue'></div>\
         <div id=z style='opacity:0;height:10px'></div>",
        "",
    );
    let items = &page.root().items;
    assert_eq!(items[1], DisplayItem::PushOpacity(0.5));
    assert_eq!(items[3], DisplayItem::PopOpacity);
    let t = page.rect("t");
    let moved = page.scene.client_rect(page.id("t")).unwrap();
    assert_eq!(moved, t.offset(10.0, 5.0));
    assert!(fills(items).contains(&(moved, BLUE)));
    assert_eq!(page.hit(moved.x + 1.0, moved.y + 1.0).as_deref(), Some("t"));
    assert_eq!(items[items.len() - 2], DisplayItem::PushOpacity(0.0));
    let z = page.rect("z");
    assert_eq!(page.hit(z.x + 1.0, z.y + 1.0).as_deref(), Some("z"));
}

#[test]
fn clipping_boxes() {
    let page = render(
        "<div id=c style='overflow:hidden;width:50px;height:20px;border:1px solid red;border-radius:4px'>\
           <div id=w style='width:200px;height:40px'></div></div>\
         <div id=p style='position:relative;overflow:clip;width:50px;height:20px'>\
           <div id=a style='position:absolute;left:30px;width:60px;height:10px;background:red'></div></div>",
        "",
    );
    let items = &page.root().items;
    let c = page.layout.boxes[&page.id("c")];
    let clip = c.padding_box();
    let push = items
        .iter()
        .position(|item| {
            *item
                == DisplayItem::PushClip {
                    rect: clip,
                    radii: [Corner::round(4.0); 4],
                }
        })
        .unwrap();
    assert_eq!(items[push + 1], DisplayItem::PopClip);
    assert_eq!(
        page.hit(clip.x + 100.0, clip.y + 5.0).as_deref(),
        Some("body")
    );
    assert_eq!(page.hit(clip.x + 10.0, clip.y + 5.0).as_deref(), Some("w"));
    let a = page.rect("a");
    let p = page.rect("p");
    let fill = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == RED))
        .unwrap();
    assert_eq!(
        items[fill - 1],
        DisplayItem::PushClip {
            rect: p,
            radii: [Corner::SQUARE; 4]
        }
    );
    assert_eq!(items[fill + 1], DisplayItem::PopClip);
    assert_eq!(page.hit(a.x + 5.0, a.y + 5.0).as_deref(), Some("a"));
    assert_eq!(page.hit(p.x + 55.0, a.y + 5.0).as_deref(), Some("body"));
}

#[test]
fn surfaces_fill_past_short_content() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=s style='overflow:auto;width:100px;height:50px'><div style='height:10px'></div></div>",
    ));
    let page = page_with(dom, "body { background: green }");
    let surface = &page.root().children[0];
    assert_eq!(surface.rect.height, 50.0);
    assert_eq!(
        fills(&surface.items)[..1],
        [(Rect::new(0.0, 0.0, 101.0, 51.0), GREEN)]
    );
}

#[test]
fn scroll_containers_become_surfaces() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=s style='overflow:auto;width:100px;height:50px;border:2px solid black;background:rgba(255,0,0,0.5)'>\
           <div id=a style='height:200px;width:300px'>text</div>\
           <div id=z style='position:relative;z-index:1;height:10px;background:green'></div></div>\
         <div id=y style='overflow-y:hidden;height:20px;background:blue'><div style='height:40px'></div></div>",
    ));
    let s = dom.get_element_by_id(Dom::DOCUMENT, "s").unwrap();
    dom.set_scroll(s, 1000.0, 30.0);
    let page = page_with(dom, "body { background: green }");
    let root = page.root();
    assert_eq!(root.children.len(), 2);
    let surface = &root.children[0];
    let outer = page.layout.boxes[&s];
    let padding = outer.padding_box();
    assert_eq!(surface.id, s.index());
    assert_eq!(surface.rect, padding);
    assert_eq!(surface.content_width, 300.0);
    assert_eq!(surface.content_height, 210.0);
    // The most it scrolls is the content less the room scrollbars leave.
    assert_eq!((surface.scroll_x, surface.scroll_y), (217.0, 30.0));
    // The fills reach a pixel past the content, which the surface's
    // window can round out to.
    assert_eq!(
        fills(&surface.items)[..2],
        [
            (Rect::new(0.0, 0.0, 301.0, 211.0), GREEN),
            (
                Rect::new(0.0, 0.0, 301.0, 211.0),
                Color::rgba(255, 0, 0, 0.5)
            )
        ]
    );
    assert_eq!(texts(&surface.items), vec!["text"]);
    let a = page.scene.client_rect(page.id("a")).unwrap();
    assert_eq!(a.x, padding.x - 217.0);
    assert_eq!(a.y, padding.y - 30.0);
    assert_eq!(
        page.hit(padding.x + 1.0, padding.y + 1.0).as_deref(),
        Some("a")
    );
    assert_eq!(
        page.hit(padding.x + 150.0, padding.y + 1.0).as_deref(),
        Some("body")
    );
    let z = page.scene.client_rect(page.id("z")).unwrap();
    assert_eq!(z.y, padding.y - 30.0 + 200.0);
    // Both scrollbars take 17 pixels of the 100 by 50 box.
    assert_eq!(outer.scrollbars, [17.0, 17.0]);
    assert!(fills(&surface.items).contains(&(Rect::new(0.0, 200.0, 83.0, 10.0), GREEN)));
    assert!(
        root.items
            .iter()
            .any(|item| matches!(item, DisplayItem::Border { rect, .. } if *rect == outer.rect))
    );
    let promoted = &root.children[1];
    assert_eq!(promoted.id, page.id("y").index());
    assert_eq!(fills(&promoted.items)[0].1, BLUE);
}

#[test]
fn viewport_overflow_and_scrolling() {
    let tall = "<div id=t style='height:1000px;width:800px'></div>\
                <div id=f style='position:fixed;top:0;left:0;width:10px;height:10px;background:red'></div>";
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(tall));
    let html = dom.document_element().unwrap();
    dom.set_scroll(html, 50.0, 5000.0);
    let page = page_with(dom, "");
    let root = page.root();
    assert_eq!(root.content_width, 808.0);
    assert_eq!(root.content_height, 1016.0);
    assert_eq!((root.scroll_x, root.scroll_y), (50.0, 716.0));
    let fixed = page.scene.client_rect(page.id("f")).unwrap();
    assert_eq!(fixed, Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(fills(&root.items).contains(&(Rect::new(50.0, 716.0, 10.0, 10.0), RED)));
    assert_eq!(page.hit(5.0, 5.0).as_deref(), Some("f"));
    assert_eq!(page.hit(50.0, 50.0).as_deref(), Some("t"));
    let t = page.scene.client_rect(page.id("t")).unwrap();
    assert_eq!((t.x, t.y), (8.0 - 50.0, 8.0 - 716.0));
    let hidden = page_with(
        Dom::from_document(&nativeboat_html::parse_document(tall)),
        "body { overflow: hidden }",
    );
    let root = hidden.root();
    assert_eq!((root.content_width, root.content_height), (400.0, 300.0));
    assert!(root.children.is_empty());
    let clipped = page_with(
        Dom::from_document(&nativeboat_html::parse_document(tall)),
        "html { overflow: clip } body { overflow: auto }",
    );
    let root = clipped.root();
    assert_eq!((root.content_width, root.content_height), (400.0, 300.0));
    assert_eq!(root.children.len(), 1);
}

#[test]
fn helpers() {
    assert_eq!(
        intersect(None, Rect::new(1.0, 2.0, 3.0, 4.0)),
        Rect::new(1.0, 2.0, 3.0, 4.0)
    );
    assert_eq!(
        intersect(
            Some(Rect::new(0.0, 0.0, 10.0, 10.0)),
            Rect::new(5.0, 5.0, 10.0, 10.0)
        ),
        Rect::new(5.0, 5.0, 5.0, 5.0)
    );
    assert_eq!(
        intersect(
            Some(Rect::new(0.0, 0.0, 1.0, 1.0)),
            Rect::new(5.0, 5.0, 1.0, 1.0)
        ),
        Rect::new(5.0, 5.0, 0.0, 0.0)
    );
    assert_eq!(
        union(Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(5.0, 5.0, 1.0, 1.0)),
        Rect::new(0.0, 0.0, 6.0, 6.0)
    );
    let stop = |position: Option<f32>| ColorStop {
        color: RED,
        position,
    };
    let positions = |stops: &[ColorStop]| -> Vec<f32> {
        gradient_stops(stops)
            .into_iter()
            .map(|(_, position)| position)
            .collect()
    };
    assert!(gradient_stops(&[]).is_empty());
    assert_eq!(positions(&[stop(None)]), vec![0.0]);
    assert_eq!(
        positions(&[stop(Some(-0.2909)), stop(Some(0.5177)), stop(Some(1.2935))]),
        vec![-0.2909, 0.5177, 1.2935]
    );
    assert_eq!(
        positions(&[stop(Some(-3.0)), stop(None), stop(Some(-1.0))]),
        vec![-3.0, -2.0, -1.0]
    );
    assert_eq!(
        positions(&[stop(Some(2.0)), stop(None), stop(Some(4.0))]),
        vec![2.0, 3.0, 4.0]
    );
    assert_eq!(
        positions(&[stop(Some(0.5)), stop(None), stop(Some(0.2)), stop(None)]),
        vec![0.5, 0.5, 0.5, 1.0]
    );
    assert_eq!(overflow_of(&ComputedStyle::default()), Overflowing::Visible);
}

#[test]
fn drawn_canvases_show_their_bitmaps() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<canvas id=c width=30 height=20></canvas><canvas id=blank></canvas>",
    ));
    let canvas = dom.get_element_by_id(Dom::DOCUMENT, "c").unwrap();
    dom.canvas_drawn(canvas, 42);
    let page = page_with(dom, "");
    let canvases: Vec<(Rect, u64, u64)> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Canvas { rect, key, version } => Some((*rect, *key, *version)),
            _ => None,
        })
        .collect();
    assert_eq!(canvases.len(), 1, "only drawn canvases");
    let (rect, key, version) = canvases[0];
    assert_eq!((key, version), (42, 1));
    assert_eq!((rect.width, rect.height), (30.0, 20.0));
}

#[test]
fn shadows_and_transforms_become_display_items() {
    let page = render(
        "<div id=s style='width: 20px; height: 10px; border: 2px solid black; border-radius: 4px; box-shadow: 2px 3px 4px 1px red, inset 1px 1px 2px 1px blue'></div>\
         <div id=t style='width: 20px; height: 10px; transform: rotate(90deg) scale(2)'>x</div>",
        "",
    );
    let shadowed = page.rect("s");
    let shadows: Vec<&DisplayItem> = page
        .root()
        .items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Shadow { .. }))
        .collect();
    assert_eq!(shadows.len(), 2);
    let DisplayItem::Shadow {
        rect,
        radii,
        blur,
        inset: false,
        clip,
        ..
    } = shadows[0]
    else {
        panic!("{shadows:?}");
    };
    assert_eq!(
        *rect,
        Rect::new(
            shadowed.x + 1.0,
            shadowed.y + 2.0,
            shadowed.width + 2.0,
            shadowed.height + 2.0
        )
    );
    assert_eq!(
        (*radii, *blur, *clip),
        ([Corner::round(5.0); 4], 4.0, shadowed)
    );
    let DisplayItem::Shadow {
        rect,
        radii,
        inset: true,
        clip,
        clip_radii,
        ..
    } = shadows[1]
    else {
        panic!("{shadows:?}");
    };
    let padding = Rect::new(shadowed.x + 2.0, shadowed.y + 2.0, 20.0, 10.0);
    assert_eq!(*clip, padding);
    assert_eq!(
        *rect,
        Rect::new(padding.x + 2.0, padding.y + 2.0, 18.0, 8.0)
    );
    assert_eq!(
        (*clip_radii, *radii),
        ([Corner::round(2.0); 4], [Corner::round(1.0); 4])
    );
    let transformed = page.rect("t");
    let (cx, cy) = (
        transformed.x + transformed.width / 2.0,
        transformed.y + transformed.height / 2.0,
    );
    let transform = page
        .root()
        .items
        .iter()
        .find_map(|item| match item {
            DisplayItem::PushTransform(transform) => Some(*transform),
            _ => None,
        })
        .unwrap();
    let expected = [0.0, 2.0, -2.0, 0.0, cx + 2.0 * cy, cy - 2.0 * cx];
    for (value, wanted) in transform.iter().zip(expected) {
        assert!(
            (value - wanted).abs() < 1e-3,
            "{transform:?} != {expected:?}"
        );
    }
    assert!(page.root().items.contains(&DisplayItem::PopTransform));
}

#[test]
fn elliptical_corners_keep_both_radii() {
    let page = render(
        "<div id=e></div><div id=p></div><div id=b></div>",
        "body{margin:0} div{width:60px;height:40px;background:red}
         #e{border-radius:20px / 10px} #p{border-radius:50%}
         #b{border-radius:20px;border-style:solid;border-width:2px 0 0 10px;
            box-shadow:inset 0 0 4px blue}",
    );
    let fills: Vec<Radii> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { color, radii, .. } if *color == RED => Some(*radii),
            _ => None,
        })
        .collect();
    let corner = |x, y| Corner { x, y };
    assert_eq!(fills[0], [corner(20.0, 10.0); 4]);
    assert_eq!(fills[1], [corner(30.0, 20.0); 4]);
    // An inset shadow's corners are the border's radii less the widths
    // beside them: across by the left or right width, down by the top or
    // bottom one.
    let inner = page.root().items.iter().find_map(|item| match item {
        DisplayItem::Shadow {
            inset: true,
            clip_radii,
            ..
        } => Some(*clip_radii),
        _ => None,
    });
    assert_eq!(
        inner,
        Some([
            corner(10.0, 18.0),
            corner(20.0, 18.0),
            corner(20.0, 20.0),
            corner(10.0, 20.0)
        ])
    );
}

#[test]
fn shadow_spread_preserves_sharp_corners_and_adjusts_small_radii() {
    assert_eq!(spread_radius(0.0, 20.0), 0.0);
    assert_eq!(spread_radius(10.0, 20.0), 27.5);
    assert_eq!(spread_radius(20.0, 20.0), 40.0);
    assert_eq!(spread_radius(20.0, -30.0), 0.0);
    assert_eq!(spread_radius(10.0, -2.0), 8.0);
    let page = render(
        "<div id=s style='width:100px;height:100px;border-radius:10px;box-shadow:0 0 0 20px red,inset 0 0 1px -20px blue'></div>",
        "",
    );
    let radii: Vec<_> = page
        .root()
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Shadow { radii, .. } => Some(*radii),
            _ => None,
        })
        .collect();
    assert_eq!(radii, vec![[Corner::round(27.5); 4]; 2]);
}

#[test]
fn shadows_that_cannot_show_are_left_out() {
    // Tailwind's rings of zero width are sharp shadows the size of the box,
    // under transparent ones: none of them shows.
    let shadows = |shadow: &str| {
        let page = render(
            &format!(
                "<div style='margin-left:0.4px;width:50.3px;height:20px;box-shadow:{shadow}'></div>"
            ),
            "",
        );
        page.root()
            .items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Shadow { .. }))
            .count()
    };
    assert_eq!(shadows("0 0 0 0 rgb(7, 10, 14)"), 0);
    assert_eq!(shadows("0 0 0 0 transparent, 0 0 4px red"), 1);
    assert_eq!(shadows("1px 1px 0 -1px red, 0 0 0 -2px red"), 0);
    assert_eq!(shadows("inset 0 0 0 0 red, inset 0 0 0 -3px red"), 0);
    // Offsets, spreads and blurs that reach past the box show.
    assert_eq!(
        shadows("1px 0 0 0 red, 0 0 0 1px red, 0 0 1px red, inset 0 0 0 1px red"),
        4
    );
}

#[test]
fn empty_positioned_quote_rules_and_dividers_have_native_geometry_and_paint() {
    let page = render(
        "<blockquote id=quote>quote</blockquote><div id=divider></div>",
        "body{margin:0} #quote{position:relative;margin:0;width:200px;height:40px;padding:0 30px} #quote::before{content:'';position:absolute;display:block;left:15px;top:0;width:2px;height:100%;background:gray} #divider{position:relative;width:100px;height:20px} #divider::before{content:'';position:absolute;display:block;left:0;top:50%;width:100%;height:2px;border-top:2px dashed blue;transform:translateY(-50%)}",
    );
    let quote = page.id("quote");
    let rule = &page.layout.generated[&quote][0];
    assert_eq!(rule.pseudo, PseudoElement::Before);
    assert_eq!(rule.geometry.rect, Rect::new(15.0, 0.0, 2.0, 40.0));
    assert_eq!(page.rect("quote"), Rect::new(0.0, 0.0, 260.0, 40.0));
    assert_eq!(page.scene.client_rect(quote), Some(page.rect("quote")));
    assert!(fills(&page.root().items).contains(&(rule.geometry.rect, Color::rgb(128, 128, 128))));
    assert_eq!(page.hit(16.0, 10.0).as_deref(), Some("quote"));
    assert!(page.root().items.iter().any(|item| matches!(item, DisplayItem::Border { widths, styles, .. } if widths[0] == 2.0 && styles[0] == BorderStyle::Dashed)));
    assert!(page.root().items.iter().any(|item| matches!(item,
        DisplayItem::Border { rect, widths, .. }
            if *rect == Rect::new(0.0, 48.0, 100.0, 4.0) && widths[0] == 2.0
    )));
    assert_eq!(texts(&page.root().items), ["quote"]);
}

#[test]
fn generated_checkbox_shaped_boxes_keep_owner_bounds_and_css_stacking() {
    let page = render(
        "<span id=box></span>",
        "body{margin:0} #box{position:absolute;left:30px;top:30px;width:12px;height:12px} #box::before{content:'';position:absolute;left:-2px;top:-2px;width:18px;height:18px;box-sizing:border-box;border:2px solid gray;background:red;border-radius:50%} #box::after{content:'';position:absolute;left:4px;top:1px;width:8px;height:4px;border:2px solid white;border-top:none;border-right:none;transform:rotate(-45deg)}",
    );
    let boxes = &page.layout.generated[&page.id("box")];
    assert_eq!(boxes.len(), 2);
    assert_eq!(boxes[0].geometry.rect, Rect::new(28.0, 28.0, 18.0, 18.0));
    assert_eq!(
        page.scene.client_rect(page.id("box")),
        Some(Rect::new(30.0, 30.0, 12.0, 12.0))
    );
    assert_eq!(page.hit(29.0, 35.0).as_deref(), Some("box"));
    assert!(page.root().items.iter().any(|item| matches!(item, DisplayItem::Fill { rect, radii, color, .. } if *rect == boxes[0].geometry.rect && *radii == [Corner::round(9.0); 4] && *color == Color::rgb(255,0,0))));
    assert!(page.root().controls.is_empty());
    assert!(page.layout.runs.is_empty());
}

#[test]
fn styled_native_toggles_keep_their_role_bounds_and_positioned_decorations() {
    for (kind, input) in [
        (ControlKind::CheckBox, "checkbox"),
        (ControlKind::Radio, "radio"),
    ] {
        let page = render(
            &format!("<input id=box type={input} checked>"),
            "body{margin:0} #box{appearance:none;position:absolute;left:30px;top:30px;width:12px;height:12px;margin:0} #box::before{content:'';position:absolute;left:-2px;top:-2px;width:18px;height:18px;box-sizing:border-box;border:2px solid gray;background:red;border-radius:50%} #box::after{content:'';position:absolute;left:4px;top:1px;width:8px;height:4px;border:2px solid white;border-top:none;border-right:none;transform:rotate(-45deg)}",
        );
        let toggle = control(&page, "box");
        assert_eq!(toggle.kind, kind);
        assert!(toggle.state.checked);
        assert!(toggle.state.bare);
        assert_eq!(toggle.rect, Rect::new(30.0, 30.0, 12.0, 12.0));
        let boxes = &page.layout.generated[&page.id("box")];
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].geometry.rect, Rect::new(28.0, 28.0, 18.0, 18.0));
        assert_eq!(page.scene.client_rect(page.id("box")), Some(toggle.rect));
        assert_eq!(page.hit(29.0, 35.0).as_deref(), Some("box"));
        assert!(page.root().items.iter().any(|item| matches!(item, DisplayItem::Fill { rect, radii, color, .. } if *rect == boxes[0].geometry.rect && *radii == [Corner::round(9.0); 4] && *color == Color::rgb(255,0,0))));
        assert_eq!(page.root().controls.len(), 1);
        assert!(page.layout.runs.is_empty());
    }
}

#[test]
fn styled_toggle_decorations_use_containing_blocks_without_growing_intrinsic_boxes() {
    let page = render(
        "<div id=parent><input id=static type=checkbox><input id=relative type=radio><input id=native type=checkbox><input id=field></div>",
        "body{margin:0} #parent{position:absolute;left:40px;top:30px;width:200px;height:100px} #static,#relative{appearance:none;margin:0} #relative{position:relative} input::before{content:'';position:absolute;left:5px;top:7px;width:80px;height:60px;background:red} #relative::after{content:'';position:fixed;left:2px;top:3px;width:6px;height:9px;background:blue}",
    );
    for id in ["static", "relative"] {
        let toggle = control(&page, id);
        assert!(toggle.state.bare);
        assert_eq!((toggle.rect.width, toggle.rect.height), (13.0, 13.0));
        assert_eq!(page.scene.client_rect(page.id(id)), Some(toggle.rect));
    }
    assert_eq!(
        page.layout.generated[&page.id("static")][0].geometry.rect,
        Rect::new(45.0, 37.0, 80.0, 60.0)
    );
    let owner = page.rect("relative");
    let generated = |pseudo| {
        page.layout.generated[&page.id("relative")]
            .iter()
            .find(|value| value.pseudo == pseudo)
            .unwrap()
            .geometry
            .rect
    };
    assert_eq!(
        generated(PseudoElement::Before),
        Rect::new(owner.x + 5.0, owner.y + 7.0, 80.0, 60.0)
    );
    assert_eq!(
        generated(PseudoElement::After),
        Rect::new(2.0, 3.0, 6.0, 9.0)
    );
    assert!(!control(&page, "field").state.bare);
    for id in ["native", "field"] {
        assert!(!page.layout.generated.contains_key(&page.id(id)));
    }
}

#[test]
fn generated_box_filtering_fixed_canvas_and_boxless_origins() {
    let page = render(
        "<span id=inline>inline</span><div id=none></div><div id=hidden></div><div id=normal></div><div id=text></div><div id=invisible></div>",
        "body{margin:0} html{background:blue} html::before{content:'';position:fixed;left:3px;top:5px;width:10px;height:10px;background:red} #inline::before{content:'';position:absolute;left:90px;top:80px;width:3px;height:4px;background:green} #none::before{content:none;position:absolute} #hidden::before{content:'HIDDEN';display:none;position:absolute} #normal::before{position:absolute;color:blue} #text::before{content:'N';position:absolute} #invisible::after{content:'';position:absolute;width:5px;height:5px;background:red;visibility:hidden;pointer-events:none}",
    );
    assert!(!page.layout.boxes.contains_key(&page.id("inline")));
    assert!(page.layout.generated.contains_key(&page.id("inline")));
    for id in ["none", "hidden", "normal"] {
        assert!(!page.layout.generated.contains_key(&page.id(id)), "{id}");
    }
    // Positioned generated text has a box, which paints its text.
    assert!(page.layout.generated.contains_key(&page.id("text")));
    assert!(texts(&page.root().items).contains(&"N"));
    assert!(!texts(&page.root().items).contains(&"HIDDEN"));
    assert!(
        fills(&page.root().items)
            .contains(&(Rect::new(3.0, 5.0, 10.0, 10.0), Color::rgb(255, 0, 0)))
    );
    assert!(
        fills(&page.root().items)
            .contains(&(Rect::new(90.0, 80.0, 3.0, 4.0), Color::rgb(0, 128, 0)))
    );
    assert_eq!(page.hit(91.0, 81.0).as_deref(), Some("inline"));
    assert!(
        !fills(&page.root().items)
            .iter()
            .any(|(rect, _)| rect.width == 5.0 && rect.height == 5.0)
    );

    // A caller may pair an old layout with new styles; missing pseudo styles
    // must omit that native decoration without changing the originating box.
    let styles = Styles::compute(&page.dom, &[], &StyleEnvironment::default());
    let scene = scene(&page.dom, &styles, &page.layout, (400.0, 300.0));
    assert!(
        !fills(&scene.tree.root.items)
            .iter()
            .any(|(_, color)| *color == Color::rgb(255, 0, 0))
    );
}

#[test]
fn inline_boxes_paint_under_their_text_and_slice_across_lines() {
    let page = render(
        "<p style='width:60px'>a <span id=s>bbbb cccc dddd</span> e</p>",
        "body { margin: 0 } p { margin: 0; font-size: 10px; line-height: 20px } \
         span { background: red; border-left: 2px solid blue; border-right: 3px solid blue; padding: 0 4px }",
    );
    let items = &page.scene.tree.root.items;
    let red: Vec<Rect> = fills(items)
        .into_iter()
        .filter(|(_, color)| *color == RED)
        .map(|(rect, _)| rect)
        .collect();
    assert!(red.len() >= 2, "{items:?}");
    // The background is painted before the text it covers.
    let first_fill = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == RED))
        .unwrap();
    let text = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Text { text, .. } if text.starts_with("bbbb")))
        .unwrap();
    assert!(first_fill < text);
    // The left border shows on the first line only, the right on the last.
    let borders: Vec<[f32; 4]> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Border { widths, .. } => Some(*widths),
            _ => None,
        })
        .collect();
    assert_eq!(borders.first().unwrap()[3], 2.0);
    assert_eq!(borders.first().unwrap()[1], 0.0);
    assert_eq!(borders.last().unwrap()[1], 3.0);
    assert_eq!(borders.last().unwrap()[3], 0.0);
}

#[test]
fn symbol_markers_paint_as_shapes_sized_from_the_font() {
    let page = render(
        "<ul><li id=disc>a</li><li id=circle style='list-style-type:circle'>b</li><li id=square style='list-style-type:square'>c</li><li id=none style='list-style-type:decimal'>d</li></ul>",
        "body{margin:0;font-size:20px;color:rgb(0,0,255)}",
    );
    let items = &page.root().items;
    // Only the number is text; the symbols are shapes.
    assert!(!texts(items).iter().any(|text| text.contains('\u{2022}')));
    assert!(texts(items).iter().any(|text| text.trim_end() == "4."));
    let shapes: Vec<(Rect, Radii, bool)> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { rect, color, radii } if *color == BLUE => {
                Some((*rect, *radii, true))
            }
            DisplayItem::Border { rect, radii, .. } => Some((*rect, *radii, false)),
            _ => None,
        })
        .collect();
    assert_eq!(shapes.len(), 3);
    let (disc, circle, square) = (shapes[0], shapes[1], shapes[2]);
    // A disc and a filled square, an outlined circle, all the same size and
    // left of their items' text.
    assert!(disc.2 && !circle.2 && square.2);
    assert!(disc.0.width > 2.0 && disc.0.width == disc.0.height);
    assert_eq!(disc.1, [Corner::round(disc.0.width / 2.0); 4]);
    assert_eq!(circle.1, [Corner::round(circle.0.width / 2.0); 4]);
    assert_eq!(square.1, [Corner::SQUARE; 4]);
    assert_eq!(
        (circle.0.width, square.0.width),
        (disc.0.width, disc.0.width)
    );
    for (shape, id) in [(disc.0, "disc"), (circle.0, "circle"), (square.0, "square")] {
        let item = page.rect(id);
        assert!(shape.x + shape.width < item.x, "{id}");
        assert!(
            shape.y > item.y && shape.y + shape.height < item.y + item.height,
            "{id}"
        );
    }
}

#[test]
fn marker_rules_color_and_size_list_markers() {
    // As MarkText's themes color list numbers and bullets; the item's text
    // keeps its own color.
    let page = render(
        "<ol><li>one</li></ol><ul><li>two</li></ul><ol><li id=plain>three</li></ol>",
        "body{margin:0;color:rgb(0,0,255)} li::marker{color:rgb(255,0,0)} \
         ol:first-child > li::marker{font-size:30px} #plain::marker{color:inherit}",
    );
    let items = &page.root().items;
    let text_color = |wanted: &str| {
        items.iter().find_map(|item| match item {
            DisplayItem::Text {
                text, color, font, ..
            } if text.trim_end() == wanted => Some((*color, font.size)),
            _ => None,
        })
    };
    assert_eq!(text_color("1."), Some((Color::rgb(255, 0, 0), 30.0)));
    assert_eq!(text_color("one").map(|(color, _)| color), Some(BLUE));
    assert_eq!(text_color("1.").map(|(_, size)| size), Some(30.0));
    // The bullet is a red shape.
    assert!(items.iter().any(|item| matches!(
        item,
        DisplayItem::Fill { color, .. } if *color == Color::rgb(255, 0, 0)
    )));
    // `inherit` takes the item's color.
    let numbers: Vec<Color> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { text, color, .. } if text.trim_end() == "1." => Some(*color),
            _ => None,
        })
        .collect();
    assert_eq!(numbers, [Color::rgb(255, 0, 0), BLUE]);
}

#[test]
fn words_that_cannot_break_before_themselves_take_earlier_text_along() {
    // Muya's inline code between hidden zero-size backticks: no line may
    // break between the code's last word and the comma after it, so when
    // the comma overflows they move down together, from the space inside
    // the code, as in Chromium.
    let placed = |width: u32| {
        let page = render(
            &format!(
                "<p style='width:{width}px'><span>some strong text, a </span><span class=h>`</span>\
                 <code>code span</code><span class=h>`</span><span>, and a </span></p>"
            ),
            "body{margin:0;font-size:16px} .h{font-size:0;white-space:nowrap} \
             code{padding:0.2em 0.4em;font-size:0.8em}",
        );
        page.root()
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, rect, .. } => Some((text.trim().to_owned(), *rect)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let at = |items: &[(String, Rect)], wanted: &str| {
        items
            .iter()
            .find(|(text, _)| text == wanted)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{wanted} in {items:?}"))
    };
    let narrow = placed(230);
    let (code, span, comma) = (
        at(&narrow, "code"),
        at(&narrow, "span"),
        at(&narrow, ", and a"),
    );
    assert!(span.y > code.y + 10.0, "{narrow:?}");
    assert!(
        (comma.y - span.y).abs() < 5.0 && comma.x > span.x,
        "{narrow:?}"
    );
    // With room for the comma, nothing moves.
    let wide = placed(260);
    assert!(at(&wide, ",").y < 25.0, "{wide:?}");
    // A word overflowing a line with no earlier opportunity stays on it.
    let page = render("<p style='width:10px'>unbreakable</p>", "body{margin:0}");
    assert_eq!(texts(&page.root().items), ["unbreakable"]);
}

#[test]
fn scrollers_in_tinted_blocks_show_the_tint() {
    // As a code block: a translucent gray around a scroller, over white.
    let page = render(
        "<pre id=p><code id=c>text</code></pre>",
        "body{background:white} #p{background:rgba(0,0,0,0.1);padding:10px} #c{display:block;overflow:auto}",
    );
    let surface = &page.root().children[0];
    assert_eq!(surface.id, page.id("c").index());
    assert_eq!(fills(&surface.items)[0].1, Color::rgb(230, 230, 230));
}

#[test]
fn foreign_object_html_is_painted_over_the_svg() {
    let page = render(
        "<svg viewBox='0 0 100 50'><rect width='100' height='50' fill='blue'/>\
         <foreignObject x='10' y='5' width='60' height='20'><div>inside</div></foreignObject></svg>",
        "body{margin:0}",
    );
    let items = &page.root().items;
    assert!(texts(items).contains(&"inside"));
    // The label comes after the shapes, so it shows on top.
    let path = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Path { .. }))
        .unwrap();
    let text = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Text { .. }))
        .unwrap();
    assert!(path < text);
}

#[test]
fn disclosure_markers_paint_as_triangles() {
    let page = render(
        "<details><summary>closed</summary></details><details open><summary>open</summary>body</details>",
        "body{margin:0;font-size:20px;color:rgb(0,0,255)}",
    );
    let items = &page.root().items;
    let triangles: Vec<&Vec<PathSegment>> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Path {
                segments,
                fill: Some(color),
                ..
            } if *color == BLUE => Some(segments),
            _ => None,
        })
        .collect();
    assert_eq!(triangles.len(), 2);
    // Closed points to the end of the line, open points down.
    let points = |segments: &Vec<PathSegment>| -> Vec<(f32, f32)> {
        segments
            .iter()
            .filter_map(|segment| match segment {
                PathSegment::Move(x, y) | PathSegment::Line(x, y) => Some((*x, *y)),
                _ => None,
            })
            .collect()
    };
    let closed = points(triangles[0]);
    assert!(closed[1].0 > closed[0].0 && closed[0].0 == closed[2].0);
    let open = points(triangles[1]);
    assert!(open[2].1 > open[0].1 && open[0].1 == open[1].1);
    assert!(!texts(items).iter().any(|text| text.contains('\u{25B8}')));
}

#[test]
fn boxes_positioned_against_relative_inlines_start_at_the_inline() {
    let page = render(
        "<p>ab<span class=e data-emoji='E'>smile</span></p><p>cd<span class=r>xy<b class=a>Z</b></span></p>",
        "body{margin:0} .e{position:relative;margin-left:16px}          .e::before{position:absolute;top:0;left:-16px;content:attr(data-emoji)}          .r{position:relative} .a{position:absolute;left:4px;top:2px}",
    );
    let text = |wanted: &str| {
        page.scene
            .tree
            .root
            .items
            .iter()
            .find_map(|item| match item {
                DisplayItem::Text { text, rect, .. } if text == wanted => Some(*rect),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{wanted} is not painted"))
    };
    let (smile, emoji) = (text("smile"), text("E"));
    // The generated text starts 16px before the span, on its line.
    assert_eq!(emoji.x, smile.x - 16.0);
    assert!((emoji.y - smile.y).abs() < 4.0, "{emoji:?} {smile:?}");
    // An absolutely positioned element is placed against the span too.
    let (inline, absolute) = (text("xy"), text("Z"));
    assert_eq!(absolute.x, inline.x + 4.0);
    assert!(
        absolute.y > inline.y - 4.0 && absolute.y < inline.y + 8.0,
        "{absolute:?} {inline:?}"
    );
}

#[test]
fn icons_shifted_into_their_shadows_keep_their_filter_and_clip() {
    let page = render(
        "<div class=w><i class=icon><i class=icon-inner style='background: url(data:image/png;base64,AAAA) no-repeat; background-size: 100%'></i></i></div>",
        "body{margin:0} .w{display:flex;width:16px;height:16px} .icon{position:relative;display:inline-block;width:100%;height:100%;overflow:hidden;color:red} .icon > i[class^='icon-']{position:relative;left:-16px;display:inline-block;width:100%;height:100%;filter:drop-shadow(16px 0 currentcolor)}",
    );
    let items = &page.scene.tree.root.items;
    let filter = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushFilter(_)))
        .unwrap();
    assert!(matches!(items[filter - 1], DisplayItem::PushClip { .. }));
    let image = items[filter..]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!(image.x, -16.0);
}

#[test]
fn absolute_boxes_before_blocks_paint_above_them() {
    // CodeMirror's line numbers precede their lines and show over them.
    let page = render(
        "<div id=line><div id=gutter></div><pre id=text>a</pre></div>",
        "body{margin:0} #line{position:relative} #gutter{position:absolute;width:10px;height:10px;background:red} pre{margin:0;background:blue}",
    );
    let colors: Vec<Color> = fills(&page.root().items)
        .into_iter()
        .map(|(_, color)| color)
        .collect();
    assert_eq!(colors[1..], [BLUE, RED]);
    assert_eq!(page.layout.boxes[&page.id("gutter")].rect.y, 0.0);
}

#[test]
fn translations_change_what_scrolls() {
    // Muya's math preview: centered by a translation, it overflows nothing.
    let page = render(
        "<div id=scroller><div id=figure><div id=preview>x</div></div><div id=pushed></div></div>",
        "body{margin:0} #scroller{overflow:auto;width:200px;height:100px}\
         #figure{position:relative;height:20px} #preview{position:absolute;top:30px;left:50%;width:100%;height:10px;transform:translateX(-50%)}\
         #pushed{position:relative;width:10px;height:10px;transform:translate(250px,150px)}",
    );
    let scroller = &page.layout.boxes[&page.id("scroller")];
    assert_eq!(scroller.content_width, 260.0);
    assert_eq!(scroller.content_height, 180.0);
    let page = render(
        "<div id=scroller><div id=preview></div></div>",
        "body{margin:0} #scroller{overflow:auto;width:200px;height:100px;position:relative}\
         #preview{position:absolute;left:50%;width:100%;height:10px;transform:translateX(-50%)}",
    );
    assert_eq!(page.layout.boxes[&page.id("scroller")].content_width, 200.0);
    // A generated rule centered the same way, as a theme's thematic break.
    let page = render(
        "<div id=scroller><p id=rule></p></div>",
        "body{margin:0} #scroller{overflow:auto;width:200px;height:100px} #rule{position:relative;height:10px}\
         #rule::before{content:'';position:absolute;left:50%;width:100%;height:2px;transform:translateX(-50%)}",
    );
    assert_eq!(page.layout.boxes[&page.id("scroller")].content_width, 200.0);
}

#[test]
fn positive_layers_leave_auto_positioned_parents_for_the_enclosing_context() {
    // Muya's math preview floats over the headings after its block.
    let page = render(
        "<div id=figure style='position:relative;height:10px'>\
           <div id=preview style='position:absolute;z-index:10;top:10px;width:30px;height:30px;background:red'></div>\
         </div>\
         <h3 id=heading style='position:relative;margin:0;height:30px;background:blue'></h3>",
        "body{margin:0}",
    );
    assert_eq!(paint_order(&page)[1..], [BLUE, RED]);
    let preview = page.rect("preview");
    assert_eq!(
        page.hit(preview.x + 1.0, preview.y + 1.0).as_deref(),
        Some("preview")
    );
    // A clip keeps its descendants, which it cuts.
    let page = render(
        "<div style='position:relative;overflow:hidden;height:10px'>\
           <div style='position:absolute;z-index:10;width:30px;height:30px;background:red'></div>\
         </div>\
         <h3 style='position:relative;margin:0;height:30px;background:blue'></h3>",
        "body{margin:0}",
    );
    assert_eq!(paint_order(&page)[1..], [RED, BLUE]);
}

#[test]
fn positive_layers_of_the_root_context_paint() {
    let page = render(
        "<input id=field><span id=option>Case</span>",
        "body{margin:0}input{position:absolute;left:20px;top:30px;width:200px;height:30px}         span{position:absolute;left:180px;top:30px;width:20px;height:30px;z-index:1}",
    );
    assert!(page.scene.client_rect(page.id("option")).is_some());
    assert_eq!(page.hit(185.0, 40.0).as_deref(), Some("option"));
}

fn images(items: &[DisplayItem]) -> Vec<Rect> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Image { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect()
}

#[test]
fn image_elements_fit_their_pictures_and_clip_them_to_their_content_edge() {
    // The test images are 40 by 20; each box is 80 by 80 unless styled.
    let painted = |style: &str| {
        let page = render(
            &format!("<img src=i.png style='width:80px;height:80px;{style}'>"),
            "body{margin:0}img{display:block}",
        );
        page.root()
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::PushClip { rect, radii } => Some(("clip", *rect, *radii)),
                DisplayItem::Image { rect, .. } => Some(("image", *rect, [Corner::SQUARE; 4])),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let image =
        |x, y, width, height| vec![("image", Rect::new(x, y, width, height), [Corner::SQUARE; 4])];
    let clipped = |x, y, width, height, radii| {
        let clip = ("clip", Rect::new(0.0, 0.0, 80.0, 80.0), radii);
        vec![
            clip,
            ("image", Rect::new(x, y, width, height), [Corner::SQUARE; 4]),
        ]
    };
    assert_eq!(painted(""), image(0.0, 0.0, 80.0, 80.0));
    assert_eq!(painted("object-fit:contain"), image(0.0, 20.0, 80.0, 40.0));
    assert_eq!(
        painted("object-fit:cover"),
        clipped(-40.0, 0.0, 160.0, 80.0, [Corner::SQUARE; 4])
    );
    assert_eq!(painted("object-fit:none"), image(20.0, 30.0, 40.0, 20.0));
    assert_eq!(
        painted("object-fit:none;object-position:right 10px top"),
        image(30.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        painted("object-fit:none;object-position:-10px 100%"),
        clipped(-10.0, 60.0, 40.0, 20.0, [Corner::SQUARE; 4])
    );
    assert_eq!(
        painted("object-fit:scale-down"),
        image(20.0, 30.0, 40.0, 20.0)
    );
    assert_eq!(
        painted("object-fit:scale-down;width:30px;height:30px"),
        image(0.0, 7.5, 30.0, 15.0)
    );
    // Nothing fits into an empty box.
    assert_eq!(painted("object-fit:contain;width:0"), Vec::new());
    // Rounded corners clip the picture along the content edge, inside the
    // border and padding.
    assert_eq!(
        painted("border-radius:50%"),
        clipped(0.0, 0.0, 80.0, 80.0, [Corner::round(40.0); 4])
    );
    let inset = painted(
        "box-sizing:border-box;width:100px;height:100px;border:4px solid;padding:6px 6px 6px 16px;border-radius:20px 50px 10px 0",
    );
    assert_eq!(
        inset[0].2,
        [
            Corner::SQUARE,
            Corner::round(40.0),
            Corner::SQUARE,
            Corner::SQUARE
        ]
    );
    assert_eq!(inset[0].1, Rect::new(20.0, 10.0, 70.0, 80.0));
}

#[test]
fn background_images_are_sized_placed_and_repeated() {
    // The test images are 40 by 20.
    let tiles = |style: &str| {
        let page = render(
            &format!(
                "<div style='width:100px;height:50px;background-image:url(i.png);{style}'></div>"
            ),
            "body{margin:0}",
        );
        images(&page.root().items)
    };
    assert_eq!(
        tiles("background-repeat:no-repeat"),
        [Rect::new(0.0, 0.0, 40.0, 20.0)]
    );
    assert_eq!(
        tiles("background:url(i.png) no-repeat center"),
        [Rect::new(30.0, 15.0, 40.0, 20.0)]
    );
    assert_eq!(
        tiles("background-repeat:no-repeat;background-position:right 5px bottom"),
        [Rect::new(55.0, 30.0, 40.0, 20.0)]
    );
    assert_eq!(
        tiles("background-repeat:no-repeat;background-size:contain"),
        [Rect::new(0.0, 0.0, 100.0, 50.0)]
    );
    assert_eq!(
        tiles("background-repeat:no-repeat;background-size:20px"),
        [Rect::new(0.0, 0.0, 20.0, 10.0)]
    );
    assert_eq!(
        tiles("background:url(i.png) no-repeat 50% / cover;height:100px"),
        [Rect::new(-50.0, 0.0, 200.0, 100.0)]
    );
    // Repeating fills the box, starting from the positioned tile.
    let across = tiles("background-repeat:repeat-x;background-position:10px 0");
    assert_eq!(across.first(), Some(&Rect::new(-30.0, 0.0, 40.0, 20.0)));
    assert_eq!(across.len(), 4);
    assert_eq!(tiles("").len(), 3 * 3);
    // `round` stretches whole tiles over the box; `space` spreads them.
    let rounded: Vec<_> = tiles("background-repeat:round no-repeat;background-size:45px")
        .into_iter()
        .map(|tile| (tile.x.round(), tile.width.round(), tile.height.round()))
        .collect();
    assert_eq!(rounded, [(0.0, 50.0, 25.0), (50.0, 50.0, 25.0)]);
    assert_eq!(
        tiles("background-repeat:space no-repeat"),
        [
            Rect::new(0.0, 0.0, 40.0, 20.0),
            Rect::new(60.0, 0.0, 40.0, 20.0)
        ]
    );
}

#[test]
fn surfaces_show_only_what_clipping_boxes_around_them_show() {
    // As CodeMirror hides its scroller's scrollbars below a box with
    // `overflow: hidden`.
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div style='overflow:hidden;height:50px'><div id=s style='overflow:scroll;height:100px'><div style='height:300px'></div></div></div>         <div id=t style='overflow:auto;height:40px'><div style='height:300px'></div></div>",
    ));
    let page = page_with(dom, "body { margin: 0 }");
    let root = page.root();
    let clip = |id: &str| {
        root.children
            .iter()
            .find(|surface| surface.id == page.id(id).index())
            .unwrap()
            .clip
    };
    assert_eq!(clip("s"), Some(Rect::new(0.0, 0.0, 400.0, 50.0)));
    assert_eq!(clip("t"), None);
}

#[test]
fn surfaces_show_through_translucent_boxes_around_them() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div style='opacity:0.5'><div id=s style='overflow:scroll;height:50px'><div id=n style='opacity:0.5'>\
         <div id=t style='overflow:scroll;height:20px'><div style='height:300px'></div></div></div></div></div>\
         <div id=u style='overflow:scroll;height:40px'></div>",
    ));
    let page = page_with(dom, "body { margin: 0 }");
    let root = page.root();
    let s = root
        .children
        .iter()
        .find(|surface| surface.id == page.id("s").index())
        .unwrap();
    let t = s
        .children
        .iter()
        .find(|surface| surface.id == page.id("t").index())
        .unwrap();
    let u = root
        .children
        .iter()
        .find(|surface| surface.id == page.id("u").index())
        .unwrap();
    // Each surface carries what its own surface does not already apply.
    assert_eq!(s.opacity, Some(0.5));
    assert_eq!(t.opacity, Some(0.5));
    assert_eq!(u.opacity, None);
}

/// The clips open around the first fill of `color`, outermost first.
fn clips_around(items: &[DisplayItem], color: Color) -> Vec<Rect> {
    let mut open: Vec<Option<Rect>> = Vec::new();
    for item in items {
        match item {
            DisplayItem::PushClip { rect, .. } => open.push(Some(*rect)),
            DisplayItem::PushTransform(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushFilter(_) => open.push(None),
            DisplayItem::PopClip
            | DisplayItem::PopTransform
            | DisplayItem::PopOpacity
            | DisplayItem::PopFilter => {
                open.pop();
            }
            DisplayItem::Fill { color: fill, .. } if *fill == color => {
                return open.into_iter().flatten().collect();
            }
            _ => {}
        }
    }
    panic!("no fill of {color:?}");
}

#[test]
fn fixed_boxes_escape_clips_inside_their_containing_block() {
    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    let page = render(
        "<div id=clipper style='position:relative;overflow:hidden;margin-top:40px;height:50px'>\
           <div style='position:relative;z-index:0'>\
             <div id=overlay style='position:fixed;top:0;left:0;width:100px;height:100px;background:red;z-index:5'></div>\
           </div>\
         </div>\
         <div id=moved style='overflow:hidden;transform:translateX(0.5px);margin-top:20px;height:30px'>\
           <div id=held style='position:fixed;top:0;left:200px;width:50px;height:200px;background:blue'></div>\
         </div>",
        "body { margin: 0 }",
    );
    // The overlay covers the window above the clipping box, under no clip,
    // and takes the pointer there.
    assert_eq!(clips_around(&page.root().items, red), Vec::<Rect>::new());
    assert_eq!(page.hit(10.0, 10.0).as_deref(), Some("overlay"));
    assert_eq!(page.hit(10.0, 95.0).as_deref(), Some("overlay"));
    // A transformed box contains fixed-position boxes and clips them.
    let moved = page.rect("moved");
    assert_eq!(clips_around(&page.root().items, blue).len(), 1);
    assert_eq!(page.hit(210.0, moved.y + 5.0).as_deref(), Some("held"));
    assert_ne!(
        page.hit(210.0, moved.y + moved.height + 20.0).as_deref(),
        Some("held")
    );
}

#[test]
fn isolated_boxes_keep_their_layers() {
    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    // The popup's high `z-index` counts only inside the isolated editor,
    // which paints below the later dialog.
    let page = render(
        "<div style='position:relative;isolation:isolate'>\
           <div style='position:absolute;width:50px;height:50px;background:red;z-index:100'></div>\
         </div>\
         <div style='position:absolute;top:0;width:50px;height:50px;background:blue;z-index:1'></div>",
        "body { margin: 0 }",
    );
    let order: Vec<Color> = fills(&page.root().items)
        .into_iter()
        .map(|(_, color)| color)
        .filter(|color| [red, blue].contains(color))
        .collect();
    assert_eq!(order, [red, blue]);
}

#[test]
fn shadows_falling_on_scroll_containers_show_through_them() {
    let page = render(
        "<div id=tabs style='position:relative;height:20px;box-shadow:0 0 9px 2px rgba(0,0,0,0.1)'></div>\
         <div id=plain style='position:relative;height:40px;overflow:auto'><div style='height:80px'></div></div>\
         <div id=far style='position:relative;margin-top:40px;height:40px;overflow:auto'><div style='height:80px'></div></div>",
        "body { margin: 0; background: white }",
    );
    let root = page.root();
    // The scroller painted after the strip, under its shadow, shows what
    // lies beneath; the one far from it fills with the background.
    assert_eq!(root.children[0].items[0], DisplayItem::Beneath);
    assert!(matches!(
        root.children[1].items[0],
        DisplayItem::Fill { .. }
    ));
}

#[test]
fn sticky_table_cells_stay_in_their_row() {
    let page = render(
        "<div style='overflow:hidden;width:300px'><table style='border-spacing:0;width:300px;table-layout:fixed'>\
           <colgroup><col width=210><col width=90></colgroup>\
           <thead><tr><th id=a>a</th><th id=b style='position:sticky;right:0'>b</th></tr></thead>\
         </table></div>",
        "body { margin: 0 } th { padding: 0 }",
    );
    let b = page.scene.client_rect(page.id("b")).unwrap();
    assert_eq!((b.x, b.width), (210.0, 90.0));
}

#[test]
fn inline_rects_report_the_fonts_content_area() {
    let page = render(
        "<p id=p style='font-size:10px;line-height:30px;margin:0'>ab <span id=s>cd</span></p>",
        "body { margin: 0 }",
    );
    // A 10 pixel font's ascent and descent, ten pixels, in the middle of
    // its 30 pixel line, as Chromium's getClientRects reports text.
    let span = page.scene.client_rect(page.id("s")).unwrap();
    assert_eq!((span.y, span.height), (10.0, 10.0));
    let text = page.dom.first_child(page.id("s")).unwrap();
    let styles = crate::Styles::default();
    let query = crate::paint::TextQuery {
        dom: &page.dom,
        styles: &styles,
        layout: &page.layout,
        measurer: &Mono,
    };
    let caret = page.scene.text_rects(&query, text, 1, 1);
    assert_eq!((caret[0].y, caret[0].height), (10.0, 10.0));
    // Blocks keep their boxes.
    assert_eq!(page.scene.client_rect(page.id("p")).unwrap().height, 30.0);
}

#[test]
fn overlays_that_scroll_show_their_background_once() {
    let page = render(
        "<div id=overlay style='position:fixed;top:0;left:0;right:0;bottom:0;overflow:auto;background:rgba(0,0,0,0.5)'>\
           <div style='height:20px'>x</div></div>",
        "body { margin: 0; background: white }",
    );
    let dim = Color::rgba(0, 0, 0, 0.5);
    // The window paints the overlay's background around its surface, which
    // shows it from beneath without painting it again.
    assert_eq!(
        fills(&page.root().items)
            .iter()
            .filter(|(_, color)| *color == dim)
            .count(),
        1
    );
    let surface = &page.root().children[0];
    assert_eq!(surface.items[0], DisplayItem::Beneath);
    assert!(fills(&surface.items).iter().all(|(_, color)| *color != dim));
}

#[test]
fn empty_inline_elements_have_an_empty_rect_where_they_sit() {
    // Between text, an empty element is as tall as its font's content area
    // and has no width; on a line of nothing else it has no box at all.
    let page = render(
        "<p id=p>ab<span id=e></span>cd</p><p><span id=alone></span></p>",
        "body{margin:0} p{margin:0;font-size:10px;line-height:20px}",
    );
    let rect = page.scene.client_rect(page.id("e")).unwrap();
    assert_eq!(rect, Rect::new(10.0, 5.0, 0.0, 10.0));
    assert_eq!(page.rect("p").height, 20.0);
    assert!(page.scene.client_rect(page.id("alone")).is_none());
    // Spaces after it at the start of a line still collapse.
    let page = render(
        "<p><span id=e></span> word</p>",
        "body{margin:0} p{margin:0;font-size:10px}",
    );
    let rect = page.scene.client_rect(page.id("e")).unwrap();
    assert_eq!((rect.x, rect.width), (0.0, 0.0));
    assert_eq!(texts(&page.root().items), ["word"]);
}

#[test]
fn text_fields_sit_on_the_baseline_of_their_text() {
    // A field's text line is in the middle of its content box and gives
    // the field its baseline, so a tall field fits its line; checkboxes sit
    // on their bottom edge.
    let page = render(
        "<div id=w><input id=i style='height:30px;border:0;padding:0'></div><div id=c><input type=checkbox style='margin:0;width:20px;height:20px'></div>",
        "body{margin:0} div{font-size:10px} input{font-size:10px}",
    );
    assert_eq!(page.rect("w").height, 30.0);
    assert_eq!(page.rect("i").y, 0.0);
    // 20px above the baseline and the strut's 3px below it.
    assert_eq!(page.rect("c").height, 23.0);
}

#[test]
fn shadows_painted_after_scroll_containers_show_over_them() {
    // A positioned tab strip paints after the editor below it, so its
    // shadow shows over the editor's content, as Chromium paints it.
    let page = render(
        "<div id=tabs style='position:relative;height:20px;box-shadow:0 0 9px 2px rgba(0,0,0,0.1)'></div>\
         <div id=editor style='height:40px;overflow:auto;background:white'><div style='height:80px'></div></div>\
         <div style='height:30px'></div>\
         <div id=far style='height:40px;overflow:auto;background:white'><div style='height:80px'></div></div>",
        "body { margin: 0; background: white } #tabs { margin-bottom: 0 }",
    );
    let root = page.root();
    let editor = &root.children[0];
    let Some(DisplayItem::Shadow { rect, .. }) = editor.items.last() else {
        panic!("{:?}", editor.items);
    };
    // In the editor's coordinates: the strip is just above it.
    assert_eq!((rect.y, rect.height), (-22.0, 24.0));
    // The scroller far below gets no shadow.
    assert!(
        !root.children[1]
            .items
            .iter()
            .any(|item| matches!(item, DisplayItem::Shadow { .. }))
    );
}

#[test]
fn relative_boxes_stay_inside_static_overflow_clips() {
    // Element Plus tabs: positioned items in a flex nav, inside a static
    // scroller that hides overflow, inside a positioned wrapper.
    let red = Color::rgb(255, 0, 0);
    let page = render(
        "<div style='position:relative;padding:0 20px;width:200px'>\
         <div id=scroll style='overflow:hidden'>\
         <div style='display:flex;position:relative;width:400px'>\
         <div style='position:relative;width:300px;height:10px;background:red'></div></div></div></div>",
        "body { margin: 0 }",
    );
    let clips = clips_around(&page.root().items, red);
    assert!(
        clips
            .iter()
            .any(|clip| clip.x == 20.0 && clip.width == 200.0),
        "{clips:?}"
    );
}

#[test]
fn negative_layers_paint_beneath_the_content_of_their_stacking_context() {
    // Muya's hidden source boxes: absolute with z-index -1 in a positioned
    // block that is no stacking context. They belong to the root context,
    // beneath the in-flow content after them, which takes the pointer.
    let page = render(
        "<div style='position:relative'><div id=under style='position:absolute;z-index:-1;         width:100px;height:100px;background:blue'></div></div><p id=over style='margin:0;height:50px'>text</p>",
        "body { margin: 0 }",
    );
    assert_eq!(page.hit(10.0, 10.0).as_deref(), Some("over"));
    assert_eq!(page.hit(10.0, 80.0).as_deref(), Some("under"));
    let items = &page.root().items;
    let fill = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == Color::rgb(0, 0, 255)))
        .unwrap();
    let text = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Text { .. }))
        .unwrap();
    assert!(fill < text, "{items:?}");
    // The same inside a scroller painted on a surface of its own, as
    // MarkText's editor is: there too the hidden box lies beneath.
    let page = render(
        "<div id=scroller style='overflow:auto;height:100px'><div style='position:relative'>         <div id=under style='position:absolute;z-index:-1;width:100px;height:100px;background:blue'></div></div>         <p id=over style='margin:0;height:50px'>text</p><div style='height:300px'></div></div>",
        "body { margin: 0 }",
    );
    assert_eq!(page.hit(10.0, 10.0).as_deref(), Some("over"));
    let surface = &page.root().children[0];
    let fill = surface
        .items
        .iter()
        .position(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == Color::rgb(0, 0, 255)))
        .unwrap();
    let text = surface
        .items
        .iter()
        .position(|item| matches!(item, DisplayItem::Text { .. }))
        .unwrap();
    assert!(fill < text, "{:?}", surface.items);
}

#[test]
fn absolute_boxes_escape_clips_inside_their_containing_block() {
    // A popover: absolute against a positioned box outside the scroller
    // that clips its other content.
    let blue = Color::rgb(0, 0, 255);
    let page = render(
        "<div style='position:relative;width:300px'>\
         <div style='overflow:hidden;width:100px;height:20px'>\
         <div id=pop style='position:absolute;left:150px;top:0;width:50px;height:10px;background:blue'></div></div></div>",
        "body { margin: 0 }",
    );
    let clips = clips_around(&page.root().items, blue);
    assert!(clips.iter().all(|clip| clip.width > 100.0), "{clips:?}");
    // It takes the pointer there too.
    assert_eq!(page.scene.hit_test(170.0, 5.0), Some(page.id("pop")));
}

#[test]
fn absolute_boxes_in_inline_containing_blocks_measure_insets_against_the_inline() {
    let page = render(
        "<p style='white-space:pre-wrap;margin:0'><span>A line</span><span id=sp>  </span>\n<span>next</span></p>\
         <p style='margin:0'><b id=b>bold <i id=end>x</i></b></p>",
        "#sp{position:relative} #sp::before{position:absolute;content:'\u{21a9}';inset-inline-start:100%} \
         #b{position:relative} #end{position:absolute;right:0;bottom:0;width:8px}",
    );
    let inline = |id: &str| {
        let node = page.id(id);
        let parts: Vec<Rect> = page
            .layout
            .inline_boxes
            .iter()
            .filter(|part| part.node == node)
            .map(|part| part.rect)
            .collect();
        parts[0]
    };
    let arrow = page
        .root()
        .items
        .iter()
        .find_map(|item| match item {
            DisplayItem::Text { text, rect, .. } if text == "\u{21a9}" => Some(*rect),
            _ => None,
        })
        .unwrap();
    // `100%` of the span's width puts the arrow right after the spaces.
    let space = inline("sp");
    assert_eq!(arrow.x, space.x + space.width);
    // `right` and `bottom` measure the inline's box.
    let bold = inline("b");
    let end = page.rect("end");
    assert_eq!(end.x + end.width, bold.x + bold.width);
    assert!((end.y + end.height - (bold.y + bold.height)).abs() < 0.01);
}

#[test]
fn vertical_align_shifts_inline_text_and_grows_its_line() {
    let page = render(
        "<p id=plain style='margin:0;line-height:20px'>H2O</p>\
         <p id=p style='margin:0;line-height:20px'>H<sub>2</sub>O x<sup>2</sup> \
         <span style='vertical-align:-4px'>low</span> <span style='vertical-align:50%'>up</span></p>",
        "",
    );
    let run = |text: &str| {
        page.layout
            .runs
            .iter()
            .find(|run| run.text == text)
            .map(|run| run.rect)
            .unwrap()
    };
    let base = run("H");
    let font = 16.0;
    // `sub` lowers the baseline by a fifth of the parent's font size and a
    // pixel, `super` raises it by a third and a pixel: the smaller text's
    // line-height box moves with it.
    let small = page
        .layout
        .runs
        .iter()
        .filter(|run| run.text == "2")
        .collect::<Vec<_>>();
    let (sub, sup) = (small[0].rect, small[1].rect);
    assert!(sub.y > sup.y);
    assert!(
        (sup.y + sup.height / 2.0 - (sub.y + sub.height / 2.0)
            + (font / 5.0 + 1.0 + font / 3.0 + 1.0))
            .abs()
            < 0.01
    );
    assert!((run("low").y - base.y - 4.0).abs() < 0.01);
    assert!((base.y - run("up").y - 10.0).abs() < 0.01);
    // The shifted text makes the line taller than the plain one.
    assert!(page.rect("p").height > page.rect("plain").height);
}

#[test]
fn generated_text_boxes_paint_with_their_transform_and_opacity() {
    let page = render(
        "<pre style='position:relative;padding-left:2.5em;margin:0'><span class=rows><span></span></span><code>a</code></pre>\
         <p style='margin:0'><span id=mark>x</span></p>",
        ".rows{position:absolute;top:0;left:0;width:2.5em;counter-reset:n} \
         .rows>span{position:absolute;left:0;width:100%;counter-increment:n} \
         .rows>span::before{display:block;padding-right:0.8em;text-align:right;transform:scale(0.8);content:counter(n)} \
         #mark{position:relative} #mark::before{position:absolute;left:100%;opacity:.5;content:'m'}",
    );
    // Each run follows the effects its generated box opened.
    let items = &page.root().items;
    let mut effects: Vec<&DisplayItem> = Vec::new();
    let mut seen = Vec::new();
    for item in items {
        match item {
            DisplayItem::PushTransform(_) | DisplayItem::PushOpacity(_) => effects.push(item),
            DisplayItem::PopTransform | DisplayItem::PopOpacity => {
                effects.pop();
            }
            DisplayItem::Text { text, rect, .. } => {
                seen.push((text.clone(), *rect, effects.last().cloned().cloned()))
            }
            _ => {}
        }
    }
    let effect = |wanted: &str| {
        seen.iter()
            .find(|(text, ..)| text == wanted)
            .map(|(_, rect, effect)| (*rect, effect.clone()))
            .unwrap()
    };
    let (number, scaled) = effect("1");
    let Some(DisplayItem::PushTransform([a, _, _, d, e, _])) = scaled else {
        panic!("{scaled:?}");
    };
    assert_eq!((a, d), (0.8, 0.8));
    // The scale is about the middle of the 40px box the number ends in,
    // past the body's margin.
    assert!((e - 0.2 * (8.0 + 20.0)).abs() < 0.01, "{e} {number:?}");
    assert_eq!(effect("m").1, Some(DisplayItem::PushOpacity(0.5)));
    assert_eq!(effect("a").1, None);
    assert_eq!(effect("x").1, None);
}

#[test]
fn tabs_advance_to_tab_stops_tab_size_apart() {
    // The test measurer's characters are 8px wide.
    let page = render(
        "<pre id=eight style='margin:0'>a\tb</pre><pre id=four style='margin:0;tab-size:4'>abc\tx\t\ty</pre>\
         <pre id=near style='margin:0;-moz-tab-size:2'>abcdefg\tz</pre><pre id=length style='margin:0;tab-size:20px'>a\tq</pre>",
        "",
    );
    let x_of = |text: &str| {
        page.layout
            .runs
            .iter()
            .find(|run| run.text == text)
            .map(|run| run.rect.x)
            .unwrap()
    };
    // Stops are 8 spaces (64px) apart by default, from the line's start.
    assert_eq!(x_of("b") - x_of("a"), 64.0);
    // `tab-size: 4`: "abc" ends at 24, the stop is at 32, the next at 64.
    assert_eq!(x_of("x") - x_of("abc"), 32.0);
    assert_eq!(x_of("y") - x_of("abc"), 96.0);
    // A stop closer than half a space is skipped: "abcdefg" ends at 56,
    // two spaces make stops every 16px, so 64 is 8px away and used.
    assert_eq!(x_of("z") - x_of("abcdefg"), 64.0);
    assert_eq!(x_of("q") - x_of("a"), 20.0);
    // Each tab is one character of its own run, which paints no glyph.
    let tabs: Vec<_> = page
        .layout
        .runs
        .iter()
        .filter(|run| run.text == "\t")
        .collect();
    assert_eq!(tabs.len(), 6);
    assert!(
        !texts(&page.root().items)
            .iter()
            .any(|text| text.contains('\t'))
    );
    assert_eq!(tabs[0].rect.width, 56.0);
}

#[test]
fn text_fields_size_their_content_box_and_buttons_their_border_box() {
    let page = render(
        "<input id=text style='width:100px'><input id=bare style='width:100px;border:none'><textarea id=area style='width:100px'></textarea>         <button id=button style='width:100px'>b</button><select id=select style='width:100px'></select><input id=search type=search style='width:100px'>",
        "",
    );
    // Chromium's text fields add their 2px borders and 1px 2px padding.
    assert_eq!(page.rect("text").width, 108.0);
    assert_eq!(page.rect("bare").width, 104.0);
    assert_eq!(page.rect("area").width, 108.0);
    for id in ["button", "select", "search"] {
        assert_eq!(page.rect(id).width, 100.0, "{id}");
    }
}

#[test]
fn translated_boxes_margins_included_make_no_scroll_container_scroll() {
    // A dialog's panel centered by `left: 50%; translateX(-50%)` whose
    // field's margin reaches past the panel, inside a fixed scrolling
    // overlay, as Element Plus dialogs hold MarkText's command palette.
    let page = render(
        "<div id=overlay style='position:fixed;top:0;right:0;bottom:0;left:0;overflow:auto'><div style='position:relative;width:300px;margin:15vh auto 50px;padding:16px;box-sizing:border-box'>\
         <header style='position:relative'><div style='position:absolute;top:0;left:50%;width:300px;transform:translateX(-50%);display:flex;flex-direction:column;align-items:center;padding:8px;box-sizing:border-box'>\
         <div style='width:100%;border:1px solid red'><input style='width:100%;margin:0 10px;border:none'></div></div></header></div></div>",
        "body{margin:0}",
    );
    let overlay = &page.layout.boxes[&page.id("overlay")];
    assert_eq!(overlay.content_width, 400.0);
    let surface = page
        .root()
        .children
        .iter()
        .find(|surface| surface.rect.width == 400.0)
        .unwrap();
    assert_eq!(surface.content_width, 400.0);
    assert!(
        surface
            .scrollbars
            .as_ref()
            .is_some_and(|bars| bars.platform.is_some())
    );
}

#[test]
fn the_windows_scrollbars_are_the_browsers_unless_author_css_styles_them() {
    // The root's `overflow`, or the body's it takes, decides the axes:
    // `hidden` shows none, and `scroll` always shows one.
    let bars = |css: &str| {
        let page = render("<div style='height:1000px'></div>", css);
        let bars = page.root().scrollbars.clone().unwrap();
        (
            bars.platform,
            bars.axes.each_ref().map(|axis| (axis.enabled, axis.force)),
        )
    };
    assert_eq!(
        bars(""),
        (
            Some(nativeboat_ir::scrollbar::Look::Classic),
            [(true, false), (true, false)]
        )
    );
    assert_eq!(
        bars("body{overflow-y:scroll}"),
        (
            Some(nativeboat_ir::scrollbar::Look::Classic),
            [(true, false), (true, true)]
        )
    );
    assert_eq!(
        bars("html{overflow:hidden}"),
        (
            Some(nativeboat_ir::scrollbar::Look::Classic),
            [(false, false), (false, false)]
        )
    );
    // Author-styled ones are as styled.
    let (platform, _) = bars("::-webkit-scrollbar{width:8px}");
    assert_eq!(platform, None);
}

#[test]
fn margins_collapse_past_the_static_place_of_absolute_boxes() {
    // Muya's HTML block: the preview's bottom margin collapses through the
    // figure past the hidden, absolutely positioned source editor, which
    // takes its static place below the collapsed margin, as in Chromium.
    let page = render(
        "<figure id=f style='margin:8px 0'><div id=p style='height:50px;margin-bottom:8px'></div>\
         <pre id=pre style='position:absolute;width:0;height:0;padding:16px;margin:0'>a</pre></figure>\
         <p id=after style='margin:0'>after</p>",
        "body{margin:0}",
    );
    assert_eq!(page.rect("f").height, 50.0);
    assert_eq!(page.rect("pre").y, 66.0);
    assert_eq!(page.rect("after").y, 66.0);
}

#[test]
fn spanning_cells_widen_their_columns_as_their_content_is_wide() {
    // Chromium's automatic table layout: the header's extra width goes to
    // the columns in proportion to their widest content, not evenly.
    let page = render(
        "<table id=t><tr><th id=h colspan=2>Merged header</th></tr><tr><td id=l>left</td><td id=r>right</td></tr>         <tr><td id=tall rowspan=2>tall</td><td id=one>one</td></tr><tr><td id=two>two</td></tr></table>",
        "body{margin:0} th,td{padding:1px}",
    );
    let (left, right) = (page.rect("l"), page.rect("r"));
    // 46.53 and 57.47, which layout rounds to whole pixels; an even share
    // would make them 48 and 56.
    assert_eq!((left.width, right.width), (47.0, 57.0));
    // The cell under a row-spanning one takes the second column.
    assert_eq!(page.rect("two").x, page.rect("one").x);
    assert_eq!(page.rect("h").width, left.width + 2.0 + right.width);
    // Border spacing also lies between the table's edges and its cells.
    assert_eq!(page.rect("h").x, 2.0);
    assert_eq!(page.rect("t").width, 2.0 + page.rect("h").width + 2.0);
}

#[test]
fn clip_paths_clip_paint_and_pointer_targets_to_their_shapes() {
    let clips = |page: &Page| -> Vec<(Rect, Radii)> {
        page.root()
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::PushClip { rect, radii } => Some((*rect, *radii)),
                _ => None,
            })
            .collect()
    };
    let base = "body{margin:0} #box{margin:20px;width:100px;height:60px;border:5px solid;padding:10px;border-radius:12px}";
    // The usual visually hidden pattern clips everything away.
    let hidden = render(
        "<div id=box><span id=text>text</span></div>",
        &format!("{base} #box{{clip-path:inset(50%)}}"),
    );
    assert_eq!(
        clips(&hidden),
        [(Rect::new(85.0, 65.0, 0.0, 0.0), [Corner::SQUARE; 4])]
    );
    assert_ne!(hidden.hit(40.0, 40.0), Some("text".into()));
    assert_ne!(hidden.hit(40.0, 40.0), Some("box".into()));
    let inset = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:inset(10px 20px round 6px)}}"),
    );
    assert_eq!(
        clips(&inset),
        [(Rect::new(40.0, 30.0, 90.0, 70.0), [Corner::round(6.0); 4])]
    );
    assert_ne!(inset.hit(35.0, 50.0), Some("box".into()));
    assert_eq!(inset.hit(60.0, 50.0), Some("box".into()));
    // A circle's percentage is of the box's normalized diagonal; it is
    // centered in the content box here.
    let circle = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:circle(10px) content-box}}"),
    );
    assert_eq!(
        clips(&circle),
        [(Rect::new(75.0, 55.0, 20.0, 20.0), [Corner::round(10.0); 4])]
    );
    let closest = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:circle(closest-side at 30px 40px)}}"),
    );
    assert_eq!(
        clips(&closest),
        [(Rect::new(20.0, 30.0, 60.0, 60.0), [Corner::round(30.0); 4])]
    );
    // A box keeps the element's rounded corners, shrunk to the box.
    let padding = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:padding-box}}"),
    );
    assert_eq!(
        clips(&padding),
        [(Rect::new(25.0, 25.0, 120.0, 80.0), [Corner::round(7.0); 4])]
    );
    // Polygons, unequal ellipses, paths and SVG clip paths clip to their
    // outlines, and only what they cover takes the pointer.
    let paths = |page: &Page| -> Vec<(Vec<PathSegment>, FillRule)> {
        page.root()
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::PushClipPath {
                    segments,
                    fill_rule,
                } => Some((segments.clone(), *fill_rule)),
                _ => None,
            })
            .collect()
    };
    let polygon = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:polygon(10px 0, 100% 20px, 50% 100%)}}"),
    );
    assert!(clips(&polygon).is_empty());
    assert_eq!(
        paths(&polygon),
        [(
            vec![
                PathSegment::Move(30.0, 20.0),
                PathSegment::Line(150.0, 40.0),
                PathSegment::Line(85.0, 110.0),
                PathSegment::Close,
            ],
            FillRule::NonZero
        )]
    );
    assert_eq!(polygon.hit(85.0, 60.0), Some("box".into()));
    // Inside the bounds, outside the triangle.
    assert_ne!(polygon.hit(35.0, 100.0), Some("box".into()));
    let ellipse = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:ellipse(60px 20px)}}"),
    );
    assert!(clips(&ellipse).is_empty() && paths(&ellipse).len() == 1);
    assert_eq!(ellipse.hit(85.0, 65.0), Some("box".into()));
    assert_ne!(ellipse.hit(85.0, 90.0), Some("box".into()));
    let path = render(
        "<div id=box></div>",
        &format!(
            "{base} #box{{clip-path:path(evenodd, 'M0 0 H40 V40 H0 Z M10 10 H30 V30 H10 Z')}}"
        ),
    );
    let (segments, rule) = &paths(&path)[0];
    assert_eq!(
        (segments[0], *rule),
        (PathSegment::Move(20.0, 20.0), FillRule::EvenOdd)
    );
    assert_eq!(path.hit(25.0, 25.0), Some("box".into()));
    // The even-odd hole.
    assert_ne!(path.hit(40.0, 40.0), Some("box".into()));
    let svg = render(
        "<svg style='position:absolute' width=0 height=0><clipPath id=c clipPathUnits=objectBoundingBox>         <rect x=0.5 y=0 width=0.5 height=1></rect></clipPath></svg><div id=box></div>",
        &format!("{base} #box{{clip-path:url(#c)}}"),
    );
    assert_eq!(paths(&svg).len(), 1);
    assert_ne!(svg.hit(40.0, 50.0), Some("box".into()));
    assert_eq!(svg.hit(130.0, 50.0), Some("box".into()));
    // A reference to nothing does not clip.
    let missing = render(
        "<div id=box></div>",
        &format!("{base} #box{{clip-path:url(#nothing)}}"),
    );
    assert!(clips(&missing).is_empty() && paths(&missing).is_empty());
    // Without a shape nothing is clipped.
    let none = render("<div id=box></div>", base);
    assert!(clips(&none).is_empty());
}

#[test]
fn masks_keep_what_their_image_covers() {
    // As Element Plus's checkmarks: a -webkit-mask shorthand with an
    // image, its repeat and size; the element and its descendants paint
    // inside the mask group.
    let page = render(
        "<div id=box><p id=text style='margin:0'>text</p></div>",
        "body{margin:0} #box{margin:10px;width:100px;height:40px;background:red;         -webkit-mask:linear-gradient(black, transparent) no-repeat;-webkit-mask-size:50% 100%}",
    );
    let items = &page.root().items;
    let start = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushMask(_)))
        .expect("a mask group");
    let end = items
        .iter()
        .position(|item| *item == DisplayItem::PopMask)
        .unwrap();
    let DisplayItem::PushMask(mask) = &items[start] else {
        unreachable!()
    };
    assert!(
        matches!(&mask[..], [DisplayItem::Gradient { rect, .. }] if *rect == Rect::new(10.0, 10.0, 100.0, 40.0)),
        "{mask:?}"
    );
    let inside = &items[start + 1..end];
    assert!(inside.iter().any(
        |item| matches!(item, DisplayItem::Fill { color, .. } if *color == Color::rgb(255, 0, 0))
    ));
    assert!(
        inside
            .iter()
            .any(|item| matches!(item, DisplayItem::Text { .. }))
    );
    // Generated boxes are masked too, as VS Code's progress badge masks a
    // block `::before` to its icon.
    let generated = render(
        "<div id=badge></div>",
        "body{margin:0} #badge{display:flex} #badge::before{content:'';display:block;width:13px;height:13px;background-color:white;mask:linear-gradient(black, transparent) no-repeat}",
    );
    let items = &generated.root().items;
    let start = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushMask(_)))
        .expect("a mask group for the generated box");
    let end = items
        .iter()
        .position(|item| *item == DisplayItem::PopMask)
        .unwrap();
    assert!(items[start + 1..end].iter().any(
        |item| matches!(item, DisplayItem::Fill { color, .. } if *color == Color::rgb(255, 255, 255))
    ));
    // No mask, no group.
    let plain = render(
        "<div id=box></div>",
        "#box{width:10px;height:10px;mask:none}",
    );
    assert!(
        !plain
            .root()
            .items
            .iter()
            .any(|item| matches!(item, DisplayItem::PushMask(_)))
    );
}

#[test]
fn elements_turned_away_with_hidden_backs_are_not_drawn() {
    let colors = |css: &str| {
        let page = render(
            "<div id=box><div id=inner></div></div>",
            &format!(
                "#box{{width:50px;height:20px;background:red}} #inner{{height:10px;background:blue}} {css}"
            ),
        );
        fills(&page.root().items)
            .into_iter()
            .map(|(_, color)| color)
            .filter(|color| *color == Color::rgb(255, 0, 0) || *color == Color::rgb(0, 0, 255))
            .count()
    };
    assert_eq!(colors(""), 2);
    // Turned half way round, the box shows its back unless that is hidden.
    assert_eq!(colors("#box{transform:rotateY(180deg)}"), 2);
    assert_eq!(
        colors("#box{transform:rotateY(180deg);backface-visibility:hidden}"),
        0
    );
    assert_eq!(
        colors("#box{transform:rotateX(120deg);backface-visibility:hidden}"),
        0
    );
    // A box facing the viewer, or mirrored in the plane, shows.
    for transform in ["rotateY(60deg)", "scaleX(-1)", "rotate(180deg)", "none"] {
        assert_eq!(
            colors(&format!(
                "#box{{transform:{transform};backface-visibility:hidden}}"
            )),
            2,
            "{transform}"
        );
    }
}

#[test]
fn right_to_left_runs_are_drawn_right_to_left() {
    let directions = |html: &str| -> Vec<(String, bool)> {
        render(html, "")
            .root()
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, rtl, .. } => Some((text.clone(), *rtl)),
                _ => None,
            })
            .collect()
    };
    // The bracket between Hebrew letters is right to left too, so it is
    // drawn mirrored.
    assert_eq!(
        directions("<p dir=rtl>\u{5d0} (\u{5d1})</p>"),
        [("\u{5d0} (\u{5d1})".to_owned(), true)]
    );
    assert_eq!(directions("<p>ab (c)</p>"), [("ab (c)".to_owned(), false)]);
}

#[test]
fn scroll_containers_under_scaling_transforms_paint_in_place() {
    let html = "<div id=pop><div id=list><p>first</p><p>item</p></div></div>";
    let base = "body{margin:0} p{margin:0;height:40px} #list{height:50px;overflow:auto}";
    // Untransformed, or only moved, the list scrolls in a native surface.
    for transform in ["none", "translateY(10px)"] {
        let page = render(html, &format!("{base} #pop{{transform:{transform}}}"));
        assert_eq!(page.root().children.len(), 1, "{transform}");
    }
    // Scaled, its content paints in place inside the transform, clipped and
    // scrolled.
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let list = dom.get_element_by_id(Dom::DOCUMENT, "list").unwrap();
    dom.set_scroll(list, 0.0, 30.0);
    let page = page_with(dom, &format!("{base} #pop{{transform:scale(0.5)}}"));
    assert!(page.root().children.is_empty());
    let items = &page.root().items;
    let pushed = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushTransform(_)))
        .unwrap();
    assert!(
        items[pushed..]
            .iter()
            .any(|item| matches!(item, DisplayItem::PushClip { .. }))
    );
    let text = items[pushed..]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Text { text, rect, .. } if text == "item" => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!(text.y, 10.0);
}

#[test]
fn controls_under_scaling_transforms_paint_in_place() {
    let html = "<div id=dialog><input id=name value=draft><input type=checkbox checked></div>";
    let base = "body{margin:0} input{width:100px;height:20px}";
    let page = render(
        html,
        &format!("{base} #dialog{{transform:translateX(5px)}}"),
    );
    assert_eq!(page.root().controls.len(), 2);
    // Scaled, they stay controls where their boxes show, their text scaled.
    let page = render(html, &format!("{base} #dialog{{transform:scale(0.5)}}"));
    let field = control(&page, "name");
    let unscaled = render(html, base);
    let full = control(&unscaled, "name");
    assert!(
        (field.rect.width * 2.0 - full.rect.width).abs() < 0.01,
        "{:?} {:?}",
        field.rect,
        full.rect
    );
    assert!((field.state.font.size * 2.0 - full.state.font.size).abs() < 0.01);
    // Rotated, they stay controls, kept out of sight, whose images paint
    // where their markers are, inside the transform; their transforms map
    // their boxes to the window.
    let page = render(
        html,
        &format!("{base} #dialog{{transform:rotate(90deg);transform-origin:0 0}}"),
    );
    assert_eq!(page.root().controls.len(), 2);
    let field = control(&page, "name");
    assert_eq!(field.state.clip, None);
    let name = page.dom.get_element_by_id(Dom::DOCUMENT, "name").unwrap();
    assert!(page.scene.takes_pointer(name));
    assert!(!page.scene.takes_pointer(Dom::DOCUMENT));
    // A turned select opens the page's own pop-up instead.
    let select = render(
        "<div id=dialog><select id=pick><option>a</option></select></div>",
        "#dialog{transform:rotate(10deg)}",
    );
    let pick = select.dom.get_element_by_id(Dom::DOCUMENT, "pick").unwrap();
    assert!(!select.scene.takes_pointer(pick));
    // Untransformed, controls take the pointer themselves.
    let plain = render(html, base);
    assert!(!plain.scene.takes_pointer(name));
    let [a, b, c, d, e, f] = field.state.transform.unwrap();
    let map = |x: f32, y: f32| (a * x + c * y + e, b * x + d * y + f);
    // The field's top edge runs down the window from its corner.
    let ((x0, y0), (x1, y1)) = (map(0.0, 0.0), map(100.0, 0.0));
    assert!((x1 - x0).abs() < 0.01 && (y1 - y0 - 100.0).abs() < 0.01);
    let shown = page
        .scene
        .client_rect(page.dom.get_element_by_id(Dom::DOCUMENT, "name").unwrap());
    assert!(
        (shown.unwrap().x + shown.unwrap().width - x0).abs() < 0.01,
        "{shown:?} {x0}"
    );
    let items = &page.root().items;
    let pushed = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushTransform(_)))
        .unwrap();
    assert!(
        items[pushed..]
            .iter()
            .any(|item| matches!(item, DisplayItem::Child(NativeChild::Control(_))))
    );
}

/// The text of every surface, with whether it is smoothed in grayscale.
fn smoothing(surface: &Surface) -> Vec<(String, bool)> {
    let mut found: Vec<(String, bool)> = surface
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text {
                text, grayscale, ..
            } => Some((text.clone(), *grayscale)),
            _ => None,
        })
        .collect();
    for child in &surface.children {
        found.extend(smoothing(child));
    }
    found.sort();
    found
}

#[test]
fn transparent_windows_show_through_where_the_page_paints_nothing() {
    let scene = |html: &str, css: &str| {
        let dom = Dom::from_document(&nativeboat_html::parse_document(html));
        let (author, _) = parse_stylesheet(css);
        let sheets = [
            SheetRef {
                sheet: user_agent_stylesheet(),
                origin: Origin::UserAgent,
            },
            SheetRef {
                sheet: &author,
                origin: Origin::Author,
            },
        ];
        let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
        let options = LayoutOptions {
            viewport: (400.0, 300.0),
            measurer: &Mono,
            image_size: &|_| None,
        };
        let layout = layout(&dom, &styles, &options);
        scene_over(
            &dom,
            &styles,
            &layout,
            (400.0, 300.0),
            None,
            Color::TRANSPARENT,
        )
    };
    // The canvas is left clear, and text not all on what the page paints
    // opaque gives up ClearType, as over Chromium's transparent base color.
    let loose = scene(
        "<p>loose</p><div style='background:white'><p>held</p></div>",
        "",
    );
    assert_eq!(fills(&loose.tree.root.items)[0].1, Color::TRANSPARENT);
    assert!(
        smoothing(&loose.tree.root)
            .iter()
            .all(|(_, grayscale)| *grayscale)
    );
    // Text all on an opaque box keeps it.
    let held = scene("<div style='background:white'><p>held</p></div>", "");
    assert!(
        smoothing(&held.tree.root)
            .iter()
            .all(|(_, grayscale)| !grayscale)
    );
    // A root with a background paints the canvas with it.
    let filled = scene("<p>filled</p>", "html { background: red }");
    assert_eq!(fills(&filled.tree.root.items)[0].1, RED);
}

#[test]
fn text_in_boxes_with_backdrop_filters_is_smoothed_in_grayscale() {
    // Chromium composites a box with a backdrop filter: its text, and its
    // descendants', gives up ClearType unless all of it lies on what the
    // layer paints opaque. Text on an opaque box goes with the rest. Text
    // around the layer keeps ClearType.
    let html = "<div id=panel><p>plain</p><p style='background:white'>opaque</p>\
        <div style='overflow:auto;height:20px'><p>scrolled</p><div style='height:100px'></div></div>\
        </div><p>outside</p>";
    let css = "#panel { background: rgba(255, 255, 255, 0.65); backdrop-filter: blur(24px) }";
    let page = render(html, css);
    let expected = [
        ("opaque", true),
        ("outside", false),
        ("plain", true),
        ("scrolled", true),
    ]
    .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(smoothing(page.root()), expected);
    // An opaque panel keeps ClearType, and so does one whose text all lies
    // on one opaque box.
    let css = "#panel { background: white; backdrop-filter: blur(24px) }";
    let page = render(html, css);
    assert!(
        smoothing(page.root())
            .iter()
            .all(|(_, grayscale)| !grayscale)
    );
    let panel = "#panel { background: rgba(255, 255, 255, 0.65); backdrop-filter: blur(24px) }";
    let html = "<div id=panel><div class=card><p>one</p><p>two</p></div></div>";
    // A layer inside, and what it holds, judges its own text; inline boxes
    // and hidden ones count as their text does.
    let nested = "<div id=panel><div class=card><p>one <span>two</span></p>        <div class=inner><p>inside</p></div><p style='display:none'>hidden</p></div></div>";
    let css = format!(
        "{panel} .card {{ background: white; padding: 4px }}          .inner {{ background: rgba(0, 0, 0, 0.1); backdrop-filter: blur(4px) }}"
    );
    let expected = [("inside", true), ("one ", false), ("two", false)]
        .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(smoothing(render(nested, &css).root()), expected);
    let page = render(
        html,
        &format!("{panel} .card {{ background: white; padding: 4px }}"),
    );
    assert!(
        smoothing(page.root())
            .iter()
            .all(|(_, grayscale)| !grayscale)
    );
    // Two opaque boxes apart are known to cover only the larger one, and a
    // rounded box only from three tenths of its radii in: text on a smaller
    // box painted after a larger one, or in the band along the corners,
    // gives it up for all.
    let html = "<div id=panel><p class=card style='height:80px'>large</p>\
        <div style='height:30px'></div><p class=card>small</p></div>";
    let page = render(html, &format!("{panel} .card {{ background: white }}"));
    assert!(
        smoothing(page.root())
            .iter()
            .all(|(_, grayscale)| *grayscale)
    );
    let html = "<div id=panel><p class=card>pill</p></div>";
    // In whole device pixels too, as Chromium judges at 125%.
    for pixel in [0.0, 0.8] {
        for (radius, grayscale) in [("4px", false), ("100px", true)] {
            let card =
                format!(".card {{ background: white; padding: 2px; border-radius: {radius} }}");
            let dom = Dom::from_document(&nativeboat_html::parse_document(html));
            let page = page_measured(dom, &format!("{panel} {card}"), u32::MAX, &Pixels(pixel));
            assert_eq!(
                smoothing(page.root()),
                [("pill".to_owned(), grayscale)],
                "{radius} {pixel}"
            );
        }
    }
}

#[test]
fn opaque_rectangles_cover_together_what_chromium_finds() {
    let rect = Rect::new;
    let half = rect(0.0, 0.0, 10.0, 10.0);
    // Side by side or stacked, two touching boxes cover their union.
    assert_eq!(
        maximum_covered(half, rect(10.0, 0.0, 10.0, 10.0)),
        rect(0.0, 0.0, 20.0, 10.0)
    );
    assert_eq!(
        maximum_covered(half, rect(0.0, 10.0, 10.0, 10.0)),
        rect(0.0, 0.0, 10.0, 20.0)
    );
    // Apart, only the larger one; crossing, the larger of the two and
    // their overlap stretched across both.
    let large = rect(30.0, 0.0, 20.0, 20.0);
    assert_eq!(maximum_covered(half, large), large);
    assert_eq!(maximum_covered(large, half), large);
    assert_eq!(
        maximum_covered(rect(0.0, 0.0, 30.0, 10.0), rect(5.0, 5.0, 20.0, 30.0)),
        rect(5.0, 0.0, 20.0, 35.0)
    );
}

#[test]
fn effects_that_could_fringe_cleartype_smooth_text_in_grayscale() {
    // As Chromium's `FilterOperations::AllowsLCDText`: one blur, or a
    // saturation of at most 100%, keeps ClearType; two filters, or a hue
    // rotation, do not, even over an opaque background.
    let html = "<p id=f style='background:white'>filtered</p><p>plain</p>";
    for (filter, grayscale) in [
        ("blur(1px)", false),
        ("saturate(0.5)", false),
        ("hue-rotate(90deg)", true),
        ("blur(1px) brightness(0.9)", true),
    ] {
        let page = render(html, &format!("#f {{ filter: {filter} }}"));
        let expected = [("filtered", grayscale), ("plain", false)]
            .map(|(text, grayscale)| (text.to_owned(), grayscale));
        assert_eq!(smoothing(page.root()), expected, "{filter}");
    }
    // A backdrop filter that could fringe ClearType keeps it from all the
    // window's layer paints, as Chromium reads that back to filter it:
    // only the filtered element's own content, all of it on something
    // opaque, keeps it.
    let html = "<p style='background:white'>before</p>\
        <nav><p style='background:white'>item</p></nav><p>after</p>";
    let css =
        "nav { background: rgba(15, 23, 42, 0.9); backdrop-filter: blur(18px) saturate(180%) }";
    let page = render(html, css);
    let expected = [("after", true), ("before", true), ("item", false)]
        .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(smoothing(page.root()), expected);
    let menu = html.replace("<nav>", "<nav><p>menu</p>");
    assert!(
        smoothing(render(&menu, css).root())
            .iter()
            .all(|(_, grayscale)| *grayscale)
    );
    // Hidden, it keeps nothing from the rest.
    let page = render(html, &format!("{css} nav {{ display: none }}"));
    assert_eq!(
        smoothing(page.root()),
        [("after", false), ("before", false)].map(|(text, gray)| (text.to_owned(), gray))
    );
    // Inside a filter that could fringe ClearType, its own content still
    // gives it up.
    let html = format!("<div style='filter: hue-rotate(90deg)'>{html}</div>");
    let page = render(&html, css);
    assert!(
        smoothing(page.root())
            .iter()
            .all(|(_, grayscale)| *grayscale)
    );
}

/// The placeholder of every control, with whether its text is smoothed in
/// grayscale.
fn control_smoothing(surface: &Surface) -> Vec<(String, bool)> {
    let mut found: Vec<(String, bool)> = surface
        .controls
        .iter()
        .map(|control| (control.state.placeholder.clone(), control.state.grayscale))
        .collect();
    for child in &surface.children {
        found.extend(control_smoothing(child));
    }
    found.sort();
    found
}

#[test]
fn text_fields_are_smoothed_as_the_text_where_they_lie() {
    // A field's text gives up ClearType where the page's text does: in a
    // composited layer, unless all the layer's text, the fields' included,
    // lies on what it paints opaque.
    let html = "<div id=panel><input placeholder=clear><input placeholder=opaque></div>\
        <input placeholder=outside class=clear>";
    let css = "#panel { background: rgba(255, 255, 255, 0.65); backdrop-filter: blur(24px) }\
        input { background: white } input[placeholder=clear], .clear { background: transparent }";
    let page = render(html, css);
    let expected = [("clear", true), ("opaque", true), ("outside", false)]
        .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(control_smoothing(page.root()), expected);
    let alone = html.replace("<input placeholder=clear>", "");
    let expected = [("opaque", false), ("outside", false)]
        .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(control_smoothing(render(&alone, css).root()), expected);
    // Under a filter that could fringe ClearType, even an opaque one does.
    let page = render(
        &format!("<div style='filter: hue-rotate(90deg)'>{html}</div>"),
        css,
    );
    assert!(
        control_smoothing(page.root())
            .iter()
            .all(|(_, grayscale)| *grayscale)
    );
}

#[test]
fn before_chromium_135_every_filter_and_backdrop_filter_gives_up_cleartype() {
    // Chromium 134 and earlier, as Electron 30 ships, keep ClearType under
    // no filter at all, and a backdrop filter of any kind keeps it from
    // what paints before it in its render target.
    let html = "<p style='background:white'>before</p>        <aside><p>item</p></aside><p style='background:white'>after</p>";
    let css = "aside { background: rgb(48, 52, 70); backdrop-filter: blur(20px) }";
    let dom = || Dom::from_document(&nativeboat_html::parse_document(html));
    let expected = |before, after| {
        [("after", after), ("before", before), ("item", false)]
            .map(|(text, grayscale)| (text.to_owned(), grayscale))
    };
    let page = page_in(dom(), css, 124);
    assert_eq!(smoothing(page.root()), expected(true, true));
    // From Chromium 135 on, one blur keeps it.
    let page = page_in(dom(), css, 135);
    assert_eq!(smoothing(page.root()), expected(false, false));
    let html = "<p id=f style='background:white'>filtered</p><p>plain</p>";
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let page = page_in(dom, "#f { filter: blur(1px) }", 134);
    let expected = [("filtered", true), ("plain", false)]
        .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(smoothing(page.root()), expected);
}

#[test]
fn text_on_scroll_containers_chromium_composites_is_smoothed_in_grayscale() {
    // A box with a negative `z-index` paints beneath the content of a scroll
    // container that makes no stacking context, which Chromium composites:
    // its text gives up ClearType unless all of it lies on what it paints
    // opaque.
    let html = "<div id=s><p>plain</p><p style='background:white'>opaque</p>\
        <span style='position:absolute;z-index:-1'>under</span>\
        <div style='height:500px'></div></div><p>outside</p>";
    let css = "#s { overflow: auto; height: 100px }";
    let page = render(html, css);
    let expected = [
        ("opaque", true),
        ("outside", false),
        ("plain", true),
        ("under", true),
    ]
    .map(|(text, grayscale)| (text.to_owned(), grayscale));
    assert_eq!(smoothing(page.root()), expected);
    // A stacking context of its own, an opaque background, content that
    // fits, and no box beneath keep ClearType, as does a box with a
    // negative `z-index` inside a stacking context of its own, and text
    // that all lies on one opaque box.
    for (html, css) in [
        (
            html.to_owned(),
            format!("{css} #s {{ position: relative; z-index: 0 }}"),
        ),
        (html.to_owned(), format!("{css} #s {{ background: white }}")),
        (
            html.to_owned(),
            "#s { overflow: auto; height: 1000px }".to_owned(),
        ),
        (html.replace("z-index:-1", "z-index:0"), css.to_owned()),
        (
            html.replace("<span", "<div style='opacity:0.5'><span")
                .replace("</span>", "</span></div>"),
            css.to_owned(),
        ),
        (
            html.replace("<span", "<div style='display:none'><span")
                .replace("</span>", "</span></div>"),
            css.to_owned(),
        ),
        (
            "<div id=s><div style='background:white'><p>card</p></div>\
                <div style='position:absolute;z-index:-1;width:5px;height:5px'></div>\
                <div style='height:500px'></div></div>"
                .to_owned(),
            css.to_owned(),
        ),
    ] {
        let page = render(&html, &css);
        assert!(
            smoothing(page.root())
                .iter()
                .all(|(_, grayscale)| !grayscale),
            "{html} {css}"
        );
    }
}
