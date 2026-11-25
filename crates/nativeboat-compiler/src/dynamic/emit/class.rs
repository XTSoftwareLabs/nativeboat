//! Class definitions: constructors, prototype and static methods and
//! accessors, instance and static fields, static blocks, private elements,
//! `super` and `new.target`. Class semantics live in
//! `nativeboat_native::class`; this module orders the evaluation of a class
//! body as `ClassDefinitionEvaluation` does.

use super::{Body, Callable, Emitter, frame, rust_str};
use oxc::ast::ast::*;
use oxc::semantic::{ScopeId, SymbolId};
use oxc::span::{GetSpan, Span};
use std::fmt::Write;

const CLASS: &str = "::nativeboat_native::class";

/// Where a field's key comes from.
enum FieldKey {
    /// An identifier name.
    Name(String),
    /// A computed or literal key evaluated at definition into a variable.
    Computed(String),
    /// A private name's key variable.
    Private(String),
}

/// A static element, run in order with the class as `this`.
enum Static<'r, 'a> {
    Field(&'r PropertyDefinition<'a>, FieldKey),
    Block(&'r StaticBlock<'a>),
}

/// The functions of a private method or accessor pair.
struct Private {
    key: String,
    r#static: bool,
    method: Option<String>,
    get: Option<String>,
    set: Option<String>,
}

impl Private {
    fn property(&self) -> String {
        let reference = |function: &Option<String>| {
            function
                .as_ref()
                .map_or("None".to_owned(), |function| format!("Some(&{function})"))
        };
        match &self.method {
            Some(method) => format!("{CLASS}::private_method(&{method})"),
            None => format!(
                "{CLASS}::private_accessor({}, {})",
                reference(&self.get),
                reference(&self.set)
            ),
        }
    }
}

/// The `name` of a method whose key is known before run time.
fn method_name(key: &PropertyKey<'_>) -> String {
    match key {
        PropertyKey::StaticIdentifier(identifier) => identifier.name.to_string(),
        PropertyKey::PrivateIdentifier(private) => format!("#{}", private.name),
        PropertyKey::StringLiteral(literal) => literal.value.to_string(),
        _ => String::new(),
    }
}

/// TypeScript-only fields have no runtime behavior.
fn type_only(property: &PropertyDefinition<'_>) -> bool {
    property.declare || property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
}

/// The name of a member written with an identifier or string key, which
/// legacy decorators receive.
fn member_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

