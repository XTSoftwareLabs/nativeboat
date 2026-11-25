use super::*;

fn codes(realm: &mut Realm) -> Value {
    let errors = crate::modules::load(realm, "internal/errors").unwrap();
    ops::get_named(realm, &errors, "codes").unwrap()
}

fn construct(realm: &mut Realm, class: &Value, args: &[Value]) -> Value {
    ops::construct(realm, class, args).unwrap()
}

fn text(realm: &mut Realm, value: &Value, name: &'static str) -> String {
    let value = ops::get_named(realm, value, name).unwrap();
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

#[test]
fn template_codes_construct_node_errors_of_their_classes() {
    let realm = &mut Realm::new();
    let codes = codes(realm);
    let premature = ops::get_named(realm, &codes, "ERR_STREAM_PREMATURE_CLOSE").unwrap();
    let error = construct(realm, &premature, &[]);
    assert_eq!(text(realm, &error, "message"), "Premature close");
    assert_eq!(text(realm, &error, "code"), "ERR_STREAM_PREMATURE_CLOSE");
    assert_eq!(text(realm, &error, "name"), "Error");

    // Other classes hang off the first one.
    let state = ops::get_named(realm, &codes, "ERR_INVALID_STATE").unwrap();
    let type_error = ops::get_named(realm, &state, "TypeError").unwrap();
    let error = construct(
        realm,
        &type_error,
        &[Value::str("Controller is already closed")],
    );
    assert_eq!(
        text(realm, &error, "message"),
        "Invalid state: Controller is already closed"
    );
    assert_eq!(text(realm, &error, "name"), "TypeError");
    let hidden = ops::get_named(realm, &state, "HideStackFramesError").unwrap();
    assert!(crate::same_value(&hidden, &state));

    let missing = ops::get_named(realm, &codes, "ERR_MISSING_OPTION").unwrap();
    let error = construct(realm, &missing, &[Value::str("options.signal")]);
    assert_eq!(text(realm, &error, "message"), "options.signal is required");

    // Codes with their own constructors keep them.
    let invalid = ops::get_named(realm, &codes, "ERR_INVALID_ARG_TYPE").unwrap();
    let error = construct(
        realm,
        &invalid,
        &[Value::str("name"), Value::str("string"), Value::Number(1.0)],
    );
    assert!(
        text(realm, &error, "message").starts_with("The \"name\" argument must be of type string.")
    );
}

#[test]
fn templates_format_strings_numbers_json_and_literal_percents() {
    let realm = &mut Realm::new();
    let object = realm.object();
    object.define(Key::name("a"), Property::data(Value::Number(1.0)));
    let args = [
        Value::str("text"),
        Value::Number(3.9),
        Value::Number(-2.5),
        Value::Object(object),
    ];
    assert_eq!(
        format(realm, "%s %d %i %j 100%% %s", &args).unwrap(),
        "text 3.9 -2 {\"a\":1} 100% %s"
    );
    let symbol = Value::Symbol(crate::Symbol::new(Some("tag".into())));
    assert_eq!(format(realm, "[%s]", &[symbol]).unwrap(), "[Symbol(tag)]");
}
