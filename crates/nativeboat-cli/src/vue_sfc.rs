//! Vue single-file component script extraction for native graph discovery.
//!
//! Script blocks are preserved at their original line numbers so the ordinary
//! TypeScript compiler can discover their imports and source blockers. The
//! unsupported component template syntax remains an explicit build error.

use nativeboat_html::{Element, Node, parse_document, parse_vue_fragment_in};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Parsed {
    pub module_source: String,
    pub template_line: Option<u32>,
    pub line_map: Vec<u32>,
}

#[cfg(test)]
pub(super) fn parse(source: &str) -> Parsed {
    parse_with_component_name(source, None)
}

pub(super) fn parse_with_component_name(source: &str, component_name: Option<&str>) -> Parsed {
    let document = parse_document(source);
    let elements = document.elements();
    let template = elements
        .iter()
        .filter(|element| element.name == "template")
        .min_by_key(|element| element.position.line);
    let mut scripts = script_blocks(source);
    scripts.sort_by_key(|(line, _, _)| *line);

    let has_setup = scripts.iter().any(|(_, _, setup)| *setup);
    let setup_scripts = scripts
        .iter()
        .filter(|(_, _, setup)| *setup)
        .cloned()
        .collect::<Vec<_>>();
    let components = if has_setup {
        setup_components(&setup_scripts, component_name)
    } else {
        BTreeMap::new()
    };
    let mut refs = if has_setup {
        setup_refs(&setup_scripts)
    } else {
        BTreeSet::new()
    };
    if has_setup {
        refs.extend(
            setup_template_props(&setup_scripts)
                .into_iter()
                .map(|name| template_prop_key(&name)),
        );
    }
    let mut render = template
        .and_then(|template| template_content(source, template))
        .and_then(|content| static_render(content, has_setup, &components, &refs));
    if has_setup {
        return script_setup_module(scripts, render, template);
    }

    let mut module_source = String::new();
    let mut current_line = 1u32;
    let mut has_regular_script = false;
    for (line, script, setup) in scripts {
        while current_line < line {
            module_source.push('\n');
            current_line += 1;
        }
        module_source.push_str(&script);
        current_line += script.bytes().filter(|byte| *byte == b'\n').count() as u32;
        if !module_source.ends_with('\n') {
            module_source.push('\n');
            current_line += 1;
        }
        has_regular_script |= !setup;
    }
    if has_regular_script {
        render = None;
    }
    if !has_regular_script {
        if !module_source.is_empty() && !module_source.ends_with('\n') {
            module_source.push('\n');
        }
        if let Some(render) = &render {
            let mut vue_import = "import { h as __nativeboat_h".to_owned();
            if render.contains("__nativeboat_resolve_component") {
                vue_import.push_str(", resolveComponent as __nativeboat_resolve_component");
            }
            if render.contains("__nativeboat_with_directives") {
                vue_import.push_str(", withDirectives as __nativeboat_with_directives, vShow as __nativeboat_v_show");
            }
            vue_import.push_str(" } from 'vue';\nexport default { render() { return ");
            module_source.push_str(&vue_import);
            module_source.push_str(render);
            module_source.push_str("; } };\n");
        } else {
            module_source.push_str("export default {};\n");
        }
    }

    Parsed {
        module_source,
        template_line: render
            .is_none()
            .then(|| template.map_or(0, |template| template.position.line.max(1)))
            .filter(|line| *line > 0),
        line_map: Vec::new(),
    }
}

