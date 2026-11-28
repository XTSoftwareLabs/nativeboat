use super::*;
use nativeboat_css::types::TextAlign;
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};

#[test]
fn ratio_only_inline_svg_uses_its_constrained_container_width() {
    for (width, ratio, height) in [(20.0, "20 20", 20.0), (80.0, "40 20", 40.0)] {
        let (dom, result) = lay_out(
            &format!(
                "<div id=container><svg id=icon viewBox='0 0 {ratio}'><path d='M0 0L20 20'/></svg></div>"
            ),
            &format!("body{{margin:0}} #container{{width:{width}px}}"),
        );
        let icon = rect(&dom, &result, "icon");
        assert_eq!((icon.width, icon.height), (width, height));
        assert_eq!(icon.x, rect(&dom, &result, "container").x);
    }
    let (dom, result) = lay_out(
        "<div style='display:flex'><span style='width:20px;height:20px'><svg id=icon viewBox='0 0 20 20'></svg></span></div>",
        "body{margin:0}",
    );
    assert_eq!(
        (
            rect(&dom, &result, "icon").width,
            rect(&dom, &result, "icon").height
        ),
        (20.0, 20.0)
    );
}

#[test]
fn ratio_only_svg_intrinsic_queries_and_known_axes_remain_bounded() {
    let unknown = Size {
        width: None,
        height: None,
    };
    for (available, expected) in [
        (
            AvailableSpace::MinContent,
            Size {
                width: 0.0,
                height: 0.0,
            },
        ),
        (
            AvailableSpace::MaxContent,
            Size {
                width: 300.0,
                height: 150.0,
            },
        ),
        (
            AvailableSpace::Definite(20.0),
            Size {
                width: 20.0,
                height: 10.0,
            },
        ),
        (
            AvailableSpace::Definite(-10.0),
            Size {
                width: 0.0,
                height: 0.0,
            },
        ),
    ] {
        assert_eq!(
            measure_leaf(0.0, 0.0, Some(2.0), unknown, available),
            expected
        );
    }
    assert_eq!(
        measure_leaf(
            0.0,
            0.0,
            Some(2.0),
            Size {
                width: None,
                height: Some(40.0)
            },
            AvailableSpace::Definite(20.0)
        ),
        Size {
            width: 80.0,
            height: 40.0
        }
    );
    let (dom, result) = lay_out(
        "<div style='width:20px'><svg id=fixed width=40 height=50 viewBox='0 0 20 20'></svg><svg id=css viewBox='0 0 20 20'></svg></div>",
        "#css{height:12px}",
    );
    assert_eq!(
        (
            rect(&dom, &result, "fixed").width,
            rect(&dom, &result, "fixed").height
        ),
        (40.0, 50.0)
    );
    assert_eq!(
        (
            rect(&dom, &result, "css").width,
            rect(&dom, &result, "css").height
        ),
        (12.0, 12.0)
    );
}

#[test]
fn block_level_buttons_fit_their_content() {
    let (dom, result) = lay_out(
        "<button id=flex>Advanced</button><div id=div>Advanced</div><button id=wide>Advanced</button><button id=sized>x</button><section><button id=item>Advanced</button></section>",
        "body{margin:0} button,div{display:flex;margin:10px auto;padding:0 20px;border:1px solid;font:10px mono} #wide{display:block} #sized{width:100px} section{display:flex} #item{margin:0}",
    );
    let fitted = rect(&dom, &result, "flex");
    // As wide as its content, centred by its auto margins, unlike a div.
    assert!(fitted.width < 200.0, "{fitted:?}");
    assert!(
        (fitted.x - (400.0 - fitted.width) / 2.0).abs() < 0.5,
        "{fitted:?}"
    );
    assert_eq!(rect(&dom, &result, "div").width, 400.0);
    assert_eq!(rect(&dom, &result, "wide").width, fitted.width);
    assert_eq!(rect(&dom, &result, "sized").width, 100.0);
    // A flex item sizes as flex layout says.
    assert_eq!(rect(&dom, &result, "item").width, fitted.width);
}

#[test]
fn padded_buttons_center_atomic_icons_without_duplicating_the_outer_box() {
    let (dom, result) = lay_out(
        "<button id=first><i class=icon><svg id=a viewBox='0 0 20 20'></svg></i></button><button id=second><i class=icon><svg id=b viewBox='0 0 20 20'></svg></i></button>",
        "body{margin:0} button{width:28px;height:28px;box-sizing:border-box;padding:5px;border:1px solid #ddd;display:inline-block;text-align:center} .icon{display:inline-flex;align-items:center;justify-content:center;width:14px;height:14px;vertical-align:middle} svg{width:14px;height:14px}",
    );
    for (owner, icon) in [("first", "a"), ("second", "b")] {
        let button = rect(&dom, &result, owner);
        let icon = rect(&dom, &result, icon);
        assert_eq!((button.width, button.height), (28.0, 28.0));
        assert_eq!((icon.width, icon.height), (14.0, 14.0));
        assert!(
            (icon.x + icon.width / 2.0 - button.x - button.width / 2.0).abs() <= 0.5,
            "{button:?} {icon:?}"
        );
        // `middle` puts the icon's middle half an x-height above the
        // baseline, within a pixel of the button's middle.
        assert!(
            (icon.y + icon.height / 2.0 - button.y - button.height / 2.0).abs() <= 1.0,
            "{button:?} {icon:?}"
        );
    }
}

#[test]
fn hidden_generated_content_preserves_whitespace_between_text_nodes() {
    for html in [
        "<div>one <span id=s> two</span></div>",
        "<div><span id=s>one</span> two</div>",
    ] {
        let (dom, ordinary) = lay_out(html, "");
        let hidden = lay_out_dom(
            &dom,
            "#s::before{content:'hidden';display:none} #s::after{content:'hidden ';display:none}",
        );
        assert_eq!(hidden, ordinary, "{html}");
    }
}

#[test]
fn marktext_list_shorthand_keeps_native_bullets_and_numbers() {
    let html = "<ul class=mu-bullet-list><li id=outer>outer<ul class=mu-bullet-list><li id=nested>nested</li></ul></li></ul><ol class=mu-order-list start=3><li id=ordered>ordered</li></ol><ul><li id=task style='list-style:none'>task</li></ul>";
    let (dom, layout) = lay_out(
        html,
        "ul.mu-bullet-list{list-style:disc outside none} ol.mu-order-list{list-style:decimal outside none} ul.mu-bullet-list ul.mu-bullet-list{list-style-type:circle}",
    );
    for (name, expected) in [("outer", "•"), ("nested", "◦"), ("ordered", "3.")] {
        let node = dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
        let marker = layout.runs.iter().find(|run| run.node == node).unwrap();
        // A marker and its space end where the item's content starts.
        assert_eq!(marker.text.trim_end(), expected);
        assert!(marker.text.ends_with('\u{a0}'));
        assert!(marker.rect.x + marker.rect.width <= layout.boxes[&node].rect.x + 0.01);
    }
    let task = dom.get_element_by_id(Dom::DOCUMENT, "task").unwrap();
    assert!(!layout.runs.iter().any(|run| run.node == task));
}

#[test]
fn disclosure_markers_stay_inside_padding_and_follow_author_styles() {
    let html = "<details style='padding:10px'><summary id=c style='padding:3px'>Closed</summary></details><details open><summary id=o>Opened</summary><summary id=later>Later</summary></details><summary id=alone>Alone</summary>";
    let (dom, result) = lay_out(html, "body{margin:0}");
    let id = |name: &str| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    for (name, symbol) in [
        ("c", Symbol::DisclosureClosed),
        ("o", Symbol::DisclosureOpen),
    ] {
        let summary = id(name);
        let marker = result.runs.iter().find(|run| run.node == summary).unwrap();
        // The triangle and its space take 0.98em, whatever the font.
        assert_eq!(marker.symbol.map(|(shape, _)| shape), Some(symbol));
        assert!((marker.rect.width - marker.font.size * 0.98).abs() < 0.01);
        assert_eq!(marker.element, summary);
        assert_eq!(marker.rect.x, result.boxes[&summary].content_box().x);
        let text = result
            .runs
            .iter()
            .find(|run| run.container == summary && run.node != summary)
            .unwrap();
        assert!(text.rect.x >= marker.rect.x + marker.rect.width);
        assert_eq!(text.rect.y, marker.rect.y);
    }
    for name in ["later", "alone"] {
        assert!(!result.runs.iter().any(|run| run.node == id(name)));
    }
    let no_marker = lay_out_dom(&dom, "#c{list-style:none}");
    assert!(!no_marker.runs.iter().any(|run| run.node == id("c")));
    let text = no_marker
        .runs
        .iter()
        .find(|run| run.text == "Closed")
        .unwrap();
    assert_eq!(text.rect.x, no_marker.boxes[&id("c")].content_box().x);
    let overridden = lay_out_dom(
        &dom,
        "#c{list-style:decimal outside} #o{list-style-type:circle} #alone{display:list-item;list-style:disc inside}",
    );
    for (name, symbol) in [("c", "1."), ("o", "\u{25E6}"), ("alone", "\u{2022}")] {
        let marker = overridden
            .runs
            .iter()
            .find(|run| run.node == id(name))
            .unwrap();
        assert_eq!(marker.text.trim_end(), symbol);
    }
    let flat = lay_out_dom(&dom, "#c{display:flex}");
    assert!(!flat.runs.iter().any(|run| run.node == id("c")));
}

#[test]
fn closed_disclosures_hide_direct_text_boxes_and_later_summaries() {
    let (mut dom, closed) = lay_out(
        "<details>hidden text<summary id=first>Heading</summary><div id=body>body</div><summary id=later>later</summary></details>",
        "",
    );
    let first = dom.get_element_by_id(Dom::DOCUMENT, "first").unwrap();
    let body = dom.get_element_by_id(Dom::DOCUMENT, "body").unwrap();
    let later = dom.get_element_by_id(Dom::DOCUMENT, "later").unwrap();
    assert!(closed.boxes.contains_key(&first));
    assert!(!closed.boxes.contains_key(&body));
    assert!(!closed.boxes.contains_key(&later));
    assert!(!closed.runs.iter().any(|run| run.text.contains("hidden")
        || run.text.contains("body")
        || run.text.contains("later")));
    let details = dom.parent_element(first).unwrap();
    dom.set_attribute(details, "open", "").unwrap();
    let open = lay_out_dom(&dom, "");
    assert!(open.boxes.contains_key(&body));
    assert!(open.boxes.contains_key(&later));
    assert!(open.runs.iter().any(|run| run.text.contains("hidden")));
}

struct Mono;

impl TextMeasurer for Mono {
    fn width(&self, text: &str, font: &Font) -> f32 {
        text.chars().count() as f32 * font.size * 0.5
    }

    fn metrics(&self, font: &Font) -> crate::FontMetrics {
        crate::FontMetrics {
            ascent: font.size * 0.8,
            descent: font.size * 0.2,
        }
    }
}

fn natural(url: &str) -> Option<(f32, f32)> {
    (!url.ends_with("unknown.png")).then_some((40.0, 20.0))
}

