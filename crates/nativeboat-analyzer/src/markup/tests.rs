use super::*;

#[test]
fn scrollbar_styles_report_rendered_parts_and_unimplemented_parts_separately() {
    let found = detect_css(
        ".editor::-webkit-scrollbar:vertical{width:12px} ::-webkit-scrollbar-thumb:hover{background:red} ::-webkit-scrollbar-track{background:black} ::-webkit-scrollbar-corner{background:blue} ::-webkit-scrollbar-button{color:red} ::-webkit-scrollbar-track-piece{color:red} div{scrollbar-width:thin;scrollbar-color:red blue}",
    );
    assert_eq!(
        found
            .iter()
            .filter(|f| f.feature.id == "css:scrollbars")
            .count(),
        6
    );
    assert!(
        found
            .iter()
            .filter(|f| f.feature.id == "css:scrollbars")
            .all(|f| f.feature.support == crate::Support::Supported)
    );
    // Background images in scrollbar parts are not drawn.
    let images = detect_css(
        "::-webkit-scrollbar-thumb{background:url(thumb.png)} ::-webkit-scrollbar-track{background-image:linear-gradient(red,blue)}",
    );
    let scrollbars: Vec<_> = images
        .iter()
        .filter(|f| f.feature.id == "css:scrollbars")
        .collect();
    assert_eq!(scrollbars.len(), 2);
    assert!(
        scrollbars
            .iter()
            .all(|f| f.feature.support == crate::Support::Partial)
    );
    assert!(
        found
            .iter()
            .any(|f| f.feature.id == "css:selector::-webkit-scrollbar-button")
    );
    assert!(
        found
            .iter()
            .any(|f| f.feature.id == "css:selector::-webkit-scrollbar-track-piece")
    );
    assert!(
        !found
            .iter()
            .any(|f| f.feature.id == "css:selector:vertical")
    );
}

#[test]
fn inline_styles_are_detected_and_stylesheet_positions_follow_html() {
    let found = detect_html(
        "<style>.x{background:linear-gradient(red,blue)}</style>\n<style>\n.x{background-image:radial-gradient(red,blue)}\n@media (width:10px){.x{display:grid}}\n</style>\n<div style=\"filter:blur(1px);color:red\"></div>",
    );
    let css: Vec<_> = found
        .iter()
        .filter(|found| found.feature.id.starts_with("css:"))
        .collect();
    assert_eq!(
        css.iter()
            .map(|found| (found.feature.id.as_str(), found.line))
            .collect::<Vec<_>>(),
        [
            ("css:gradient", 1),
            ("css:gradient", 3),
            ("css:media", 0),
            ("css:grid", 4),
            ("css:filter", 6)
        ]
    );
    assert_eq!(css[0].column, 11);
    assert_eq!(css[1].column, 4);
    assert_eq!(css[4].column, 1);
    assert_eq!(css[0].feature.support, crate::catalog::Support::Supported);
    assert_eq!(css[1].feature.support, crate::catalog::Support::Partial);
    assert_eq!(css[4].feature.support, crate::catalog::Support::Supported);
    assert!(
        detect_html("<style></style><span style=\"color:red\"></span>")
            .iter()
            .all(|found| !found.feature.id.starts_with("css:"))
    );
}

#[test]
fn draggable_attributes_and_inline_handlers_are_reported() {
    let found =
        detect_html("<div draggable=true></div>\n<div ondrop='receive(event)'></div>\n<div></div>");
    let drops: Vec<_> = found
        .iter()
        .filter(|item| item.feature.id == "web:drag-drop")
        .map(|item| item.line)
        .collect();
    assert_eq!(drops, [1, 2]);
}

fn ids(found: &[Located]) -> Vec<(String, u32)> {
    found
        .iter()
        .map(|item| (item.feature.id.clone(), item.line))
        .collect()
}

