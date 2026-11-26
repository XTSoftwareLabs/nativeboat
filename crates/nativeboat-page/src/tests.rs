use super::*;
use alloc::{string::ToString, vec};
use nativeboat_layout::FontMetrics;
use nativeboat_native::web::BlankPage;

struct Measurer;
impl TextMeasurer for Measurer {
    fn width(&self, text: &str, _: &nativeboat_ir::Font) -> f32 {
        text.chars().count() as f32 * 8.0
    }
    fn metrics(&self, _: &nativeboat_ir::Font) -> FontMetrics {
        FontMetrics {
            ascent: 12.0,
            descent: 4.0,
        }
    }
}

fn page(html: &str) -> (DocumentPage, Rc<RefCell<Dom>>) {
    let page = DocumentPage::new(
        Rc::new(BlankPage {
            url: "file:///editor.html".to_string(),
            viewport: (800.0, 600.0),
        }),
        Rc::new(Measurer),
    );
    let dom = Rc::new(RefCell::new(Dom::from_document(
        &nativeboat_html::parse_document(html),
    )));
    page.set_document(dom.clone());
    (page, dom)
}

#[test]
fn geometry_styles_and_painting_retain_the_live_document_ids() {
    let (page, dom) = page("<main id='editor'>hello</main>");
    page.extend_stylesheets(vec!["body { margin: 0 } #editor { box-sizing: border-box; width: 200px; height: 100px; padding: 10px; border: 2px solid black; --accent: red; color: red }".to_string()]);
    let editor = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "editor")
        .unwrap();
    let bounds = page.rect(editor).unwrap();
    assert_eq!((bounds.width, bounds.height), (200.0, 100.0));
    assert_eq!(
        page.client(editor),
        Some(Rect {
            x: 2.0,
            y: 2.0,
            width: 196.0,
            height: 96.0
        })
    );
    assert_eq!(page.computed_style_value(editor, "--accent"), "red");
    assert!(
        page.computed_style(editor)
            .iter()
            .any(|(name, _)| name == "color")
    );
    assert_eq!(page.element_from_point(20.0, 20.0), Some(editor));
    assert!(page.scene().unwrap().client_rect(editor).is_some());
    let style = page
        .rendered
        .borrow()
        .as_ref()
        .unwrap()
        .styles
        .get(editor)
        .unwrap()
        .clone();
    let text = dom.borrow().first_child(editor).unwrap();
    dom.borrow_mut().set_data(text, "world");
    assert_eq!(page.rect(editor).unwrap().width, 200.0);
    let cached = page.rendered().unwrap();
    assert!(Rc::ptr_eq(&style, cached.styles.get(editor).unwrap()));
    assert_eq!(cached.layout.runs[0].text, "world");
    let scene_before = page.scene().unwrap();
    dom.borrow_mut()
        .set_attribute(editor, "style", "width: 300px")
        .unwrap();
    assert_eq!(page.rect(editor).unwrap().width, 300.0);
    assert_eq!(scene_before.client_rect(editor).unwrap().width, 200.0);
}

#[test]
fn viewport_and_cssom_changes_invalidate_cached_geometry() {
    let (page, dom) = page(
        "<style id='theme'>body { margin: 0 } #editor { width: 50vw; height: 30px }</style><main id='editor'></main>",
    );
    let editor = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "editor")
        .unwrap();
    let theme = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "theme")
        .unwrap();
    assert_eq!(page.rect(editor).unwrap().width, 400.0);
    page.set_viewport(400.0, 300.0);
    assert_eq!(page.rect(editor).unwrap().width, 200.0);
    assert!(page.match_media("(max-width: 500px)"));
    page.sheet_rules(theme, "#editor { width: 120px }");
    assert_eq!(page.rect(editor).unwrap().width, 120.0);
    let mut environment = page.environment.get();
    environment.media.dark = true;
    page.set_environment(environment);
    assert!(page.match_media("(prefers-color-scheme: dark)"));
}

#[test]
fn the_windows_scrollbars_leave_media_queries_and_viewport_units_the_whole_window() {
    // As in Chromium: the root lays out in what the window's scrollbars
    // leave, and its client width is that, while media queries, `vw` and
    // the viewport itself include the bars.
    let (page, dom) = page("<main id='editor'></main><div id='wide'></div>");
    page.extend_stylesheets(vec![
        "body { margin: 0 } #wide { width: 100vw; height: 1px }          @media (min-width: 800px) { #editor { height: 10px } }"
            .to_string(),
    ]);
    let id = |name: &str| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let root = dom.borrow().document_element().unwrap();
    page.set_scrollbar_room((15.0, 15.0));
    assert_eq!(page.rect(id("editor")).unwrap().width, 785.0);
    assert_eq!(page.rect(id("editor")).unwrap().height, 10.0);
    assert_eq!(page.rect(id("wide")).unwrap().width, 800.0);
    let client = page.client(root).unwrap();
    assert_eq!((client.width, client.height), (785.0, 585.0));
    assert_eq!(page.viewport(), (800.0, 600.0));
    assert!(page.match_media("(min-width: 800px)"));
    page.set_scrollbar_room((0.0, 0.0));
    assert_eq!(page.rect(id("editor")).unwrap().width, 800.0);
}