fn lay_out(html: &str, css: &str) -> (Dom, LayoutResult) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let result = lay_out_dom(&dom, css);
    (dom, result)
}

fn lay_out_dom(dom: &Dom, css: &str) -> LayoutResult {
    lay_out_dom_in(dom, css, &StyleEnvironment::default())
}

fn lay_out_dom_in(dom: &Dom, css: &str, environment: &StyleEnvironment) -> LayoutResult {
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
    let styles = Styles::compute(dom, &sheets, environment);
    let options = LayoutOptions {
        viewport: (400.0, 300.0),
        measurer: &Mono,
        image_size: &natural,
    };
    layout(dom, &styles, &options)
}

#[test]
fn boxes_lie_at_sixty_fourths_of_device_pixels_as_chromium_lays_them_out() {
    /// Measures as [`Mono`], for device pixels of a size in CSS pixels.
    struct Device(f32);
    impl TextMeasurer for Device {
        fn width(&self, text: &str, font: &Font) -> f32 {
            Mono.width(text, font)
        }
        fn metrics(&self, font: &Font) -> crate::FontMetrics {
            Mono.metrics(font)
        }
        fn device_pixel(&self) -> f32 {
            self.0
        }
    }
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=lines>x<br>y<br>z</div><div id=after>w</div>",
    ));
    let css = "body { margin: 0 } div { font-size: 14px; line-height: 22.4px } #after { margin-top: 0.31px }";
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
    let laid = |pixel: f32| {
        let options = LayoutOptions {
            viewport: (400.0, 300.0),
            measurer: &Device(pixel),
            image_size: &natural,
        };
        let result = layout(&dom, &styles, &options);
        (rect(&dom, &result, "lines"), rect(&dom, &result, "after"))
    };
    // At 125%, three lines 22.4 pixels high stay 67.2 high, and the margin
    // rounds to the nearest sixty-fourth of a device pixel.
    let (lines, after) = laid(0.8);
    assert!((lines.height - 67.2).abs() < 1e-3, "{lines:?}");
    assert!((after.y - (67.2 + 0.3125)).abs() < 1e-3, "{after:?}");
    // Where device pixels are unknown, boxes lie at whole pixels.
    let (lines, after) = laid(0.0);
    assert_eq!((lines.height, after.y), (67.0, 68.0));
}

fn rect(dom: &Dom, result: &LayoutResult, id: &str) -> Rect {
    let node = dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    result.boxes[&node].rect
}

fn runs(result: &LayoutResult) -> Vec<(&str, f32, f32)> {
    result
        .runs
        .iter()
        .map(|run| (run.text.as_str(), run.rect.x, run.rect.y))
        .collect()
}

#[test]
fn block_flow_and_root_margins() {
    let (dom, result) = lay_out(
        "<div id=a style='height:10px'></div><div id=b style='margin-top:5px;height:10px'></div>",
        "",
    );
    let html = dom.document_element().unwrap();
    assert_eq!(result.boxes[&html].rect, Rect::new(0.0, 0.0, 400.0, 41.0));
    assert_eq!(rect(&dom, &result, "a"), Rect::new(8.0, 8.0, 384.0, 10.0));
    assert_eq!(rect(&dom, &result, "b"), Rect::new(8.0, 23.0, 384.0, 10.0));
    assert_eq!((result.width, result.height), (400.0, 300.0));
}

#[test]
fn flex_rows_with_controls() {
    let (dom, result) = lay_out(
        "<div class=c><input id=i><button id=b>Save</button></div>",
        ".c { display: flex; gap: 8px; padding: 4px }",
    );
    assert_eq!(rect(&dom, &result, "i"), Rect::new(12.0, 12.0, 141.0, 22.0));
    assert_eq!(rect(&dom, &result, "b"), Rect::new(161.0, 12.0, 43.0, 22.0));
}

#[test]
fn percentage_controls_shrink_with_their_containers() {
    // Element Plus number inputs: the wrapper's min-content width must not
    // include the input's intrinsic 141px.
    let (dom, result) = lay_out(
        "<div id=outer><div id=wrap><input id=i></div></div>         <div id=fixed><div id=wrap2><input id=j></div></div>",
        "#outer, #fixed { display: inline-flex; width: 100px }         #wrap, #wrap2 { display: inline-flex; flex-grow: 1; padding: 1px 11px }         #i { width: 100%; flex-grow: 1; padding: 0; border: 0 }         #j { flex-grow: 1; padding: 0; border: 0 }",
    );
    assert_eq!(rect(&dom, &result, "wrap").width, 100.0);
    assert_eq!(rect(&dom, &result, "i").width, 78.0);
    assert!(rect(&dom, &result, "wrap2").width > 100.0);
}

#[test]
fn grid_tracks() {
    let (dom, result) = lay_out(
        "<div style='display:grid;grid-template-columns:100px 1fr;width:300px'><div id=a>a</div><div id=b>b</div><input id=c></div>",
        "",
    );
    assert_eq!(rect(&dom, &result, "a").width, 100.0);
    assert_eq!(rect(&dom, &result, "b"), Rect::new(108.0, 8.0, 200.0, 19.0));
    assert_eq!(rect(&dom, &result, "c").x, 8.0);
}

#[test]
fn inline_text_wraps_with_styles() {
    let (dom, result) = lay_out(
        "<p id=p style='width:60px;margin:0'>Hello <b id=b>world</b> wraps</p>\
         <div style='text-decoration:line-through'>x<a href=y>link</a></div>",
        "",
    );
    assert_eq!(
        runs(&result),
        vec![
            ("Hello", 8.0, 8.0),
            ("world", 8.0, 27.2),
            ("wraps", 8.0, 46.4),
            ("x", 8.0, 66.0),
            ("link", 16.0, 66.0),
        ]
    );
    let bold = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    assert_eq!(result.runs[1].element, bold);
    assert_eq!(result.runs[1].font.weight, 700);
    assert!(result.runs[3].line_through && !result.runs[3].underline);
    assert!(result.runs[4].underline && !result.runs[4].line_through);
    let through: Vec<_> = result.runs[4]
        .decorations
        .iter()
        .map(|decoration| (decoration.line, decoration.color))
        .collect();
    assert_eq!(
        through,
        vec![(nativeboat_ir::DecorationLine::LineThrough, Color::BLACK)]
    );
}

#[test]
fn decorations_reach_block_descendants_in_their_own_color_and_style() {
    use nativeboat_css::types::TextDecorationStyle as Style;
    use nativeboat_ir::DecorationLine as Line;
    let (_, result) = lay_out(
        "<div style='text-decoration:underline wavy red;color:green'><p style='color:blue'>a</p>\
         <span style='display:inline-block'>b</span><u>c</u></div>\
         <p style='text-decoration:overline 3px'>d</p><p style='text-underline-position:under;text-decoration:underline'>e</p>",
        "p { margin: 0 }",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["a", "b", "c", "d", "e"]);
    let drawn = |index: usize| -> Vec<(Line, Color, Style, Option<f32>, bool)> {
        result.runs[index]
            .decorations
            .iter()
            .map(|decoration| {
                (
                    decoration.line,
                    decoration.color,
                    decoration.style,
                    decoration.thickness,
                    decoration.under,
                )
            })
            .collect()
    };
    let wavy = (
        Line::Underline,
        Color::rgb(255, 0, 0),
        Style::Wavy,
        None,
        false,
    );
    // A block inside the decorating box draws its decoration.
    assert!(!result.runs[0].underline);
    assert_eq!(drawn(0), vec![wavy]);
    // An inline block does not.
    assert!(!result.runs[1].underline && drawn(1).is_empty());
    // Text in its decoration color has the font draw its own underline.
    assert!(result.runs[2].underline);
    assert_eq!(drawn(2), vec![wavy]);
    assert_eq!(
        drawn(3),
        vec![(Line::Overline, Color::BLACK, Style::Solid, Some(3.0), false)]
    );
    assert!(!result.runs[4].underline);
    assert_eq!(
        drawn(4),
        vec![(Line::Underline, Color::BLACK, Style::Solid, None, true)]
    );
}

#[test]
fn text_beside_inline_blocks_keeps_its_line_after_rounding() {
    // At x = 8.6 the 31.5px run rounds to a 31px box: its lines still break
    // at the width it was sized to.
    let (_, result) = lay_out(
        "<p style='margin:0;padding-left:0.6px;font-size:9px'>aaa bbb<span style='display:inline-block;width:10px;height:5px'></span></p>",
        "",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["aaa bbb"]);
}

#[test]
fn borders_keep_their_device_pixel_widths_wherever_the_box_falls() {
    // At 125%, a 2px border is two device pixels, 1.6px, on every side of
    // a box at a fractional position; rounding its edges would give 1 or 2.
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div style='margin-left:10.6px;width:15px;height:15px;border:2px solid;border-radius:50%' id=b></div>",
    ));
    let mut environment = StyleEnvironment::default();
    environment.media.resolution = 1.25;
    let result = lay_out_dom_in(&dom, "body { margin: 0 }", &environment);
    let node = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    assert_eq!(result.boxes[&node].border, [1.6; 4]);
}

#[test]
fn generated_content_and_line_breaks() {
    let (_, result) = lay_out(
        "<p data-x=X>body<br>next<span>!</span></p><p class=e>e</p>",
        "p { margin: 0 } p::before { content: attr(data-x) \"-\" } span::after { content: '?'; text-decoration: underline } .e::before { content: none }",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["X-", "body", "next", "!", "?", "e"]);
    assert!(result.runs[4].underline);
    assert_eq!(result.runs[2].rect.y, result.runs[0].rect.y + 19.2);
}

#[test]
fn generated_content_shows_counters() {
    // Line numbers as Prism draws them: positioned rows numbered by a counter.
    let (_, result) = lay_out(
        "<pre><span class=rows><span></span><span></span></span>a\nb</pre>",
        "pre { margin: 0; position: relative; padding-left: 40px } \
         .rows { position: absolute; left: 0; top: 0; counter-reset: line } \
         .rows > span { display: block; counter-increment: line } \
         .rows > span::before { content: counter(line) '.'; display: block }",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["1.", "2.", "a", "b"]);
}

#[test]
fn hidden_and_contents_elements() {
    let (dom, result) = lay_out(
        "<div id=h style='display:none'>gone</div><div id=c style='display:contents'><p id=p>kept</p></div>",
        "",
    );
    let hidden = dom.get_element_by_id(Dom::DOCUMENT, "h").unwrap();
    let contents = dom.get_element_by_id(Dom::DOCUMENT, "c").unwrap();
    assert!(!result.boxes.contains_key(&hidden));
    assert!(!result.boxes.contains_key(&contents));
    assert!(
        result
            .boxes
            .contains_key(&dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap())
    );
}

