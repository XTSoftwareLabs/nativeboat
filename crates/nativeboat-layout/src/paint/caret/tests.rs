use super::*;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, scene};
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};
use nativeboat_ir::Font;

/// Every character is half the font size wide.
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

struct Page {
    dom: Dom,
    styles: Styles,
    layout: LayoutResult,
    scene: Scene,
}

impl Page {
    fn new(html: &str, css: &str) -> Page {
        Page::from_dom(
            Dom::from_document(&nativeboat_html::parse_document(html)),
            css,
        )
    }

    fn from_dom(dom: Dom, css: &str) -> Page {
        let base = "body { margin: 0; font-size: 10px; line-height: 10px } p { margin: 0 }";
        let (author, _) = parse_stylesheet(&format!("{base} {css}"));
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
        let scene = scene(&dom, &styles, &layout, (400.0, 300.0));
        Page {
            dom,
            styles,
            layout,
            scene,
        }
    }

    fn query(&self) -> TextQuery<'_> {
        TextQuery {
            dom: &self.dom,
            styles: &self.styles,
            layout: &self.layout,
            measurer: &Mono,
        }
    }

    fn id(&self, id: &str) -> NodeId {
        self.dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
    }

    /// The text node child of an element.
    fn text(&self, id: &str) -> NodeId {
        let element = self.id(id);
        self.dom
            .children(element)
            .into_iter()
            .find(|child| self.dom.node_type(*child) == node_type::TEXT)
            .unwrap()
    }

    fn caret(&self, x: f32, y: f32) -> Option<(NodeId, u32)> {
        self.scene.caret_from_point(&self.query(), x, y)
    }

    fn rects(&self, node: NodeId, start: u32, end: u32) -> Vec<Rect> {
        self.scene.text_rects(&self.query(), node, start, end)
    }
}

#[test]
fn laid_out_text_aligns_with_collapsed_dropped_and_expanded_source() {
    assert_eq!(boundaries("a   b", &["a b"]), vec![vec![0, 1, 4, 5]]);
    assert_eq!(
        boundaries("hello world", &["hello", "world"]),
        vec![vec![0, 1, 2, 3, 4, 5], vec![6, 7, 8, 9, 10, 11]]
    );
    assert_eq!(boundaries("  x", &["x"]), vec![vec![2, 3]]);
    assert_eq!(
        boundaries("a\tb", &["a        b"]),
        vec![vec![0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3]]
    );
    assert_eq!(
        boundaries("\u{1F600}x", &["\u{1F600}x"]),
        vec![vec![0, 2, 3]]
    );
    assert_eq!(boundaries("\u{DF}", &["SS"]), vec![vec![0, 1, 1]]);
    assert_eq!(
        boundaries("a\n\nb", &["a", "b"]),
        vec![vec![0, 1], vec![3, 4]]
    );
}

#[test]
fn carets_land_on_the_nearest_character_boundary_of_the_line() {
    let page = Page::new("<p id=p>hello world</p><p id=q>ab <b id=b>cd</b></p>", "");
    let hello = page.text("p");
    assert_eq!(page.caret(12.0, 5.0), Some((hello, 2)));
    assert_eq!(page.caret(13.0, 5.0), Some((hello, 3)));
    assert_eq!(page.caret(0.0, 5.0), Some((hello, 0)));
    // Right of the text, the caret is at the end of the line.
    assert_eq!(page.caret(300.0, 5.0), Some((hello, 11)));
    let bold = page.text("b");
    assert_eq!(page.caret(17.0, 15.0), Some((bold, 0)));
    assert_eq!(page.caret(21.0, 15.0), Some((bold, 1)));
    // Nothing is painted below the document.
    assert_eq!(page.caret(395.0, 299.0), None);
}

#[test]
fn carets_in_boxes_without_text_enter_their_first_rendered_descendant() {
    let page = Page::new(
        "<div id=e style='height:20px'><span id=s></span></div>\
         <p id=q style='height:20px'><span id=c><br></span></p>\
         <div id=w style='height:20px'> <!--x--> <i id=i></i></div>",
        "",
    );
    assert_eq!(page.caret(5.0, 5.0), Some((page.id("s"), 0)));
    assert_eq!(page.caret(5.0, 25.0), Some((page.id("c"), 0)));
    assert_eq!(page.caret(5.0, 45.0), Some((page.id("i"), 0)));
}