#[test]
fn transparent_windows_leave_the_canvas_clear() {
    let (page, _) = page("<p>loose</p>");
    let canvas = |page: &DocumentPage| match page.scene().unwrap().tree.root.items.first() {
        Some(DisplayItem::Fill { color, .. }) => *color,
        other => panic!("the canvas fill: {other:?}"),
    };
    assert_eq!(canvas(&page), Color::WHITE);
    page.set_transparent(true);
    page.set_transparent(true);
    assert_eq!(canvas(&page), Color::TRANSPARENT);
    page.set_transparent(false);
    assert_eq!(canvas(&page), Color::WHITE);
}

#[test]
fn text_queries_and_color_parsing_use_the_native_scene() {
    let (page, dom) = page("<main id='editor'>hello</main>");
    page.extend_stylesheets(vec![
        "body { margin: 0 } main { font: 16px monospace; line-height: 16px }".to_string(),
    ]);
    let editor = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "editor")
        .unwrap();
    let text = dom.borrow().first_child(editor).unwrap();
    assert!(!page.text_rects(text, 0, 5).is_empty());
    assert!(page.caret_from_point(8.0, 8.0).is_some());
    assert!(page.scroll_size(editor).is_some());
    assert_eq!(page.parse_color("#ff0000"), Some([255.0, 0.0, 0.0, 1.0]));
    assert!(page.parse_color("invalid-color").is_none());
}

#[test]
fn scrolling_honors_alignment_and_native_scroll_limits() {
    let (page, dom) = page(
        "<div id='container'><div style='height:180px'></div><div id='target' style='height:20px'></div><div style='height:200px'></div></div>",
    );
    page.extend_stylesheets(vec![
        "body{margin:0} #container{width:100px;height:100px;overflow:auto}".to_string(),
    ]);
    let container = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "container")
        .unwrap();
    let target = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "target")
        .unwrap();
    for (alignment, expected) in [
        (ScrollAlign::Start, 180.0),
        (ScrollAlign::Center, 140.0),
        (ScrollAlign::End, 100.0),
        (ScrollAlign::Nearest, 100.0),
    ] {
        dom.borrow_mut().set_scroll(container, 0.0, 0.0);
        page.scroll_into_view(target, alignment, ScrollAlign::Nearest);
        assert_eq!(dom.borrow().scroll(container).1, expected, "{alignment:?}");
    }
    let revision = page.revision();
    page.set_viewport(800.0, 600.0);
    assert_eq!(page.revision(), revision);
}

#[test]
fn app_regions_mark_the_parts_of_a_frameless_window_that_drag_it() {
    let (page, _) = page(
        "<header id='title'><span>MarkText</span><button>x</button></header><main>text</main>",
    );
    page.extend_stylesheets(vec!["body { margin: 0 } header { height: 30px; -webkit-app-region: drag } button { width: 20px; height: 20px; app-region: no-drag }".to_string()]);
    assert!(page.drags(5.0, 5.0));
    assert!(page.drags(100.0, 25.0));
    assert!(!page.drags(1.0, 40.0));
    assert!(!page.drags(500.0, 500.0));

    let empty = DocumentPage::new(
        Rc::new(BlankPage {
            url: "file:///editor.html".to_string(),
            viewport: (800.0, 600.0),
        }),
        Rc::new(Measurer),
    );
    assert!(!empty.drags(5.0, 5.0));
}

