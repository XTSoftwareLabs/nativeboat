//! Feature detection in HTML pages and stylesheets.

use crate::catalog::{
    Feature, css_at_rule, css_property, css_selector, css_unknown_property, css_variables, element,
    html_basics, keyframes_with_static, language, scrollbar_images, web,
};
use nativeboat_css::{
    ComputedStyle, PropertyId, Rule, parse_declaration_list, parse_stylesheet, shorthand,
};
use nativeboat_html::{Node, Position, parse_document};

/// A feature found at a line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// The feature.
    pub feature: Feature,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
}

fn located(feature: Feature, position: Position) -> Located {
    Located {
        feature,
        line: position.line,
        column: position.column,
    }
}

/// Whether a URL names something outside the application.
pub(crate) fn is_remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("//")
}

/// Detects features in an HTML page.
pub fn detect_html(source: &str) -> Vec<Located> {
    let document = parse_document(source);
    let mut found = Vec::new();
    for element_node in document.elements() {
        let position = element_node.position;
        if element_node.attributes.iter().any(|attribute| {
            matches!(
                attribute.name.as_str(),
                "draggable"
                    | "ondragstart"
                    | "ondrag"
                    | "ondragenter"
                    | "ondragleave"
                    | "ondragover"
                    | "ondrop"
                    | "ondragend"
            )
        }) {
            found.push(located(
                web("drag-drop").expect("drag-drop is in the catalog"),
                position,
            ));
        }
        if let Some(feature) = element(&element_node.name) {
            found.push(located(feature, position));
        } else if !matches!(element_node.name.as_str(), "html" | "head" | "body") {
            found.push(located(html_basics(), position));
        }
        let remote = match element_node.name.as_str() {
            "script" => element_node.attribute("src").is_some_and(is_remote),
            "link" => {
                element_node.attribute("rel") == Some("stylesheet")
                    && element_node.attribute("href").is_some_and(is_remote)
            }
            _ => false,
        };
        if remote && let Some(feature) = language("remote-script") {
            found.push(located(feature, position));
        }
        if element_node.name == "style" {
            let start = match element_node.children.first() {
                Some(Node::Text(text)) => text.position,
                _ => position,
            };
            for mut feature in detect_css(&element_node.text_content()) {
                if feature.line != 0 {
                    if feature.line == 1 {
                        feature.column += start.column - 1;
                    }
                    feature.line += start.line - 1;
                }
                found.push(feature);
            }
        }
        if let Some(style) = element_node.attribute("style") {
            for declaration in parse_declaration_list(style) {
                if let Some(feature) = declaration_features(&declaration.name, &declaration.value) {
                    // Attributes do not retain individual source positions;
                    // anchor these findings at the containing start tag.
                    found.push(located(feature, position));
                }
            }
        }
    }
    found
}

fn known_property(name: &str) -> bool {
    let name = shorthand::unprefixed(name);
    name.starts_with("--")
        || PropertyId::from_name(name).is_some()
        || shorthand::alias(name).is_some()
        || shorthand::longhands_of(name).is_some()
        || name == "all"
}

/// Properties with no visible effect in a Windows desktop application:
/// macOS font smoothing, touch and mobile settings, aural and paged media,
/// rendering hints, and `@font-face` descriptors, which the web fonts
/// feature reports.
const NO_EFFECT: &[&str] = &[
    "font-smoothing",
    "osx-font-smoothing",
    "text-rendering",
    "font-display",
    "src",
    "unicode-range",
    "touch-action",
    "touch-callout",
    "tap-highlight-color",
    "text-size-adjust",
    "will-change",
    "speak",
    "orphans",
    "widows",
    "page-break-inside",
    "page-break-before",
    "page-break-after",
    "break-inside",
    "break-before",
    "break-after",
];

/// Values that leave a property the style engine does not implement at its
/// initial behavior, which the native target has. `overflow-anchor: none`
/// asks for what the native target does: it never anchors scrolling.
const INITIAL_VALUES: &[(&str, &[&str])] = &[
    // `none` takes the native look away from form controls, which the
    // cascade implements.
    ("appearance", &["auto", "none"]),
    ("background-attachment", &["scroll"]),
    ("background-clip", &["border-box"]),
    ("background-origin", &["padding-box"]),
    ("color-scheme", &["normal"]),
    ("contain", &["none"]),
    ("empty-cells", &["show"]),
    ("field-sizing", &["fixed"]),
    ("font-feature-settings", &["normal"]),
    ("font-stretch", &["normal", "100%"]),
    ("font-variant", &["normal"]),
    // Text forms ligatures only where a script requires them.
    ("font-variant-ligatures", &["none"]),
    ("hyphens", &["manual"]),
    ("interpolate-size", &["numeric-only"]),
    ("list-style-image", &["none"]),
    ("mix-blend-mode", &["normal"]),
    ("object-position", &["50% 50%", "center", "center center"]),
    ("overflow-anchor", &["none"]),
    ("overscroll-behavior", &["auto"]),
    ("quotes", &["auto"]),
    ("scroll-behavior", &["auto"]),
    ("scrollbar-gutter", &["auto"]),
    ("text-decoration-style", &["solid"]),
    ("text-decoration-thickness", &["auto"]),
    ("text-shadow", &["none"]),
    ("text-underline-offset", &["auto"]),
    ("transition-behavior", &["normal"]),
    ("user-drag", &["auto"]),
    ("word-spacing", &["normal", "0", "0px"]),
    ("writing-mode", &["horizontal-tb"]),
];