#[test]
fn carets_skip_clipped_and_generated_text() {
    let page = Page::new(
        "<div id=c style='height:30px'><span id=a>aa</span>\
         <div style='height:10px;overflow:hidden'><br>bb</div></div>\
         <p id=p><span id=g></span>xyz</p>",
        "#g::before { content: '>>' }",
    );
    // The nearest text is clipped away from the point, so the caret goes to
    // the nearest visible text.
    assert_eq!(page.caret(2.0, 25.0), Some((page.text("a"), 0)));
    // Generated content takes no caret: the paragraph's own text does.
    let xyz = page.dom.children(page.id("p"))[1];
    assert_eq!(page.caret(2.0, 35.0), Some((xyz, 0)));
}

#[test]
fn carets_in_collapsed_transforms_fall_back_to_the_start_of_the_text() {
    let page = Page::new(
        "<div style='height:20px'>\
         <span id=s style='display:inline-block;transform:scale(0)'>ab</span></div>",
        "",
    );
    assert_eq!(page.caret(5.0, 5.0), Some((page.text("s"), 0)));
}

#[test]
fn transformed_text_maps_points_and_rectangles_through_the_transform() {
    let page = Page::new(
        "<p id=p>abcd</p>",
        "#p { width: 20px; transform: scale(2) }",
    );
    let text = page.text("p");
    // The paragraph scales about its center, (10, 5).
    assert_eq!(page.caret(11.0, 5.0), Some((text, 2)));
    assert_eq!(
        page.rects(text, 1, 3),
        vec![Rect::new(0.0, -5.0, 20.0, 20.0)]
    );
}

#[test]
fn ranges_cover_each_line_they_touch() {
    let page = Page::new(
        "<p id=p style='width:30px'>hello world</p>\
         <p id=h><span id=s style='visibility:hidden'>hid</span></p>",
        "",
    );
    let text = page.text("p");
    assert_eq!(
        page.rects(text, 3, 8),
        vec![
            Rect::new(15.0, 0.0, 10.0, 10.0),
            Rect::new(0.0, 10.0, 10.0, 10.0)
        ]
    );
    assert_eq!(page.rects(text, 8, 3), page.rects(text, 3, 8));
    // A point on the second line takes a caret there.
    assert_eq!(page.caret(3.0, 15.0), Some((text, 7)));
    // A caret at a line break shows at the end of the first line.
    assert_eq!(
        page.rects(text, 5, 5),
        vec![Rect::new(25.0, 0.0, 0.0, 10.0)]
    );
    assert_eq!(
        page.rects(text, 9, 9),
        vec![Rect::new(15.0, 10.0, 0.0, 10.0)]
    );
    assert_eq!(page.rects(text, 12, 14), Vec::<Rect>::new());
    // Hidden text has its rectangles, as Chromium reports them; elements
    // have no text rectangles.
    assert_eq!(
        page.rects(page.text("s"), 0, 3),
        vec![Rect::new(0.0, 20.0, 15.0, 10.0)]
    );
    assert_eq!(page.rects(page.id("p"), 0, 1), Vec::<Rect>::new());
}

#[test]
fn points_between_blocks_go_to_the_block_after_them() {
    let page = Page::new(
        "<div id=c><p id=a style='margin-bottom:20px'>first</p><p id=b>second</p></div>",
        "",
    );
    let second = page.text("b");
    // Just below the first paragraph, nearer to it, as Chromium places it.
    assert_eq!(page.caret(0.0, 12.0), Some((second, 0)));
    assert_eq!(page.caret(300.0, 28.0), Some((second, 6)));
    // Inside a paragraph, its own line.
    assert_eq!(page.caret(0.0, 5.0), Some((page.text("a"), 0)));
}

#[test]
fn text_at_font_size_zero_is_an_empty_rectangle_on_the_baseline() {
    // As Muya hides syntax markers, ranges and carets in the text and its
    // element report an empty rectangle where it sits, as Chromium does.
    let page = Page::new(
        "<p id=p>ab<span id=z style='font-size:0'>xy</span>cd</p>",
        "",
    );
    let hidden = page.text("z");
    let place = Rect::new(10.0, 8.0, 0.0, 0.0);
    assert_eq!(page.rects(hidden, 0, 2), vec![place]);
    assert_eq!(page.rects(hidden, 1, 1), vec![place]);
    assert_eq!(page.scene.client_rect(page.id("z")), Some(place));
    // The text around it keeps its place.
    assert_eq!(
        page.rects(page.text("p"), 0, 2),
        vec![Rect::new(0.0, 0.0, 10.0, 10.0)]
    );
}