#[test]
fn selections_and_the_caret_paint_over_editable_text() {
    let (page, dom) = page("<div id='editor' contenteditable>hello world</div>");
    page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
    let items = |page: &DocumentPage| page.scene().unwrap().tree.root.items.clone();
    let (editor, text) = {
        let dom = dom.borrow();
        let editor = dom.get_element_by_id(Dom::DOCUMENT, "editor").unwrap();
        (editor, dom.first_child(editor).unwrap())
    };
    let fills = |items: &[nativeboat_ir::DisplayItem]| {
        items
            .iter()
            .filter(|item| matches!(item, nativeboat_ir::DisplayItem::Fill { .. }))
            .count()
    };
    let unselected = fills(&items(&page));
    // A selection over "world".
    {
        let mut dom = dom.borrow_mut();
        let range = dom.create_range((text, 6), (text, 11));
        dom.set_selection(Some(range));
    }
    assert_eq!(fills(&items(&page)), unselected + 1);
    // A collapsed selection in the focused editing host is a caret.
    {
        let mut dom = dom.borrow_mut();
        let range = dom.create_range((text, 2), (text, 2));
        dom.set_selection(Some(range));
        dom.set_focus(Some(editor), false);
    }
    assert!(
        items(&page)
            .iter()
            .any(|item| matches!(item, nativeboat_ir::DisplayItem::Caret { .. }))
    );
}

#[test]
fn uncommon_queries_and_scroll_positions_follow_the_document() {
    let (page, dom) = page(
        "<div id='container'><div style='height:180px'></div><div id='target' style='height:20px'></div><div id='tall' style='height:150px'></div><div style='height:200px'></div><p id='hidden'>x</p></div>",
    );
    page.extend_stylesheets(vec![
        "body{margin:0} #container{width:100px;height:100px;overflow:auto;scroll-padding-top:10px} #hidden{display:none} #target{color:blue}".to_string(),
    ]);
    assert!(alloc::format!("{page:?}").contains("DocumentPage"));
    let element = |id| dom.borrow().get_element_by_id(Dom::DOCUMENT, id).unwrap();
    let (container, target, tall, hidden) = (
        element("container"),
        element("target"),
        element("tall"),
        element("hidden"),
    );
    // Content above the scrollport scrolls back by the nearest edge; the
    // scroll padding keeps room at the top.
    dom.borrow_mut().set_scroll(container, 0.0, 250.0);
    page.scroll_into_view(target, ScrollAlign::Nearest, ScrollAlign::Nearest);
    assert_eq!(dom.borrow().scroll(container).1, 170.0);
    // Content taller than the scrollport aligns its end when that is in view.
    dom.borrow_mut().set_scroll(container, 0.0, 280.0);
    page.scroll_into_view(tall, ScrollAlign::Nearest, ScrollAlign::Nearest);
    assert!(dom.borrow().scroll(container).1 < 280.0);
    // Elements without boxes do not scroll.
    page.scroll_into_view(hidden, ScrollAlign::Start, ScrollAlign::Start);
    let root = dom.borrow().document_element().unwrap();
    assert_eq!(page.scroll_size(root), Some((800.0, 600.0)));
    assert_eq!(page.computed_style_value(target, "color"), "rgb(0, 0, 255)");
    assert_eq!(page.device_pixel_ratio(), 1.0);
    assert_eq!(page.parse_color("red blue"), None);
    assert_eq!(page.parse_color("currentcolor"), Some([0.0, 0.0, 0.0, 1.0]));
    // A page without a document scrolls nothing.
    let blank = DocumentPage::new(
        Rc::new(BlankPage {
            url: "file:///blank.html".to_string(),
            viewport: (10.0, 10.0),
        }),
        Rc::new(Measurer),
    );
    blank.scroll_into_view(target, ScrollAlign::Start, ScrollAlign::Start);
    assert_eq!(blank.url(), "file:///blank.html");
}

#[test]
fn style_queries_skip_layout_and_new_style_elements_parse_alone() {
    let (page, dom) = page("<head><style>p { color: red }</style></head><p id='p'>text</p>");
    page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
    let p = dom.borrow().get_element_by_id(Dom::DOCUMENT, "p").unwrap();
    assert!(page.scene().is_some());
    let first_sheets = page.parsed.borrow().as_ref().unwrap().sheets.clone();
    // A style query after a change computes styles, not the layout.
    dom.borrow_mut()
        .set_attribute(p, "style", "color: blue")
        .unwrap();
    assert_eq!(page.computed_style_value(p, "color"), "rgb(0, 0, 255)");
    let laid_out = page.rendered.borrow().as_ref().unwrap().styles.clone();
    let styled = page.styled.borrow().as_ref().unwrap().styles.clone();
    assert!(!Rc::ptr_eq(&laid_out, &styled));
    // A new `<style>` element is parsed; the others are reused.
    let head = dom
        .borrow()
        .document_element()
        .and_then(|html| dom.borrow().first_child(html))
        .unwrap();
    let style = dom.borrow_mut().create_element("style").unwrap();
    let text = dom.borrow_mut().create_text("p { margin: 1px }");
    dom.borrow_mut().append_child(style, text).unwrap();
    dom.borrow_mut().append_child(head, style).unwrap();
    assert!(page.scene().is_some());
    let sheets = page.parsed.borrow().as_ref().unwrap().sheets.clone();
    assert_eq!(sheets.len(), 3);
    assert!(Rc::ptr_eq(&sheets[0], &first_sheets[0]));
    assert!(Rc::ptr_eq(&sheets[1], &first_sheets[1]));
}