impl Emitter<'_> {
    /// A class declaration statement, which binds like `let`.
    pub(super) fn class_declaration(&mut self, class: &Class<'_>) -> String {
        if class.declare {
            return String::new();
        }
        let binding = class.id.as_ref().expect("class declarations are named");
        let value = self.class(class, &binding.name);
        let symbol = binding.symbol_id();
        if self.captured(symbol) {
            // The class initialized its cell before its static elements ran.
            format!("let _ = {value};\n")
        } else {
            self.frame().declared.insert(symbol);
            format!("let mut b{} = {value};\n", symbol.index())
        }
    }

    /// A class definition evaluating to its constructor.
    pub(super) fn class(&mut self, class: &Class<'_>, name: &str) -> String {
        let id = self.id();
        let scope = class.scope_id();
        let mut code = String::from("{\n");
        code.push_str(&self.scope_bindings(scope, &[]));
        // The heritage is evaluated outside the class's private names.
        let parents = match &class.heritage {
            Some(heritage) => {
                let value = self.expression(&heritage.expression);
                let parents = self.fail(&format!("{CLASS}::heritage(realm, Some(&__heritage))"));
                format!("{{ let __heritage = {value}; {parents} }}")
            }
            None => self.fail(&format!("{CLASS}::heritage(realm, None)")),
        };
        let _ = writeln!(code, "let parents{id} = {parents};");
        let _ = writeln!(
            code,
            "let proto{id} = Object::new(parents{id}.0, ::nativeboat_native::Kind::Ordinary);"
        );
        let mut names: Vec<(String, String)> = Vec::new();
        for element in &class.body.body {
            let key = match element {
                ClassElement::MethodDefinition(method) => &method.key,
                ClassElement::PropertyDefinition(property) => &property.key,
                _ => continue,
            };
            if let PropertyKey::PrivateIdentifier(private) = key
                && !names.iter().any(|(name, _)| name == private.name.as_str())
            {
                let key = format!("private{id}_{}", names.len());
                let description = rust_str(&format!("#{}", private.name));
                let value = if matches!(element, ClassElement::MethodDefinition(_)) {
                    format!(
                        "::nativeboat_native::Symbol::class_member({}, {description})",
                        rust_str(name)
                    )
                } else {
                    format!(
                        "::nativeboat_native::Symbol::new(Some(::nativeboat_native::JsStr::from({description})))"
                    )
                };
                let _ = writeln!(code, "let {key} = {value};");
                names.push((private.name.to_string(), key));
            }
        }
        self.classes.push(names);
        let instance = class.body.body.iter().any(|element| {
            matches!(element, ClassElement::PropertyDefinition(property)
                if !property.r#static && !type_only(property))
                || matches!(element, ClassElement::MethodDefinition(method)
                    if !method.r#static
                        && matches!(method.key, PropertyKey::PrivateIdentifier(_))
                        && method.value.body.is_some())
        });
        if instance {
            let _ = writeln!(
                code,
                "let fields{id} = ::alloc::rc::Rc::new(::core::cell::OnceCell::<Object>::new());"
            );
        }
        let constructor = class.body.body.iter().find_map(|element| match element {
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor
                    && method.value.body.is_some() =>
            {
                Some(&*method.value)
            }
            _ => None,
        });
        let (construct, length) =
            self.constructor(id, constructor, class.heritage.is_some(), instance);
        let _ = writeln!(
            code,
            "let class{id} = {CLASS}::create(realm, {}, {length}, &proto{id}, parents{id}.1, {construct});",
            rust_str(name)
        );
        let mut locals = Vec::new();
        let mut privates: Vec<Private> = Vec::new();
        let mut fields = Vec::new();
        let mut statics = Vec::new();
        let mut constructor_parameters: Vec<(u32, &Decorator<'_>)> = Vec::new();
        // Members decorated as TypeScript's `experimentalDecorators` has it:
        // whether static, the key, whether a method or accessor, and the
        // member's decorators with its parameters'.
        let mut decorations = Vec::new();
        let legacy = self.legacy_decorators();
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => {
                    let parameters: Vec<(u32, &Decorator<'_>)> = method
                        .value
                        .params
                        .items
                        .iter()
                        .enumerate()
                        .flat_map(|(index, parameter)| {
                            parameter
                                .decorators
                                .iter()
                                .map(move |decorator| (index as u32, decorator))
                        })
                        .collect();
                    let decorated = !method.decorators.is_empty() || !parameters.is_empty();
                    if legacy && decorated && method.value.body.is_some() {
                        if method.kind == MethodDefinitionKind::Constructor {
                            constructor_parameters = parameters;
                        } else if let Some(name) = member_name(&method.key) {
                            let key = self.name_key(&name);
                            let member: Vec<&Decorator<'_>> = method.decorators.iter().collect();
                            decorations.push((method.r#static, key, true, member, parameters));
                        } else {
                            self.unsupported(
                                method.key.span(),
                                "decorators of computed or private members are not compiled yet",
                            );
                        }
                    }
                    code.push_str(&self.class_method(id, method, &mut locals, &mut privates));
                }
                ClassElement::PropertyDefinition(property) if !type_only(property) => {
                    if !property.decorators.is_empty() {
                        if !legacy {
                            self.unsupported(
                                property.decorators[0].span,
                                "decorators are not compiled yet",
                            );
                        } else if let Some(name) = member_name(&property.key) {
                            let key = self.name_key(&name);
                            let member: Vec<&Decorator<'_>> = property.decorators.iter().collect();
                            decorations.push((property.r#static, key, false, member, Vec::new()));
                        } else {
                            self.unsupported(
                                property.key.span(),
                                "decorators of computed or private members are not compiled yet",
                            );
                        }
                    }
                    // With `useDefineForClassFields: false`, a field without
                    // an initializer only declares its type.
                    if self.declares_only(property) {
                        continue;
                    }
                    let key = self.field_key(id, &property.key, &mut code, &mut locals);
                    if property.r#static {
                        statics.push(Static::Field(property, key));
                    } else {
                        fields.push((property, key));
                    }
                }
                ClassElement::StaticBlock(block) => statics.push(Static::Block(block)),
                ClassElement::AccessorProperty(accessor) => {
                    self.unsupported(accessor.span, "auto-accessors are not compiled yet");
                }
                ClassElement::PropertyDefinition(_) | ClassElement::TSIndexSignature(_) => {}
            }
        }
        // The class binding is initialized before static elements run.
        if let Some(binding) = &class.id {
            let symbol = binding.symbol_id();
            if self.captured(symbol) {
                let _ = writeln!(
                    code,
                    "binding::initialize(&c{}, Value::Object(class{id}.clone()));",
                    symbol.index()
                );
            } else if !class.is_declaration() {
                let _ = writeln!(
                    code,
                    "let mut b{} = Value::Object(class{id}.clone());",
                    symbol.index()
                );
                self.frame().declared.insert(symbol);
            }
        }
        if instance {
            let initializer = self.initializer(scope, &format!("proto{id}"), &locals, |emitter| {
                let mut body = String::new();
                // Private methods are installed before any field.
                for private in privates.iter().filter(|private| !private.r#static) {
                    let define = emitter.fail(&format!(
                        "{CLASS}::private_define(realm, &this{}, &{}, {})",
                        scope.index(),
                        private.key,
                        private.property()
                    ));
                    let _ = writeln!(body, "let _ = {define};");
                }
                for (property, key) in &fields {
                    body.push_str(&emitter.field(scope, property, key));
                }
                body
            });
            let _ = writeln!(code, "let _ = fields{id}.set({initializer});");
        }
        for private in privates.iter().filter(|private| private.r#static) {
            let define = self.fail(&format!(
                "{CLASS}::private_define(realm, &Value::Object(class{id}.clone()), &{}, {})",
                private.key,
                private.property()
            ));
            let _ = writeln!(code, "let _ = {define};");
        }
        for element in &statics {
            let initializer = match element {
                Static::Field(property, key) => {
                    self.initializer(scope, &format!("class{id}"), &locals, |emitter| {
                        emitter.field(scope, property, key)
                    })
                }
                Static::Block(block) => {
                    let scope = block.scope_id();
                    self.initializer(scope, &format!("class{id}"), &locals, |emitter| {
                        emitter.block(scope, &block.body)
                    })
                }
            };
            let call = self.fail(&format!(
                "ops::call(realm, &Value::Object(__initializer), Value::Object(class{id}.clone()), &[])"
            ));
            let _ = writeln!(
                code,
                "{{ let __initializer = {initializer}; let _ = {call}; }}"
            );
        }
        // `tsc` decorates instance members, then static members, in the
        // order they are written, then the constructor's parameters, once
        // the class is defined.
        decorations.sort_by_key(|(r#static, ..)| *r#static);
        for (r#static, key, method, member, parameters) in decorations {
            let mut decorators = Vec::new();
            for decorator in member {
                let value = self.expression(&decorator.expression);
                decorators.push(format!("{CLASS}::MemberDecorator::Member({value})"));
            }
            for (index, decorator) in parameters {
                let value = self.expression(&decorator.expression);
                decorators.push(format!(
                    "{CLASS}::MemberDecorator::Parameter({index}, {value})"
                ));
            }
            let target = if r#static {
                format!("Value::Object(class{id}.clone())")
            } else {
                format!("Value::Object(proto{id}.clone())")
            };
            let decorate = self.fail(&format!(
                "{CLASS}::decorate_member(realm, &{target}, {key}, &__decorators, {method})"
            ));
            let _ = writeln!(
                code,
                "{{ let __decorators = ::alloc::vec![{}]; {decorate}; }}",
                decorators.join(", ")
            );
        }
        if !constructor_parameters.is_empty() {
            let mut decorators = Vec::new();
            for (index, decorator) in constructor_parameters {
                let value = self.expression(&decorator.expression);
                decorators.push(format!("({index}, {value})"));
            }
            let decorate = self.fail(&format!(
                "{CLASS}::decorate_parameters(realm, &Value::Object(class{id}.clone()), &__decorators)"
            ));
            let _ = writeln!(
                code,
                "{{ let __decorators = ::alloc::vec![{}]; {decorate}; }}",
                decorators.join(", ")
            );
        }
        self.classes.pop();
        if class.decorators.is_empty() {
            let _ = write!(code, "Value::Object(class{id})\n}}");
        } else {
            let _ = writeln!(
                code,
                "let mut __decorated_class = Value::Object(class{id});"
            );
            let decorators = class
                .decorators
                .iter()
                .enumerate()
                .map(|(index, decorator)| {
                    let value = self.expression(&decorator.expression);
                    let _ = writeln!(code, "let __decorator{index} = {value};");
                    index
                })
                .collect::<Vec<_>>();
            for index in decorators.into_iter().rev() {
                let apply = self.fail(&format!(
                    "ops::call(realm, &__decorator{index}, Value::Undefined, &[__decorated_class.clone()])"
                ));
                let _ = writeln!(
                    code,
                    "let __decorated_result = {apply}; if !matches!(__decorated_result, Value::Undefined) {{ __decorated_class = __decorated_result; }}"
                );
            }
            if class.is_declaration()
                && let Some(binding) = &class.id
                && self.captured(binding.symbol_id())
            {
                let index = binding.symbol_id().index();
                let _ = writeln!(
                    code,
                    "binding::write(realm, &c{index}, {}, __decorated_class.clone())?;",
                    rust_str(&binding.name)
                );
            }
            let _ = write!(code, "__decorated_class\n}}");
        }
        code
    }

    /// The construction closure for `class::create`, and the class's length.
    fn constructor(
        &mut self,
        id: usize,
        function: Option<&Function<'_>>,
        derived: bool,
        instance: bool,
    ) -> (String, u32) {
        let (fields, owned, capture) = if instance {
            (
                format!("fields{id}.get()"),
                format!("fields{id}.get().cloned()"),
                format!("let fields{id} = fields{id}.clone();\n"),
            )
        } else {
            ("None".to_owned(), "None".to_owned(), String::new())
        };
        let signature = "move |realm: &mut Realm, __function: &Object, __new_target: Object, args: &[Value]| -> Result<Object, Thrown>";
        let Some(function) = function else {
            let construct = if derived {
                format!(
                    "{CLASS}::default_derived(realm, &{CLASS}::frame(__function.clone(), __new_target, {owned}), args)"
                )
            } else {
                format!("{CLASS}::default_base(realm, &__new_target, {fields})")
            };
            return (format!("{{\n{capture}{signature} {{ {construct} }}\n}}"), 0);
        };
        let scope = function.scope_id();
        let index = scope.index();
        let mut parameter_properties = Vec::new();
        for parameter in &function.params.items {
            if parameter.accessibility.is_some() || parameter.readonly || parameter.r#override {
                if let BindingPattern::BindingIdentifier(binding) = &parameter.pattern {
                    parameter_properties.push((binding.name.to_string(), binding.symbol_id()));
                } else {
                    self.unsupported(
                        parameter.span,
                        "TypeScript parameter properties require a simple identifier",
                    );
                }
            }
        }
        // With `useDefineForClassFields: false`, `tsc` assigns parameter
        // properties first and then runs field initializers, which may read
        // them.
        let deferred = instance && self.assigns_fields() && !parameter_properties.is_empty();
        if deferred {
            self.deferred_fields.insert(scope, fields.clone());
        }
        self.constructor_parameter_properties
            .insert(scope, parameter_properties);
        self.homes.insert(scope);
        self.constructors.insert(scope, derived);
        let mut captures = self.captures(scope);
        let _ = writeln!(captures, "let home{index} = proto{id}.clone();");
        captures.push_str(&capture);
        let setup = match (derived, deferred) {
            (true, false) => format!(
                "let this{index} = {CLASS}::frame(__function.clone(), __new_target.clone(), {owned});"
            ),
            (true, true) => format!(
                "let this{index} = {CLASS}::frame_deferred(__function.clone(), __new_target.clone(), {owned});"
            ),
            (false, false) => format!(
                "let this{index} = Value::Object({CLASS}::create_this(realm, &__new_target)?); {CLASS}::initialize(realm, {fields}, &this{index})?;"
            ),
            (false, true) => format!(
                "let this{index} = Value::Object({CLASS}::create_this(realm, &__new_target)?);"
            ),
        };
        let body = function.body.as_ref().expect("constructors have bodies");
        let (head, code, result, length) = self.function_body(
            scope,
            &function.params,
            Body::Block(&body.statements),
            "",
            false,
        );
        let code = head + &code;
        let finish = if derived {
            format!("{CLASS}::finish(realm, &this{index}, __returned)")
        } else {
            format!("Ok({CLASS}::finish_base(&this{index}, __returned))")
        };
        (
            format!(
                "{{\n{captures}{signature} {{\nlet target{index} = Value::Object(__new_target.clone());\n{setup}\nlet __returned = (|realm: &mut Realm| -> ::nativeboat_native::Completion {{\n{code}{result}\n}})(realm)?;\n{finish}\n}}\n}}"
            ),
            length,
        )
    }

    /// Defines a method or accessor on the prototype or the class, or records
    /// a private one.
    fn class_method(
        &mut self,
        id: usize,
        method: &MethodDefinition<'_>,
        locals: &mut Vec<String>,
        privates: &mut Vec<Private>,
    ) -> String {
        let function = &method.value;
        if method.kind == MethodDefinitionKind::Constructor || function.body.is_none() {
            return String::new();
        }
        if !self.legacy_decorators()
            && let Some(decorator) = method.decorators.first()
        {
            self.unsupported(decorator.span, "decorators are not compiled yet");
        }
        let home = if method.r#static {
            format!("class{id}")
        } else {
            format!("proto{id}")
        };
        let prefix = match method.kind {
            MethodDefinitionKind::Get => "get ",
            MethodDefinitionKind::Set => "set ",
            _ => "",
        };
        let name = format!("{prefix}{}", method_name(&method.key));
        if let PropertyKey::PrivateIdentifier(private) = &method.key {
            let key = self.private_name(&private.name);
            let value = self.class_function(function, &name, &home);
            let local = format!("method{id}_{}", locals.len());
            locals.push(local.clone());
            let position = match privates.iter().position(|entry| entry.key == key) {
                Some(position) => position,
                None => {
                    privates.push(Private {
                        key,
                        r#static: method.r#static,
                        method: None,
                        get: None,
                        set: None,
                    });
                    privates.len() - 1
                }
            };
            let entry = &mut privates[position];
            match method.kind {
                MethodDefinitionKind::Get => entry.get = Some(local.clone()),
                MethodDefinitionKind::Set => entry.set = Some(local.clone()),
                _ => entry.method = Some(local.clone()),
            }
            return format!("let {local} = {value};\n");
        }
        let key = self.property_key(&method.key);
        let value = self.class_function(function, &name, &home);
        let define = match method.kind {
            MethodDefinitionKind::Get => {
                format!("{CLASS}::define_accessor(&{home}, __key, Some(__value), None);")
            }
            MethodDefinitionKind::Set => {
                format!("{CLASS}::define_accessor(&{home}, __key, None, Some(__value));")
            }
            _ => format!("{CLASS}::define_method(&{home}, __key, __value);"),
        };
        format!("{{ let __key = {key}; let __value = {value}; {define} }}\n")
    }

    /// A class method's function object, whose `super` refers to `home`.
    fn class_function(&mut self, function: &Function<'_>, name: &str, home: &str) -> String {
        let body = function.body.as_ref().expect("methods have bodies");
        self.closure(
            name,
            function.scope_id(),
            &function.params,
            Body::Block(&body.statements),
            false,
            false,
            Some(home),
            Callable::of(function),
            true,
        )
    }

    /// A field's key; computed keys are evaluated at class definition.
    fn field_key(
        &mut self,
        id: usize,
        key: &PropertyKey<'_>,
        code: &mut String,
        locals: &mut Vec<String>,
    ) -> FieldKey {
        match key {
            PropertyKey::PrivateIdentifier(private) => {
                FieldKey::Private(self.private_name(&private.name))
            }
            PropertyKey::StaticIdentifier(identifier) => {
                FieldKey::Name(identifier.name.to_string())
            }
            key => {
                let local = format!("key{id}_{}", locals.len());
                locals.push(local.clone());
                let value = self.property_key(key);
                let _ = writeln!(code, "let {local} = {value};");
                FieldKey::Computed(local)
            }
        }
    }

    /// Initializes one field on `this` of the initializer for `scope`.
    fn field(
        &mut self,
        scope: ScopeId,
        property: &PropertyDefinition<'_>,
        key: &FieldKey,
    ) -> String {
        let value = property
            .value
            .as_ref()
            .map_or("Value::Undefined".to_owned(), |value| {
                self.expression(value)
            });
        let this = format!("this{}", scope.index());
        // With `useDefineForClassFields: false`, `tsc` assigns fields in the
        // constructor, so setters along the prototype chain run.
        let assign = self.assigns_fields();
        let define = match key {
            FieldKey::Name(name) if assign => {
                let key = self.name_key(name);
                self.fail(&format!("ops::set(realm, &{this}, &{key}, __value)"))
            }
            FieldKey::Name(name) => {
                let key = self.name_key(name);
                self.fail(&format!("{CLASS}::define_field(realm, &{this}, {key}, __value)"))
            }
            FieldKey::Computed(local) if assign => {
                self.fail(&format!("ops::set(realm, &{this}, &{local}, __value)"))
            }
            FieldKey::Computed(local) => self.fail(&format!(
                "{CLASS}::define_field(realm, &{this}, {local}.clone(), __value)"
            )),
            FieldKey::Private(private) => self.fail(&format!(
                "{CLASS}::private_define(realm, &{this}, &{private}, ::nativeboat_native::Property::data(__value))"
            )),
        };
        format!("{{ let __value = {value}; let _ = {define}; }}\n")
    }

    /// A function object running class element code with `this` bound for
    /// `scope` and `super` referring to `home`.
    fn initializer(
        &mut self,
        scope: ScopeId,
        home: &str,
        locals: &[String],
        body: impl FnOnce(&mut Self) -> String,
    ) -> String {
        let index = scope.index();
        let mut captures = self.captures(scope);
        for local in locals {
            let _ = writeln!(captures, "let {local} = {local}.clone();");
        }
        let _ = writeln!(captures, "let home{index} = {home}.clone();");
        self.homes.insert(scope);
        self.frames.push(frame(scope));
        let code = body(self);
        self.frames.pop();
        format!(
            "{{\n{captures}realm.function(\"\", 0, false, move |realm: &mut Realm, this: Value, args: &[Value]| -> ::nativeboat_native::Completion {{\nlet this{index} = this;\n{code}Ok(Value::Undefined)\n}})\n}}"
        )
    }

    /// The innermost function whose `this` is in scope.
    fn this_owner(&self) -> ScopeId {
        self.frames
            .iter()
            .rev()
            .find(|frame| !self.scoping.scope_flags(frame.scope).is_arrow())
            .expect("the module frame is not an arrow")
            .scope
    }

    /// `this`, checked for initialization in derived constructors.
    pub(super) fn this_value(&mut self) -> String {
        let owner = self.this_owner();
        if self.constructors.get(&owner) == Some(&true) {
            self.fail(&format!(
                "{CLASS}::this_value(realm, &this{})",
                owner.index()
            ))
        } else {
            format!("this{}.clone()", owner.index())
        }
    }

    pub(super) fn new_target(&mut self) -> String {
        let owner = self.this_owner();
        if self.constructors.contains_key(&owner) || self.function_targets.contains(&owner) {
            format!("target{}.clone()", owner.index())
        } else {
            // Class methods/field initializers have no construction target.
            // Invalid module-level new.target is rejected by the parser.
            "Value::Undefined".to_owned()
        }
    }

    /// Whether TypeScript's `useDefineForClassFields: false` applies to this
    /// module's classes.
    pub(super) fn assigns_fields(&self) -> bool {
        self.typescript && self.options.assign_class_fields
    }

    /// Whether a field only declares its type, as a field without an
    /// initializer does under `useDefineForClassFields: false`. Private
    /// (`#name`) fields are always fields.
    fn declares_only(&self, property: &PropertyDefinition<'_>) -> bool {
        self.assigns_fields()
            && property.value.is_none()
            && !matches!(property.key, PropertyKey::PrivateIdentifier(_))
    }

    /// Whether TypeScript's `experimentalDecorators` applies to this
    /// module's classes.
    pub(super) fn legacy_decorators(&self) -> bool {
        self.typescript && self.options.legacy_decorators
    }

    /// Stores a TypeScript parameter property on `target`: defines it, or
    /// with `useDefineForClassFields: false` assigns it, as `tsc` emits
    /// `this.name = name`.
    pub(super) fn parameter_property(
        &mut self,
        name: &str,
        symbol: SymbolId,
        target: &str,
    ) -> String {
        let index = symbol.index();
        let value = if self.captured(symbol) {
            self.fail(&format!(
                "binding::read(realm, &c{index}, {})",
                rust_str(name)
            ))
        } else {
            format!("b{index}.clone()")
        };
        let key = self.name_key(name);
        let store = if self.assigns_fields() {
            self.fail(&format!("ops::set(realm, &{target}, &{key}, __value)"))
        } else {
            self.fail(&format!(
                "{CLASS}::define_field(realm, &{target}, {key}, __value)"
            ))
        };
        format!("{{ let __value = {value}; {store}; }}\n")
    }

    /// `super(...args)` in a derived constructor or its arrow functions.
    pub(super) fn super_call(&mut self, arguments: &[Argument<'_>]) -> String {
        let owner = self.this_owner();
        let arguments = self.arguments(arguments);
        let call = self.fail(&format!(
            "{CLASS}::super_call(realm, &this{}, &__arguments)",
            owner.index()
        ));
        let mut properties = String::new();
        let initializers = self
            .constructor_parameter_properties
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let target = if initializers.is_empty() {
            String::new()
        } else {
            let target = self.this_value();
            format!("let __target = {target};")
        };
        for (name, symbol) in initializers {
            properties.push_str(&self.parameter_property(&name, symbol, "__target"));
        }
        if self.deferred_fields.contains_key(&owner) {
            let initialize = self.fail(&format!(
                "{CLASS}::initialize_deferred(realm, &this{}, &__target)",
                owner.index()
            ));
            let _ = writeln!(properties, "{initialize};");
        }
        format!(
            "{{ {arguments} let __super_result = {call}; {target} {properties} __super_result }}"
        )
    }

    /// `super.name` or `super[key]` into `__member`, with `this` as
    /// `__object` for a method call.
    pub(super) fn super_member_parts(&mut self, span: Span, key: &str) -> String {
        let owner = self.this_owner();
        if !self.homes.contains(&owner) {
            self.unsupported(span, "super outside class methods is not compiled yet");
            return "let __object = Value::Undefined; let __member = Value::Undefined; ".to_owned();
        }
        let this = self.this_value();
        let get = self.fail(&format!(
            "{CLASS}::super_get(realm, &home{}, &__object, &__name)",
            owner.index()
        ));
        format!("let __object = {this}; let __name = {key}; let __member = {get}; ")
    }

    /// A `super[key]` assignment target: reads and writes go through the
    /// home object's prototype with `this` as the receiver. `key` is the
    /// Rust expression of the property key.
    pub(super) fn super_target(&mut self, span: Span, key: String) -> (String, String, String) {
        let owner = self.this_owner();
        if !self.homes.contains(&owner) {
            self.unsupported(span, "super outside class methods is not compiled yet");
            return (String::new(), "Value::Undefined".to_owned(), String::new());
        }
        let this = self.this_value();
        let home = format!("home{}", owner.index());
        let read = self.fail(&format!(
            "{CLASS}::super_get(realm, &{home}, &__object, &__name)"
        ));
        let write = self.fail(&format!(
            "{CLASS}::super_set(realm, &{home}, &__object, &__name, __value.clone())"
        ));
        (
            format!("let __object = {this}; let __name = {key}; "),
            read,
            format!("{write};\n"),
        )
    }

    /// `object.#name` into `__member`, keeping `__object` for a method call.
    pub(super) fn private_member_parts(
        &mut self,
        field: &PrivateFieldExpression<'_>,
        chain: Option<&str>,
    ) -> String {
        let object = self.expression_in(&field.object, chain);
        let guard = self.optional(field.optional, chain);
        let name = self.private_name(&field.field.name);
        let get = self.fail(&format!("{CLASS}::private_get(realm, &__object, &{name})"));
        format!("let __object = {object}; {guard}let __member = {get}; ")
    }

    /// `#name in value`.
    pub(super) fn private_in(&mut self, test: &PrivateInExpression<'_>) -> String {
        let name = self.private_name(&test.left.name);
        let value = self.expression(&test.right);
        let result = self.fail(&format!("{CLASS}::private_in(realm, &{name}, &__right)"));
        format!("{{ let __right = {value}; Value::Boolean({result}) }}")
    }

    /// The Rust variable holding a private name's key.
    pub(super) fn private_name(&self, name: &str) -> String {
        self.classes
            .iter()
            .rev()
            .flatten()
            .find(|(private, _)| private == name)
            .map(|(_, key)| key.clone())
            .expect("private names are declared by an enclosing class")
    }
}
