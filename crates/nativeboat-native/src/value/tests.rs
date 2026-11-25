use super::*;
use crate::object::Kind;

#[test]
fn values_convert_from_native_data() {
    assert!(matches!(Value::from(1.5), Value::Number(1.5)));
    assert!(matches!(Value::from(true), Value::Boolean(true)));
    assert!(matches!(Value::from(JsStr::from("s")), Value::String(_)));
    let object = Object::new(None, Kind::Ordinary);
    let value = Value::from(object.clone());
    assert_eq!(value.as_object(), Some(&object));
    assert_eq!(Value::Null.as_object(), None);
    assert!(Value::default().is_nullish());
    assert!(Value::Null.is_nullish());
    assert!(!Value::str("").is_nullish());
    let thrown = Thrown(Value::Null);
    assert!(format!("{thrown:?}").contains("Null"));
}

#[test]
fn symbols_convert_to_values() {
    let symbol = crate::Symbol::new(None);
    assert!(matches!(Value::from(symbol.clone()), Value::Symbol(found) if found == symbol));
}