#[test]
fn printing_lays_out_print_styles_between_margins_and_breaks_between_lines() {
    let lines: String = (0..10)
        .map(|index| alloc::format!("<p>line {index}</p>"))
        .collect();
    let (page, _) = page(&alloc::format!("<main id=screen>screen only</main>{lines}"));
    page.extend_stylesheets(vec![
        "body { margin: 0 } p { margin: 0; line-height: 30px } \
         @media print { #screen { display: none } } \
         @page { margin: 10px 20px }"
            .to_string(),
    ]);
    let screen = page.environment.get();
    let layout = page.print_layout((240.0, 120.0), [None; 4]).unwrap();
    assert_eq!(layout.margins, [10.0, 20.0, 10.0, 20.0]);
    assert_eq!(layout.content, (200.0, 100.0));
    // The line from 90 to 120 starts the second page instead of being cut.
    assert_eq!(&layout.breaks[..3], [0.0, 90.0, 180.0]);
    let texts: Vec<&str> = layout
        .tree
        .root
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !texts.iter().any(|text| text.contains("screen")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text.contains("line 9")),
        "{texts:?}"
    );
    // The screen layout comes back.
    assert_eq!(page.environment.get(), screen);
    assert!(
        page.scene()
            .unwrap()
            .tree
            .root
            .items
            .iter()
            .any(|item| matches!(
                item,
                DisplayItem::Text { text, .. } if text.contains("screen")
            ))
    );
    // Given margins win, and the default is a centimetre.
    let layout = page
        .print_layout((240.0, 120.0), [Some(0.0), None, Some(0.0), Some(0.0)])
        .unwrap();
    assert_eq!(layout.margins, [0.0, 20.0, 0.0, 0.0]);
    let (bare, _) = self::page("<p>x</p>");
    let layout = bare.print_layout((400.0, 400.0), [None; 4]).unwrap();
    assert_eq!(layout.margins, [96.0 / 2.54; 4]);
    assert_eq!(layout.breaks, [0.0]);
}

#[test]
fn pages_print_into_pdf_documents() {
    use nativeboat_native::app_host::{PrintOutcome, PrintRequest, PrintTarget};
    let (page, _) = self::page("<h1>Printed</h1><p>A page printed into a PDF.</p>");
    let mut request = PrintRequest {
        target: PrintTarget::Pdf,
        paper: Some((210.0, 297.0)),
        landscape: false,
        margins: [None; 4],
        name: "Test document".into(),
        background: true,
    };
    match print_page(&page, 0, &request) {
        Ok(PrintOutcome::Pdf(bytes)) => assert!(bytes.starts_with(b"%PDF")),
        // Windows images without the PDF printer cannot print.
        Err(message) => assert!(message.contains("open the printer"), "{message}"),
        Ok(other) => panic!("{other:?}"),
    }
    request.target = PrintTarget::Printer("No Such Printer 12345".into());
    assert!(
        print_page(&page, 0, &request)
            .unwrap_err()
            .contains("open the printer")
    );
    // A page without a document has nothing to print.
    let blank = DocumentPage::new(
        Rc::new(BlankPage {
            url: "file:///blank.html".to_string(),
            viewport: (10.0, 10.0),
        }),
        Rc::new(Measurer),
    );
    request.target = PrintTarget::Pdf;
    if let Err(message) = print_page(&blank, 0, &request) {
        assert!(
            message.contains("no document") || message.contains("open the printer"),
            "{message}"
        );
    }
}

#[test]
fn printing_without_backgrounds_leaves_out_fills_and_gradients() {
    use nativeboat_ir::{Rect, Surface};
    let fill = DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, 1.0, 1.0),
        color: nativeboat_css::Color::rgb(1, 2, 3),
        radii: [nativeboat_ir::Corner::SQUARE; 4],
    };
    let gradient = DisplayItem::Gradient {
        rect: Rect::new(0.0, 0.0, 1.0, 1.0),
        angle: 0.0,
        stops: vec![],
        radii: [nativeboat_ir::Corner::SQUARE; 4],
    };
    let mut surface = Surface {
        items: vec![fill.clone(), DisplayItem::PopClip],
        children: vec![Surface {
            items: vec![gradient, DisplayItem::PopOpacity],
            ..Surface::default()
        }],
        ..Surface::default()
    };
    without_backgrounds(&mut surface);
    assert_eq!(surface.items, [DisplayItem::PopClip]);
    assert_eq!(surface.children[0].items, [DisplayItem::PopOpacity]);
}