#[test]
fn positioned_boxes_use_their_containing_block() {
    let (dom, result) = lay_out(
        "<div style='position:relative;margin:10px;width:100px;height:100px'><span>\
         <div id=abs style='position:absolute;left:5px;top:6px;width:10px;height:10px'></div></span></div>\
         <div style='margin:30px'><div id=fixed style='position:fixed;left:1px;top:2px;width:5px;height:5px'></div>\
         <div id=loose style='position:absolute;right:0;bottom:0;width:4px;height:4px'></div></div>",
        "",
    );
    assert_eq!(
        rect(&dom, &result, "abs"),
        Rect::new(23.0, 16.0, 10.0, 10.0)
    );
    assert_eq!(rect(&dom, &result, "fixed"), Rect::new(1.0, 2.0, 5.0, 5.0));
    assert_eq!(
        rect(&dom, &result, "loose"),
        Rect::new(396.0, 296.0, 4.0, 4.0)
    );
}

#[test]
fn atomic_boxes_inside_text_make_rows() {
    let (dom, result) = lay_out(
        "<div id=row style='width:300px;text-align:center'>Label <button id=btn>Go</button> tail</div>\
         <div id=right style='text-align:right'><button id=r>R</button></div>\
         <div><p>block</p>text <input id=i2> more<span id=s><div>inner</div></span></div>",
        "",
    );
    let button = rect(&dom, &result, "btn");
    assert!(button.x > 8.0 + 40.0);
    // The space before the button stays on the line.
    assert_eq!(result.runs[0].text, "Label ");
    let right = rect(&dom, &result, "r");
    assert_eq!(right.x + right.width, 392.0);
    let input = rect(&dom, &result, "i2");
    assert!(input.x > 8.0);
    assert!(
        result
            .boxes
            .contains_key(&dom.get_element_by_id(Dom::DOCUMENT, "s").unwrap())
    );
}

#[test]
fn styled_buttons_lay_out_their_content() {
    let (dom, result) = lay_out(
        "<button id=b class=s>Go <b>now</b></button>         <button id=r class=s>a<span style='display:inline-block;width:5px;height:5px'></span></button>         <button id=n>native</button><button id=f class=s style='display:flex'><span>x</span></button>",
        ".s { background: red; height: 40px; font-size: 10px; line-height: 10px; padding: 0; border: 0; margin: 0 }",
    );
    let top = |id: &str| rect(&dom, &result, id).y;
    let run = |text: &str| {
        result
            .runs
            .iter()
            .find(|run| run.text.trim() == text)
            .map(|run| run.rect.y)
    };
    // Content is centered vertically, unless the button lays it out itself.
    assert_eq!(run("Go"), Some(top("b") + 15.0));
    assert_eq!(run("a"), Some(top("r") + 15.0));
    assert_eq!(run("x"), Some(top("f")));
    // Buttons without author styling stay native controls.
    assert_eq!(run("native"), None);
}