#[test]
fn html_pages() {
    let found = detect_html(
        "<!doctype html>\n<html><head>\n<script src=\"https://cdn.example/x.js\"></script>\n\
         <script src=\"./main.js\"></script>\n<script>inline()</script>\n\
         <link rel=\"stylesheet\" href=\"//cdn.example/x.css\">\n<link rel=\"stylesheet\" href=\"app.css\">\n\
         <link rel=\"icon\" href=\"https://x/icon.png\">\n</head><body>\n<div><iframe></iframe><button></button></div>\n</body></html>",
    );
    assert_eq!(
        ids(&found),
        vec![
            ("html:basics".to_owned(), 3),
            ("language:remote-script".to_owned(), 3),
            ("html:basics".to_owned(), 4),
            ("html:basics".to_owned(), 5),
            ("html:basics".to_owned(), 6),
            ("language:remote-script".to_owned(), 6),
            ("html:basics".to_owned(), 7),
            ("html:basics".to_owned(), 8),
            ("html:basics".to_owned(), 10),
            ("html:iframe".to_owned(), 10),
            ("html:controls".to_owned(), 10),
        ]
    );
}

#[test]
fn unimplemented_properties_count_unless_they_change_nothing() {
    let found = detect_css(
        ".a { -webkit-font-smoothing: antialiased; word-spacing: normal; hyphens: manual; \
         direction: ltr; resize: none; touch-action: none; appearance: auto; }\n\
         .b { writing-mode: vertical-rl; direction: rtl; unicode-bidi: isolate; }\n\
         .c { hyphens: auto; }\n\
         .d { word-spacing: 4px; }\n\
         .e { text-shadow: none; overflow-anchor: none !important; appearance: none; \
         -webkit-appearance: none; mix-blend-mode: normal; font-variant-ligatures: none; }\n\
         .f { mix-blend-mode: multiply; font-variant-ligatures: normal; }\n\
         .g { -moz-appearance: none; -ms-overflow-style: none; -mox-box-sizing: border-box; -webkit-touch-callout: none; }\n",
    );
    assert_eq!(
        ids(&found),
        vec![
            ("css:property:writing-mode".to_owned(), 2),
            ("css:property:hyphens".to_owned(), 3),
            ("css:property:word-spacing".to_owned(), 4),
            ("css:blend".to_owned(), 6),
            ("css:property:font-variant-ligatures".to_owned(), 6),
        ]
    );
}

#[test]
fn stylesheets() {
    let found = detect_css(
        ":root { --brand: red; }\n\
         .row { display: flex; -webkit-transform: none; zoom: 2; appearance: textfield; color: var(--brand); }\n\
         a:state(fr) { filter: blur(1px); }\n\
         @media (max-width: 10px) { .x { display: grid } }\n\
         @supports (display: grid) { .y { float: left } }\n\
         @layer base { .z { margin: 0 } }\n\
         @import url(\"x.css\");\n\
         @font-face { font-family: X }\n\
         @keyframes spin { to { opacity: 1 } }\n\
         @container (min-width: 1px) { .q {} }\n\
         @page { margin: 1in }\n",
    );
    assert_eq!(
        ids(&found),
        vec![
            ("css:variables".to_owned(), 1),
            ("css:flexbox".to_owned(), 2),
            ("css:transform".to_owned(), 2),
            ("css:property:zoom".to_owned(), 2),
            ("css:appearance".to_owned(), 2),
            ("css:selector:state".to_owned(), 3),
            ("css:filter".to_owned(), 3),
            ("css:media".to_owned(), 0),
            ("css:grid".to_owned(), 4),
            ("css:supports".to_owned(), 0),
            ("css:float".to_owned(), 5),
            ("css:layer".to_owned(), 0),
            ("css:import".to_owned(), 7),
            ("css:font-face".to_owned(), 0),
            ("css:animation".to_owned(), 0),
            ("css:container".to_owned(), 0),
        ]
    );
    assert!(known_property("margin-inline-start"));
    assert!(known_property("all"));
    assert!(known_property("--x"));
    assert!(!known_property("zoom"));
}

#[test]
fn keyframes_are_judged_by_the_properties_they_animate() {
    let support = |css: &str| {
        detect_css(css)
            .into_iter()
            .find(|found| found.feature.id == "css:animation")
            .map(|found| (found.feature.support, found.feature.note))
            .unwrap()
    };
    assert_eq!(
        support("@keyframes spin { from { transform: rotate(0); opacity: 0; animation-timing-function: ease } to { transform: rotate(360deg); opacity: 1 } }").0,
        crate::Support::Supported
    );
    let (level, note) = support(
        "@keyframes reveal { from { clip-path: inset(0 100% 0 0); text-transform: none } to { clip-path: inset(0); text-transform: uppercase } }",
    );
    assert_eq!(level, crate::Support::Partial);
    assert!(
        note.starts_with("text-transform keep their value"),
        "{note}"
    );
}
