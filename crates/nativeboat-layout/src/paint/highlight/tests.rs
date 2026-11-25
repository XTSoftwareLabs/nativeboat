use super::*;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, scene_with};
use nativeboat_css::{Origin, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};
use nativeboat_dom::node_type;
use nativeboat_ir::{DisplayItem, Font};

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
    css: String,
    misspellings: Vec<(NodeId, Vec<(u32, u32)>)>,
}

impl Page {
    fn new(html: &str, css: &str) -> Page {
        let base = "body { margin: 0; font-size: 10px; line-height: 10px } p { margin: 0 }";
        Page {
            dom: Dom::from_document(&nativeboat_html::parse_document(html)),
            css: format!("{base} {css}"),
            misspellings: Vec::new(),
        }
    }

    fn id(&self, id: &str) -> NodeId {
        self.dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
    }

    /// The first text node in an element.
    fn text(&self, id: &str) -> NodeId {
        self.dom
            .descendants(self.id(id))
            .into_iter()
            .find(|node| self.dom.node_type(*node) == node_type::TEXT)
            .unwrap()
    }

    fn select(&mut self, start: (NodeId, u32), end: (NodeId, u32)) {
        let range = self.dom.create_range(start, end);
        self.dom.set_selection(Some(range));
    }

    fn focus(&mut self, id: &str) {
        let node = self.id(id);
        assert!(self.dom.set_focus(Some(node), false).is_some());
    }

    /// The display items of the root surface after the canvas, summarized.
    fn painted(&self) -> Vec<String> {
        let (author, _) = parse_stylesheet(&self.css);
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
        let styles = Styles::compute(&self.dom, &sheets, &StyleEnvironment::default());
        let options = LayoutOptions {
            viewport: (400.0, 300.0),
            measurer: &Mono,
            image_size: &|_| None,
        };
        let layout = layout(&self.dom, &styles, &options);
        let mut highlight = Highlight::new(&self.dom, &styles, &sheets, &Mono);
        highlight.mark_misspellings(&self.dom, &styles, &sheets, self.misspellings.clone());
        let scene = scene_with(
            &self.dom,
            &styles,
            &layout,
            (400.0, 300.0),
            Some(&highlight),
        );
        scene.tree.root.items[1..].iter().map(summary).collect()
    }

    /// The caret item, if one is painted.
    fn caret(&self) -> Option<String> {
        self.painted()
            .into_iter()
            .find(|item| item.starts_with("caret"))
    }
}

fn summary(item: &DisplayItem) -> String {
    match item {
        DisplayItem::Fill { rect, color, .. } => {
            format!("fill {} {} {}", rect.x, rect.width, color.to_css())
        }
        DisplayItem::PushClip { rect, .. } => format!("clip {} {}", rect.x, rect.width),
        DisplayItem::PopClip => "pop".to_owned(),
        DisplayItem::Text { text, color, .. } => format!("{text} {}", color.to_css()),
        DisplayItem::Caret { rect, color } => format!(
            "caret {} {} {} {} {}",
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            color.to_css()
        ),
        DisplayItem::Path {
            segments,
            stroke: Some(stroke),
            ..
        } => {
            let xs: Vec<f32> = segments
                .iter()
                .filter_map(|segment| match segment {
                    nativeboat_ir::PathSegment::Move(x, _)
                    | nativeboat_ir::PathSegment::Line(x, _) => Some(*x),
                    _ => None,
                })
                .collect();
            format!(
                "wave {} {} {}",
                xs[0],
                xs[xs.len() - 1],
                stroke.color.to_css()
            )
        }
        other => format!("{other:?}"),
    }
}

const BLACK: &str = "rgb(0, 0, 0)";
const WHITE: &str = "rgb(255, 255, 255)";

#[test]
fn selected_text_paints_in_the_system_highlight_colors() {
    let mut page = Page::new("<p id=p>hello world</p>", "");
    assert_eq!(page.painted(), [format!("hello world {BLACK}")]);
    let text = page.text("p");
    page.select((text, 2), (text, 7));
    assert_eq!(
        page.painted(),
        [
            "fill 10 25 rgb(0, 120, 215)".to_owned(),
            "clip -10 20".to_owned(),
            format!("hello world {BLACK}"),
            "pop".to_owned(),
            "clip 10 25".to_owned(),
            format!("hello world {WHITE}"),
            "pop".to_owned(),
            "clip 35 30".to_owned(),
            format!("hello world {BLACK}"),
            "pop".to_owned(),
        ]
    );
    // A whole run needs no clip around the selected part.
    page.select((text, 0), (text, 11));
    assert_eq!(
        page.painted(),
        [
            "fill 0 55 rgb(0, 120, 215)".to_owned(),
            "clip 0 55".to_owned(),
            format!("hello world {WHITE}"),
            "pop".to_owned(),
        ]
    );
}

#[test]
fn selection_styles_color_selected_text() {
    let mut page = Page::new(
        "<p id=p>hello world</p>",
        "::selection { background: rgb(255, 255, 0) }",
    );
    let text = page.text("p");
    page.select((text, 6), (text, 11));
    // An inherited color keeps the text's own.
    assert_eq!(
        page.painted(),
        [
            "fill 30 25 rgb(255, 255, 0)".to_owned(),
            format!("hello world {BLACK}")
        ]
    );
    page.css
        .push_str(" p::selection { color: transparent; background: transparent }");
    assert_eq!(
        page.painted(),
        [
            "clip -10 40".to_owned(),
            format!("hello world {BLACK}"),
            "pop".to_owned(),
        ]
    );
}

