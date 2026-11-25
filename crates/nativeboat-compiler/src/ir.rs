//! Typed native behavior IR and Rust code generation.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scalar {
    Number,
    Boolean,
}

impl Scalar {
    fn rust(self) -> &'static str {
        match self {
            Scalar::Number => "f64",
            Scalar::Boolean => "bool",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Expr {
    pub kind: Kind,
    pub scalar: Scalar,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Kind {
    Number(f64),
    Boolean(bool),
    Binding(usize),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Abs(Box<Expr>),
    Call(usize, Vec<Expr>),
    Extremum(bool, Vec<Expr>),
}

impl Expr {
    fn rust(&self) -> String {
        match &self.kind {
            Kind::Number(value) => {
                if value.is_infinite() {
                    "f64::INFINITY".to_owned()
                } else {
                    format!("{value:?}_f64")
                }
            }
            Kind::Boolean(value) => value.to_string(),
            Kind::Binding(id) => format!("nb_{id}"),
            Kind::Unary(operator, value) => format!("({operator}{})", value.rust()),
            Kind::Binary(operator, left, right) => {
                format!("({} {operator} {})", left.rust(), right.rust())
            }
            Kind::Conditional(test, yes, no) => format!(
                "(if {} {{ {} }} else {{ {} }})",
                test.rust(),
                yes.rust(),
                no.rust()
            ),
            Kind::Abs(value) => format!("({}).abs()", value.rust()),
            Kind::Call(id, arguments) => format!(
                "nb_{id}({})",
                arguments
                    .iter()
                    .map(Expr::rust)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Kind::Extremum(maximum, arguments) => {
                let values = arguments
                    .iter()
                    .map(Expr::rust)
                    .collect::<Vec<_>>()
                    .join(", ");
                let (initial, comparison, sign) = if *maximum {
                    ("f64::NEG_INFINITY", ">", "positive")
                } else {
                    ("f64::INFINITY", "<", "negative")
                };
                // Evaluate every argument before inspecting values. Native f64::min/max
                // do not implement JavaScript's NaN and signed-zero rules.
                format!(
                    "{{ let nb_args: [f64; {}] = [{values}]; let mut nb_result = {initial}; for nb_value in nb_args {{ if nb_value.is_nan() {{ nb_result = f64::NAN; }} else if nb_value {comparison} nb_result || (nb_value == 0.0 && nb_result == 0.0 && nb_value.is_sign_{sign}()) {{ nb_result = nb_value; }} }} nb_result }}",
                    arguments.len()
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Stmt {
    Bind(usize, Expr, bool),
    Assign(usize, Expr),
    Evaluate(Expr),
    Return(Expr),
    If(Expr, Block, Option<Block>),
    Block(Block),
    Empty,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Block {
    pub statements: Vec<Stmt>,
    pub result: Option<Scalar>,
    pub returns: bool,
}

impl Block {
    fn rust(&self) -> String {
        let mut output = String::from("{\n");
        for statement in &self.statements {
            let text = match statement {
                Stmt::Bind(id, value, mutable) => format!(
                    "let {}nb_{id}: {} = {};\n",
                    if *mutable { "mut " } else { "" },
                    value.scalar.rust(),
                    value.rust()
                ),
                Stmt::Assign(id, value) => format!("nb_{id} = {};\n", value.rust()),
                Stmt::Evaluate(value) => format!("let _ = {};\n", value.rust()),
                Stmt::Return(value) => format!("return {};\n", value.rust()),
                Stmt::If(test, yes, no) => {
                    let alternate = no
                        .as_ref()
                        .map_or(String::new(), |body| format!(" else {}", body.rust()));
                    format!("if {} {}{alternate}\n", test.rust(), yes.rust())
                }
                Stmt::Block(body) => format!("{}\n", body.rust()),
                Stmt::Empty => String::new(),
            };
            output.push_str(&text);
        }
        output.push('}');
        output
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Function {
    pub source_name: String,
    pub id: usize,
    pub exported: bool,
    pub parameters: Vec<(usize, Scalar)>,
    pub body: Block,
    pub scalar: Scalar,
}

/// A fully checked native module; its IR cannot be constructed unchecked.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModule {
    pub(crate) constants: Vec<(usize, Expr)>,
    pub(crate) functions: Vec<Function>,
}

impl NativeModule {
    /// Native symbol for a directly exported source function.
    pub fn export_name(&self, source_name: &str) -> Option<String> {
        self.functions
            .iter()
            .find(|function| function.exported && function.source_name == source_name)
            .map(|function| format!("nb_{}", function.id))
    }

    /// Standalone native Rust source. No engine, source text or DOM runtime
    /// is embedded. Link an entry point with a native toolchain to execute it.
    pub fn rust_source(&self) -> String {
        let mut output = "// Generated native application behavior.\n#![allow(dead_code, unused_parens, unused_mut, unused_variables, unused_assignments, non_upper_case_globals)]\n".to_owned();
        for (id, value) in &self.constants {
            output.push_str(&format!(
                "const nb_{id}: {} = {};\n",
                value.scalar.rust(),
                value.rust()
            ));
        }
        for function in &self.functions {
            let parameters = function
                .parameters
                .iter()
                .map(|(id, scalar)| format!("mut nb_{id}: {}", scalar.rust()))
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!(
                "pub fn nb_{}({parameters}) -> {} {}\n",
                function.id,
                function.scalar.rust(),
                function.body.rust()
            ));
        }
        output
    }
}