#[test]
fn carets_left_of_hidden_markers_go_after_them() {
    // Muya hides a heading's markers at `font-size: 0`. A point left of
    // the line, or above it, puts the caret after them, where the visible
    // text starts, as Chromium places it.
    let page = Page::new(
        "<div id=d style='padding-left:20px'><p id=a>x</p>\
         <h2 id=h style='font-size:24px;line-height:1.4;margin:5px 0 0'>\
         <span id=m style='font-size:0'>##</span><span id=s style='font-size:0'> </span>Heading</h2></div>",
        "",
    );
    let heading = page.dom.children(page.id("h"))[2];
    assert_eq!(page.caret(5.0, 12.0), Some((heading, 0)));
    assert_eq!(page.caret(5.0, 25.0), Some((heading, 0)));
}

#[test]
fn carets_in_preformatted_tokens_sit_on_their_line() {
    let page = Page::new(
        "<pre id=p style='margin:0;line-height:1.6;font-size:14px;padding:15px'><code style='display:block'>\
         <span>function</span> f() {\n  <span id=r>return</span> a<span id=s>;</span>\n}</code></pre>",
        "",
    );
    let semicolon = page.text("s");
    let line = page.scene.client_rect(page.id("s")).unwrap();
    let caret = page.rects(semicolon, 1, 1);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert_eq!((caret[0].y, caret[0].height), (line.y, line.height));
}

#[test]
fn carets_between_line_breaks_have_no_rectangle() {
    // As Chromium reports them: a caret beside text has a rectangle, one
    // between line breaks or in text of only line breaks has none.
    let page = Page::new(
        "<pre id=p style='margin:0'>a\n\nb</pre><pre id=q style='margin:0'><span>a</span>\n\n<span>b</span></pre>",
        "",
    );
    let text = page.text("p");
    let counts: Vec<usize> = (0..=4)
        .map(|offset| page.rects(text, offset, offset).len())
        .collect();
    assert_eq!(counts, [1, 1, 0, 1, 1]);
    let breaks = page.dom.children(page.id("q"))[1];
    for offset in 0..=2 {
        assert!(page.rects(breaks, offset, offset).is_empty());
    }
}

#[test]
fn carets_in_scrolled_preformatted_surfaces_are_in_window_coordinates() {
    // Muya's HTML block: a scrolling pre inside the scrolled editor.
    let html = "<div id=e style='height:200px;overflow:auto'><div style='height:150px'></div>\
         <pre id=p style='overflow:auto;margin:0;padding:15px;font-size:10px;line-height:20px'><code>\
         <span><span>&lt;div</span><span id=g>&gt;</span></span>\n\n<span>&lt;/div&gt;</span></code></pre>\
         <div style='height:600px'></div></div>";
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let mut dom = dom;
    let editor = dom.get_element_by_id(Dom::DOCUMENT, "e").unwrap();
    dom.set_scroll(editor, 0.0, 100.0);
    let page = Page::from_dom(dom, "");
    let bracket = page.text("g");
    let caret = page.rects(bracket, 1, 1);
    let tag = page.scene.client_rect(page.id("g")).unwrap();
    assert_eq!(caret.len(), 1);
    // At the end of the bracket, on its line, where the element is.
    assert_eq!((caret[0].x, caret[0].y), (tag.x + tag.width, tag.y));
    assert_eq!(tag.y, 150.0 - 100.0 + 15.0 + 5.0);
}

#[test]
fn right_to_left_text_places_its_first_characters_on_the_right() {
    // Three 5px characters at the right of a 400px line.
    let page = Page::new("<p id=p dir=rtl>\u{5d0}\u{5d1}\u{5d2}</p>", "");
    let text = page.text("p");
    assert_eq!(page.rects(text, 0, 0), [Rect::new(400.0, 0.0, 0.0, 10.0)]);
    assert_eq!(page.rects(text, 1, 1), [Rect::new(395.0, 0.0, 0.0, 10.0)]);
    assert_eq!(page.rects(text, 0, 1), [Rect::new(395.0, 0.0, 5.0, 10.0)]);
    assert_eq!(page.caret(398.0, 5.0), Some((text, 0)));
    assert_eq!(page.caret(387.0, 5.0), Some((text, 3)));
    // Left-to-right text is unchanged.
    let page = Page::new("<p id=p>abc</p>", "");
    let text = page.text("p");
    assert_eq!(page.rects(text, 1, 1), [Rect::new(5.0, 0.0, 0.0, 10.0)]);
    assert_eq!(page.caret(13.0, 5.0), Some((text, 3)));
}