#[test]
fn background_images_load_for_painting() {
    let url = "data:image/bmp;base64,Qk1GAAAAAAAAADYAAAAoAAAAAgAAAAIAAAABABgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD/AP8AAAD/AAD///8AAA==";
    let inline = url.replace(",", ", ");
    let (page, _) = self::page(&alloc::format!(
        "<i style='display:inline-block;width:16px;height:16px;background:url({url}) no-repeat'></i><span style=\"background:url('{inline}')\">x</span>"
    ));
    let _ = nativeboat_win32::init();
    page.scene().unwrap();
    assert_eq!(nativeboat_win32::image_size(url), Some((2.0, 2.0)));
    assert_eq!(nativeboat_win32::image_size(&inline), Some((2.0, 2.0)));
}

#[test]
fn network_background_images_are_wanted_once_and_shown_when_added() {
    let url = "https://example.test/thumb.bmp";
    let (page, _) = self::page(&alloc::format!(
        "<i style='display:inline-block;width:16px;height:16px;background:url({url})'></i>"
    ));
    let _ = nativeboat_win32::init();
    page.scene().unwrap();
    assert_eq!(page.take_wanted_images(), [url]);
    // Asked for once: later renderings wait for the download.
    page.scene().unwrap();
    assert!(page.take_wanted_images().is_empty());
    let bytes = assets::fetch("", "data:image/bmp;base64,Qk1GAAAAAAAAADYAAAAoAAAAAgAAAAIAAAABABgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD/AP8AAAD/AAD///8AAA==").unwrap();
    assert!(page.image_pixels(url).is_none());
    assert!(page.add_image(url, &bytes));
    assert_eq!(page.image_size(url), Some((2.0, 2.0)));
    // A notification shows the downloaded image's pixels, the top row's
    // second one white.
    let (width, height, pixels) = page.image_pixels(url).unwrap();
    assert_eq!((width, height, pixels.len()), (2, 2, 16));
    assert_eq!(pixels[4..8], [255, 255, 255, 255]);
}

#[test]
fn translucent_overlays_show_what_lies_beneath_them() {
    let (page, _) = self::page(
        "<div id=app><div class=editor>Editor text</div><div class=overlay><div class=dialog>Dialog</div></div></div>",
    );
    page.extend_stylesheets(vec!["html,body{margin:0;height:100%} body{position:fixed;width:100%} body.hidden{overflow:hidden} .editor{position:absolute;inset:0} .overlay{position:fixed;inset:0;z-index:2007;background:rgba(0,0,0,.5);overflow:auto} .dialog{width:100px;height:50px;background:white;margin:40px auto}".to_string()]);
    let document = page.document().unwrap();
    let body = document
        .borrow()
        .get_elements_by_tag_name(nativeboat_dom::Dom::DOCUMENT, "body")[0];
    document
        .borrow_mut()
        .set_attribute(body, "class", "hidden")
        .unwrap();
    let scene = page.scene().unwrap();
    // The overlay scrolls on a surface of its own, which shows what lies
    // beneath it instead of an opaque backdrop.
    let overlay = &scene.tree.root.children[0];
    assert_eq!(overlay.items.first(), Some(&DisplayItem::Beneath));
    // An opaque scroller keeps its backdrop.
    assert!(matches!(
        scene.tree.root.items.first(),
        Some(DisplayItem::Fill { .. })
    ));
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut core::ffi::c_void, model: u32) -> i32;
}

