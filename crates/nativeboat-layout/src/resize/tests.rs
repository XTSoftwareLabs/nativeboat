use super::*;
use crate::paint::scene;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, TextMeasurer};
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};
use nativeboat_ir::{DisplayItem, Font, NativeChild};

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

fn render(dom: Dom, css: &str) -> Page {
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
        image_size: &|_| Some((40.0, 20.0)),
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

const CSS: &str = "body{margin:0} \
    #notes{appearance:none;width:200px;height:100px;padding:8px 10px;border:1px solid;box-sizing:border-box;resize:vertical} \
    #pane{width:150px;height:60px;overflow:auto;resize:both} \
    #plain{width:150px;height:60px;resize:both}";

fn page(css: &str) -> Page {
    render(
        Dom::from_document(&nativeboat_html::parse_document(
            "<textarea id=notes></textarea><div id=pane><p>text</p></div><div id=plain></div>",
        )),
        css,
    )
}

#[test]
fn text_areas_and_scroll_containers_show_grips_above_their_content() {
    let page = page(CSS);
    let id = |name| page.dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (notes, pane) = (id("notes"), id("pane"));
    // The grip sits in the corner inside the border, above the native
    // control: two dark lines and their light edges follow the control.
    let items = &page.scene.tree.root.items;
    let control = items
        .iter()
        .position(|item| *item == DisplayItem::Child(NativeChild::Control(notes.index())))
        .unwrap();
    let paths = items[control..]
        .iter()
        .filter(|item| matches!(item, DisplayItem::Path { .. }))
        .count();
    assert!(paths >= 2, "{items:?}");
    assert_eq!(resizer_at(&page.scene, &page.dom, 190.0, 92.0), Some(notes));
    assert_eq!(resizer_at(&page.scene, &page.dom, 170.0, 92.0), None);
    assert_eq!(resizer_at(&page.scene, &page.dom, 199.5, 99.5), None);
    // Scroll containers get one over their content's surface; boxes that
    // do not clip their content do not.
    let top = page.layout.boxes[&pane].rect.y;
    assert_eq!(
        resizer_at(&page.scene, &page.dom, 145.0, top + 55.0),
        Some(pane)
    );
    let plain = page.layout.boxes[&id("plain")].rect;
    assert_eq!(
        resizer_at(&page.scene, &page.dom, plain.x + 145.0, plain.y + 55.0),
        None
    );
    assert_eq!(cursor(&page.styles, notes), Cursor::NsResize);
    assert_eq!(cursor(&page.styles, pane), Cursor::NwseResize);
    // `resize: none` takes the grip of a text area away.
    let page = self::page(&format!("{CSS} #notes{{resize:none}}"));
    assert_eq!(resizer_at(&page.scene, &page.dom, 190.0, 92.0), None);
}

#[test]
fn dragging_a_grip_sizes_the_box_in_its_box_sizing_down_to_a_minimum() {
    let mut page = page(CSS);
    let id = |page: &Page, name| page.dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let notes = id(&page, "notes");
    let resizing = Resizing::start(&page.scene, &page.dom, 190.0, 92.0).unwrap();
    assert_eq!(resizing.node(), notes);
    // A vertical grip changes only the height; the border box follows the
    // pointer, and the text area's margins become explicit.
    resizing.drag(
        &page.scene,
        &page.styles,
        &page.layout,
        &mut page.dom,
        250.0,
        152.4,
    );
    let style = page.dom.inline_style(notes);
    assert_eq!(style.get("height"), "160px");
    assert_eq!(style.get("width"), "");
    assert_eq!(style.get("margin-top"), "0px");
    resizing.drag(
        &page.scene,
        &page.styles,
        &page.layout,
        &mut page.dom,
        190.0,
        -50.0,
    );
    assert_eq!(page.dom.inline_style(notes).get("height"), "15px");
    // A content-box scroll container takes its content size.
    let pane = id(&page, "pane");
    let top = page.layout.boxes[&pane].rect.y;
    let resizing = Resizing::start(&page.scene, &page.dom, 145.0, top + 55.0).unwrap();
    resizing.drag(
        &page.scene,
        &page.styles,
        &page.layout,
        &mut page.dom,
        175.0,
        top + 45.0,
    );
    let style = page.dom.inline_style(pane);
    assert_eq!(
        (style.get("width"), style.get("height")),
        ("180px".to_owned(), "50px".to_owned())
    );
    assert_eq!(style.get("margin-left"), "");
    // A drag that does not move the corner changes nothing.
    let page = self::page(CSS);
    let mut dom = page.dom.clone();
    let resizing = Resizing::start(&page.scene, &dom, 190.0, 92.0).unwrap();
    resizing.drag(
        &page.scene,
        &page.styles,
        &page.layout,
        &mut dom,
        190.0,
        92.0,
    );
    assert_eq!(dom.inline_style(notes).css_text(), "");
}