fn script_blocks(source: &str) -> Vec<(u32, String, bool)> {
    let mut scripts = Vec::new();
    let mut cursor = 0;
    while let Some((start, tag)) = ["<script", "<template", "<style"]
        .into_iter()
        .filter_map(|tag| {
            source[cursor..]
                .find(tag)
                .map(|offset| (cursor + offset, tag))
        })
        .min_by_key(|(offset, _)| *offset)
    {
        if tag != "<script" {
            let closing_tag = if tag == "<template" {
                "</template>"
            } else {
                "</style>"
            };
            let Some(relative_end) = source[start..].rfind(closing_tag) else {
                break;
            };
            cursor = start + relative_end + closing_tag.len();
            continue;
        }
        let after_name = start + "<script".len();
        if source
            .as_bytes()
            .get(after_name)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>')
        {
            cursor = after_name;
            continue;
        }
        let Some(opening_end) = opening_tag_end(source, start) else {
            break;
        };
        let opening = &source[after_name..opening_end];
        let setup = opening
            .split_ascii_whitespace()
            .any(|attribute| attribute == "setup" || attribute.starts_with("setup="));
        let content_start = opening_end + 1;
        let Some(relative_end) = source[content_start..].find("</script>") else {
            break;
        };
        let content_end = content_start + relative_end;
        let line = source[..content_start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32
            + 1;
        scripts.push((line, source[content_start..content_end].to_owned(), setup));
        cursor = content_end + "</script>".len();
    }
    scripts
}

fn script_setup_module(
    scripts: Vec<(u32, String, bool)>,
    render: Option<String>,
    template: Option<&&Element>,
) -> Parsed {
    use oxc::ast::ast::Statement;
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let has_define_expose = scripts
        .iter()
        .any(|(_, script, _)| script.contains("defineExpose"));
    let has_script_options = scripts
        .iter()
        .any(|(_, script, setup)| !setup && script.contains("export default"));
    let mut imports = Vec::<(String, u32)>::new();
    let mut bodies = Vec::<(String, u32)>::new();
    let mut module_bodies = Vec::<(String, u32)>::new();
    for (source_line, original_script, is_setup) in scripts {
        let script = if is_setup {
            original_script
        } else {
            original_script.replacen("export default", "const __nativeboat_script_options =", 1)
        };
        let allocator = oxc::allocator::Allocator::default();
        let parsed = Parser::new(&allocator, &script, SourceType::ts()).parse();
        let mut cursor = 0;
        let target = if is_setup {
            &mut bodies
        } else {
            &mut module_bodies
        };
        for statement in parsed.program.body {
            if let Statement::ImportDeclaration(import) = statement {
                let start = import.span.start as usize;
                let end = import.span.end as usize;
                if start < cursor || end > script.len() {
                    continue;
                }
                let import_line = source_line
                    + script[..start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count() as u32;
                let mut import_source = script[start..end].to_owned();
                if !import_source.ends_with(';') {
                    import_source.push(';');
                }
                import_source.push('\n');
                imports.push((import_source, import_line));
                let body_source_line = source_line
                    + script[..cursor]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count() as u32;
                target.push((mask_source(&script[cursor..start]), body_source_line));
                let import_source_line = source_line
                    + script[..start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count() as u32;
                target.push((mask_source(&script[start..end]), import_source_line));
                cursor = end;
            }
        }
        let body_line = source_line
            + script[..cursor]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count() as u32;
        let mut tail = script[cursor..].to_owned();
        if !tail.ends_with('\n') {
            tail.push('\n');
        }
        target.push((tail, body_line));
    }

    for (body, _) in &mut bodies {
        *body = lower_setup_macros(body);
    }

    let mut module_source = String::new();
    let mut line_map = Vec::new();
    for (import, source_line) in imports {
        append_mapped(&mut module_source, &mut line_map, &import, source_line);
    }
    for (body, source_line) in module_bodies {
        append_mapped(&mut module_source, &mut line_map, &body, source_line);
    }
    if render.is_some() {
        let mut imports_vue = "import { h as __nativeboat_h".to_owned();
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_to_display_string"))
        {
            imports_vue.push_str(", toDisplayString as __nativeboat_to_display_string");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_create_comment_vnode"))
        {
            imports_vue.push_str(", createCommentVNode as __nativeboat_create_comment_vnode");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_render_slot"))
        {
            imports_vue.push_str(", renderSlot as __nativeboat_render_slot");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_render_list"))
        {
            imports_vue.push_str(", renderList as __nativeboat_render_list");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_Fragment"))
        {
            imports_vue.push_str(", Fragment as __nativeboat_Fragment");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_resolve_component"))
        {
            imports_vue.push_str(", resolveComponent as __nativeboat_resolve_component");
        }
        if render
            .as_deref()
            .is_some_and(|render| render.contains("__nativeboat_with_directives"))
        {
            imports_vue.push_str(
                ", withDirectives as __nativeboat_with_directives, vShow as __nativeboat_v_show",
            );
        }
        imports_vue.push_str(" } from 'vue';\n");
        append_mapped(&mut module_source, &mut line_map, &imports_vue, 0);
    }
    let options_spread = if has_script_options {
        "...__nativeboat_script_options, "
    } else {
        ""
    };
    let setup_start = if has_define_expose {
        format!(
            "export default {{ {options_spread}setup(__props, {{ expose: __nativeboat_expose, slots: __nativeboat_slots, emit: __nativeboat_emit }}) {{\n"
        )
    } else {
        format!(
            "export default {{ {options_spread}setup(__props, {{ expose: __nativeboat_expose, slots: __nativeboat_slots, emit: __nativeboat_emit }}) {{\n__nativeboat_expose();\n"
        )
    };
    append_mapped(&mut module_source, &mut line_map, &setup_start, 0);
    if bodies
        .iter()
        .any(|(body, _)| body.contains("__nativeboat_with_defaults"))
    {
        append_mapped(
            &mut module_source,
            &mut line_map,
            "const __nativeboat_with_defaults = (props, defaults) => { const result = { ...defaults, ...props }; for (const key of Object.keys(defaults)) if (props[key] === undefined) result[key] = defaults[key]; return result; };\n",
            0,
        );
    }
    if bodies
        .iter()
        .any(|(body, _)| body.contains("__nativeboat_resolve_props"))
    {
        append_mapped(
            &mut module_source,
            &mut line_map,
            "const __nativeboat_resolve_props = (props, options) => { const result = { ...props }; for (const key of Object.keys(options)) { const option = options[key]; if (result[key] === undefined && option && typeof option === 'object' && 'default' in option) result[key] = typeof option.default === 'function' && option.type !== Function ? option.default() : option.default; } return result; };\n",
            0,
        );
    }
    for (body, source_line) in bodies {
        append_mapped(&mut module_source, &mut line_map, &body, source_line);
    }
    append_mapped(
        &mut module_source,
        &mut line_map,
        &format!(
            "return () => {};\n}} }};\n",
            render.as_deref().unwrap_or("undefined")
        ),
        0,
    );

    Parsed {
        module_source,
        template_line: render
            .is_none()
            .then(|| template.map_or(0, |template| template.position.line.max(1)))
            .filter(|line| *line > 0),
        line_map,
    }
}

fn mask_source(source: &str) -> String {
    source
        .chars()
        .map(|character| if character == '\n' { '\n' } else { ' ' })
        .collect()
}

fn lower_setup_macros(source: &str) -> String {
    use oxc::ast::ast::{Argument, CallExpression, Expression};
    use oxc::ast_visit::{Visit, walk};
    use oxc::parser::Parser;
    use oxc::span::{GetSpan, SourceType};

    #[derive(Default)]
    struct MacroRewriter<'a> {
        source: &'a str,
        edits: Vec<(u32, u32, String)>,
    }

    fn callee_name<'a>(expression: &Expression<'a>) -> Option<&'a str> {
        match expression.without_parentheses() {
            Expression::Identifier(identifier) => Some(identifier.name.as_str()),
            Expression::TSInstantiationExpression(instantiation) => {
                callee_name(&instantiation.expression)
            }
            _ => None,
        }
    }

    impl<'a> Visit<'a> for MacroRewriter<'_> {
        fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
            let (replacement, start, end) = match callee_name(&call.callee) {
                Some("defineProps") => {
                    let replacement = match call.arguments.first() {
                        Some(argument) if !matches!(argument, Argument::SpreadElement(_)) => {
                            let span = argument.span();
                            format!(
                                "__nativeboat_resolve_props(__props, {})",
                                &self.source[span.start as usize..span.end as usize]
                            )
                        }
                        _ => "__props".to_owned(),
                    };
                    (replacement, call.span.start, call.span.end)
                }
                Some("withDefaults") => {
                    let Some(defaults) = call.arguments.get(1) else {
                        walk::walk_call_expression(self, call);
                        return;
                    };
                    let Some(Argument::CallExpression(props_call)) = call.arguments.first() else {
                        walk::walk_call_expression(self, call);
                        return;
                    };
                    if callee_name(&props_call.callee) != Some("defineProps") {
                        walk::walk_call_expression(self, call);
                        return;
                    }
                    let span = defaults.span();
                    (
                        format!(
                            "__nativeboat_with_defaults(__props, {})",
                            &self.source[span.start as usize..span.end as usize]
                        ),
                        call.span.start,
                        call.span.end,
                    )
                }
                Some("defineEmits") => (
                    "__nativeboat_emit".to_owned(),
                    call.span.start,
                    call.span.end,
                ),
                Some("defineExpose") => {
                    let replacement = match call.arguments.first() {
                        Some(argument) => {
                            let span = argument.span();
                            format!(
                                "__nativeboat_expose({})",
                                &self.source[span.start as usize..span.end as usize]
                            )
                        }
                        None => "__nativeboat_expose()".to_owned(),
                    };
                    (replacement, call.span.start, call.span.end)
                }
                _ => {
                    walk::walk_call_expression(self, call);
                    return;
                }
            };
            self.edits.push((start, end, replacement));
            walk::walk_call_expression(self, call);
        }
    }

    let allocator = oxc::allocator::Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    let mut rewriter = MacroRewriter {
        source,
        edits: Vec::new(),
    };
    rewriter.visit_program(&parsed.program);
    let mut edits = rewriter.edits;
    edits.sort_by_key(|(start, end, _)| (*start, std::cmp::Reverse(*end)));
    let mut outer = Vec::new();
    for edit in edits {
        if outer
            .iter()
            .any(|(start, end, _): &(u32, u32, String)| *start <= edit.0 && edit.1 <= *end)
        {
            continue;
        }
        outer.push(edit);
    }
    outer.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut output = source.to_owned();
    for (start, end, replacement) in outer {
        output.replace_range(start as usize..end as usize, &replacement);
    }
    output
}

fn append_mapped(output: &mut String, line_map: &mut Vec<u32>, source: &str, source_line: u32) {
    let mut line_offset = 0;
    let mut new_line = output.is_empty() || output.ends_with('\n');
    for line in source.split_inclusive('\n') {
        if new_line {
            line_map.push(if source_line == 0 {
                0
            } else {
                source_line + line_offset
            });
        }
        output.push_str(line);
        if line.ends_with('\n') {
            line_offset += 1;
            new_line = true;
        } else {
            new_line = false;
        }
    }
}

/// Lowers a static SFC template into a Vue render function. Bindings, refs and
/// directives remain explicit blockers until their semantics are implemented.
fn template_content<'a>(source: &'a str, template: &nativeboat_html::Element) -> Option<&'a str> {
    let _ = template;
    let mut cursor = 0;
    let opening = loop {
        let candidate = cursor + source[cursor..].find("<template")?;
        let inside_script = source[..candidate]
            .rfind("<script")
            .is_some_and(|start| !source[start..candidate].contains("</script>"));
        let inside_style = source[..candidate]
            .rfind("<style")
            .is_some_and(|start| !source[start..candidate].contains("</style>"));
        if inside_script || inside_style {
            let (tag, length) = if inside_script {
                ("</script>", "</script>".len())
            } else {
                ("</style>", "</style>".len())
            };
            cursor = source[candidate..].find(tag)? + candidate + length;
            continue;
        }
        break candidate;
    };
    let end = opening_tag_end(source, opening)?;
    let content_start = end + 1;
    let content_end = source[content_start..].rfind("</template>")? + content_start;
    Some(&source[content_start..content_end])
}

fn opening_tag_end(source: &str, opening: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in source[opening..].char_indices() {
        match (quote, character) {
            (Some(current), character) if current == character => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '>') => return Some(opening + offset),
            _ => {}
        }
    }
    None
}

fn static_render(
    template: &str,
    inline_setup: bool,
    components: &BTreeMap<String, String>,
    refs: &BTreeSet<String>,
) -> Option<String> {
    let nodes = parse_vue_fragment_in(template, "div");
    let mut roots = Vec::new();
    for node in &nodes {
        match node {
            Node::Element(element) => roots.push(element),
            Node::Text(text) if text.text.trim().is_empty() => {}
            Node::Comment(_) => {}
            Node::Text(_) => return None,
        }
    }
    let [root] = roots.as_slice() else {
        return None;
    };
    static_element(root, inline_setup, components, refs)
}

/// Converts a static SVG asset imported as a Vue component to the same Vue
/// virtual-node render shape produced by the project's SVG component loader.
pub(super) fn svg_module_source(source: &str) -> Option<String> {
    let render = static_render(source, false, &BTreeMap::new(), &BTreeSet::new())?;
    Some(format!(
        "import {{ h as __nativeboat_h }} from 'vue';\nexport default {{ render() {{ return {render}; }} }};\n"
    ))
}

