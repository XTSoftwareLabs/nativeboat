//! Symbols: unique values usable as property keys.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::string::JsStr;
use alloc::rc::{Rc, Weak};
use core::fmt;
use core::hash::{Hash, Hasher};

/// A symbol. Equality and hashing are identity.
#[derive(Clone)]
pub struct Symbol(Rc<(Option<JsStr>, bool, Option<JsStr>)>);

impl Symbol {
    /// A new symbol with an optional description.
    pub fn new(description: Option<JsStr>) -> Symbol {
        Symbol(Rc::new((description, false, None)))
    }

    /// A Node private key, omitted from property enumeration.
    pub(crate) fn private(description: &str) -> Symbol {
        Symbol(Rc::new((Some(JsStr::from(description)), true, None)))
    }

    /// A private method/accessor key with its lexical class for brand errors.
    pub fn class_member(owner: &str, description: &str) -> Symbol {
        Symbol(Rc::new((
            Some(JsStr::from(description)),
            false,
            Some(JsStr::from(owner)),
        )))
    }

    pub(crate) fn member_owner(&self) -> Option<&JsStr> {
        self.0.2.as_ref()
    }

    pub(crate) fn is_private(&self) -> bool {
        self.0.1
    }

    /// The description given when the symbol was created.
    pub fn description(&self) -> Option<&JsStr> {
        self.0.0.as_ref()
    }

    /// A handle that does not keep the symbol alive.
    pub fn downgrade(&self) -> WeakSymbol {
        WeakSymbol(Rc::downgrade(&self.0))
    }

    /// `SymbolDescriptiveString`: `Symbol(description)`.
    pub fn descriptive(&self) -> JsStr {
        let description = self.description().cloned().unwrap_or_default();
        JsStr::from("Symbol(")
            .concat(&description)
            .concat(&JsStr::from(")"))
    }
}

/// A symbol held without keeping it alive, as weak collections hold keys.
#[derive(Clone, Debug)]
pub struct WeakSymbol(Weak<(Option<JsStr>, bool, Option<JsStr>)>);

impl WeakSymbol {
    /// The symbol, while it is alive.
    pub fn upgrade(&self) -> Option<Symbol> {
        self.0.upgrade().map(Symbol)
    }

    /// The symbol's identity; reused only after the symbol is gone.
    pub fn address(&self) -> usize {
        self.0.as_ptr() as *const u8 as usize
    }
}

impl Symbol {
    /// The symbol's identity as an address.
    pub(crate) fn address(&self) -> usize {
        Rc::as_ptr(&self.0) as *const u8 as usize
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Symbol) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.0).hash(state);
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.descriptive())
    }
}

/// The well-known symbols the language's protocols look up.
#[derive(Debug, Clone)]
pub struct WellKnown {
    /// `Symbol.asyncDispose`.
    pub async_dispose: Symbol,
    /// `Symbol.asyncIterator`.
    pub async_iterator: Symbol,
    /// `Symbol.dispose`.
    pub dispose: Symbol,
    /// `Symbol.isConcatSpreadable`.
    pub is_concat_spreadable: Symbol,
    /// `Symbol.iterator`.
    pub iterator: Symbol,
    /// `Symbol.hasInstance`.
    pub has_instance: Symbol,
    /// `Symbol.toPrimitive`.
    pub to_primitive: Symbol,
    /// `Symbol.toStringTag`.
    pub to_string_tag: Symbol,
    /// `Symbol.match`.
    pub match_: Symbol,
    /// `Symbol.matchAll`.
    pub match_all: Symbol,
    /// `Symbol.replace`.
    pub replace: Symbol,
    /// `Symbol.search`.
    pub search: Symbol,
    /// `Symbol.species`.
    pub species: Symbol,
    /// `Symbol.split`.
    pub split: Symbol,
    /// `Symbol.unscopables`.
    pub unscopables: Symbol,
    /// The private key under which a compiled function keeps the source
    /// text `Function.prototype.toString` returns.
    pub(crate) source_text: Symbol,
}

impl WellKnown {
    /// A new agent's protocol symbols; realms of one agent share them.
    pub(crate) fn create() -> WellKnown {
        let symbol = |name: &str| Symbol::new(Some(JsStr::from(name)));
        WellKnown {
            async_dispose: symbol("Symbol.asyncDispose"),
            async_iterator: symbol("Symbol.asyncIterator"),
            dispose: symbol("Symbol.dispose"),
            is_concat_spreadable: symbol("Symbol.isConcatSpreadable"),
            iterator: symbol("Symbol.iterator"),
            has_instance: symbol("Symbol.hasInstance"),
            to_primitive: symbol("Symbol.toPrimitive"),
            to_string_tag: symbol("Symbol.toStringTag"),
            match_: symbol("Symbol.match"),
            match_all: symbol("Symbol.matchAll"),
            replace: symbol("Symbol.replace"),
            search: symbol("Symbol.search"),
            species: symbol("Symbol.species"),
            split: symbol("Symbol.split"),
            unscopables: symbol("Symbol.unscopables"),
            source_text: Symbol::private("nativeboat.function.sourceText"),
        }
    }

    pub(crate) fn entries(&self) -> [(&'static str, &Symbol); 15] {
        [
            ("asyncDispose", &self.async_dispose),
            ("asyncIterator", &self.async_iterator),
            ("dispose", &self.dispose),
            ("hasInstance", &self.has_instance),
            ("isConcatSpreadable", &self.is_concat_spreadable),
            ("iterator", &self.iterator),
            ("match", &self.match_),
            ("matchAll", &self.match_all),
            ("replace", &self.replace),
            ("search", &self.search),
            ("species", &self.species),
            ("split", &self.split),
            ("toPrimitive", &self.to_primitive),
            ("toStringTag", &self.to_string_tag),
            ("unscopables", &self.unscopables),
        ]
    }
}

#[cfg(test)]
mod tests;