/// Whether a declaration of a property the style engine does not implement
/// changes nothing in the native target.
fn without_effect(name: &str, value: &str) -> bool {
    // Chromium knows no vendor prefix but `-webkit-`: it ignores such
    // properties, as the native target does.
    if name.starts_with('-') && !name.starts_with("-webkit-") && !name.starts_with("--") {
        return true;
    }
    let name = ["-webkit-", "-moz-", "-ms-"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name);
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    NO_EFFECT.contains(&name)
        || INITIAL_VALUES.iter().any(|(property, values)| {
            *property == name && (values.contains(&value.as_str()) || value == "initial")
        })
}

fn declaration_features(name: &str, value: &str) -> Option<Feature> {
    if name.starts_with("--") {
        return Some(css_variables());
    }
    if let Some(feature) = css_property(shorthand::unprefixed(name), value) {
        return Some(feature);
    }
    if known_property(name) || without_effect(name, value) {
        return None;
    }
    Some(css_unknown_property(name))
}

/// Detects features in a stylesheet.
pub fn detect_css(source: &str) -> Vec<Located> {
    let (sheet, _) = parse_stylesheet(source);
    let mut found = Vec::new();
    let at_rule = |found: &mut Vec<Located>, name: &str, position: Position| {
        if let Some(feature) = css_at_rule(name) {
            found.push(located(feature, position));
        }
    };
    let unknown = Position::default();
    let mut stack: Vec<&Rule> = sheet.rules.iter().rev().collect();
    while let Some(rule) = stack.pop() {
        match rule {
            Rule::Style(style) => {
                let position = Position {
                    line: style.position.line,
                    column: style.position.column,
                };
                for pseudo in style.selectors.unsupported() {
                    found.push(located(css_selector(&pseudo), position));
                }
                for selector in &style.selectors.0 {
                    use nativeboat_css::PseudoElement;
                    match &selector.pseudo_element {
                        Some(
                            PseudoElement::Scrollbar
                            | PseudoElement::ScrollbarTrack
                            | PseudoElement::ScrollbarThumb
                            | PseudoElement::ScrollbarCorner,
                        ) => {
                            let images = style.declarations.iter().any(|declaration| {
                                let value = declaration.value.to_ascii_lowercase();
                                declaration.name.starts_with("background")
                                    && (value.contains("url(") || value.contains("gradient("))
                            });
                            found.push(located(
                                if images {
                                    scrollbar_images()
                                } else {
                                    css_property("scrollbar-width", "auto").unwrap()
                                },
                                position,
                            ));
                        }
                        Some(PseudoElement::Other(name))
                            if name.starts_with("-webkit-scrollbar") =>
                        {
                            found.push(located(css_selector(&format!("::{name}")), position));
                        }
                        _ => {}
                    }
                }
                for declaration in &style.declarations {
                    if let Some(feature) =
                        declaration_features(&declaration.name, &declaration.value)
                    {
                        let position = Position {
                            line: declaration.position.line,
                            column: declaration.position.column,
                        };
                        found.push(located(feature, position));
                    }
                }
            }
            Rule::Media { rules, .. } => {
                at_rule(&mut found, "media", unknown);
                stack.extend(rules.iter().rev());
            }
            Rule::Supports { rules, .. } => {
                at_rule(&mut found, "supports", unknown);
                stack.extend(rules.iter().rev());
            }
            Rule::Layer { rules, .. } => {
                at_rule(&mut found, "layer", unknown);
                stack.extend(rules.iter().rev());
            }
            Rule::Container { rules, .. } => {
                at_rule(&mut found, "container", unknown);
                stack.extend(rules.iter().rev());
            }
            Rule::LayerOrder(_) => at_rule(&mut found, "layer", unknown),
            Rule::Property { .. } => at_rule(&mut found, "property", unknown),
            Rule::Import { position, .. } => at_rule(
                &mut found,
                "import",
                Position {
                    line: position.line,
                    column: position.column,
                },
            ),
            Rule::FontFace(_) => at_rule(&mut found, "font-face", unknown),
            Rule::Page { .. } => at_rule(&mut found, "page", unknown),
            Rule::Keyframes { keyframes, .. } => {
                // Properties animations do not interpolate keep their value.
                let mut fixed: Vec<&str> = Vec::new();
                for declaration in keyframes.iter().flat_map(|frame| &frame.declarations) {
                    let name = shorthand::unprefixed(&declaration.name);
                    if name.starts_with("animation") || name.starts_with("--") {
                        continue;
                    }
                    let ids = PropertyId::from_name(name)
                        .map(|id| vec![id])
                        .or_else(|| shorthand::longhands_of(name))
                        .unwrap_or_default();
                    if (ids.is_empty()
                        || ids.iter().any(|id| !ComputedStyle::ANIMATABLE.contains(id)))
                        && !fixed.contains(&name)
                    {
                        fixed.push(name);
                    }
                }
                if fixed.is_empty() {
                    at_rule(&mut found, "keyframes", unknown);
                } else {
                    found.push(located(keyframes_with_static(&fixed), unknown));
                }
            }
            Rule::Unknown { name, position, .. } => at_rule(
                &mut found,
                name,
                Position {
                    line: position.line,
                    column: position.column,
                },
            ),
        }
    }
    found
}

#[cfg(test)]
mod tests;
