//! Destructuring: binding patterns (declarations, parameters, `catch`,
//! loop heads) and assignment patterns. Every bound name is declared before
//! the pattern runs, so array patterns can evaluate inside a labeled region
//! that closes their iterator when a nested step throws.

use super::{Emitter, Exit};
use crate::dynamic::emit::rust_str;
use oxc::ast::ast::*;
use std::fmt::Write;

/// How a pattern's names are bound.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// New bindings created here: parameters, `catch` and loop heads.
    Fresh,
    /// `let`/`const`, whose shared cells already exist in their scope.
    Lexical,
    /// `var`, declared at the top of the function.
    Var,
}

impl Emitter<'_> {
    /// A fresh Rust variable name.
    pub(super) fn temporary(&mut self, prefix: &str) -> String {
        format!("__{prefix}{}", self.id())
    }

    /// Declares every name a pattern binds before the pattern runs.
    pub(super) fn predeclare(&mut self, pattern: &BindingPattern<'_>, mode: Mode) -> String {
        let mut code = String::new();
        for binding in pattern.get_binding_identifiers() {
            let symbol = binding.symbol_id();
            let index = symbol.index();
            if self.captured(symbol) {
                if mode == Mode::Fresh {
                    let _ = writeln!(code, "let c{index} = binding::new(None);");
                }
            } else if mode != Mode::Var {
                let _ = writeln!(code, "let mut b{index} = Value::Undefined;");
                self.frame().declared.insert(symbol);
            }
        }
        code
    }

    /// Stores the Rust variable `value` into a declared name.
    fn store(&mut self, binding: &BindingIdentifier<'_>, value: &str, mode: Mode) -> String {
        self.store_value(binding, &format!("{value}.clone()"), mode)
    }

    /// Stores the value of a Rust expression in a binding.
    pub(super) fn store_value(
        &mut self,
        binding: &BindingIdentifier<'_>,
        value: &str,
        mode: Mode,
    ) -> String {
        let index = binding.symbol_id().index();
        if !self.captured(binding.symbol_id()) {
            return format!("b{index} = {value};\n");
        }
        if mode == Mode::Var {
            let write = self.fail(&format!(
                "binding::write(realm, &c{index}, {}, {value})",
                rust_str(&binding.name)
            ));
            return format!("{write};\n");
        }
        format!("binding::initialize(&c{index}, {value});\n")
    }

    /// `value` with a default for `undefined`, into a new variable.
    fn defaulted(&mut self, value: &str, default: &Expression<'_>) -> (String, String) {
        let fallback = self.expression(default);
        let name = self.temporary("d");
        let code = format!(
            "let {name} = if matches!({value}, Value::Undefined) {{ {fallback} }} else {{ {value}.clone() }};\n"
        );
        (code, name)
    }

    /// Binds the Rust variable `value` to a pattern.
    pub(super) fn bind(&mut self, pattern: &BindingPattern<'_>, value: &str, mode: Mode) -> String {
        match pattern {
            BindingPattern::BindingIdentifier(binding) => self.store(binding, value, mode),
            BindingPattern::AssignmentPattern(pattern) => {
                let (mut code, name) = self.defaulted(value, &pattern.right);
                code.push_str(&self.bind(&pattern.left, &name, mode));
                code
            }
            BindingPattern::ObjectPattern(pattern) => {
                let mut code = self.coercible(value);
                let mut keys = Vec::new();
                for property in &pattern.properties {
                    let key = self.temporary("k");
                    let key_code = self.property_key(&property.key);
                    let element = self.temporary("v");
                    let get = self.get_key(&format!("&{value}"), &format!("&{key}"));
                    let _ = writeln!(code, "let {key} = {key_code};\nlet {element} = {get};");
                    code.push_str(&self.bind(&property.value, &element, mode));
                    keys.push(key);
                }
                if let Some(rest) = &pattern.rest {
                    let (rest_code, name) = self.object_rest(value, &keys);
                    code.push_str(&rest_code);
                    code.push_str(&self.bind(&rest.argument, &name, mode));
                }
                code
            }
            BindingPattern::ArrayPattern(pattern) => {
                let mut region = |emitter: &mut Self, iterator: &str| {
                    let mut code = String::new();
                    for element in &pattern.elements {
                        let step = emitter.fail(&format!("{iterator}.step(realm)"));
                        match element {
                            None => {
                                let skip = emitter.fail(&format!("{iterator}.skip(realm)"));
                                let _ = writeln!(code, "{skip};");
                            }
                            Some(element) => {
                                let name = emitter.temporary("e");
                                let _ = writeln!(code, "let {name} = {step}.unwrap_or_default();");
                                code.push_str(&emitter.bind(element, &name, mode));
                            }
                        }
                    }
                    if let Some(rest) = &pattern.rest {
                        let (rest_code, name) = emitter.array_rest(iterator);
                        code.push_str(&rest_code);
                        code.push_str(&emitter.bind(&rest.argument, &name, mode));
                    }
                    code
                };
                self.iterate_region(value, &mut region)
            }
        }
    }

    /// Throws the language's TypeError when destructuring null or undefined.
    fn coercible(&mut self, value: &str) -> String {
        let error = self.error_value("Type", "Cannot destructure a null or undefined value");
        format!("if {value}.is_nullish() {{ let _ = {error}; }}\n")
    }

    /// An object rest element: the remaining own enumerable properties.
    fn object_rest(&mut self, value: &str, keys: &[String]) -> (String, String) {
        let target = self.temporary("r");
        let excluded = keys
            .iter()
            .map(|key| format!("{key}.clone()"))
            .collect::<Vec<_>>()
            .join(", ");
        let copy = self.fail(&format!(
            "::nativeboat_native::iter::copy_properties(realm, &{target}, &{value}, &[{excluded}])"
        ));
        let name = self.temporary("v");
        let code = format!(
            "let {target} = realm.object();\n{copy};\nlet {name} = Value::Object({target});\n"
        );
        (code, name)
    }

    /// An array rest element: the iterator's remaining values.
    fn array_rest(&mut self, iterator: &str) -> (String, String) {
        let rest = self.fail(&format!(
            "::nativeboat_native::iter::rest(realm, &mut {iterator})"
        ));
        let name = self.temporary("v");
        let code = format!(
            "let {name} = {{ let __rest = {rest}; Value::Object(realm.array(__rest)) }};\n"
        );
        (code, name)
    }

    /// Iterates `value` for an array pattern: `body` runs in a region whose
    /// throws close the iterator first; a normal finish closes it if values
    /// remain.
    fn iterate_region(
        &mut self,
        value: &str,
        body: &mut dyn FnMut(&mut Self, &str) -> String,
    ) -> String {
        let iterator = self.temporary("i");
        let slot = self.temporary("t");
        let label = format!("'{}", self.temporary("a"));
        let start = self.fail(&format!(
            "::nativeboat_native::iter::iterate(realm, &{value})"
        ));
        self.frame().exits.push(Exit::Attempt {
            slot: slot.clone(),
            label: label.clone(),
        });
        let inner = body(self, &iterator);
        self.frame().exits.pop();
        let rethrow = self.throw("__closed");
        let close = self.fail(&format!("{iterator}.close(realm)"));
        format!(
            "let mut {iterator} = {start};\nlet mut {slot}: Option<Thrown> = None;\n{label}: {{\n{inner}}}\n\
             if let Some(__thrown) = {slot} {{ let __closed = {iterator}.close_thrown(realm, __thrown); {rethrow}}}\n{close};\n"
        )
    }

    /// Assigns the Rust variable `value` to a destructuring assignment target.
    pub(super) fn assign_target(&mut self, target: &AssignmentTarget<'_>, value: &str) -> String {
        match target {
            AssignmentTarget::ArrayAssignmentTarget(pattern) => {
                let mut region = |emitter: &mut Self, iterator: &str| {
                    let mut code = String::new();
                    for element in &pattern.elements {
                        let step = emitter.fail(&format!("{iterator}.step(realm)"));
                        match element {
                            None => {
                                let skip = emitter.fail(&format!("{iterator}.skip(realm)"));
                                let _ = writeln!(code, "{skip};");
                            }
                            Some(element) => {
                                let read = format!("{step}.unwrap_or_default()");
                                code.push_str(&emitter.assign_element(element, &read));
                            }
                        }
                    }
                    if let Some(rest) = &pattern.rest {
                        let rest_value = emitter.fail(&format!(
                            "::nativeboat_native::iter::rest(realm, &mut {iterator})"
                        ));
                        let read = format!(
                            "{{ let __rest = {rest_value}; Value::Object(realm.array(__rest)) }}"
                        );
                        code.push_str(&emitter.assign_read(&rest.target, &read, None));
                    }
                    code
                };
                self.iterate_region(value, &mut region)
            }
            AssignmentTarget::ObjectAssignmentTarget(pattern) => {
                let mut code = self.coercible(value);
                let mut keys = Vec::new();
                for property in &pattern.properties {
                    let key = self.temporary("k");
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                            let name = self.name_key(&shorthand.binding.name);
                            let _ = writeln!(code, "let {key} = {name};");
                            let get = self.get_key(&format!("&{value}"), &format!("&{key}"));
                            let (setup, _, write) = self.identifier_target(&shorthand.binding);
                            code.push_str(&setup);
                            let mut element = self.temporary("v");
                            let _ = writeln!(code, "let {element} = {get};");
                            if let Some(init) = &shorthand.init {
                                let (default_code, name) = self.defaulted(&element, init);
                                code.push_str(&default_code);
                                element = name;
                            }
                            let _ = writeln!(code, "{{ let __value = {element}.clone(); {write}}}");
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                            let key_code = self.property_key(&property.name);
                            let _ = writeln!(code, "let {key} = {key_code};");
                            let read = self.get_key(&format!("&{value}"), &format!("&{key}"));
                            code.push_str(&self.assign_element(&property.binding, &read));
                        }
                    }
                    keys.push(key);
                }
                if let Some(rest) = &pattern.rest {
                    let (rest_code, name) = self.object_rest(value, &keys);
                    code.push_str(&rest_code);
                    code.push_str(&self.assign_read(&rest.target, &name, None));
                }
                code
            }
            simple => self.assign_read(simple, value, None),
        }
    }

    /// An element of an assignment pattern, with its optional default.
    fn assign_element(&mut self, element: &AssignmentTargetMaybeDefault<'_>, read: &str) -> String {
        match element {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(element) => {
                self.assign_read(&element.binding, read, Some(&element.init))
            }
            target => self.assign_read(target.to_assignment_target(), read, None),
        }
    }

    /// Assigns the value `read` produces. A simple target's reference (its
    /// object and key) is evaluated before `read`, as the language specifies;
    /// nested patterns destructure the value.
    fn assign_read(
        &mut self,
        target: &AssignmentTarget<'_>,
        read: &str,
        init: Option<&Expression<'_>>,
    ) -> String {
        let reference = target
            .as_simple_assignment_target()
            .and_then(|simple| self.target(simple));
        let (setup, write) = match reference {
            Some((setup, _, write)) => (setup, Some(write)),
            None => (String::new(), None),
        };
        let mut name = self.temporary("v");
        let mut code = format!("{setup}let {name} = {read};\n");
        if let Some(init) = init {
            let (default_code, defaulted) = self.defaulted(&name, init);
            code.push_str(&default_code);
            name = defaulted;
        }
        match write {
            Some(write) => {
                let _ = writeln!(code, "{{ let __value = {name}.clone(); {write}}}");
            }
            // Nested patterns; unsupported simple targets were reported.
            None if target.as_assignment_target_pattern().is_some() => {
                code.push_str(&self.assign_target(target, &name));
            }
            None => {}
        }
        code
    }
}
