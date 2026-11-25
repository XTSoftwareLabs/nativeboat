//! `switch`, `for`-`in`, `for`-`of` and per-iteration `let` bindings.

use super::patterns::Mode;
use super::{Emitter, Exit, Loop};
use oxc::ast::ast::*;
use oxc::semantic::SymbolId;
use std::fmt::Write;

impl Emitter<'_> {
    /// `switch`: the first strictly equal case (default otherwise) and every
    /// clause after it run, inside one block that `break` leaves.
    pub(super) fn switch(&mut self, statement: &SwitchStatement<'_>) -> String {
        let labels = std::mem::take(&mut self.frame().labels);
        let id = self.id();
        let discriminant = self.expression(&statement.discriminant);
        let mut selection = String::new();
        let mut default = statement.cases.len();
        for (index, case) in statement.cases.iter().enumerate() {
            match &case.test {
                Some(test) => {
                    let test = self.expression(test);
                    let _ = write!(
                        selection,
                        "if ops::strict_equals(&__discriminant, &{test}) {{ break 'm{id} {index}usize; }} "
                    );
                }
                None => default = index,
            }
        }
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'s{id}"),
            next: None,
            guards,
            iterator: None,
        });
        // Clauses share the switch's scope: declare its bindings up front.
        let scope = statement.scope_id();
        let mut bindings = self.scope_bindings(scope, &[]);
        let mut body = String::new();
        bindings.push_str(&self.hoist(statement.cases.iter().flat_map(|case| {
            case.consequent
                .iter()
                .filter_map(|statement| match statement {
                    Statement::FunctionDeclaration(function) => Some(&**function),
                    _ => None,
                })
        })));
        for (index, case) in statement.cases.iter().enumerate() {
            let mut clause = String::new();
            for statement in &case.consequent {
                clause.push_str(&self.statement(statement));
            }
            let _ = writeln!(body, "if __case <= {index} {{\n{clause}}}");
        }
        self.frame().loops.pop();
        format!(
            "'s{id}: {{\nlet __discriminant = {discriminant};\n{bindings}let __case: usize = 'm{id}: {{ {selection}{default}usize }};\n{body}}}\n"
        )
    }

    /// The loop head of `for`-`in`/`for`-`of`: binds the Rust variable
    /// `value` to its declaration or assignment target for one iteration.
    fn head(&mut self, left: &ForStatementLeft<'_>, value: &str) -> String {
        match left {
            ForStatementLeft::VariableDeclaration(declaration) => {
                let mode = if declaration.kind == VariableDeclarationKind::Var {
                    Mode::Var
                } else {
                    Mode::Fresh
                };
                let pattern = &declaration.declarations[0].id;
                let mut code = self.predeclare(pattern, mode);
                code.push_str(&self.bind(pattern, value, mode));
                code
            }
            target => self.assign_target(target.to_assignment_target(), value),
        }
    }

    /// `for`-`in`: enumerable string keys, skipping keys deleted meanwhile.
    pub(super) fn for_in(&mut self, statement: &ForInStatement<'_>) -> String {
        let labels = std::mem::take(&mut self.frame().labels);
        let id = self.id();
        let scope = self.scope_bindings(statement.scope_id(), &[]);
        let object = self.expression(&statement.right);
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'l{id}"),
            next: Some(format!("'n{id}")),
            guards,
            iterator: None,
        });
        let present = self.fail("ops::has_property(realm, &__key, &__object)");
        let keys = self.fail("::nativeboat_native::iter::for_in_keys(realm, &__object)");
        let value = self.temporary("key_value");
        let head = self.head(&statement.left, &value);
        let body = self.nested(&statement.body);
        self.frame().loops.pop();
        format!(
            "{{\n{scope}let __object = {object};\n\
             'l{id}: for __key in {keys} {{\n\
             if matches!(__object, Value::Object(_)) && !{present} {{ continue 'l{id}; }}\n\
             let {value} = Value::String(__key.to_js());\n{head}'n{id}: {body}\n}}\n}}\n"
        )
    }

    /// `for`-`of`: throws from the loop body close the iterator first; so do
    /// `break`, `continue` to an outer loop and `return` (see `jump`).
    pub(super) fn for_of(&mut self, statement: &ForOfStatement<'_>) -> String {
        let labels = std::mem::take(&mut self.frame().labels);
        let id = self.id();
        let iterator = format!("__iterator{id}");
        let scope = self.scope_bindings(statement.scope_id(), &[]);
        let iterable = self.expression(&statement.right);
        let method = if statement.r#await {
            "iterate_async"
        } else {
            "iterate"
        };
        let start = self.fail(&format!(
            "::nativeboat_native::iter::{method}(realm, &__iterable)"
        ));
        let step = self.fail(&format!("{iterator}.step(realm)"));
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'l{id}"),
            next: Some(format!("'n{id}")),
            guards,
            iterator: Some(iterator.clone()),
        });
        self.frame().exits.push(Exit::Attempt {
            slot: format!("__body{id}"),
            label: format!("'b{id}"),
        });
        let head = self.head(&statement.left, "__item");
        let body = self.nested(&statement.body);
        self.frame().exits.pop();
        self.frame().loops.pop();
        let rethrow = self.throw("__closed");
        format!(
            "{{\n{scope}let __iterable = {iterable};\nlet mut {iterator} = {start};\n\
             'l{id}: loop {{\nlet Some(__item) = {step} else {{ break 'l{id}; }};\n\
             let mut __body{id}: Option<Thrown> = None;\n'b{id}: {{\n{head}'n{id}: {body}\n}}\n\
             if let Some(__thrown) = __body{id} {{ let __closed = {iterator}.close_thrown(realm, __thrown); {rethrow}}}\n}}\n}}\n"
        )
    }

    /// Closing the `for`-`of` iterators that a jump or `return` leaves:
    /// `inner` loops from the innermost outward.
    pub(super) fn close_iterators(&mut self, inner: usize) -> String {
        let iterators: Vec<String> = self.frame().loops[inner..]
            .iter()
            .rev()
            .filter_map(|target| target.iterator.clone())
            .collect();
        let mut code = String::new();
        for iterator in iterators {
            let close = self.fail(&format!("{iterator}.close(realm)"));
            let _ = write!(code, "{close}; ");
        }
        code
    }

    /// A `for` statement whose `let` bindings closures capture: each
    /// iteration gets fresh cells holding the previous iteration's values,
    /// and the update runs on the new cells, as the language specifies.
    pub(super) fn per_iteration(
        &mut self,
        statement: &ForStatement<'_>,
        declaration: &VariableDeclaration<'_>,
        labels: Vec<String>,
        captured: &[SymbolId],
        mut setup: String,
    ) -> String {
        let id = self.id();
        setup.push_str(&self.variables(declaration));
        let guards = self.frame().guards;
        self.frame().loops.push(Loop {
            names: labels,
            exit: format!("'l{id}"),
            next: Some(format!("'n{id}")),
            guards,
            iterator: None,
        });
        let test = statement
            .test
            .as_ref()
            .map(|test| {
                let test = self.expression(test);
                format!("if !ops::to_boolean(&{test}) {{ break 'l{id}; }}\n")
            })
            .unwrap_or_default();
        let update = statement
            .update
            .as_ref()
            .map(|update| format!("if !__first {{ let _ = {}; }}\n", self.expression(update)))
            .unwrap_or_default();
        let body = self.nested(&statement.body);
        self.frame().loops.pop();
        let mut carry = String::new();
        let mut copy = String::new();
        let mut store = String::new();
        for symbol in captured {
            let index = symbol.index();
            let _ = writeln!(carry, "let mut k{index} = c{index}.clone();");
            let _ = writeln!(
                copy,
                "let c{index} = binding::new(k{index}.borrow().clone());"
            );
            let _ = writeln!(store, "k{index} = c{index}.clone();");
        }
        format!(
            "{{\n{setup}{carry}let mut __first = true;\n'l{id}: loop {{\n{copy}{update}__first = false;\n{test}'n{id}: {body}\n{store}}}\n}}\n"
        )
    }
}
