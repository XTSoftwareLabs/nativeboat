//! End-to-end document preparation for native renderer hosts.

use crate::prelude::*;

use crate::paint::{Scene, scene};
use crate::styles::Styles;
use crate::text::TextMeasurer;
use crate::tree::{LayoutOptions, LayoutResult, layout};
use nativeboat_css::{
    Origin, SheetRef, StyleEnvironment, Stylesheet, parse_stylesheet, user_agent_stylesheet,
};
use nativeboat_dom::Dom;
use nativeboat_html::parse_document;
use nativeboat_ir::NativeTree;

/// A document after native parsing, style resolution, layout and scene build.
#[derive(Debug)]
pub struct NativePage {
    /// The parsed HTML document used by style and layout operations.
    pub dom: Dom,
    /// The computed styles for its elements.
    pub styles: Styles,
    /// The laid out geometry and text runs.
    pub layout: LayoutResult,
    /// The platform-independent paint and control tree.
    pub scene: Scene,
}

impl NativePage {
    /// The display-list and controls tree consumed by platform backends.
    pub fn native_tree(&self) -> &NativeTree {
        &self.scene.tree
    }
}

/// Parses a document and its author sheets, then computes styles, layout and
/// a paint scene for the supplied viewport.
pub fn layout_document(
    html: &str,
    stylesheets: &[String],
    viewport: (f32, f32),
    measurer: &dyn TextMeasurer,
    image_size: &dyn Fn(&str) -> Option<(f32, f32)>,
) -> NativePage {
    let parsed_document = parse_document(html);
    let dom = Dom::from_document(&parsed_document);

    let parsed: Vec<Stylesheet> = stylesheets
        .iter()
        .map(|source| parse_stylesheet(source).0)
        .collect();
    let mut sheets = Vec::with_capacity(parsed.len() + 1);
    sheets.push(SheetRef {
        sheet: user_agent_stylesheet(),
        origin: Origin::UserAgent,
    });
    sheets.extend(parsed.iter().map(|sheet| SheetRef {
        sheet,
        origin: Origin::Author,
    }));

    let mut environment = StyleEnvironment::default();
    environment.media.width = f64::from(viewport.0);
    environment.media.height = f64::from(viewport.1);
    environment.standard_font = measurer.standard_family();
    let styles = Styles::compute(&dom, &sheets, &environment);
    let options = LayoutOptions {
        viewport,
        measurer,
        image_size,
    };
    let layout = layout(&dom, &styles, &options);
    let scene = scene(&dom, &styles, &layout, viewport);
    NativePage {
        dom,
        styles,
        layout,
        scene,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nativeboat_css::Color;
    use nativeboat_dom::Dom;
    use nativeboat_ir::DisplayItem;

    struct Measurer;

    impl TextMeasurer for Measurer {
        fn width(&self, text: &str, _: &nativeboat_ir::Font) -> f32 {
            text.chars().count() as f32 * 8.0
        }

        fn metrics(&self, _: &nativeboat_ir::Font) -> crate::FontMetrics {
            crate::FontMetrics {
                ascent: 10.0,
                descent: 3.0,
            }
        }
    }

    #[test]
    fn html_sheets_styles_layout_and_scene_are_connected() {
        let stylesheets = vec![
            "body { margin: 0 } #editor { width: 120px; height: 60px; background-color: #ff0000 }"
                .to_owned(),
        ];
        let page = layout_document(
            "<main id='editor'>ready</main>",
            &stylesheets,
            (300.0, 200.0),
            &Measurer,
            &|_| None,
        );
        let editor = page.dom.get_element_by_id(Dom::DOCUMENT, "editor").unwrap();
        let geometry = page.layout.boxes.get(&editor).unwrap();
        assert_eq!((geometry.rect.width, geometry.rect.height), (120.0, 60.0));
        assert_eq!(
            page.styles.get(editor).unwrap().background_color,
            Color::rgb(255, 0, 0)
        );
        assert!(page.scene.tree.root.items.iter().any(|item| matches!(
            item,
            DisplayItem::Fill { color, .. } if *color == Color::rgb(255, 0, 0)
        )));
    }
}