fn setup_components(
    scripts: &[(u32, String, bool)],
    component_name: Option<&str>,
) -> BTreeMap<String, String> {
    use oxc::ast::ast::{ImportDeclarationSpecifier, Statement};
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let mut components = BTreeMap::new();
    for (_, script, _) in scripts {
        let allocator = oxc::allocator::Allocator::default();
        let parsed = Parser::new(&allocator, script, SourceType::ts()).parse();
        for statement in parsed.program.body {
            let Statement::ImportDeclaration(import) = statement else {
                continue;
            };
            if import.import_kind == oxc::ast::ast::ImportOrExportKind::Type {
                continue;
            }
            for specifier in import.specifiers.iter().flatten() {
                let local = match specifier {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                        Some(specifier.local.name.as_str())
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(specifier)
                        if specifier.import_kind != oxc::ast::ast::ImportOrExportKind::Type =>
                    {
                        Some(specifier.local.name.as_str())
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => None,
                    _ => None,
                };
                let Some(local) = local else { continue };
                for tag in component_tags(local) {
                    components.insert(tag, local.to_owned());
                }
            }
        }
    }
    if let Some(component_name) = component_name {
        let mut pascal = String::new();
        let mut uppercase = true;
        for character in component_name.chars() {
            if character == '-' || character == '_' {
                uppercase = true;
            } else if uppercase {
                pascal.push(character.to_ascii_uppercase());
                uppercase = false;
            } else {
                pascal.push(character);
            }
        }
        if !pascal.is_empty() {
            for tag in component_tags(&pascal) {
                if !is_html_tag(&tag) {
                    components.insert(tag, format!("\u{1}self:{pascal}"));
                }
            }
        }
    }
    components
}

fn setup_refs(scripts: &[(u32, String, bool)]) -> BTreeSet<String> {
    use oxc::ast::ast::{
        BindingPattern, Expression, ImportDeclarationSpecifier, ModuleExportName, Statement,
    };
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let mut refs = BTreeSet::new();
    for (_, script, _) in scripts {
        let allocator = oxc::allocator::Allocator::default();
        let parsed = Parser::new(&allocator, script, SourceType::ts()).parse();
        let mut helpers = BTreeSet::from([
            "ref",
            "shallowRef",
            "computed",
            "customRef",
            "toRef",
            "useTemplateRef",
        ]);
        for statement in &parsed.program.body {
            let Statement::ImportDeclaration(import) = statement else {
                continue;
            };
            if import.source.value != "vue" {
                continue;
            }
            for specifier in import.specifiers.iter().flatten() {
                if let ImportDeclarationSpecifier::ImportSpecifier(specifier) = specifier {
                    let imported = match &specifier.imported {
                        ModuleExportName::IdentifierName(name) => name.name.as_str(),
                        ModuleExportName::IdentifierReference(name) => name.name.as_str(),
                        ModuleExportName::StringLiteral(name) => name.value.as_str(),
                    };
                    if matches!(
                        imported,
                        "ref"
                            | "shallowRef"
                            | "computed"
                            | "customRef"
                            | "toRef"
                            | "useTemplateRef"
                    ) {
                        helpers.insert(specifier.local.name.as_str());
                    }
                }
            }
        }
        for statement in &parsed.program.body {
            let Statement::VariableDeclaration(declaration) = statement else {
                continue;
            };
            for declarator in &declaration.declarations {
                let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
                    continue;
                };
                let Some(Expression::CallExpression(call)) = declarator
                    .init
                    .as_ref()
                    .map(Expression::without_parentheses)
                else {
                    continue;
                };
                let callee = match call.callee.without_parentheses() {
                    Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                    Expression::TSInstantiationExpression(instantiation) => {
                        match instantiation.expression.without_parentheses() {
                            Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if callee.is_some_and(|callee| helpers.contains(callee)) {
                    refs.insert(binding.name.to_string());
                }
            }
        }
    }
    refs
}

fn template_prop_key(name: &str) -> String {
    format!("\u{1}prop:{name}")
}

fn setup_template_props(scripts: &[(u32, String, bool)]) -> BTreeSet<String> {
    use oxc::ast::ast::{Argument, Expression, PropertyKey, Statement, TSSignature, TSType};
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    fn macro_call<'a>(
        expression: &'a Expression<'a>,
        expected: &str,
    ) -> Option<&'a oxc::ast::ast::CallExpression<'a>> {
        let Expression::CallExpression(call) = expression.without_parentheses() else {
            return None;
        };
        let callee = match call.callee.without_parentheses() {
            Expression::Identifier(identifier) => Some(identifier.name.as_str()),
            Expression::TSInstantiationExpression(instantiation) => {
                match instantiation.expression.without_parentheses() {
                    Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                    _ => None,
                }
            }
            _ => None,
        }?;
        (callee == expected).then_some(call)
    }

    fn callee_name<'a>(call: &'a oxc::ast::ast::CallExpression<'a>) -> Option<&'a str> {
        match call.callee.without_parentheses() {
            Expression::Identifier(identifier) => Some(identifier.name.as_str()),
            Expression::TSInstantiationExpression(instantiation) => {
                match instantiation.expression.without_parentheses() {
                    Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn property_names(ty: &TSType<'_>) -> Vec<String> {
        let TSType::TSTypeLiteral(literal) = ty else {
            return Vec::new();
        };
        literal
            .members
            .iter()
            .filter_map(|member| {
                let TSSignature::TSPropertySignature(property) = member else {
                    return None;
                };
                match &property.key {
                    PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
                    PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
                    _ => None,
                }
            })
            .collect()
    }

    /// Property names of a props declaration's runtime argument: an object's
    /// keys or an array's strings.
    fn runtime_names(argument: Option<&Argument<'_>>) -> Vec<String> {
        match argument {
            Some(Argument::ObjectExpression(object)) => object
                .properties
                .iter()
                .filter_map(|property| match property {
                    oxc::ast::ast::ObjectPropertyKind::ObjectProperty(property) => {
                        match &property.key {
                            PropertyKey::StaticIdentifier(name) => Some(name.name.to_string()),
                            PropertyKey::StringLiteral(name) => Some(name.value.to_string()),
                            _ => None,
                        }
                    }
                    oxc::ast::ast::ObjectPropertyKind::SpreadProperty(_) => None,
                })
                .collect(),
            Some(Argument::ArrayExpression(array)) => array
                .elements
                .iter()
                .filter_map(|element| match element {
                    oxc::ast::ast::ArrayExpressionElement::StringLiteral(name) => {
                        Some(name.value.to_string())
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    let mut names = BTreeSet::new();
    for (_, script, _) in scripts {
        let allocator = oxc::allocator::Allocator::default();
        let parsed = Parser::new(&allocator, script, SourceType::ts()).parse();
        // Local interfaces and type aliases a type-based declaration may name.
        let mut local_types: Vec<(String, Vec<String>)> = Vec::new();
        for statement in &parsed.program.body {
            match statement.as_declaration() {
                Some(oxc::ast::ast::Declaration::TSInterfaceDeclaration(interface)) => {
                    let members = interface
                        .body
                        .body
                        .iter()
                        .filter_map(|member| match member {
                            TSSignature::TSPropertySignature(property) => match &property.key {
                                PropertyKey::StaticIdentifier(name) => Some(name.name.to_string()),
                                PropertyKey::StringLiteral(name) => Some(name.value.to_string()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect();
                    local_types.push((interface.id.name.to_string(), members));
                }
                Some(oxc::ast::ast::Declaration::TSTypeAliasDeclaration(alias)) => {
                    local_types.push((
                        alias.id.name.to_string(),
                        property_names(&alias.type_annotation),
                    ));
                }
                _ => {}
            }
        }
        // `defineProps(...)` as a statement or a declaration's initializer.
        let mut expressions = Vec::new();
        for statement in &parsed.program.body {
            match statement {
                Statement::ExpressionStatement(statement) => {
                    expressions.push(statement.expression.without_parentheses());
                }
                Statement::VariableDeclaration(declaration) => {
                    for declarator in &declaration.declarations {
                        if let Some(init) = &declarator.init {
                            expressions.push(init.without_parentheses());
                        }
                    }
                }
                _ => {}
            }
        }
        for expression in expressions {
            let props_call = if let Some(call) = macro_call(expression, "defineProps") {
                Some(call)
            } else if let Some(defaults_call) = macro_call(expression, "withDefaults") {
                match defaults_call.arguments.first() {
                    Some(Argument::CallExpression(call)) => {
                        (callee_name(call) == Some("defineProps")).then_some(&**call)
                    }
                    _ => None,
                }
            } else {
                None
            };
            let Some(props_call) = props_call else {
                continue;
            };
            if let Some(ty) = props_call
                .type_arguments
                .as_ref()
                .and_then(|arguments| arguments.params.first())
            {
                names.extend(property_names(ty));
                if let TSType::TSTypeReference(reference) = ty
                    && let oxc::ast::ast::TSTypeName::IdentifierReference(name) =
                        &reference.type_name
                    && let Some((_, members)) = local_types
                        .iter()
                        .find(|(local, _)| local == name.name.as_str())
                {
                    names.extend(members.iter().cloned());
                }
            }
            names.extend(runtime_names(props_call.arguments.first()));
        }
    }
    names
}

fn component_tags(local: &str) -> Vec<String> {
    let mut kebab = String::new();
    for (index, character) in local.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index > 0 {
                kebab.push('-');
            }
            kebab.push(character.to_ascii_lowercase());
        } else {
            kebab.push(character);
        }
    }
    let mut tags = vec![local.to_owned(), local.to_ascii_lowercase(), kebab];
    tags.sort();
    tags.dedup();
    tags
}

fn camelize(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    let mut uppercase = false;
    for character in name.chars() {
        if character == '-' {
            uppercase = true;
        } else if uppercase {
            output.push(character.to_ascii_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn slot_scope(source: &str) -> Option<(String, Vec<String>)> {
    let source = source.trim();
    if source.is_empty() {
        return Some((String::new(), Vec::new()));
    }
    if is_identifier(source) {
        return Some((source.to_owned(), vec![source.to_owned()]));
    }
    let members = source.strip_prefix('{')?.strip_suffix('}')?;
    let mut names = Vec::new();
    for member in members
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let binding = member
            .split_once(':')
            .map_or(member, |(_, binding)| binding.trim());
        if !is_identifier(binding) {
            return None;
        }
        names.push(binding.to_owned());
    }
    Some((source.to_owned(), names))
}

fn is_model_attribute(name: &str) -> bool {
    name == "v-model" || name.starts_with("v-model:") || name.starts_with("v-model.")
}

fn is_global_component(name: &str) -> bool {
    name.starts_with("el-")
        || matches!(
            name,
            "router-view"
                | "router-link"
                | "transition"
                | "Transition"
                | "transition-group"
                | "TransitionGroup"
                | "teleport"
                | "Teleport"
                | "keep-alive"
                | "KeepAlive"
                | "suspense"
                | "Suspense"
        )
}

fn lower_model(
    element: &Element,
    attribute: &nativeboat_html::Attribute,
    component: bool,
    refs: &BTreeSet<String>,
) -> Option<Vec<String>> {
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let tail = attribute.name.strip_prefix("v-model")?;
    let (argument, modifiers) = if let Some(tail) = tail.strip_prefix(':') {
        let (argument, modifiers) = tail.split_once('.').unwrap_or((tail, ""));
        if argument.is_empty() || argument.starts_with('[') || argument.ends_with(']') {
            return None;
        }
        (argument, modifiers)
    } else if let Some(modifiers) = tail.strip_prefix('.') {
        ("modelValue", modifiers)
    } else {
        ("modelValue", "")
    };
    let modifiers = modifiers
        .split('.')
        .filter(|modifier| !modifier.is_empty())
        .collect::<Vec<_>>();
    if modifiers
        .iter()
        .any(|modifier| !matches!(*modifier, "trim" | "number" | "lazy"))
    {
        return None;
    }

    let expression = attribute.value.trim();
    if expression.is_empty() {
        return None;
    }
    let assignment = format!("({expression}) = __nativeboat_model_value;");
    let allocator = oxc::allocator::Allocator::default();
    let parsed = Parser::new(&allocator, &assignment, SourceType::ts()).parse();
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let target = template_expression(expression, refs);
    let mut output = Vec::new();
    if component {
        let argument = camelize(argument);
        output.push(format!(
            "{}: ({})",
            js_string(&argument),
            template_expression(expression, refs)
        ));
        output.push(format!(
            "{}: ($event) => ({} = $event)",
            js_string(&format!("onUpdate:{argument}")),
            target
        ));
        if !modifiers.is_empty() {
            let key = if argument == "modelValue" {
                "modelModifiers".to_owned()
            } else {
                format!("{argument}Modifiers")
            };
            output.push(format!(
                "{}: {{{}}}",
                js_string(&key),
                modifiers
                    .iter()
                    .map(|modifier| format!("{}: true", js_string(modifier)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        return Some(output);
    }
    if argument != "modelValue" {
        return None;
    }
    let input_type = element
        .attributes
        .iter()
        .find(|attribute| attribute.name == "type")
        .map(|attribute| attribute.value.as_str());
    let is_checkbox = element.name == "input" && input_type == Some("checkbox");
    let value = if is_checkbox {
        "$event.target.checked".to_owned()
    } else {
        let mut value = "$event.target.value".to_owned();
        if modifiers.contains(&"trim") {
            value.push_str(".trim()");
        }
        if modifiers.contains(&"number") {
            value = format!("Number({value})");
        }
        value
    };
    output.push(format!(
        "{}: ({})",
        js_string(if is_checkbox { "checked" } else { "value" }),
        template_expression(expression, refs)
    ));
    output.push(format!(
        "{}: ($event) => ({} = {})",
        js_string(
            if modifiers.contains(&"lazy") || is_checkbox || element.name == "select" {
                "onChange"
            } else {
                "onInput"
            }
        ),
        target,
        value
    ));
    Some(output)
}

fn event_handler(
    event: &str,
    expression: &str,
    refs: &BTreeSet<String>,
) -> Option<(String, String)> {
    use oxc::ast::ast::{Expression, Statement};
    use oxc::parser::Parser;
    use oxc::span::SourceType;

    let mut pieces = event.split('.');
    let event_name = pieces.next()?.trim();
    if event_name.is_empty() {
        return None;
    }
    let mut modifiers = Vec::new();
    for modifier in pieces {
        let check = match modifier {
            "stop" => "$event.stopPropagation();".to_owned(),
            "prevent" => "$event.preventDefault();".to_owned(),
            "self" => "if ($event.target !== $event.currentTarget) return;".to_owned(),
            "middle" => "if ($event.button !== 1) return;".to_owned(),
            "left" if event_name.starts_with("mouse") || event_name == "click" => {
                "if ($event.button !== 0) return;".to_owned()
            }
            "right" if event_name.starts_with("mouse") || event_name == "click" => {
                "if ($event.button !== 2) return;".to_owned()
            }
            "enter" => "if ($event.key !== 'Enter') return;".to_owned(),
            "esc" => "if ($event.key !== 'Escape') return;".to_owned(),
            "space" => "if ($event.key !== ' ') return;".to_owned(),
            "tab" => "if ($event.key !== 'Tab') return;".to_owned(),
            "delete" => {
                "if ($event.key !== 'Backspace' && $event.key !== 'Delete') return;".to_owned()
            }
            "up" => "if ($event.key !== 'ArrowUp') return;".to_owned(),
            "down" => "if ($event.key !== 'ArrowDown') return;".to_owned(),
            "left" => "if ($event.key !== 'ArrowLeft') return;".to_owned(),
            "right" => "if ($event.key !== 'ArrowRight') return;".to_owned(),
            _ => return None,
        };
        modifiers.push(check);
    }

    let expression = template_expression(expression.trim(), refs);
    let expression = expression.trim();
    if expression.is_empty() {
        return None;
    }
    let source = format!("const __nativeboat_event_handler = {expression};");
    let allocator = oxc::allocator::Allocator::default();
    let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
    if !parsed.diagnostics.is_empty() {
        return None;
    }
    let Statement::VariableDeclaration(declaration) = parsed.program.body.first()? else {
        return None;
    };
    let handler = declaration.declarations.first()?.init.as_ref()?;
    let accepts_event = matches!(
        handler.without_parentheses(),
        Expression::Identifier(_)
            | Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_)
    );

    let mut normalized_event = camelize(event_name);
    normalized_event.get_mut(..1)?.make_ascii_uppercase();
    let property = format!("on{normalized_event}");
    if modifiers.is_empty() && accepts_event {
        return Some((property, format!("({expression})")));
    }
    let invocation = if accepts_event {
        format!("({expression})($event);")
    } else {
        format!("{expression};")
    };
    Some((
        property,
        format!("($event) => {{ {} {invocation} }}", modifiers.join(" ")),
    ))
}

fn static_element(
    element: &Element,
    inline_setup: bool,
    components: &BTreeMap<String, String>,
    refs: &BTreeSet<String>,
) -> Option<String> {
    let iteration = directive_value(element, "v-for");
    let condition = directive_value(element, "v-if");
    if has_directive(element, "v-else") || has_directive(element, "v-else-if") {
        return None;
    }
    if iteration.is_some() && condition.is_some() {
        return None;
    }
    if (condition.is_some() || iteration.is_some()) && !inline_setup {
        return None;
    }
    let show = directive_value(element, "v-show");
    if show.is_some() && !inline_setup {
        return None;
    }
    let mut item_refs = refs.clone();
    let iteration_parts = if let Some(iteration) = iteration {
        Some(split_for_expression(iteration)?)
    } else {
        None
    };
    if let Some((aliases, _)) = iteration_parts {
        for alias in aliases
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split(',')
            .map(str::trim)
        {
            if is_identifier(alias) {
                item_refs.remove(alias);
            }
        }
    }
    let mut vnode = static_element_content(element, inline_setup, components, &item_refs)?;
    vnode = apply_v_show(element, vnode, refs)?;
    if let Some((aliases, source)) = iteration_parts {
        let parameters = if aliases.starts_with('(') && aliases.ends_with(')') {
            aliases.trim().to_owned()
        } else {
            format!("({})", aliases.trim())
        };
        return Some(format!(
            "__nativeboat_h(__nativeboat_Fragment, null, __nativeboat_render_list(({}), {} => ({})))",
            template_expression(source.trim(), refs),
            parameters,
            vnode
        ));
    }
    condition.map_or(Some(vnode.clone()), |condition| {
        (!condition.trim().is_empty()).then(|| {
            format!(
                "({}) ? ({vnode}) : __nativeboat_create_comment_vnode()",
                template_expression(condition.trim(), refs)
            )
        })
    })
}

fn apply_v_show(element: &Element, vnode: String, refs: &BTreeSet<String>) -> Option<String> {
    let Some(show) = directive_value(element, "v-show") else {
        return Some(vnode);
    };
    if show.trim().is_empty() {
        return None;
    }
    Some(format!(
        "__nativeboat_with_directives({vnode}, [[__nativeboat_v_show, ({})]])",
        template_expression(show.trim(), refs)
    ))
}

fn static_element_content(
    element: &Element,
    inline_setup: bool,
    components: &BTreeMap<String, String>,
    refs: &BTreeSet<String>,
) -> Option<String> {
    if element.name == "template" {
        if !inline_setup
            || element.attributes.iter().any(|attribute| {
                !matches!(
                    attribute.name.as_str(),
                    "v-if" | "v-else-if" | "v-else" | "v-for"
                )
            })
        {
            return None;
        }
        let children = static_children(&element.children, inline_setup, components, refs)?;
        return Some(format!(
            "__nativeboat_h(__nativeboat_Fragment, null, [{}])",
            children.join(", ")
        ));
    }
    if element.name == "slot" {
        if !inline_setup {
            return None;
        }
        let mut slot_name = "\"default\"".to_owned();
        for attribute in &element.attributes {
            if attribute.name == "name" {
                slot_name = js_string(&attribute.value);
            } else {
                let name = attribute
                    .name
                    .strip_prefix(':')
                    .or_else(|| attribute.name.strip_prefix("v-bind:"))?;
                if name != "name" || attribute.value.trim().is_empty() {
                    return None;
                }
                slot_name = format!("({})", template_expression(attribute.value.trim(), refs));
            }
        }
        let fallback = static_children(&element.children, inline_setup, components, refs)?;
        return Some(format!(
            "__nativeboat_render_slot(__nativeboat_slots, {slot_name}, null, () => [{}])",
            fallback.join(", ")
        ));
    }
    let dynamic_tag = if element.name == "component" {
        let is_attribute = element
            .attributes
            .iter()
            .find(|attribute| attribute.name == ":is" || attribute.name == "v-bind:is")?;
        if !inline_setup || is_attribute.value.trim().is_empty() {
            return None;
        }
        Some(template_expression(is_attribute.value.trim(), refs))
    } else {
        None
    };
    let component = components.get(&element.name);
    let global_component = is_global_component(&element.name);
    let is_component = component.is_some() || global_component || dynamic_tag.is_some();
    if !is_component && !is_html_tag(&element.name) {
        return None;
    }
    let mut props = Vec::new();
    let model = element
        .attributes
        .iter()
        .find(|attribute| is_model_attribute(&attribute.name));
    if let Some(model) = model {
        if !inline_setup {
            return None;
        }
        let lowered = lower_model(element, model, is_component, refs)?;
        props.extend(lowered);
    }
    let html = element
        .attributes
        .iter()
        .find(|attribute| attribute.name == "v-html");
    if let Some(html) = html {
        if !inline_setup || is_component || html.value.trim().is_empty() {
            return None;
        }
        props.push(format!(
            "\"innerHTML\": ({})",
            template_expression(html.value.trim(), refs)
        ));
    }
    for attribute in &element.attributes {
        if is_model_attribute(&attribute.name) {
            continue;
        }
        if matches!(
            attribute.name.as_str(),
            "v-if" | "v-else-if" | "v-else" | "v-for" | "v-show" | "v-html"
        ) {
            if !inline_setup {
                return None;
            }
            continue;
        }
        if attribute.name == "ref" {
            if !inline_setup || !refs.contains(attribute.value.trim()) {
                return None;
            }
            let target = format!("{}.value", attribute.value.trim());
            props.push(format!("\"ref\": ($el) => ({target} = $el)"));
            continue;
        }
        let event = attribute
            .name
            .strip_prefix('@')
            .or_else(|| attribute.name.strip_prefix("v-on:"));
        if let Some(event) = event {
            if !inline_setup {
                return None;
            }
            let (name, handler) = event_handler(event, &attribute.value, refs)?;
            props.push(format!("{}: {handler}", js_string(&name)));
            continue;
        }
        let binding = attribute
            .name
            .strip_prefix(':')
            .or_else(|| attribute.name.strip_prefix("v-bind:"));
        if attribute.name == "v-bind" {
            if !inline_setup || attribute.value.trim().is_empty() {
                return None;
            }
            props.push(format!(
                "...({})",
                template_expression(attribute.value.trim(), refs)
            ));
            continue;
        }
        if element.name == "component" && binding == Some("is") {
            continue;
        }
        if attribute.name.starts_with('#')
            || (attribute.name.starts_with("v-") && binding.is_none())
            || attribute.name.starts_with("on")
            || matches!(attribute.name.as_str(), "is")
        {
            return None;
        }
        if let Some(name) = binding {
            if !inline_setup || name.is_empty() || attribute.value.trim().is_empty() {
                return None;
            }
            props.push(format!(
                "{}: ({})",
                js_string(&camelize(name)),
                template_expression(attribute.value.trim(), refs)
            ));
        } else {
            props.push(format!(
                "{}: {}",
                js_string(&attribute.name),
                js_string(&attribute.value)
            ));
        }
    }
    let tag = dynamic_tag.unwrap_or_else(|| {
        component.map_or_else(
            || {
                if global_component {
                    format!(
                        "__nativeboat_resolve_component({})",
                        js_string(&element.name)
                    )
                } else {
                    js_string(&element.name)
                }
            },
            |name| {
                name.strip_prefix("\u{1}self:").map_or_else(
                    || name.clone(),
                    |self_name| format!("__nativeboat_resolve_component({})", js_string(self_name)),
                )
            },
        )
    });
    let props = if props.is_empty() {
        "null".to_owned()
    } else {
        format!("{{{}}}", props.join(", "))
    };
    if is_component {
        let mut slots = Vec::new();
        let mut default_children = Vec::new();
        for child in &element.children {
            match child {
                Node::Element(child) if child.name == "template" => {
                    let slot_attribute = child
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name.starts_with('#'))?;
                    let slot = slot_attribute.name.strip_prefix('#')?;
                    if child.attributes.len() != 1 || slot.is_empty() {
                        return None;
                    }
                    let (scope, names) = slot_scope(&slot_attribute.value)?;
                    let mut scoped_refs = refs.clone();
                    for name in names {
                        scoped_refs.remove(&name);
                    }
                    let values =
                        static_children(&child.children, inline_setup, components, &scoped_refs)?;
                    slots.push(format!(
                        "{}: {} => [{}]",
                        js_string(slot),
                        if scope.is_empty() {
                            "()".to_owned()
                        } else {
                            format!("({scope})")
                        },
                        values.join(", ")
                    ));
                }
                _ => default_children.extend(static_children(
                    std::slice::from_ref(child),
                    inline_setup,
                    components,
                    refs,
                )?),
            }
        }
        if !default_children.is_empty() {
            if slots.iter().any(|slot| slot.starts_with("\"default\":")) {
                return None;
            }
            slots.push(format!(
                "\"default\": () => [{}]",
                default_children.join(", ")
            ));
        }
        if slots.is_empty() {
            return Some(format!("__nativeboat_h({tag}, {props})"));
        }
        return Some(format!(
            "__nativeboat_h({tag}, {props}, {{{}}})",
            slots.join(", ")
        ));
    }
    let children = if html.is_some() {
        Vec::new()
    } else {
        static_children(&element.children, inline_setup, components, refs)?
    };
    if children.is_empty() {
        Some(format!("__nativeboat_h({tag}, {props})"))
    } else {
        Some(format!(
            "__nativeboat_h({tag}, {props}, [{}])",
            children.join(", ")
        ))
    }
}

fn static_children(
    nodes: &[Node],
    inline_setup: bool,
    components: &BTreeMap<String, String>,
    refs: &BTreeSet<String>,
) -> Option<Vec<String>> {
    let mut children = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        let child = &nodes[index];
        match child {
            Node::Element(child) => {
                if has_directive(child, "v-if") {
                    let (conditional, next) =
                        conditional_chain(nodes, index, inline_setup, components, refs)?;
                    children.push(conditional);
                    index = next;
                    continue;
                }
                if child.name == "template" {
                    return None;
                }
                if has_directive(child, "v-else") || has_directive(child, "v-else-if") {
                    return None;
                }
                children.push(static_element(child, inline_setup, components, refs)?);
            }
            Node::Text(text) => children.extend(static_text(&text.text, inline_setup, refs)?),
            Node::Comment(_) => {}
        }
        index += 1;
    }
    Some(children)
}

fn conditional_chain(
    nodes: &[Node],
    start: usize,
    inline_setup: bool,
    components: &BTreeMap<String, String>,
    refs: &BTreeSet<String>,
) -> Option<(String, usize)> {
    if !inline_setup {
        return None;
    }
    let mut branches = Vec::new();
    let mut cursor = start;
    let mut fallback = None;
    loop {
        let Node::Element(element) = nodes.get(cursor)? else {
            return None;
        };
        let is_else = has_directive(element, "v-else");
        let condition = if is_else {
            None
        } else if let Some(condition) = directive_value(element, "v-if") {
            Some(condition.trim())
        } else {
            Some(directive_value(element, "v-else-if")?.trim())
        };
        if is_else {
            if fallback.is_some() || has_directive(element, "v-else-if") {
                return None;
            }
            fallback = Some(apply_v_show(
                element,
                static_element_content(element, inline_setup, components, refs)?,
                refs,
            )?);
            cursor += 1;
            break;
        }
        let condition = condition.filter(|condition| !condition.is_empty())?;
        if has_directive(element, "v-else")
            || has_directive(element, "v-if") && !branches.is_empty()
        {
            return None;
        }
        branches.push((
            condition.to_owned(),
            apply_v_show(
                element,
                static_element_content(element, inline_setup, components, refs)?,
                refs,
            )?,
        ));
        cursor += 1;
        let mut next = cursor;
        while let Some(node) = nodes.get(next) {
            match node {
                Node::Comment(_) => next += 1,
                Node::Text(text) if text.text.trim().is_empty() => next += 1,
                _ => break,
            }
        }
        match nodes.get(next) {
            Some(Node::Element(element))
                if has_directive(element, "v-else-if") || has_directive(element, "v-else") =>
            {
                cursor = next;
            }
            _ => break,
        }
    }
    let mut expression = fallback.unwrap_or_else(|| "__nativeboat_create_comment_vnode()".into());
    for (condition, vnode) in branches.into_iter().rev() {
        expression = format!("({condition}) ? ({vnode}) : ({expression})");
    }
    Some((expression, cursor))
}

fn directive_value<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn has_directive(element: &Element, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name == name)
}

fn split_for_expression(expression: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0i32;
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let character = bytes[index] as char;
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        match character {
            '\'' | '"' | '`' => quote = Some(character),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && bytes[index].is_ascii_whitespace() {
            let separator = expression[index..]
                .find(" in ")
                .map(|offset| (offset, 4))
                .or_else(|| expression[index..].find(" of ").map(|offset| (offset, 4)));
            if let Some((offset, length)) = separator {
                let split = index + offset;
                let aliases = expression[..split].trim();
                let source = expression[split + length..].trim();
                if aliases.is_empty() || source.is_empty() {
                    return None;
                }
                return Some((aliases, source));
            }
        }
        index += 1;
    }
    None
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first == '$' || first.is_ascii_alphabetic())
        && characters.all(|character| {
            character == '_' || character == '$' || character.is_ascii_alphanumeric()
        })
}

fn static_text(text: &str, inline_setup: bool, refs: &BTreeSet<String>) -> Option<Vec<String>> {
    if !text.contains("{{") && !text.contains("}}") {
        return if text.trim().is_empty() {
            Some(Vec::new())
        } else {
            Some(vec![js_string(text)])
        };
    }
    if !inline_setup {
        return None;
    }

    let mut children = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find("{{") {
        let start = cursor + relative_start;
        let expression_start = start + 2;
        let end = interpolation_end(text, expression_start)?;
        let expression = text[expression_start..end].trim();
        if expression.is_empty() {
            return None;
        }
        let literal = &text[cursor..start];
        if !literal.is_empty() && !literal.trim().is_empty() {
            children.push(js_string(literal));
        }
        children.push(format!(
            "__nativeboat_to_display_string({})",
            template_expression(expression, refs)
        ));
        cursor = end + 2;
    }
    if text[cursor..].contains("{{") || text[cursor..].contains("}}") {
        return None;
    }
    let literal = &text[cursor..];
    if !literal.is_empty() && !literal.trim().is_empty() {
        children.push(js_string(literal));
    }
    Some(children)
}

fn interpolation_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut cursor = start;
    let mut quote = None;
    let mut escaped = false;
    let mut delimiters = Vec::new();
    while cursor + 1 < bytes.len() {
        let byte = bytes[cursor];
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' => delimiters.push(b')'),
            b'[' => delimiters.push(b']'),
            b'{' => delimiters.push(b'}'),
            b')' | b']' | b'}' if delimiters.last() == Some(&byte) => {
                delimiters.pop();
            }
            b'}' if delimiters.is_empty() && bytes[cursor + 1] == b'}' => return Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn template_expression(expression: &str, refs: &BTreeSet<String>) -> String {
    use oxc::ast::ast::IdentifierReference;
    use oxc::ast_visit::Visit;
    use oxc::parser::Parser;
    use oxc::semantic::{Scoping, SemanticBuilder};
    use oxc::span::{SourceType, Span};

    struct RefReads<'a> {
        names: &'a BTreeSet<String>,
        scoping: &'a Scoping,
        spans: Vec<(Span, String)>,
    }

    impl<'a> Visit<'a> for RefReads<'_> {
        fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
            let name = identifier.name.as_str();
            let is_ref = self.names.contains(name);
            let is_prop = self.names.contains(&template_prop_key(name));
            if (is_ref || is_prop)
                && self
                    .scoping
                    .get_reference(identifier.reference_id())
                    .symbol_id()
                    .is_none()
            {
                let replacement = if is_prop {
                    format!("__props.{name}")
                } else {
                    format!("{name}.value")
                };
                self.spans.push((identifier.span, replacement));
            }
        }
    }

    if refs.is_empty() {
        return expression.to_owned();
    }
    let prefix = "const __nativeboat_template_expression = (";
    let source = format!("{prefix}{expression});");
    let allocator = oxc::allocator::Allocator::default();
    let parsed = Parser::new(&allocator, &source, SourceType::ts()).parse();
    if parsed.diagnostics.has_errors() {
        return expression.to_owned();
    }
    let semantic = SemanticBuilder::new().build(&parsed.program);
    if semantic.diagnostics.has_errors() {
        return expression.to_owned();
    }
    let scoping = semantic.semantic.into_scoping();
    let mut reads = RefReads {
        names: refs,
        scoping: &scoping,
        spans: Vec::new(),
    };
    reads.visit_program(&parsed.program);
    let expression_start = prefix.len();
    let mut spans = reads.spans;
    spans.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut output = expression.to_owned();
    for (span, replacement) in spans {
        let start = span.start as usize - expression_start;
        let end = span.end as usize - expression_start;
        if end <= output.len() && output.is_char_boundary(start) && output.is_char_boundary(end) {
            output.replace_range(start..end, &replacement);
        }
    }
    output
}

fn is_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "article"
            | "abbr"
            | "address"
            | "area"
            | "aside"
            | "b"
            | "bdi"
            | "bdo"
            | "body"
            | "blockquote"
            | "br"
            | "button"
            | "canvas"
            | "caption"
            | "code"
            | "circle"
            | "col"
            | "colgroup"
            | "data"
            | "datalist"
            | "dd"
            | "del"
            | "details"
            | "dfn"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "g"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "html"
            | "i"
            | "iframe"
            | "img"
            | "ins"
            | "input"
            | "kbd"
            | "label"
            | "legend"
            | "li"
            | "line"
            | "main"
            | "map"
            | "mark"
            | "meter"
            | "nav"
            | "object"
            | "ol"
            | "option"
            | "optgroup"
            | "output"
            | "p"
            | "path"
            | "picture"
            | "polygon"
            | "polyline"
            | "pre"
            | "progress"
            | "q"
            | "rect"
            | "rp"
            | "rt"
            | "ruby"
            | "s"
            | "samp"
            | "search"
            | "section"
            | "select"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "summary"
            | "sup"
            | "svg"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "textarea"
            | "time"
            | "title"
            | "tr"
            | "ul"
            | "var"
            | "video"
            | "use"
    )
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string literal cannot fail")
}

pub(super) fn template_diagnostic(path: &std::path::Path, line: u32) -> String {
    format!(
        "{}:{line}:1: native Vue template compilation into native document operations is not implemented",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_script_setup_at_original_lines_and_reports_the_template() {
        let parsed = parse(
            "<template>\n  <div>{{ value }}</div>\n</template>\n<script setup lang=\"ts\">\nconst value: number = 7\n</script>\n",
        );
        assert_eq!(parsed.template_line, None);
        assert!(parsed.module_source.contains("const value: number = 7"));
        assert!(parsed.module_source.contains("setup(__props, { expose:"));
        assert!(parsed.module_source.contains(
            "return () => __nativeboat_h(\"div\", null, [__nativeboat_to_display_string(value)]);"
        ));
    }

    #[test]
    fn merges_regular_script_options_with_setup_rendering() {
        let parsed = parse(
            "<script lang=\"ts\">\nexport default { name: 'sample' }\n</script>\n<script setup lang=\"ts\">\nconst value = 3\n</script>\n",
        );
        assert!(
            parsed
                .module_source
                .contains("const __nativeboat_script_options = { name: 'sample' }")
        );
        assert!(
            parsed
                .module_source
                .contains("export default { ...__nativeboat_script_options, setup(")
        );
        assert!(parsed.module_source.contains("const value = 3"));
        assert_eq!(parsed.module_source.matches("export default").count(), 1);
        assert_eq!(parsed.template_line, None, "{parsed:?}");

        let mixed_template = parse(
            "<template><div>{{ value }}</div></template>\n<script setup>\nconst value = 3\n</script>\n<script>\nexport default { name: 'sample' }\n</script>",
        );
        assert_eq!(
            mixed_template.template_line, None,
            "{}",
            mixed_template.module_source
        );
        assert!(
            mixed_template
                .module_source
                .contains("...__nativeboat_script_options")
        );
        assert!(
            mixed_template
                .module_source
                .contains("__nativeboat_h(\"div\", null")
        );
    }

    #[test]
    fn empty_components_get_a_default_export() {
        let parsed = parse("<template><unknown-route-view /></template>");
        assert_eq!(parsed.module_source, "export default {};\n");
        assert_eq!(parsed.template_line, Some(1));
    }

    #[test]
    fn lowers_a_static_html_template_to_a_native_compiled_render_function() {
        let source = "<template data-note='>'>\n  <div class=\"pref-separator\" />\n</template>\n<style>.pref-separator { height: 2px }</style>";
        let parsed = parse(source);
        assert_eq!(parsed.template_line, None, "{parsed:?}");
        assert!(
            parsed
                .module_source
                .contains("import { h as __nativeboat_h } from 'vue'")
        );
        assert!(parsed.module_source.contains(
            "export default { render() { return __nativeboat_h(\"div\", {\"class\": \"pref-separator\"}); } };"
        ));
    }

    #[test]
    fn lowers_interpolations_with_nested_objects_and_closing_braces_in_strings() {
        let text = "{{ translate('label }} literal', { name: record.author }) }}";
        let lowered = static_text(text, true, &BTreeSet::new()).unwrap();
        assert_eq!(
            lowered,
            vec![
                "__nativeboat_to_display_string(translate('label }} literal', { name: record.author }))"
            ]
        );
        assert!(static_text("{{ translate('unterminated')", true, &BTreeSet::new()).is_none());
    }

    #[test]
    fn places_script_setup_imports_at_module_scope_and_render_in_the_setup_closure() {
        let parsed = parse(
            "<script setup>\nimport value from './value.js'\nconst local = value\n</script>\n<template><div>{{ local }}</div></template>",
        );
        assert_eq!(parsed.template_line, None);
        assert!(
            parsed
                .module_source
                .starts_with("import value from './value.js';\n"),
            "{}",
            parsed.module_source
        );
        assert!(parsed.module_source.contains("setup(__props, { expose:"));
        assert!(parsed.module_source.contains("const local = value"));
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_h(\"div\", null, [__nativeboat_to_display_string(local)])")
        );
        assert!(
            parsed
                .module_source
                .contains("toDisplayString as __nativeboat_to_display_string")
        );
    }

    #[test]
    fn compiles_imported_setup_components_with_dynamic_props_and_named_slots() {
        let parsed = parse(
            r#"<script setup>
import Compound from './compound.vue'
const value = 4
const label = 'Title'
</script>
<template><Compound :value="value" v-bind:heading="label"><template #head><span>{{ label }}</span></template><button>ok</button></Compound></template>"#,
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("import Compound from './compound.vue';")
        );
        assert!(parsed.module_source.contains("\"value\": (value)"));
        assert!(parsed.module_source.contains("\"heading\": (label)"));
        assert!(parsed.module_source.contains(
            "\"head\": () => [__nativeboat_h(\"span\", null, [__nativeboat_to_display_string(label)])]"
        ));
        assert!(
            parsed
                .module_source
                .contains("\"default\": () => [__nativeboat_h(\"button\", null, [\"ok\"])]")
        );

        let scoped = parse(
            r#"<script setup>
import Select from './select.vue'
</script>
<template><Select><template #default="{ item, index: row }"><span>{{ item.name }} {{ row }}</span></template></Select></template>"#,
        );
        assert_eq!(scoped.template_line, None, "{}", scoped.module_source);
        assert!(
            scoped
                .module_source
                .contains("\"default\": ({ item, index: row }) => [")
        );
        assert!(
            scoped
                .module_source
                .contains("__nativeboat_to_display_string(item.name)")
        );
        assert!(
            scoped
                .module_source
                .contains("__nativeboat_to_display_string(row)")
        );

        let script_after_template = parse(
            "<template><div><Compound><template #head><b>heading</b></template></Compound></div></template>\n<script setup>\nimport Compound from './compound.vue'\n</script>",
        );
        assert_eq!(
            script_after_template.template_line, None,
            "{}",
            script_after_template.module_source
        );
        assert!(
            script_after_template
                .module_source
                .contains("import Compound from './compound.vue';")
        );
    }

    #[test]
    fn lowers_script_setup_props_defaults_emits_and_expose_macros() {
        let parsed = parse(
            r#"<script setup lang="ts">
const props = withDefaults(defineProps<{ title?: string; count?: number }>(), { title: 'fallback', count: 2 })
const emit = defineEmits<{ (event: 'change', value: string): void }>()
defineExpose({ title: props.title })
</script>
<template><span>{{ props.title }}</span></template>"#,
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(!parsed.module_source.contains("defineProps"));
        assert!(!parsed.module_source.contains("withDefaults"));
        assert!(!parsed.module_source.contains("defineEmits"));
        assert!(!parsed.module_source.contains("defineExpose"));
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_with_defaults(__props, { title: 'fallback', count: 2 })")
        );
        assert!(
            parsed
                .module_source
                .contains("const emit = __nativeboat_emit")
        );
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_expose({ title: props.title })")
        );
        assert!(
            parsed
                .module_source
                .contains("key of Object.keys(defaults)")
        );
    }

    #[test]
    fn does_not_guess_unknown_components_or_malformed_slot_shapes() {
        for template in [
            "<UnknownComponent />",
            "<Compound><template #head #body>content</template></Compound>",
            "<Compound><template #head>one</template><template #default>two</template>three</Compound>",
        ] {
            let source = format!(
                "<script setup>\nimport Compound from './compound.vue'\n</script>\n<template>{template}</template>"
            );
            let parsed = parse(&source);
            assert!(parsed.template_line.is_some(), "{template}: {parsed:?}");
        }
    }

    #[test]
    fn compiles_v_if_else_if_and_else_as_ordered_render_branches() {
        let parsed = parse(
            "<script setup>\nconst state = 'second'\n</script>\n<template><div><span v-if=\"state === 'first'\">first</span><i v-else-if=\"state === 'second'\">second</i><b v-else>other</b></div></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(parsed.module_source.contains(
            "(state === 'first') ? (__nativeboat_h(\"span\", null, [\"first\"])) : ((state === 'second') ? (__nativeboat_h(\"i\", null, [\"second\"])) : (__nativeboat_h(\"b\", null, [\"other\"])))"
        ));

        let parsed = parse(
            "<script setup>\nconst visible = false\n</script>\n<template><div><span v-if=\"visible\">shown</span></div></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_create_comment_vnode()")
        );
        assert!(
            parsed
                .module_source
                .contains("createCommentVNode as __nativeboat_create_comment_vnode")
        );
    }

    #[test]
    fn compiles_v_for_aliases_into_fragment_list_rendering() {
        let parsed = parse(
            "<script setup>\nconst items = ['a', 'b']\n</script>\n<template><ul><li v-for=\"(item, index) in items\" :key=\"index\">{{ index }}:{{ item }}</li></ul></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(parsed.module_source.contains("__nativeboat_render_list"));
        assert!(parsed.module_source.contains("(item, index)"));
        assert!(
            parsed
                .module_source
                .contains("renderList as __nativeboat_render_list")
        );
    }

    #[test]
    fn compiles_event_handlers_and_common_event_modifiers() {
        assert_eq!(
            event_handler("click", "activate", &BTreeSet::new()),
            Some(("onClick".into(), "(activate)".into()))
        );
        assert_eq!(
            event_handler("click.stop.prevent", "activate($event)", &BTreeSet::new()),
            Some((
                "onClick".into(),
                "($event) => { $event.stopPropagation(); $event.preventDefault(); activate($event); }".into()
            ))
        );
        assert!(event_handler("click.unknown", "activate", &BTreeSet::new()).is_none());
    }

    #[test]
    fn lowers_setup_refs_global_components_and_component_model_bindings() {
        let parsed = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst checked = ref(false)\n</script>\n<template><el-switch v-model=\"checked\" /></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("resolveComponent as __nativeboat_resolve_component")
        );
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_resolve_component(\"el-switch\")")
        );
        assert!(
            parsed
                .module_source
                .contains("\"modelValue\": (checked.value)")
        );
        assert!(
            parsed
                .module_source
                .contains("\"onUpdate:modelValue\": ($event) => (checked.value = $event)")
        );

        let parsed = parse("<template><router-view /></template>");
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_resolve_component(\"router-view\")")
        );
        let refs = BTreeSet::from(["ready".to_owned(), "item".to_owned()]);
        assert_eq!(
            template_expression("!ready && items.some((item) => item > 0)", &refs),
            "!ready.value && items.some((item) => item > 0)"
        );
    }

    #[test]
    fn lowers_direct_typed_props_and_recursive_sfc_names() {
        let source = "<script setup lang=\"ts\">\ndefineProps<{ size?: number; label: string }>()\n</script>\n<template><span :title=\"label\">{{ size }}</span></template>";
        let scripts = script_blocks(source);
        assert_eq!(
            setup_template_props(&scripts),
            BTreeSet::from(["label".into(), "size".into()])
        );
        let parsed = parse(source);
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed.module_source.contains("__props.label"),
            "{}",
            parsed.module_source
        );
        assert!(
            parsed.module_source.contains("__props.size"),
            "{}",
            parsed.module_source
        );

        let recursive = parse_with_component_name(
            "<script setup>const props = defineProps<{ children: unknown[] }>()</script><template><tree-folder v-for=\"child of props.children\" /></template>",
            Some("treeFolder"),
        );
        assert_eq!(recursive.template_line, None, "{}", recursive.module_source);
        assert!(
            recursive
                .module_source
                .contains("__nativeboat_resolve_component(\"TreeFolder\")")
        );
    }

    #[test]
    fn template_props_come_from_every_define_props_form() {
        let props = |script: &str| {
            setup_template_props(&script_blocks(&format!(
                "<script setup lang=\"ts\">
{script}
</script>"
            )))
        };
        assert_eq!(
            props("const props = defineProps<{ active?: boolean; 'file-name': string }>()"),
            BTreeSet::from(["active".into(), "file-name".into()])
        );
        assert_eq!(
            props(
                "interface Props { title: string; count?: number; [key: string]: unknown }
const { title } = withDefaults(defineProps<Props>(), { count: 1 })"
            ),
            BTreeSet::from(["count".into(), "title".into()])
        );
        assert_eq!(
            props(
                "type Props = { open: boolean }
const props = defineProps<Props>()"
            ),
            BTreeSet::from(["open".into()])
        );
        assert_eq!(
            props(
                "const props = defineProps({ visible: Boolean, 'label': String, ...shared, [computed]: Number })"
            ),
            BTreeSet::from(["label".into(), "visible".into()])
        );
        assert_eq!(
            props(
                "defineProps(['first', second, 'third'])
const unused = 1
let pending"
            ),
            BTreeSet::from(["first".into(), "third".into()])
        );
        assert!(props("const props = defineProps<Unknown>()").is_empty());
        let parsed = parse(
            "<script setup lang=\"ts\">
const props = defineProps<{ active?: boolean }>()
</script>
<template><div :class=\"[{ active: active }]\" /></template>",
        );
        assert!(
            parsed.module_source.contains("__props.active"),
            "{}",
            parsed.module_source
        );
    }

    #[test]
    fn lowers_native_checkbox_models_and_rejects_invalid_model_targets() {
        let parsed = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst checked = ref(false)\n</script>\n<template><input type=\"checkbox\" v-model=\"checked\" /></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("\"checked\": (checked.value)")
        );
        assert!(
            parsed
                .module_source
                .contains("\"onChange\": ($event) => (checked.value = $event.target.checked)")
        );

        let parsed = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst tableChecker = ref({ rows: 1 })\n</script>\n<template><el-input v-model=\"tableChecker.rows\" /></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(parsed.module_source.contains("(tableChecker.value.rows)"));
        assert!(
            parsed
                .module_source
                .contains("(tableChecker.value.rows = $event)")
        );

        let parsed = parse(
            "<script setup>\nconst value = 3\n</script>\n<template><input v-model=\"value()\" /></template>",
        );
        assert_eq!(parsed.template_line, Some(4), "{}", parsed.module_source);
    }

    #[test]
    fn lowers_v_show_template_refs_dynamic_components_and_object_spreads() {
        let parsed = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst visible = ref(false)\nconst control = ref('input')\nconst input = ref(null)\nconst attrs = { title: 'name' }\n</script>\n<template><component :is=\"control\" v-bind=\"attrs\" v-show=\"visible\" ref=\"input\" /></template>",
        );
        assert_eq!(parsed.template_line, None, "{}", parsed.module_source);
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_h(control.value, {...(attrs)")
        );
        assert!(
            parsed
                .module_source
                .contains("\"ref\": ($el) => (input.value = $el)")
        );
        assert!(
            parsed
                .module_source
                .contains("__nativeboat_v_show, (visible.value)")
        );
        assert!(
            parsed
                .module_source
                .contains("withDirectives as __nativeboat_with_directives")
        );

        let conditional = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst visible = ref(true)\nconst selected = ref(true)\n</script>\n<template><main><div v-if=\"selected\" v-show=\"visible\"></div><span v-else v-show=\"visible\"></span></main></template>",
        );
        assert_eq!(
            conditional.template_line, None,
            "{}",
            conditional.module_source
        );
        assert_eq!(
            conditional
                .module_source
                .matches("__nativeboat_with_directives")
                .count(),
            3,
            "{}",
            conditional.module_source
        );

        let html = parse(
            "<script setup>\nimport { ref } from 'vue'\nconst markup = ref('<b>theme</b>')\n</script>\n<template><div v-html=\"markup\" /></template>",
        );
        assert_eq!(html.template_line, None, "{}", html.module_source);
        assert!(html.module_source.contains("\"innerHTML\": (markup.value)"));
        assert!(
            html.module_source
                .contains("__nativeboat_h(\"div\", {\"innerHTML\": (markup.value)})")
        );

        let fragment = parse(
            "<script setup>\nconst selected = true\n</script>\n<template><main><template v-if=\"selected\"><span>a</span><b>b</b></template></main></template>",
        );
        assert_eq!(fragment.template_line, None, "{}", fragment.module_source);
        assert!(
            fragment
                .module_source
                .contains("__nativeboat_h(__nativeboat_Fragment, null")
        );
        assert!(
            fragment
                .module_source
                .contains("Fragment as __nativeboat_Fragment")
        );
    }

    #[test]
    fn keeps_dynamic_and_component_templates_as_source_located_blockers() {
        for template in [
            "<div>{{ value }}</div>",
            "<div ref=\"container\" />",
            "<div v-if=\"visible\" />",
            "<div :title=\"title\" />",
            "<div @click=\"activate\" />",
            "<div #default />",
            "<div onclick=\"activate()\" />",
            "<div></div><span></span>",
            "<SearchResultItem />",
        ] {
            let parsed = parse(&format!("<template>{template}</template>"));
            assert_eq!(parsed.template_line, Some(1), "{template}");
            assert!(
                !parsed.module_source.contains("__nativeboat_h"),
                "{template}"
            );
        }
    }

    #[test]
    fn compiles_static_svg_as_a_vue_component_and_rejects_non_svg_markup() {
        let generated =
            svg_module_source("<svg viewBox='0 0 20 20'><path d='M0 0' /></svg>").unwrap();
        assert!(generated.contains(
            "export default { render() { return __nativeboat_h(\"svg\", {\"viewBox\": \"0 0 20 20\"}, [__nativeboat_h(\"path\", {\"d\": \"M0 0\"})]); } };"
        ));
        assert!(svg_module_source("<svg><script>alert(1)</script></svg>").is_none());
    }

    #[test]
    fn maps_template_errors_to_the_component_path_and_line() {
        let path = std::path::Path::new("src/view.vue");
        assert_eq!(
            template_diagnostic(path, 4),
            "src/view.vue:4:1: native Vue template compilation into native document operations is not implemented"
        );
    }
}
