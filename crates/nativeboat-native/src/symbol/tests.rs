use super::*;
use std::collections::HashSet;

#[test]
fn symbols_are_unique_and_describe_themselves() {
    let named = Symbol::new(Some(JsStr::from("tag")));
    let same_name = Symbol::new(Some(JsStr::from("tag")));
    let anonymous = Symbol::new(None);
    assert_eq!(named, named.clone());
    assert_ne!(named, same_name);
    assert_eq!(named.description(), Some(&JsStr::from("tag")));
    assert_eq!(anonymous.description(), None);
    assert_eq!(named.descriptive(), JsStr::from("Symbol(tag)"));
    assert_eq!(anonymous.descriptive(), JsStr::from("Symbol()"));
    assert_eq!(format!("{named:?}"), "Symbol(tag)");
    let set: HashSet<Symbol> = [named.clone(), named.clone(), same_name]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
    let known = WellKnown::create();
    assert_eq!(
        known.iterator.descriptive(),
        JsStr::from("Symbol(Symbol.iterator)")
    );
    // Each agent has its own protocol symbols.
    assert_ne!(known.iterator, WellKnown::create().iterator);
    assert!(format!("{known:?}").contains("Symbol.hasInstance"));
}