#[test]
fn control_sizes() {
    let (dom, result) = lay_out(
        "<textarea id=ta cols=10 rows=3></textarea><input id=cb type=checkbox><input id=rd type=radio>\
         <select id=sel><option>short</option><option>a longer one</option></select>\
         <select id=list multiple><option>x</option></select><input id=range type=range>\
         <progress id=prog></progress><input id=sized size=5><input id=submit type=submit>\
         <input id=file type=file><input id=reset type=reset><input id=plain type=button>\
         <input id=valued type=button value=OK><input id=bad size=0>",
        "* { padding: 0 !important; margin: 0 !important; font-size: 10px !important; line-height: 10px !important } \
         :not(#sel) { border: none !important }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    assert_eq!(size("ta"), (67.0, 30.0));
    assert_eq!(size("cb"), (13.0, 13.0));
    assert_eq!(size("rd"), (13.0, 13.0));
    // Its own border around the native drop-down.
    assert_eq!(size("sel"), (86.0, 12.0));
    assert_eq!(size("list"), (22.0, 40.0));
    assert_eq!(size("range"), (129.0, 16.0));
    assert_eq!(size("prog"), (160.0, 16.0));
    assert_eq!(size("sized"), (25.0, 10.0));
    assert_eq!(size("submit"), (30.0, 10.0));
    assert_eq!(size("file"), (55.0, 10.0));
    assert_eq!(size("reset"), (25.0, 10.0));
    assert_eq!(size("plain"), (0.0, 10.0));
    assert_eq!(size("valued"), (10.0, 10.0));
    assert_eq!(size("bad"), (100.0, 10.0));
}

#[test]
fn drawn_selects_and_date_fields() {
    let (dom, result) = lay_out(
        "<select id=sel class=s><option>short</option><option selected>a longer one</option></select>\
         <select id=first class=s><option> two  words </option><option>abc</option></select>\
         <select id=empty class=s></select>\
         <select id=bare class=s style='appearance:none'><option>abc</option></select>\
         <select id=han class=s><option>所有格式</option></select>\
         <select id=normal class=s style='line-height:normal'><option>abc</option></select>\
         <input id=date type=date><input id=time type=time>\
         <input id=moment type=datetime-local><input id=month type=month>",
        "* { padding: 0; margin: 0; border: 0; font-size: 10px; line-height: 10px } .s { background: white }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    // A styled select shows its selected option and room for the arrow, as
    // wide as its widest option. Its label's line has a pixel above and
    // below it, unless the select has no appearance, and is as tall for
    // any text.
    assert_eq!(size("sel"), (80.0, 12.0));
    assert_eq!(size("first"), (65.0, 12.0));
    assert_eq!(size("empty"), (20.0, 12.0));
    assert_eq!(size("bare").1, 10.0);
    assert_eq!(size("han").1, 12.0);
    // At `line-height: normal` the line is the font's ascent and descent,
    // without the line gap a normal line adds.
    assert_eq!(size("normal").1, 12.0);
    let labels: Vec<&str> = runs(&result).into_iter().map(|run| run.0).collect();
    assert_eq!(
        labels,
        vec!["a longer one", "two words", "abc", "所有格式", "abc"]
    );
    assert_eq!(runs(&result)[0].1, rect(&dom, &result, "sel").x);
    let select = |id: &str| dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    assert_eq!(select_label(&dom, select("first")), "two words");
    assert_eq!(select_label(&dom, select("empty")), "");
    // Date and time fields fit their pictures and the picker's button.
    assert_eq!(size("date"), (80.0, 10.0));
    assert_eq!(size("time"), (64.0, 10.0));
    assert_eq!(size("moment"), (130.0, 10.0));
    assert_eq!(size("month"), (100.0, 10.0));
}

#[test]
fn image_sizes() {
    let (dom, result) = lay_out(
        "<img id=both src=a.png width=10 height=10><img id=w src=a.png width=20><img id=h src=a.png height=40px>\
         <img id=n src=a.png><img id=none><img id=u src=unknown.png width=7><svg id=svg width=5 height=6></svg>",
        "img, svg { display: block }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    assert_eq!(size("both"), (10.0, 10.0));
    assert_eq!(size("w"), (20.0, 10.0));
    assert_eq!(size("h"), (80.0, 40.0));
    assert_eq!(size("n"), (40.0, 20.0));
    assert_eq!(size("none"), (0.0, 0.0));
    assert_eq!(size("u"), (7.0, 0.0));
    assert_eq!(size("svg"), (5.0, 6.0));
}

#[test]
fn iframe_sizes() {
    // As Chromium sizes them: 300×150 with a 2px inset border, whatever
    // their `src` names, unless their attributes or styles size them.
    let (dom, result) = lay_out(
        "<iframe id=empty></iframe><iframe id=page src=a.png></iframe>\
         <iframe id=sized width=100 height=40></iframe><iframe id=styled style='width:50px'></iframe>",
        "iframe { display: block }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    assert_eq!(size("empty"), (304.0, 154.0));
    assert_eq!(size("page"), (304.0, 154.0));
    assert_eq!(size("sized"), (104.0, 44.0));
    assert_eq!(size("styled"), (54.0, 154.0));
}

#[test]
fn video_sizes() {
    // As Electron 38 sizes them: with no picture yet, a video is 300×150
    // and keeps no ratio; then it takes its picture's size, its poster's
    // when it has one, else its first frame's.
    let (dom, result) = lay_out(
        "<video id=empty></video><video id=wide style='width:200px'></video>\
         <video id=unread src=unknown.png></video><video id=frame src=a.mp4></video>\
         <video id=half src=a.mp4 width=20></video><video id=source><source src=a.mp4></video>\
         <video id=poster poster=p.png src=unknown.png></video>",
        "video { display: block }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    assert_eq!(size("empty"), (300.0, 150.0));
    assert_eq!(size("wide"), (200.0, 150.0));
    assert_eq!(size("unread"), (300.0, 150.0));
    assert_eq!(size("frame"), (40.0, 20.0));
    assert_eq!(size("half"), (20.0, 10.0));
    assert_eq!(size("source"), (40.0, 20.0));
    assert_eq!(size("poster"), (40.0, 20.0));
    let video = |id: &str| dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    assert_eq!(
        crate::video_picture(&dom, video("frame")).as_deref(),
        Some("nativeboat-video-frame:a.mp4")
    );
    assert_eq!(crate::media_url(&dom, video("source")), Some("a.mp4"));
    assert_eq!(
        crate::video_picture(&dom, video("poster")).as_deref(),
        Some("p.png")
    );
    assert_eq!(crate::video_picture(&dom, video("empty")), None);
}

#[test]
fn images_sized_by_percentages_contribute_nothing_to_min_content_widths() {
    // Sizes from Electron 38.
    let (dom, result) = lay_out(
        "<div style='display:flex;width:27px'><div style='display:flex'><img id=logo src=a.png></div></div>\
         <div style='display:inline-block;width:min-content'><img id=line src=a.png></div>\
         <div style='display:inline-block;width:min-content'><img id=calc src=a.png></div>\
         <div style='display:flex;width:20px'><img id=flex src=a.png></div>\
         <div style='float:left'><img id=float src=a.png></div>\
         <table style='width:10px'><tr><td><img id=cell src=a.png></td></tr></table>",
        "#logo { width: 32px; height: 32px; max-width: 100% }\
         #line { width: 40px; max-width: 100% }\
         #calc { width: calc(50% + 4px) }\
         #flex, #cell { display: block; max-width: 100% }\
         #float { display: block; width: 100% }",
    );
    let size = |id: &str| {
        let rect = rect(&dom, &result, id);
        (rect.width, rect.height)
    };
    // As PicGo's sidebar logo: the row holding it may shrink to the 27
    // pixels there are, and the logo, at most as wide as the row, with it.
    assert_eq!(size("logo"), (27.0, 32.0));
    // A box as narrow as its content is as narrow as the percentage lets
    // it be, and the image then fits it.
    assert_eq!(size("line").0, 0.0);
    // So does a percentage in `calc()`: the box has no width, and the image
    // is the 4 pixels the percentage of none adds to.
    assert_eq!(size("calc").0, 4.0);
    // Where the percentages resolve, they limit the image as before.
    assert_eq!(size("flex"), (20.0, 10.0));
    // The most a box would take for its content still treats them as
    // auto, so a float is as wide as the image.
    assert_eq!(size("float"), (40.0, 20.0));
    // A table cell may shrink the image too.
    assert!(size("cell").0 < 10.0);
}

#[test]
fn flex_text_and_whitespace() {
    let (_, result) = lay_out(
        "<div style='display:flex'>  loose text <b>bold</b>  </div>\n  <div>\n</div>",
        "",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["loose text", "bold"]);
}

#[test]
fn empty_and_unstyled_documents() {
    let dom = Dom::new();
    let result = lay_out_dom(&dom, "");
    assert!(result.boxes.is_empty());
    assert_eq!((result.width, result.height), (400.0, 300.0));
    let (_, hidden) = lay_out("<p>x</p>", "html { display: none }");
    assert!(hidden.boxes.is_empty());
    let mut dom = Dom::from_document(&nativeboat_html::parse_document("<p>x</p>"));
    let (author, _) = parse_stylesheet("");
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let body = dom.html_child("body").unwrap();
    let late = dom.create_element("div").unwrap();
    dom.append_child(body, late).unwrap();
    let options = LayoutOptions {
        viewport: (100.0, 100.0),
        measurer: &Mono,
        image_size: &natural,
    };
    assert!(format!("{options:?}").contains("viewport"));
    let result = layout(&dom, &styles, &options);
    assert!(!result.boxes.contains_key(&late));
}

#[test]
fn box_geometry_helpers() {
    let geometry = BoxGeometry {
        margin: [0.0; 4],
        rect: Rect::new(0.0, 0.0, 100.0, 50.0),
        padding: [1.0, 2.0, 3.0, 4.0],
        border: [5.0, 6.0, 7.0, 8.0],
        content_width: 100.0,
        content_height: 50.0,
        scrollbars: [0.0; 2],
    };
    assert_eq!(geometry.padding_box(), Rect::new(8.0, 5.0, 86.0, 38.0));
    assert_eq!(geometry.content_box(), Rect::new(12.0, 6.0, 80.0, 34.0));
    let scrolled = BoxGeometry {
        scrollbars: [17.0, 10.0],
        ..geometry
    };
    assert_eq!(scrolled.client_box(), Rect::new(8.0, 5.0, 69.0, 28.0));
    assert_eq!(scrolled.content_box(), Rect::new(12.0, 6.0, 63.0, 24.0));
    let tiny = BoxGeometry {
        padding: [9.0; 4],
        border: [9.0; 4],
        ..BoxGeometry::default()
    };
    assert_eq!(tiny.content_box().width, 0.0);
}

#[test]
fn leaf_and_inline_measurement() {
    let unknown = Size {
        width: None,
        height: None,
    };
    assert_eq!(
        measure_leaf(4.0, 2.0, None, unknown, AvailableSpace::MaxContent),
        Size {
            width: 4.0,
            height: 2.0
        }
    );
    let width_known = Size {
        width: Some(10.0),
        height: None,
    };
    assert_eq!(
        measure_leaf(4.0, 2.0, Some(2.0), width_known, AvailableSpace::MaxContent),
        Size {
            width: 10.0,
            height: 5.0
        }
    );
    let height_known = Size {
        width: None,
        height: Some(10.0),
    };
    assert_eq!(
        measure_leaf(
            4.0,
            2.0,
            Some(2.0),
            height_known,
            AvailableSpace::MaxContent
        ),
        Size {
            width: 20.0,
            height: 10.0
        }
    );
    assert_eq!(
        measure_leaf(4.0, 2.0, None, width_known, AvailableSpace::MaxContent),
        Size {
            width: 10.0,
            height: 2.0
        }
    );
    let dom = Dom::new();
    let mut after_space = true;
    let content = InlineContent {
        strut_font: Font {
            size: 2.0,
            ..Font::default()
        },
        atoms: Vec::new(),
        segments: vec![Segment {
            shift: 0.0,
            tab: (0.0, 0.0),
            pieces: crate::text::pieces(
                "aa bb",
                nativeboat_css::types::WhiteSpace::Normal,
                &mut after_space,
            ),
            font: Font {
                size: 2.0,
                ..Font::default()
            },
            line_height: 1.0,
            normal_line: false,
            color: Color::BLACK,
            underline: false,
            underline_offset: None,
            line_through: false,
            decorations: alloc::rc::Rc::from([]),
            letter_spacing: 0.0,
            white_space: nativeboat_css::types::WhiteSpace::Normal,
            word_breaking: WordBreaking::Normal,
            node: Dom::DOCUMENT,
            element: Dom::DOCUMENT,
            boxes: alloc::rc::Rc::from([]),
            edge: None,
            symbol: None,
            level: 0,
            overridden: false,
        }],
        text_align: TextAlign::Start,
        direction: nativeboat_css::types::Direction::Ltr,
        indent: 0.0,
        strut: 1.0,
        ellipsis: false,
        clamp: None,
    };
    let _ = dom;
    let available = |width: AvailableSpace| Size {
        width,
        height: AvailableSpace::MaxContent,
    };
    assert_eq!(
        measure_inline(
            &content,
            &Mono,
            unknown,
            available(AvailableSpace::MaxContent),
            &[]
        )
        .0,
        Size {
            width: 5.0,
            height: 1.0
        }
    );
    assert_eq!(
        measure_inline(
            &content,
            &Mono,
            unknown,
            available(AvailableSpace::MinContent),
            &[]
        )
        .0,
        Size {
            width: 2.0,
            height: 2.0
        }
    );
    assert_eq!(
        measure_inline(
            &content,
            &Mono,
            unknown,
            available(AvailableSpace::Definite(3.0)),
            &[]
        )
        .0,
        // Shrink-to-fit: min(max-content, max(min-content, space)).
        Size {
            width: 3.0,
            height: 2.0
        }
    );
    let fixed = Size {
        width: Some(9.0),
        height: Some(7.0),
    };
    assert_eq!(
        measure_inline(
            &content,
            &Mono,
            fixed,
            available(AvailableSpace::MinContent),
            &[]
        )
        .0,
        Size {
            width: 9.0,
            height: 7.0
        }
    );
    // The baselines of the first and last lines: the font's ascent and half
    // the leading below each line's top.
    let (_, _, baselines) = measure_inline(
        &content,
        &Mono,
        unknown,
        available(AvailableSpace::MinContent),
        &[],
    );
    assert!((baselines.first.unwrap() - 1.1).abs() < 1e-4);
    assert!((baselines.last.unwrap() - 2.1).abs() < 1e-4);
    let empty = InlineContent {
        strut_font: Font::default(),
        atoms: Vec::new(),
        segments: Vec::new(),
        ..content
    };
    assert_eq!(
        measure_inline(
            &empty,
            &Mono,
            unknown,
            available(AvailableSpace::MaxContent),
            &[]
        )
        .2,
        Baselines::NONE
    );
}

#[test]
fn calc_widths_and_more_generated_content() {
    let (dom, result) = lay_out(
        "<div id=c style='width:calc(100% - 20px)'>x</div><div class=g><span class=s>in<b style='display:none'>no</b></span></div>",
        ".g::after { content: 'A' } .s::before { content: 'B' }",
    );
    assert_eq!(rect(&dom, &result, "c").width, 364.0);
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, vec!["x", "B", "in", "A"]);
}

#[test]
fn generated_content_needs_items() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<input id=i placeholder=p>",
    ));
    let (author, _) = parse_stylesheet("input::placeholder { color: red }");
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let options = LayoutOptions {
        viewport: (10.0, 10.0),
        measurer: &Mono,
        image_size: &natural,
    };
    let builder = Builder {
        dom: &dom,
        styles: &styles,
        options: &options,
        calcs: CalcStore::default(),
        nodes: Vec::new(),
        deferred: Vec::new(),
        cells: HashMap::new(),
        anonymous_cells: HashSet::new(),
        generated: HashMap::new(),
        anchored: HashMap::new(),
        statics: HashMap::new(),
        boxes: Vec::new(),
        shift: 0.0,
        bidi: (0, false),
        images: HashMap::new(),
        counters: core::cell::OnceCell::new(),
    };
    let input = dom.get_element_by_id(Dom::DOCUMENT, "i").unwrap();
    let mut after_space = true;
    assert!(
        builder
            .generated(
                input,
                &PseudoElement::Placeholder,
                (false, false),
                &mut after_space
            )
            .is_none()
    );
    let mut tree = Tree {
        nodes: builder.nodes,
        measurer: &Mono,
        compressing: false,
    };
    tree.nodes.push(Node {
        dom: None,
        kind: Kind::Container(Mode::Block),
        style: Style::DEFAULT,
        children: Vec::new(),
        cache: Cache::new(),
        unrounded: Layout::with_order(0),
        layout: Layout::with_order(0),
        atom_sizes: Vec::new(),
        lines: core::cell::RefCell::new(None),
        float_lines: None,
        cell_width: None,
        fixed_table: false,
        text_line: None,
        compressed: None,
    });
    tree.cache_clear(taffy::NodeId::from(0usize));
}

#[test]
fn tables_lay_out_as_grids() {
    let html = "<table id=t><caption id=c>Cap</caption><colgroup><col></colgroup>\
        <tfoot><tr><td id=f>total</td></tr></tfoot>\
        <thead><tr><th id=h1>Name</th><th id=h2>Qty</th></tr></thead>\
        <tbody><tr><td id=a>apple</td><td id=b>3</td></tr><tr><td id=w colspan=2>wide</td></tr></tbody></table>";
    let (dom, result) = lay_out(html, "body { margin: 0 } td, th { padding: 0 }");
    let [caption, h1, h2, a, b, w, f] =
        ["c", "h1", "h2", "a", "b", "w", "f"].map(|id| rect(&dom, &result, id));
    assert_eq!((a.x, b.x), (h1.x, h2.x));
    assert_eq!(b.x, a.x + a.width + 2.0);
    assert_eq!(w.width, a.width + b.width + 2.0);
    // The caption sits above the table's border, as wide as the table.
    let table = rect(&dom, &result, "t");
    assert_eq!((caption.x, caption.width), (table.x, table.width));
    assert_eq!(caption.y + caption.height, table.y);
    assert!(h1.y < a.y && a.y < w.y && w.y < f.y);
    let (dom, result) = lay_out(
        "<table><tr><td id=r rowspan=2>r</td><td id=x colspan=abc>x</td></tr><tr><td id=y colspan=0>y</td></tr></table>\
         <div style='display:table-row'><span id=orphan style='display:table-cell'>o</span></div>",
        "table { border-collapse: collapse } td { padding: 0 }",
    );
    let [r, x, y] = ["r", "x", "y"].map(|id| rect(&dom, &result, id));
    assert_eq!(y.x, x.x);
    assert_eq!(x.x, r.x + r.width);
    assert!(r.height > x.height);
    assert!(rect(&dom, &result, "orphan").width > 0.0);
    // Tables built from CSS display values have rows without groups.
    let (dom, result) = lay_out(
        "<div style='display:table'><div style='display:table-row'><div id=p style='display:table-cell'>p</div>         <span style='display:block'>loose</span><div id=q style='display:table-cell'>q</div></div></div>",
        "",
    );
    assert!(rect(&dom, &result, "q").x > rect(&dom, &result, "p").x);
}

#[test]
fn inline_boxes_cover_their_text_and_take_their_padding() {
    let (dom, result) = lay_out(
        "<p id=p>a<code id=c>bb</code>c <span id=s>dd ee</span></p>",
        "body { margin: 0; width: 1000px } p { margin: 0; font-size: 10px; line-height: 20px }          code { background: red; padding: 2px 3px; margin: 0 4px; border: 1px solid blue }          span { background: green }",
    );
    let text = |text: &str| {
        let run = result
            .runs
            .iter()
            .find(|run| run.text.trim() == text)
            .unwrap_or_else(|| panic!("{:?}", runs(&result)));
        (run.rect.x, run.rect.width)
    };
    let code = dom.get_element_by_id(Dom::DOCUMENT, "c").unwrap();
    let span = dom.get_element_by_id(Dom::DOCUMENT, "s").unwrap();
    let parts = |node| {
        result
            .inline_boxes
            .iter()
            .filter(|part| part.node == node)
            .collect::<Vec<_>>()
    };
    let [code_part] = parts(code)[..] else {
        panic!("{:?}", result.inline_boxes);
    };
    // The box starts after its margin and covers border, padding and text.
    let (a, bb, c) = (text("a"), text("bb"), text("c"));
    assert_eq!(code_part.rect.x, a.0 + a.1 + 4.0);
    assert_eq!(bb.0, code_part.rect.x + 1.0 + 3.0);
    assert_eq!(
        code_part.rect.x + code_part.rect.width,
        bb.0 + bb.1 + 3.0 + 1.0
    );
    assert_eq!(c.0, code_part.rect.x + code_part.rect.width + 4.0);
    assert!(code_part.start && code_part.end);
    // Above and below the content area by border and padding.
    assert!(code_part.rect.height > 10.0);
    let [span_part] = parts(span)[..] else {
        panic!("{:?}", result.inline_boxes);
    };
    assert!(span_part.rect.width > 0.0 && span_part.start && span_part.end);
}

#[test]
fn text_at_font_size_zero_takes_no_space_and_shows_nothing() {
    let (_, result) = lay_out(
        "<p>a<span style='font-size:0'>(hidden)</span>b</p>",
        "body { margin: 0 } p { margin: 0; font-size: 10px }",
    );
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(texts, ["a", "(hidden)", "b"]);
    // The hidden text has no width, and its content area is an empty
    // rectangle on the baseline, as Chromium reports it.
    let hidden = &result.runs[1];
    assert_eq!(hidden.rect.width, 0.0);
    assert_eq!(hidden.content.1, 0.0);
    assert_eq!(
        hidden.rect.x,
        result.runs[0].rect.x + result.runs[0].rect.width
    );
    assert_eq!(result.runs[2].rect.x, hidden.rect.x);
}

#[test]
fn tables_fit_their_content_unless_given_a_width() {
    let (dom, result) = lay_out(
        "<table id=fit><tr><td>a</td><td>bb</td></tr></table>         <table id=full style='width:100%'><tr><td>a</td></tr></table>         <div id=block>x</div>",
        "body { margin: 0; width: 400px } td { padding: 0 }",
    );
    let [fit, full, block] = ["fit", "full", "block"].map(|id| rect(&dom, &result, id));
    assert_eq!(block.width, 400.0);
    assert!(fit.width > 0.0 && fit.width < 100.0, "{fit:?}");
    assert_eq!(full.width, 400.0);
}

#[test]
fn table_cells_align_their_content_vertically() {
    let (dom, result) = lay_out(
        "<table><tr><td id=tall><div style='height:50px'></div></td><td id=mid>m</td>\
         <td id=top style='vertical-align:top'>t</td><td id=bottom style='vertical-align:bottom'>b</td>\
         <td id=base style='vertical-align:baseline'>s</td><td style='display:none'>gone</td><td id=flex style='display:flex'><span>f</span></td></tr></table>",
        "body { margin: 0 } table { border-spacing: 0 } td { padding: 0; font-size: 10px; line-height: 10px }",
    );
    let y = |text: &str| {
        runs(&result)
            .into_iter()
            .find(|run| run.0 == text)
            .map(|run| run.2)
            .unwrap()
    };
    // Cells fill the row; their content is centered by default, as browsers
    // style table rows.
    assert_eq!(rect(&dom, &result, "mid").height, 50.0);
    assert_eq!(y("m"), 20.0);
    assert_eq!(y("t"), 0.0);
    assert_eq!(y("b"), 40.0);
    assert_eq!(y("s"), 0.0);
    // A box in a row that is not a cell keeps its own height in an
    // anonymous cell.
    let flex = rect(&dom, &result, "flex");
    assert_eq!((flex.y, flex.height), (0.0, 10.0));
    assert_eq!(y("f"), 0.0);
}

#[test]
fn baseline_alignment_lines_up_first_lines() {
    let (dom, result) = lay_out(
        "<label class=item><input id=box type=checkbox><span id=label>Completed reminders</span><span id=count>0 items</span></label>",
        "body { margin: 0 } .item { display: grid; grid-template-columns: auto 1fr; align-items: baseline } \
         #label { font-size: 10px; line-height: 20px } #count { grid-column: 2 } input { margin: 0 }",
    );
    // The check box sits on the label's first line however narrow the
    // label is measured.
    let check = rect(&dom, &result, "box");
    let label = rect(&dom, &result, "label");
    assert_eq!(label.y, 0.0);
    assert_eq!(check.y + check.height, 13.0);
    assert!(rect(&dom, &result, "count").y >= label.y + label.height);
}

#[test]
fn long_words_wrap_as_styled() {
    let (dom, result) = lay_out(
        "<div style='display:grid;width:100px'><p id=any class=w style='overflow-wrap:anywhere'>abcdefghijklmnopqrstuvwxyz</p></div>\
         <div style='display:grid;width:100px'><p id=word class=w style='overflow-wrap:break-word'>abcdefghijklmnopqrstuvwxyz</p></div>\
         <p id=all class=w style='width:100px;word-break:break-all'>ab abcdefghijklmnopqrstuvwxyz</p>\
         <p id=old class=w style='width:100px;word-break:break-word'>abcdefghijklmnopqrstuvwxyz</p>",
        "body { margin: 0 } .w { margin: 0; font-size: 10px; line-height: 10px }",
    );
    // `anywhere` lets a grid track shrink to the container; `break-word`
    // keeps the word's width as the minimum.
    assert_eq!(rect(&dom, &result, "any").width, 100.0);
    assert_eq!(rect(&dom, &result, "any").height, 20.0);
    assert_eq!(rect(&dom, &result, "word").width, 130.0);
    assert_eq!(rect(&dom, &result, "all").height, 20.0);
    assert_eq!(rect(&dom, &result, "old").height, 20.0);
    let lines: Vec<&str> = runs(&result).into_iter().map(|run| run.0).collect();
    assert!(lines.contains(&"ab abcdefghijklmnopq"));
}

#[test]
fn list_markers() {
    let html = "<ul><li id=a>one</li><li>two</li><li style='list-style-type:none'>none</li></ul>\
        <ol start=3><li>x</li><li value=10>y</li><li>z</li></ol>\
        <ol reversed><li>r1</li><li>r2</li></ol>\
        <ol style='list-style-type:upper-roman'><li>q</li></ol>\
        <ul style='list-style-position:inside'><li id=i>in</li></ul>\
        <details><summary id=s>More</summary></details><details open><summary>Less</summary></details>";
    let (dom, result) = lay_out(html, "");
    let texts: Vec<&str> = runs(&result).iter().map(|run| run.0).collect();
    for marker in [
        "\u{2022}", "3.", "10.", "11.", "2.", "I.", "\u{25B8}", "\u{25BE}",
    ] {
        assert!(
            texts.iter().any(|text| text.trim_end() == marker),
            "{marker} missing from {texts:?}"
        );
    }
    assert_eq!(
        texts
            .iter()
            .filter(|text| text.trim_end() == "\u{2022}")
            .count(),
        // Two outside the items and one inside.
        3
    );
    assert!(texts.iter().any(|text| text.trim_end() == "1."));
    let item = rect(&dom, &result, "a");
    let bullet = result
        .runs
        .iter()
        .find(|run| run.text.trim_end() == "\u{2022}")
        .unwrap();
    // A symbol starts two thirds of the ascent (12.8px at 16px) and 8px
    // before the item's content.
    assert_eq!(bullet.rect.x, item.x - (8.0 + 8.0));
    assert!(bullet.rect.width > 0.0);
    let one = result.runs.iter().find(|run| run.text == "one").unwrap();
    assert_eq!(bullet.rect.y, one.rect.y);
    let inside = rect(&dom, &result, "i");
    assert!(
        result
            .runs
            .iter()
            .any(|run| run.text.starts_with('\u{2022}') && run.rect.x >= inside.x)
    );
}

#[test]
fn marker_numbering() {
    use crate::counters::{alphabetic, roman};
    assert_eq!(alphabetic(1), "A");
    assert_eq!(alphabetic(26), "Z");
    assert_eq!(alphabetic(27), "AA");
    assert_eq!(roman(1994), "MCMXCIV");
    let text = |kind, ordinal| marker_text(kind, ordinal).unwrap();
    assert_eq!(text(ListStyleType::Circle, 1), "\u{25E6}");
    assert_eq!(text(ListStyleType::Square, 1), "\u{25AA}");
    assert_eq!(text(ListStyleType::DisclosureClosed, 1), "\u{25B8}");
    assert_eq!(text(ListStyleType::DisclosureOpen, 1), "\u{25BE}");
    assert_eq!(text(ListStyleType::DecimalLeadingZero, 7), "07.");
    assert_eq!(text(ListStyleType::LowerAlpha, 28), "ab.");
    assert_eq!(text(ListStyleType::UpperAlpha, 3), "C.");
    assert_eq!(text(ListStyleType::LowerRoman, 4), "iv.");
    assert_eq!(text(ListStyleType::UpperRoman, 4000), "4000.");
    assert_eq!(text(ListStyleType::LowerAlpha, 0), "0.");
    assert_eq!(text(ListStyleType::Decimal, -2), "-2.");
    assert_eq!(marker_text(ListStyleType::None, 1), None);
}

#[test]
fn broken_images_show_their_alternative_text() {
    let (dom, result) = lay_out(
        "<img id=broken src=unknown.png alt=' Logo '><img id=sized src=unknown.png alt=x width=5>         <img id=plain src=unknown.png alt=''><img id=fine src=ok.png alt=Fine>",
        "",
    );
    let texts: Vec<&str> = runs(&result).iter().map(|run| run.0).collect();
    assert_eq!(texts, ["Logo"]);
    assert!(rect(&dom, &result, "broken").width > 0.0);
    assert_eq!(rect(&dom, &result, "sized").width, 5.0);
    assert_eq!(rect(&dom, &result, "fine").width, 40.0);
}

#[test]
fn absolute_flex_boxes_shrink_to_sized_svg_content() {
    let (dom, result) = lay_out(
        "<div id=b><div id=d><svg id=s width='10' height='10'></svg></div></div>",
        "body { margin: 0 } #b { position: relative; display: block; width: 46px; height: 32px } \
         #d { position: absolute; display: inline-flex; top: 50%; left: 50% }",
    );
    let [d, s] = ["d", "s"].map(|id| rect(&dom, &result, id));
    assert_eq!((s.width, s.height), (10.0, 10.0), "{s:?}");
    assert_eq!(
        (d.x, d.y, d.width, d.height),
        (23.0, 16.0, 10.0, 10.0),
        "{d:?}"
    );
}

#[test]
fn intrinsic_widths_fit_the_content() {
    let words = "aa bbbb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy zz";
    let (dom, result) = lay_out(
        &format!(
            "<div id=max style='width:max-content'>ab cd</div>\
             <table id=table style='display:block;width:max-content;max-width:100%'><tr><td>ab</td></tr></table>\
             <div id=min style='width:min-content'>ab cdef</div>\
             <div id=fit style='width:fit-content'>{words}</div>\
             <div id=auto>ab</div>"
        ),
        "",
    );
    let max = rect(&dom, &result, "max");
    let min = rect(&dom, &result, "min");
    let fit = rect(&dom, &result, "fit");
    assert!(max.width > 0.0 && max.width < 100.0, "{max:?}");
    assert!(rect(&dom, &result, "table").width < 100.0);
    // The longest word.
    assert!(min.width > 0.0 && min.width < max.width, "{min:?}");
    // Long text fills the line and no more.
    assert_eq!(fit.width, 384.0);
    assert_eq!(rect(&dom, &result, "auto").width, 384.0);
}

#[test]
fn positioned_generated_text_is_placed_by_its_insets() {
    // As Muya shows emoji: the glyph is an absolutely positioned `::before`
    // of an empty inline-block, moved up and left of it.
    let (dom, result) = lay_out(
        "<p style='font-size:10px;line-height:20px;margin:0'>ab<span id=s class=e>cd</span>.</p>",
        ".e{display:inline-block;width:0;height:0;overflow:visible;position:relative;         vertical-align:middle;margin-left:10px;color:transparent}         .e::before{content:'X';position:absolute;top:-8px;left:-10px}",
    );
    let span = rect(&dom, &result, "s");
    let x = runs(&result)
        .into_iter()
        .find(|(text, _, _)| *text == "X")
        .unwrap();
    assert_eq!((x.1, x.2), (span.x - 10.0, span.y - 8.0));
    // Out of the flow, it takes no room on the line.
    let period = runs(&result)
        .into_iter()
        .find(|(text, _, _)| *text == ".")
        .unwrap();
    assert_eq!(period.1, span.x);
}

#[test]
fn rows_outside_tables_sit_in_an_anonymous_table() {
    // GitHub's markdown CSS makes tables blocks of their content's width.
    let (dom, result) = lay_out(
        "<table id=t style='display:block;width:max-content'><tbody><tr><td id=a>a</td><td id=b>bb</td></tr><tr><td id=c>c</td><td id=d>d</td></tr></tbody></table>\
         <div id=rows><div style='display:table-row'><span id=e style='display:table-cell'>e</span><span id=f style='display:table-cell'>f</span></div></div>",
        "",
    );
    let (a, b, c, d) = (
        rect(&dom, &result, "a"),
        rect(&dom, &result, "b"),
        rect(&dom, &result, "c"),
        rect(&dom, &result, "d"),
    );
    assert_eq!(a.y, b.y);
    assert!(b.x > a.x);
    assert!(c.y > a.y);
    assert_eq!((c.x, d.x), (a.x, b.x));
    assert!(rect(&dom, &result, "t").width < 100.0);
    let (e, f) = (rect(&dom, &result, "e"), rect(&dom, &result, "f"));
    assert_eq!(e.y, f.y);
    assert!(f.x > e.x);
    // The block itself still fills its line.
    assert_eq!(rect(&dom, &result, "rows").width, 384.0);
}

#[test]
fn atomic_boxes_sit_on_the_baseline_as_vertical_align_says() {
    // Text at 10px on 20px lines reaches 13px above its baseline.
    let (dom, result) = lay_out(
        "<div style='font-size:10px;line-height:20px'>ab<span id=b class=x style='width:10px;height:30px'></span>\
         <span id=l class=x style='width:5px;height:4px;vertical-align:-3px'></span>\
         <span id=t class=x style='width:5px;height:5px;vertical-align:top'></span>\
         <span id=m class=x style='width:5px;height:6px;vertical-align:middle'></span></div>",
        ".x{display:inline-block}",
    );
    // The tall box sits on the baseline, which moves 30px below the top.
    assert_eq!(rect(&dom, &result, "b"), Rect::new(18.0, 8.0, 10.0, 30.0));
    let text = runs(&result)[0];
    assert_eq!((text.0, text.2), ("ab", 8.0 + 30.0 - 13.0));
    // 3px below the baseline, at the line's top, and half an x-height above
    // it in the middle.
    assert_eq!(rect(&dom, &result, "l").y, 8.0 + 30.0 + 3.0 - 4.0);
    assert_eq!(rect(&dom, &result, "t").y, 8.0);
    assert!((rect(&dom, &result, "m").y - (8.0 + 30.0 + 0.5 - 6.0)).abs() <= 0.5);
    let line = dom
        .get_element_by_id(Dom::DOCUMENT, "b")
        .and_then(|node| dom.parent_element(node))
        .unwrap();
    assert_eq!(result.boxes[&line].rect.height, 37.0);
}

#[test]
fn atomic_boxes_wrap_with_the_text_unless_it_may_not() {
    let boxes = "<span class=x></span><span class=x></span><span class=x></span>";
    let (dom, result) = lay_out(
        &format!(
            "<div id=wrap style='width:50px'>{boxes}</div><div id=one style='width:50px;white-space:nowrap'>{boxes}</div>"
        ),
        ".x{display:inline-block;width:20px;height:10px}",
    );
    let lines = |id: &str| {
        let parent = dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
        let mut tops: Vec<f32> = dom
            .child_elements(parent)
            .into_iter()
            .map(|child| result.boxes[&child].rect.y)
            .collect();
        tops.dedup();
        tops.len()
    };
    assert_eq!(lines("wrap"), 2);
    assert_eq!(lines("one"), 1);
}

#[test]
fn inline_elements_holding_boxes_keep_their_text_on_the_baseline() {
    // As KaTeX nests its inline-blocks in inline spans.
    let (dom, result) = lay_out(
        "<p style='font-size:10px;line-height:20px;margin:0'>ab<span id=k style='font-size:20px;line-height:24px'><span id=x style='display:inline-block'>cd</span></span></p>",
        "",
    );
    let text = runs(&result);
    let (ab, cd) = (text[0], text[1]);
    // Both baselines meet: 13px below "ab"'s line top, and 16 + 2 below
    // "cd"'s.
    assert_eq!(ab.0, "ab");
    assert_eq!(cd.0, "cd");
    assert!((ab.2 + 13.0 - (cd.2 + 18.0)).abs() < 0.01, "{text:?}");
    let _ = rect(&dom, &result, "x");
}

#[test]
fn foreign_objects_lay_out_their_html_where_the_svg_puts_them() {
    // As Mermaid labels its nodes: HTML in a translated foreignObject.
    let (dom, result) = lay_out(
        "<div style='margin:0'><svg id=s width='100%' viewBox='-8 -8 200 100'>\
         <g transform='translate(20,30)'><foreignObject width='50' height='20'>\
         <div id=d style='display:table-cell;white-space:nowrap'>label text</div></foreignObject></g>\
         <foreignObject id=none style='display:none'><div id=n>x</div></foreignObject></svg></div>",
        "body{margin:0}",
    );
    let svg = rect(&dom, &result, "s");
    let label = rect(&dom, &result, "d");
    assert_eq!((label.x, label.y), (svg.x + 8.0 + 20.0, svg.y + 8.0 + 30.0));
    // A cell outside a table fits its text, beyond its zero-width parent.
    assert_eq!(label.width, 80.0);
    let hidden = dom.get_element_by_id(Dom::DOCUMENT, "n").unwrap();
    assert!(!result.boxes.contains_key(&hidden));
}

#[test]
fn boxes_in_lines_resolve_percentage_heights_against_their_container() {
    // As Muya's toolbar icons fill their 16px inline-block holders.
    let (dom, result) = lay_out(
        "<div><i id=holder style='display:inline-block;width:16px;height:16px'><i id=icon style='display:inline-block;width:100%;height:100%'></i></i></div>",
        "",
    );
    let icon = rect(&dom, &result, "icon");
    assert_eq!((icon.width, icon.height), (16.0, 16.0));
    assert_eq!(rect(&dom, &result, "holder").height, 16.0);
}

#[test]
fn empty_inline_boxes_make_no_line_unless_decorated() {
    let (dom, result) = lay_out(
        "<div id=a><span></span> <b></b></div>\
         <div id=b style='display:table-cell;line-height:1.5'><span></span></div>\
         <div id=c><span style='padding-top:2px'></span></div>\
         <div id=d><span style='padding-left:2px'></span></div>",
        "",
    );
    assert_eq!(rect(&dom, &result, "a").height, 0.0);
    assert_eq!(rect(&dom, &result, "b").height, 0.0);
    assert!(rect(&dom, &result, "c").height > 0.0);
    assert!(rect(&dom, &result, "d").height > 0.0);
}

#[test]
fn fixed_boxes_make_nothing_scroll() {
    // MarkText's body is fixed and parks its menus far to the right.
    let css = "body{position:fixed;margin:0;width:100vw;height:100vh}\
               #menu{position:absolute;right:-9999px;top:-9999px;width:100px;height:100px}";
    let (_, result) = lay_out("<div id=menu></div>", css);
    assert_eq!((result.width, result.height), (400.0, 300.0));
    let (_, result) = lay_out(
        "<div id=menu style='position:absolute;left:1000px;top:500px;width:10px;height:10px'></div>",
        "",
    );
    assert_eq!((result.width, result.height), (1010.0, 510.0));
}

fn styles_of(dom: &Dom, css: &str) -> Styles {
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
    Styles::compute(dom, &sheets, &StyleEnvironment::default())
}

#[test]
fn cached_layouts_match_fresh_ones_after_edits() {
    let css = "body{margin:0} .row{display:flex;gap:4px} .row>div{flex:1} \
               .abs{position:absolute;right:0;top:5px;width:20px} figure{position:relative} \
               td{padding:2px} .ib{display:inline-block;width:30px;height:10px}";
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<h1 id=h>Title</h1><p id=p>Some <b>bold</b> text <span class=ib></span> here</p>\
         <div class=row><div id=a>left side</div><div id=b>right side with more words</div></div>\
         <figure id=f><div class=abs id=abs>x</div><p id=q>figure text</p></figure>\
         <table><tr><td id=c>1</td><td>two</td></tr></table><ul><li id=li>item</li></ul>",
    ));
    let options = LayoutOptions {
        viewport: (300.0, 200.0),
        measurer: &Mono,
        image_size: &natural,
    };
    let mut cache = LayoutCache::default();
    let check = |dom: &Dom, cache: &mut LayoutCache| {
        let styles = styles_of(dom, css);
        let cached = layout_cached(dom, &styles, &options, cache);
        assert_eq!(cached, layout(dom, &styles, &options));
    };
    check(&dom, &mut cache);
    let total = cache.nodes.len();
    // The same document again reuses everything.
    check(&dom, &mut cache);
    assert_eq!(cache.reused(), total);
    let text_of = |dom: &Dom, id: &str| {
        dom.first_child(dom.get_element_by_id(Dom::DOCUMENT, id).unwrap())
            .unwrap()
    };
    // Typing into a paragraph, until it wraps.
    let text = text_of(&dom, "p");
    for typed in [
        "Some x",
        "Some xyz and a lot more words that wrap over lines ",
    ] {
        dom.set_data(text, typed);
        check(&dom, &mut cache);
        // Only the paragraph and the boxes around it are laid out again.
        assert!(cache.reused() > total / 2, "{} of {total}", cache.reused());
    }
    // A flex item grows, its sibling shrinks.
    let left = text_of(&dom, "a");
    dom.set_data(left, "left side now much longer than before");
    check(&dom, &mut cache);
    // Elements come and go.
    let q = dom.get_element_by_id(Dom::DOCUMENT, "q").unwrap();
    let extra = dom.create_element("p").unwrap();
    let words = dom.create_text("added paragraph");
    dom.append_child(extra, words).unwrap();
    dom.append_child(dom.parent(q).unwrap(), extra).unwrap();
    check(&dom, &mut cache);
    dom.remove_child(dom.parent(q).unwrap(), q).unwrap();
    check(&dom, &mut cache);
    // Styles change.
    let cell = dom.get_element_by_id(Dom::DOCUMENT, "c").unwrap();
    dom.set_attribute(cell, "style", "width:100px").unwrap();
    check(&dom, &mut cache);
    let heading = dom.get_element_by_id(Dom::DOCUMENT, "h").unwrap();
    dom.set_attribute(heading, "style", "display:none").unwrap();
    check(&dom, &mut cache);
    // Another viewport.
    let narrow = LayoutOptions {
        viewport: (120.0, 200.0),
        ..options
    };
    let styles = styles_of(&dom, css);
    assert_eq!(
        layout_cached(&dom, &styles, &narrow, &mut cache),
        layout(&dom, &styles, &narrow)
    );
}