#[test]
fn misspelled_words_of_the_focused_editor_are_underlined() {
    use nativeboat_native::spelling;
    // SAFETY: initializes COM for this test thread, as window loops do.
    unsafe { CoInitializeEx(core::ptr::null_mut(), 2) };
    if !spelling::available_languages()
        .iter()
        .any(|language| language == "en-US")
    {
        return;
    }
    spelling::set_languages(&["en-US".to_string()]);
    let (page, dom) = page(
        "<div id='editor' contenteditable>helo wrld <span id='code' spellcheck='false'>wrld</span></div>",
    );
    page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
    let waves = |page: &DocumentPage| {
        page.scene()
            .unwrap()
            .tree
            .root
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    nativeboat_ir::DisplayItem::Path {
                        stroke: Some(_),
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(waves(&page), 0);
    let (editor, text) = {
        let dom = dom.borrow();
        let editor = dom.get_element_by_id(Dom::DOCUMENT, "editor").unwrap();
        (editor, dom.first_child(editor).unwrap())
    };
    {
        let mut dom = dom.borrow_mut();
        dom.set_focus(Some(editor), false);
        let range = dom.create_range((text, 10), (text, 10));
        dom.set_selection(Some(range));
    }
    // "helo" and "wrld", not the text that spellcheck=false excludes.
    assert_eq!(waves(&page), 2);
    // The word being typed at the caret is not marked yet.
    {
        let mut dom = dom.borrow_mut();
        let range = dom.create_range((text, 2), (text, 2));
        dom.set_selection(Some(range));
    }
    assert_eq!(waves(&page), 1);
    assert!(page.spelling_due().is_some());
    // Once typing pauses, the word at the caret is marked too.
    page.spelling_due.set(Some(0.0));
    page.refresh_spelling();
    assert_eq!(waves(&page), 2);
    assert_eq!(page.spelling_due(), None);
    // Context menus find the word a point is in, marked or not yet.
    assert_eq!(page.misspelling(text, 2), Some((0, 4)));
    assert_eq!(page.misspelling(text, 9), Some((5, 9)));
    assert_eq!(page.misspelling(editor, 0), None);
    spelling::set_enabled(false);
    dom.borrow_mut().set_data(text, "helo wrld again");
    assert_eq!(waves(&page), 0);
    spelling::set_enabled(true);
    spelling::set_languages(&[]);
}

#[test]
fn scales_change_the_resolution_and_lay_out_again() {
    let (page, _dom) = page("<p>text</p>");
    page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
    let before = page.scene().unwrap();
    page.set_scale(2.0);
    assert!(page.match_media("(min-resolution: 2dppx)"));
    page.set_scale(2.0);
    assert!(!Rc::ptr_eq(&before, &page.scene().unwrap()));
}

#[test]
fn text_changes_keep_styles_and_lay_out_again() {
    let (page, dom) = page("<span id='word'>a</span>");
    page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
    let word = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "word")
        .unwrap();
    let before = page.rect(word).unwrap().width;
    let styles = page.styled().unwrap().styles.clone();
    let text = dom.borrow().first_child(word).unwrap();
    dom.borrow_mut().set_data(text, "a longer word");
    // No selector sees the text, so the styles stay; the layout does not.
    assert!(Rc::ptr_eq(&page.styled().unwrap().styles, &styles));
    assert!(page.rect(word).unwrap().width > before);
    dom.borrow_mut().set_attribute(word, "class", "x").unwrap();
    assert!(!Rc::ptr_eq(&page.styled().unwrap().styles, &styles));
}

#[test]
fn style_sheet_links_apply_in_document_order_with_urls_from_their_files() {
    // The sheet is a file in the temporary directory, which the page, in a
    // folder below it, links by a relative path.
    let sheet = nativeboat_win32::temporary_output("css");
    assert!(nativeboat_native::platform::write_file(
        &sheet,
        b"\xef\xbb\xbfp { color: rgb(1, 2, 3); background-image: url('img/x.png') } em { color: rgb(9, 9, 9) }",
    ));
    let (folder, name) = sheet.rsplit_once('\\').unwrap();
    let folder = folder.replace('\\', "/");
    let page = DocumentPage::new(
        Rc::new(BlankPage {
            url: alloc::format!("file:///{folder}/app/index.html"),
            viewport: (800.0, 600.0),
        }),
        Rc::new(Measurer),
    );
    let dom = Rc::new(RefCell::new(Dom::from_document(
        &nativeboat_html::parse_document(&alloc::format!(
            "<head><style>em {{ color: rgb(5, 5, 5) }}</style><link rel=stylesheet href='../{name}'>\
             <link rel='alternate stylesheet' href='../{name}'><link rel=stylesheet media=print href='missing.css'>\
             <style>p {{ color: rgb(4, 4, 4) }}</style></head><p id=p>text <em id=em>e</em></p>"
        )),
    )));
    page.set_document(dom.clone());
    let id = |name: &str| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    // A later `<style>` wins over the link before it, the link over the
    // `<style>` before it.
    assert_eq!(page.computed_style_value(id("p"), "color"), "rgb(4, 4, 4)");
    assert_eq!(page.computed_style_value(id("em"), "color"), "rgb(9, 9, 9)");
    // `url()`s are the sheet's.
    let image = page.computed_style_value(id("p"), "background-image");
    assert!(
        image.contains(&alloc::format!("{folder}/img/x.png")),
        "{image}"
    );
}

#[test]
fn style_sheets_import_the_sheets_they_name() {
    // Three files in the temporary directory: the linked sheet imports the
    // second beside it, and a `<style>` element imports the third.
    let first = nativeboat_win32::temporary_output("css");
    let second = nativeboat_win32::temporary_output("css");
    let third = nativeboat_win32::temporary_output("css");
    let name = |path: &str| String::from(path.rsplit_once('\\').unwrap().1);
    let folder = first.rsplit_once('\\').unwrap().0.replace('\\', "/");
    let write = |path: &str, text: String| {
        assert!(nativeboat_native::platform::write_file(
            path,
            text.as_bytes()
        ));
    };
    write(
        &first,
        alloc::format!(
            "@charset 'utf-8'; /* a comment */ @import url(\"{}\") screen; @import '{}' print; p {{ color: rgb(1, 1, 1) }}",
            name(&second),
            name(&third)
        ),
    );
    write(
        &second,
        String::from("p, em { color: rgb(2, 2, 2) } em { background-image: url(x.png) }"),
    );
    write(&third, String::from("em { color: rgb(3, 3, 3) }"));
    let page = DocumentPage::new(
        Rc::new(BlankPage {
            url: alloc::format!("file:///{folder}/index.html"),
            viewport: (800.0, 600.0),
        }),
        Rc::new(Measurer),
    );
    let dom = Rc::new(RefCell::new(Dom::from_document(
        &nativeboat_html::parse_document(&alloc::format!(
            "<head><link rel=stylesheet href='{}'><style>@import '{}'; strong {{ color: rgb(4, 4, 4) }}</style></head>\
             <p id=p>p <em id=em>em</em> <strong id=s>s</strong></p>",
            name(&first),
            name(&third)
        )),
    )));
    page.set_document(dom.clone());
    let id = |name: &str| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    // The importing sheet's own rules follow what it imports; the print
    // import does not apply; the `<style>` element's import does.
    assert_eq!(page.computed_style_value(id("p"), "color"), "rgb(1, 1, 1)");
    assert_eq!(page.computed_style_value(id("em"), "color"), "rgb(3, 3, 3)");
    assert_eq!(page.computed_style_value(id("s"), "color"), "rgb(4, 4, 4)");
    let image = page.computed_style_value(id("em"), "background-image");
    assert!(image.contains(&alloc::format!("{folder}/x.png")), "{image}");
}

#[test]
fn empty_inline_elements_alone_on_a_line_sit_where_the_content_starts() {
    // Chromium reports an empty rectangle at the content box's start, as an
    // editor's empty code line, rather than no rectangle.
    let (page, dom) = page(
        "<div id=block><code><span id=empty></span></code></div><div id=gone style='display:none'><span id=hidden></span></div>",
    );
    page.extend_stylesheets(vec![
        "body { margin: 0 } #block { margin-top: 30px; padding: 5px 7px; border: 2px solid }"
            .to_string(),
    ]);
    let id = |name| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    assert_eq!(
        page.rect(id("empty")),
        Some(Rect {
            x: 9.0,
            y: 37.0,
            width: 0.0,
            height: 0.0
        })
    );
    assert_eq!(page.rect(id("hidden")), None);
    assert_eq!(page.rect(id("gone")), None);
}

#[test]
fn container_query_units_follow_their_containers_laid_out_size() {
    let (page, dom) = page(
        "<style>body { margin: 0 } #column { container-type: inline-size; width: 50%; padding: 0 10px }\
         #half { width: 50cqw; height: 10px } #free { width: 10cqw; height: 10px }</style>\
         <div id='column'><div id='half'></div></div><div id='free'></div>",
    );
    let id = |name| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    // Half the column's 400px content box; outside any container the
    // units take the viewport's size.
    assert_eq!(page.rect(id("half")).unwrap().width, 200.0);
    assert_eq!(page.rect(id("free")).unwrap().width, 80.0);
    page.set_viewport(400.0, 300.0);
    assert_eq!(page.rect(id("half")).unwrap().width, 100.0);
    assert_eq!(page.rect(id("free")).unwrap().width, 40.0);
}

#[test]
fn container_rules_apply_by_their_query_containers_size() {
    let (page, dom) = page(
        "<style>body { margin: 0 } .card { container: card / inline-size } #wide { width: 300px } #narrow { width: 100px }\
         .item { width: 10px; height: 10px }\
         @container card (max-width: 200px) { .item { width: 20px } }\
         @container (min-width: 250px) { .item { height: 30px } }\
         @container other (min-width: 1px) { .item { width: 40px } }\
         @container (height > 0px) { .item { height: 50px } }</style>\
         <div class='card' id='wide'><div class='item' id='a'></div></div>\
         <div class='card' id='narrow'><div class='item' id='b'></div></div>\
         <div class='item' id='c'></div>",
    );
    let id = |name| dom.borrow().get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let size = |name| {
        let rect = page.rect(id(name)).unwrap();
        (rect.width, rect.height)
    };
    // Named queries choose containers with that name; an element outside
    // any container, and a height query of an inline-size container, match
    // nothing.
    assert_eq!(size("a"), (10.0, 30.0));
    assert_eq!(size("b"), (20.0, 10.0));
    assert_eq!(size("c"), (10.0, 10.0));
}

#[test]
fn transitions_present_frame_values_and_report_their_events() {
    let (page, dom) = page("<div id='box'></div>");
    page.extend_stylesheets(vec![
        "body { margin: 0 } #box { width: 100px; height: 10px; transition: width 1s linear } #box.wide { width: 300px } #box::before { content: ''; opacity: 0; transition: opacity 1s linear } #box.wide::before { opacity: 1 }"
            .to_string(),
    ]);
    let node = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "box")
        .unwrap();
    assert_eq!(page.rect(node).unwrap().width, 100.0);
    assert!(!page.animating());
    dom.borrow_mut()
        .set_attribute(node, "class", "wide")
        .unwrap();
    // The change starts transitions, which show from the next frame.
    let started = nativeboat_native::platform::now();
    assert_eq!(page.rect(node).unwrap().width, 100.0);
    assert!(page.animating());
    page.advance_frame(started + 500.0);
    let width = page.rect(node).unwrap().width;
    assert!((190.0..=220.0).contains(&width), "{width}");
    let computed: f32 = page
        .computed_style_value(node, "width")
        .trim_end_matches("px")
        .parse()
        .unwrap();
    assert!((computed - width).abs() < 1.0, "{computed} {width}");
    page.advance_frame(started + 2000.0);
    assert_eq!(page.rect(node).unwrap().width, 300.0);
    let events: Vec<_> = page
        .take_transition_events()
        .into_iter()
        .map(|event| (event.kind, event.pseudo, event.property))
        .collect();
    for event in [
        ("transitionrun", "", "width"),
        ("transitionrun", "::before", "opacity"),
        ("transitionstart", "", "width"),
        ("transitionend", "", "width"),
        ("transitionend", "::before", "opacity"),
    ] {
        assert!(events.contains(&event), "{event:?} in {events:?}");
    }
    assert!(!page.animating());
}