#[test]
fn selections_across_nodes_use_the_nearest_selection_style() {
    let mut page = Page::new(
        "<p id=p>ab<b id=b>cd</b>ef</p><p id=q style='width: 15px'>gh ij</p>",
        "p::selection { color: rgb(255, 0, 0); background: rgb(0, 0, 255) }",
    );
    let first = page.text("p");
    let last = page.dom.last_child(page.id("p")).unwrap();
    page.select((first, 1), (last, 1));
    let painted = page.painted();
    let fills = |painted: &[String]| -> Vec<String> {
        painted
            .iter()
            .filter(|item| item.starts_with("fill"))
            .cloned()
            .collect()
    };
    assert_eq!(
        fills(&painted),
        [
            "fill 5 5 rgb(0, 0, 255)",
            "fill 10 10 rgb(0, 0, 255)",
            "fill 20 5 rgb(0, 0, 255)"
        ]
    );
    assert!(painted.contains(&"cd rgb(255, 0, 0)".to_owned()));
    // Only the first line of a wrapped text node is selected.
    let wrapped = page.text("q");
    page.select((wrapped, 0), (wrapped, 2));
    let painted = page.painted();
    assert_eq!(fills(&painted), ["fill 0 10 rgb(0, 0, 255)"]);
    assert!(painted.contains(&format!("ij {BLACK}")));
}

#[test]
fn carets_show_in_the_focused_editing_host() {
    let mut page = Page::new("<div id=e contenteditable>hello</div><p id=p>plain</p>", "");
    let text = page.text("e");
    page.select((text, 2), (text, 2));
    // Not before the host has the focus.
    assert_eq!(page.caret(), None);
    page.focus("e");
    assert_eq!(
        page.painted(),
        [
            format!("hello {BLACK}"),
            format!("caret 10 0 1 10 {BLACK}"),
            format!("plain {BLACK}")
        ]
    );
    // Its color is the caret color, and a transparent one hides it.
    page.css.push_str(" div { caret-color: rgb(255, 0, 0) }");
    assert_eq!(
        page.caret(),
        Some("caret 10 0 1 10 rgb(255, 0, 0)".to_owned())
    );
    page.css.push_str(" div { caret-color: transparent }");
    assert_eq!(page.caret(), None);
    // Not outside the host.
    page.css.push_str(" div { caret-color: auto }");
    let plain = page.text("p");
    page.select((plain, 1), (plain, 1));
    assert_eq!(page.caret(), None);
    // Nor without a selection.
    page.dom.set_selection(None);
    assert_eq!(page.caret(), None);
}

#[test]
fn carets_between_elements_follow_the_nearest_text() {
    let mut page = Page::new("<div id=e contenteditable><b>ab</b><i>cd</i></div>", "");
    page.focus("e");
    let host = page.id("e");
    page.select((host, 1), (host, 1));
    assert_eq!(page.caret(), Some(format!("caret 10 0 1 10 {BLACK}")));
    page.select((host, 0), (host, 0));
    assert_eq!(page.caret(), Some(format!("caret 0 0 1 10 {BLACK}")));
    page.select((host, 2), (host, 2));
    assert_eq!(page.caret(), Some(format!("caret 20 0 1 10 {BLACK}")));
}

#[test]
fn carets_on_empty_lines_follow_the_text_alignment() {
    let mut page = Page::new(
        "<div id=e contenteditable style='width: 100px'><p id=p></p><p id=s> </p></div>",
        "",
    );
    page.focus("e");
    let paragraph = page.id("p");
    page.select((paragraph, 0), (paragraph, 0));
    assert_eq!(page.caret(), Some(format!("caret 0 0 1 10 {BLACK}")));
    page.css.push_str(" #p { text-align: center }");
    assert_eq!(page.caret(), Some(format!("caret 50 0 1 10 {BLACK}")));
    page.css.push_str(" #p { text-align: right }");
    assert_eq!(page.caret(), Some(format!("caret 99 0 1 10 {BLACK}")));
    // A text node without painted text places it like its element.
    let space = page.text("s");
    page.select((space, 0), (space, 0));
    assert_eq!(page.caret(), Some(format!("caret 0 0 1 10 {BLACK}")));
}

#[test]
fn carets_in_zero_size_text_do_not_show() {
    // As Muya hides the `#` of a heading while the caret is before it.
    let mut page = Page::new(
        "<div id=e contenteditable><span id=h style='font-size:0'># </span>Title</div>",
        "",
    );
    page.focus("e");
    let hidden = page.text("h");
    page.select((hidden, 0), (hidden, 0));
    assert_eq!(page.caret(), None);
}

#[test]
fn misspelled_words_have_wavy_red_underlines_unless_styles_hide_them() {
    let mut page = Page::new(
        "<p id=p>helo wrld</p><div class=quiet><p id=q>helo</p></div>",
        ".quiet ::spelling-error { text-decoration: none }",
    );
    let (text, quiet) = (page.text("p"), page.text("q"));
    page.misspellings = vec![(text, vec![(0, 4), (5, 9)]), (quiet, vec![(0, 4)])];
    assert_eq!(
        page.painted(),
        [
            format!("helo wrld {BLACK}"),
            "wave 0 20 rgb(255, 0, 0)".to_owned(),
            "wave 25 45 rgb(255, 0, 0)".to_owned(),
            format!("helo {BLACK}"),
        ]
    );
}