#[test]
#[ignore = "timing"]
fn cached_layout_timing() {
    let mut html = String::new();
    for index in 0..500 {
        html.push_str(&format!(
            "<p id=p{index}>Paragraph {index} with <b>some</b> <i>styled</i> words that wrap over lines in a narrow column</p>"
        ));
    }
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(&html));
    let options = LayoutOptions {
        viewport: (300.0, 200.0),
        measurer: &Mono,
        image_size: &natural,
    };
    let mut cache = LayoutCache::default();
    let styles = styles_of(&dom, "");
    layout_cached(&dom, &styles, &options, &mut cache);
    let text = dom
        .first_child(dom.get_element_by_id(Dom::DOCUMENT, "p250").unwrap())
        .unwrap();
    let mut fresh = std::time::Duration::ZERO;
    let mut cached = std::time::Duration::ZERO;
    for round in 0..20 {
        dom.set_data(text, &format!("Edited {round}"));
        let styles = styles_of(&dom, "");
        let started = std::time::Instant::now();
        layout(&dom, &styles, &options);
        fresh += started.elapsed();
        let started = std::time::Instant::now();
        layout_cached(&dom, &styles, &options, &mut cache);
        cached += started.elapsed();
        dom.set_data(text, &format!("Edited again {round}"));
        let styles = styles_of(&dom, "");
        let started = std::time::Instant::now();
        layout_cached(&dom, &styles, &options, &mut cache);
        cached += started.elapsed();
    }
    let cached = cached / 2;
    eprintln!(
        "fresh {:?} cached {:?} reused {}",
        fresh / 20,
        cached / 20,
        cache.reused()
    );
}