#[test]
fn keyframe_animations_run_from_the_pages_style_sheets() {
    let (page, dom) = page("<div id='spinner'></div>");
    page.extend_stylesheets(vec![
        "@keyframes grow { from { width: 100px } to { width: 300px } } body { margin: 0 } #spinner { height: 10px; animation: grow 1s linear infinite }"
            .to_string(),
    ]);
    let node = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "spinner")
        .unwrap();
    let started = nativeboat_native::platform::now();
    page.rect(node);
    assert!(page.animating());
    page.advance_frame(started + 2500.0);
    let width = page.rect(node).unwrap().width;
    assert!((190.0..=215.0).contains(&width), "{width}");
    let events: Vec<_> = page
        .take_transition_events()
        .into_iter()
        .map(|event| (event.kind, event.name))
        .collect();
    assert!(events.contains(&("animationstart", "grow".to_string())));
    assert!(
        events.contains(&("animationiteration", "grow".to_string())),
        "{events:?}"
    );
    assert!(page.animating());
}

#[test]
fn downloaded_images_are_laid_out_once_they_arrive() {
    // A 2x3 PNG.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 3, 8, 2,
        0, 0, 0, 54, 136, 73, 214, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 68,
        12, 40, 20, 0, 68, 208, 5, 251, 164, 207, 222, 128, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
        96, 130,
    ];
    let url = "https://images.example/downloaded.png";
    let (page, dom) = page(&alloc::format!("<img id='picture' src='{url}'>"));
    let picture = dom
        .borrow()
        .get_element_by_id(Dom::DOCUMENT, "picture")
        .unwrap();
    // Nothing is known of a network image until it is downloaded.
    assert_eq!(page.image_size(url), None);
    let before = page.rect(picture).unwrap();
    assert_ne!((before.width, before.height), (2.0, 3.0));
    assert!(!page.add_image(url, b"not an image"));
    assert!(page.add_image(url, PNG));
    assert_eq!(page.image_size(url), Some((2.0, 3.0)));
    let after = page.rect(picture).unwrap();
    assert_eq!((after.width, after.height), (2.0, 3.0));
}