#[test]
fn clipped_text_ends_in_an_ellipsis() {
    let (_, result) = lay_out(
        "<p style='width:60px;margin:0;overflow:hidden;white-space:nowrap;\
         text-overflow:ellipsis'>abcdefghijkl</p>\
         <p style='width:60px;margin:0;white-space:nowrap;text-overflow:ellipsis'>abcdefghijkl</p>",
        "",
    );
    assert_eq!(
        runs(&result),
        vec![
            ("abcdef", 8.0, 8.0),
            ("\u{2026}", 56.0, 8.0),
            ("abcdefghijkl", 8.0, 27.0),
        ]
    );
}

#[test]
fn line_clamps_cut_vertical_webkit_boxes_off_with_an_ellipsis() {
    let (dom, result) = lay_out(
        "<div id=d style='width:60px;display:-webkit-box;-webkit-box-orient:vertical;\
         -webkit-line-clamp:2;overflow:hidden'>aaa bbb ccc ddd eee</div>",
        "",
    );
    let texts: Vec<_> = runs(&result).into_iter().map(|run| run.0).collect();
    assert_eq!(texts.concat(), "aaa bbbccc dd\u{2026}");
    let mark = result.runs.last().unwrap();
    assert_eq!(mark.text, "\u{2026}");
    assert_eq!(
        mark.node,
        dom.get_element_by_id(Dom::DOCUMENT, "d").unwrap()
    );
    assert!(mark.rect.x + mark.rect.width <= 68.0);
    assert_eq!(rect(&dom, &result, "d").height, 38.0);
}

#[test]
fn scrollbars_take_room_from_their_boxes() {
    let (dom, result) = lay_out(
        "<div id=tall style='overflow:auto;width:100px;height:40px'><div id=t style='height:80px'></div></div>\
         <div id=short style='overflow:auto;width:100px;height:40px'><div id=s style='height:20px'></div></div>\
         <div id=always style='overflow:scroll;width:100px;height:40px'><div id=a></div></div>\
         <div id=thin class=thin style='overflow-y:auto;width:100px;height:40px'><div id=n style='height:80px'></div></div>\
         <div id=none style='overflow-y:auto;scrollbar-width:none;width:100px;height:40px'><div id=o style='height:80px'></div></div>\
         <p id=p style='overflow-y:auto;width:96px;height:20px;margin:0'>aaa bbb ccc ddd eee</p>         <div id=menu style='overflow-y:auto;width:100px;height:40px'><div id=m style='position:relative;height:40px'>           <i style='position:absolute;top:50%;height:100%;width:4px;transform:translateY(-50%)'></i></div></div>",
        ".thin::-webkit-scrollbar { width: 12px }",
    );
    let width = |id: &str| rect(&dom, &result, id).width;
    let bars = |id: &str| {
        let node = dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
        result.boxes[&node].scrollbars
    };
    // Content that overflows down takes a vertical scrollbar's width.
    assert_eq!((width("t"), bars("tall")), (83.0, [17.0, 0.0]));
    assert_eq!((width("s"), bars("short")), (100.0, [0.0, 0.0]));
    // `scroll` shows both, overflowing or not.
    assert_eq!((width("a"), bars("always")), (83.0, [17.0, 17.0]));
    // Content that fits scrolls only the client area, as `scrollWidth` is.
    let always = &result.boxes[&dom.get_element_by_id(Dom::DOCUMENT, "always").unwrap()];
    assert_eq!((always.content_width, always.content_height), (83.0, 23.0));
    assert_eq!((width("n"), bars("thin")), (88.0, [12.0, 0.0]));
    assert_eq!((width("o"), bars("none")), (100.0, [0.0, 0.0]));
    // A marker centered on the last item by a translation hangs below it
    // only before the translation: it takes no scrollbar.
    assert_eq!((width("m"), bars("menu")), (100.0, [0.0, 0.0]));
    // Lines wrap in the room the scrollbar leaves: 79 pixels hold nine
    // characters of 8 pixels.
    let lines: Vec<_> = runs(&result)
        .into_iter()
        .filter(|(text, ..)| {
            ["aaa", "bbb", "ccc", "ddd", "eee"]
                .iter()
                .any(|word| text.contains(word))
        })
        .map(|(text, ..)| text)
        .collect();
    assert_eq!(lines, ["aaa bbb", "ccc ddd", "eee"]);
}

#[test]
fn columns_take_the_widths_col_elements_give() {
    let (dom, result) = lay_out(
        "<table id=t style='border-spacing:0'><colgroup><col width=120><col width='40'></colgroup>\
           <tr><td id=a>a</td><td id=b>b</td><td id=c>c</td></tr></table>\
         <table style='border-spacing:0'><col span=2 width=30><colgroup width=50></colgroup>\
           <tr><td id=d>d</td><td id=e>e</td><td id=f>f</td><td id=g width=0>g</td></tr></table>\
         <table style='border-spacing:0'><tr><td id=h width=70>h</td></tr></table>",
        "td { padding: 0 }",
    );
    let width = |id: &str| rect(&dom, &result, id).width;
    // Sized columns are as wide as their col elements say; others fit
    // their content.
    assert_eq!((width("a"), width("b")), (120.0, 40.0));
    assert!(width("c") < 40.0);
    // A span repeats a column's width; a column group without columns
    // sizes as many as it spans.
    assert_eq!((width("d"), width("e"), width("f")), (30.0, 30.0, 50.0));
    // Cells map their width attribute; zero maps none.
    assert!(width("g") < 30.0);
    assert_eq!(width("h"), 70.0);
}

#[test]
fn inline_elements_around_blocks_split_as_blocks() {
    let (dom, result) = lay_out(
        "<div style='width:300px'><code id=c><span id=b style='display:block'>ab</span></code>         <span id=s><img src=x width=10 height=10></span></div>",
        "body { margin: 0 }",
    );
    // The block takes the containing block's width, and the inline element
    // around it spans it, as Chromium's block-in-inline split lays them out.
    assert_eq!(rect(&dom, &result, "b").width, 300.0);
    assert_eq!(rect(&dom, &result, "c").width, 300.0);
    // An inline element around replaced content stays inline.
    assert!(rect(&dom, &result, "s").width < 300.0);
    let (dom, result) = lay_out(
        "<div style='width:300px'><span id=f>a<span style='display:block;float:left'>b</span></span></div>",
        "body { margin: 0 }",
    );
    // A float inside splits nothing.
    assert!(rect(&dom, &result, "f").width < 300.0);
}

#[test]
fn block_generated_content_is_a_block_of_its_own() {
    // Prism's line numbers: a block ::before aligned right in its row.
    let (_, result) = lay_out(
        "<div id=r style='width:100px'><span id=s style='display:block'>x</span></div>",
        "#s::before { content: '7'; display: block; text-align: right; padding-right: 10px }",
    );
    let seven = result.runs.iter().find(|run| run.text == "7").unwrap();
    let x = result.runs.iter().find(|run| run.text == "x").unwrap();
    // On its own line, at the right less its padding.
    assert!(seven.rect.y < x.rect.y);
    assert_eq!(seven.rect.x + seven.rect.width, 8.0 + 100.0 - 10.0);
}

#[test]
fn letter_spacing_widens_transformed_text() {
    let (_, result) = lay_out(
        "<div id=a>abc</div><div id=b style='text-transform:uppercase'>abc</div>",
        "div { letter-spacing: 1px; font-size: 13px; line-height: 120%; display: flex; padding: 10px 14px; font-weight: 500 }",
    );
    // Mono's 6.5 pixels a character and a pixel after each, uppercased too.
    let widths: Vec<f32> = result.runs.iter().map(|run| run.rect.width).collect();
    assert_eq!(widths, vec![22.5, 22.5]);
}

#[test]
fn absolute_boxes_keep_their_static_position_in_their_parent() {
    // As Muya's HTML block briefly is: an absolute box with automatic
    // insets inside a static block, its containing block further out.
    let (dom, result) = lay_out(
        "<div id=c style='position:relative;padding:5px'><p style='height:30px;margin:0'></p>         <div id=f style='margin-left:40px'><span>a</span><pre id=a style='position:absolute;margin:3px'>x</pre></div></div>",
        "body { margin: 0 } p { margin: 0 }",
    );
    let figure = rect(&dom, &result, "f");
    let placed = rect(&dom, &result, "a");
    // Across and down from where it would have been: after the line in
    // its parent, past its own margin.
    assert_eq!(placed.x, figure.x + 3.0);
    assert!(placed.y > figure.y, "{placed:?} {figure:?}");
    // Insets given place it against the containing block instead.
    let (dom, result) = lay_out(
        "<div id=c style='position:relative'><div style='margin-left:40px'><pre id=a style='position:absolute;left:0;margin:0'>x</pre></div></div>",
        "body { margin: 0 }",
    );
    assert_eq!(rect(&dom, &result, "a").x, 0.0);
}

#[test]
fn empty_blocks_let_margins_collapse_through_and_floats_stay_in_their_context() {
    // An empty block, with an empty child or an absolutely positioned one,
    // is no gap: the margins around it collapse into one, as in Chromium.
    for html in [
        "<p id=a>a</p><figure></figure><p id=b>b</p>",
        "<p id=a>a</p><figure><div></div></figure><p id=b>b</p>",
        "<p id=a>a</p><figure><div></div><pre style='position:absolute;margin:0'>x</pre></figure><p id=b>b</p>",
    ] {
        let (dom, result) = lay_out(
            html,
            "body{margin:0} p,figure{margin:8px 0;line-height:20px}",
        );
        assert_eq!(rect(&dom, &result, "a").y, 8.0, "{html}");
        assert_eq!(rect(&dom, &result, "b").y, 36.0, "{html}");
    }
    // A block holding only floats is empty; one starting its own context
    // holds them.
    let (dom, result) = lay_out(
        "<div id=plain><div style='float:left;width:10px;height:30px'></div></div>         <div id=root style='overflow:hidden'><div style='float:left;width:10px;height:30px'></div></div>",
        "body{margin:0}",
    );
    assert_eq!(rect(&dom, &result, "plain").height, 0.0);
    assert_eq!(rect(&dom, &result, "root").height, 30.0);
}

#[test]
fn right_to_left_text_and_rows_start_on_the_right() {
    // A right-to-left box clips a long left-to-right path from the left,
    // as an application title bar does, keeping its end in view.
    let (dom, result) = lay_out(
        "<div style='display:flex;width:100px'><span id=p style='direction:rtl;overflow:hidden;white-space:nowrap;min-width:0'>\
         <bdi dir=ltr><span>/home/someone/</span><span>user</span></bdi></span></div>\
         <p dir=rtl style='width:100px;margin:0'>ab</p>\
         <div dir=auto>\u{5e9}\u{5dc}\u{5d5}\u{5dd}</div>\
         <div style='display:flex;direction:rtl;width:100px'><i id=a style='width:10px'></i><i id=b style='width:20px'></i></div>",
        "",
    );
    assert_eq!(rect(&dom, &result, "p").width, 100.0);
    let runs = runs(&result);
    assert_eq!(runs[0].0, "/home/someone/");
    assert_eq!((runs[0].1, runs[1].1), (-36.0, 76.0));
    assert_eq!((runs[2].0, runs[2].1), ("ab", 92.0));
    assert_eq!(runs[3].1, 360.0);
    assert_eq!(rect(&dom, &result, "a").x, 98.0);
    assert_eq!(rect(&dom, &result, "b").x, 78.0);
}

#[test]
fn positioned_generated_text_of_inline_elements_follows_them() {
    // As Muya marks a hard line break: trailing spaces in a relatively
    // positioned span, whose absolutely positioned `::before` shows a mark
    // just after them.
    let (_, result) = lay_out(
        "<div style='width:300px;position:relative;padding-top:300px'><p style='height:100px;margin:0'></p><p style='position:relative;font-size:10px;line-height:20px;margin:0'><span style='display:block;white-space:pre-wrap'>this line<span><span class=s>  </span></span><span>\n</span>next</span></p><p style='height:200px'></p><p>end</p></div>",
        ".s{position:relative} .s::before{position:absolute;content:'R';inset-inline-start:100%}",
    );
    let found = runs(&result);
    let line = found
        .iter()
        .find(|(text, _, _)| *text == "this line")
        .unwrap();
    let mark = found.iter().find(|(text, _, _)| *text == "R").unwrap();
    assert_eq!(mark.2, line.2, "{found:?}");
    assert!(
        mark.1 >= line.1 + 45.0 && mark.1 < line.1 + 80.0,
        "{found:?}"
    );
}

#[test]
fn lines_beside_floats_shorten_and_widen_below_them() {
    let css = "body{margin:0} div{width:100px;font-size:10px;line-height:10px} p{margin:0}";
    let (dom, result) = lay_out(
        "<div><i style='float:left;width:40px;height:25px'></i>\
         <p id=t>aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk</p></div>",
        css,
    );
    assert_eq!(
        runs(&result),
        vec![
            ("aaaa bbbb", 40.0, 0.0),
            ("cccc dddd", 40.0, 10.0),
            ("eeee ffff", 40.0, 20.0),
            ("gggg hhhh iiii jjjj", 0.0, 30.0),
            ("kkkk", 0.0, 40.0),
        ]
    );
    // The paragraph's box still spans the float.
    assert_eq!(rect(&dom, &result, "t"), Rect::new(0.0, 0.0, 100.0, 50.0));
    // A line whose first word does not fit beside a float moves below it;
    // right floats leave the room on the left.
    let (dom, result) = lay_out(
        "<div><i style='float:right;width:90px;height:15px'></i><p id=t>aaaa</p></div>\
         <div><i style='float:right;width:50px;height:10px'></i><p>bb cc</p></div>",
        css,
    );
    assert_eq!(
        runs(&result),
        vec![("aaaa", 0.0, 15.0), ("bb cc", 0.0, 25.0)]
    );
    assert_eq!(rect(&dom, &result, "t").height, 25.0);
}

#[test]
fn tables_size_columns_by_their_cells_widths_and_layout() {
    let css = "body{margin:0} table{border-spacing:0} td{padding:0;font-size:10px}";
    let width = |dom: &Dom, result: &LayoutResult, id: &str| rect(dom, result, id).width;
    // A share of the table goes to its column, which every cell of it
    // fills; a length keeps its column at it, and the automatic column
    // takes the room to spare.
    let (dom, result) = lay_out(
        "<table style='width:400px'><tr><td id=a>auto</td><td id=b style='width:50%'>half</td>\
         <td id=c style='width:40px'>x</td></tr><tr><td>1</td><td id=d>2</td><td>3</td></tr></table>",
        css,
    );
    assert_eq!(width(&dom, &result, "b"), 200.0);
    assert_eq!(width(&dom, &result, "d"), 200.0);
    assert_eq!(width(&dom, &result, "c"), 40.0);
    assert_eq!(width(&dom, &result, "a"), 160.0);
    // A table of automatic width grows to give a share its part.
    let (dom, result) = lay_out(
        "<table><tr><td id=a>abcd</td><td id=b style='width:50%'>ab</td></tr></table>",
        css,
    );
    assert_eq!(width(&dom, &result, "a"), 20.0);
    assert_eq!(width(&dom, &result, "b"), 20.0);
    // `table-layout: fixed` takes the first row's widths and shares the
    // rest, whatever the cells hold.
    let (dom, result) = lay_out(
        "<table style='table-layout:fixed;width:300px'><tr><td id=a style='width:100px'>a</td>\
         <td id=b>b</td><td id=c>a much longer cell</td></tr></table>",
        css,
    );
    assert_eq!(width(&dom, &result, "a"), 100.0);
    assert_eq!(width(&dom, &result, "b"), 100.0);
    assert_eq!(width(&dom, &result, "c"), 100.0);
}

#[test]
fn captions_sit_beside_their_tables_which_keep_their_margins_and_widths() {
    let html = "<table id=t><caption id=top>Top</caption><caption id=bottom>Bottom</caption>        <tr><td id=cell>cell</td></tr></table>";
    let css = "body { margin: 0 } table { margin: 10px; border: 3px solid; width: 50% }         #bottom { caption-side: bottom }";
    let (dom, result) = lay_out(html, css);
    let [table, top, bottom, cell] =
        ["t", "top", "bottom", "cell"].map(|id| rect(&dom, &result, id));
    // The margins go around the captions too; the width is the table's
    // border box's, a share of the page.
    assert_eq!((top.x, top.y), (10.0, 10.0));
    assert_eq!(table.y, top.y + top.height);
    assert_eq!(bottom.y, table.y + table.height);
    assert_eq!(table.width, 400.0 / 2.0);
    assert_eq!((top.width, bottom.width), (table.width, table.width));
    assert!(cell.y > table.y && cell.y + cell.height < bottom.y);
    // A floated table keeps its caption inside its border.
    let (dom, result) = lay_out(
        html,
        "body { margin: 0 } table { float: left; border: 3px solid }",
    );
    let [table, top] = ["t", "top"].map(|id| rect(&dom, &result, id));
    assert!(top.y > table.y);
}

#[test]
fn hosts_lay_out_their_shadow_trees_with_slotted_children_in_their_slots() {
    use nativeboat_dom::ShadowRootMode;
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=host>light<b slot=end>slotted</b><i>unshown</i></div>",
    ));
    let host = dom.get_element_by_id(Dom::DOCUMENT, "host").unwrap();
    let root = dom
        .attach_shadow(host, ShadowRootMode::Closed, false)
        .unwrap();
    dom.set_inner_html(
        root,
        "<p>before</p><slot name=end>fallback</slot><slot name=none>fallback</slot>",
    );
    let result = lay_out_dom(&dom, "");
    let texts: Vec<&str> = result.runs.iter().map(|run| run.text.trim()).collect();
    assert!(texts.contains(&"before"), "{texts:?}");
    assert!(texts.contains(&"slotted"), "{texts:?}");
    assert_eq!(
        texts.iter().filter(|text| **text == "fallback").count(),
        1,
        "{texts:?}"
    );
    assert!(!texts.contains(&"light"), "{texts:?}");
    assert!(!texts.contains(&"unshown"), "{texts:?}");
}
