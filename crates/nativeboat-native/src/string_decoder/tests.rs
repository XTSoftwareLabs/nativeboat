use super::*;
use crate::{
    ops,
    typed_array_builtin::{self, Element},
};

const VECTORS: &[&[u8]] = &[
    &[],
    &[65],
    &[65, 66, 67],
    &[0x61, 0xc2, 0xa2, 0x62],
    &[0xe4, 0xb8, 0xad],
    &[0xf0, 0x9f, 0x9a, 0x80],
    &[0xc0, 0xaf],
    &[0xed, 0xa0, 0x80],
    &[0xf4, 0x90, 0x80, 0x80],
    &[0x80],
    &[0x80, 0x80, 0x80, 0x80, 0x80],
    &[0xc2, 65],
    &[0xe2, 0x82, 65],
    &[0xf0, 0x9f, 0x9a, 65],
    &[0xff, 0xfe, 0x41, 0],
    &[0x3d, 0xd8, 0x80, 0xde],
    &[0x3d, 0xd8, 0x80],
    &[0xf8, 0xbf, 0xbf, 0xbf],
    &[0xc2],
    &[0xe2, 0x82],
    &[0xf0, 0x9f, 0x9a],
    &[0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67],
];

fn index(realm: &mut Realm, value: &Value, at: u32) -> Value {
    ops::get(realm, value, &Key::Index(at)).unwrap()
}
fn number(realm: &mut Realm, value: &Value) -> usize {
    ops::to_number(realm, value).unwrap() as usize
}
fn numbers(realm: &mut Realm, value: &Value) -> Vec<u16> {
    let length = ops::get(realm, value, &Key::name("length")).unwrap();
    let length = number(realm, &length);
    (0..length)
        .map(|at| {
            let value = index(realm, value, at as u32);
            number(realm, &value) as u16
        })
        .collect()
}
fn assert_state(realm: &mut Realm, state: &[u8; 7], expected: &Value) {
    let missing = index(realm, expected, 0);
    let total = index(realm, expected, 1);
    let bytes = index(realm, expected, 2);
    assert_eq!(usize::from(state[MISSING]), number(realm, &missing));
    assert_eq!(
        usize::from(state[MISSING]) + usize::from(state[BUFFERED]),
        number(realm, &total)
    );
    assert_eq!(
        state[..4]
            .iter()
            .map(|byte| u16::from(*byte))
            .collect::<Vec<_>>(),
        numbers(realm, &bytes)
    );
}

#[test]
fn streaming_outputs_and_visible_state_match_all_node_reference_splits() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    let json = ops::get(&mut realm, &global, &Key::name("JSON")).unwrap();
    let parse = ops::get(&mut realm, &json, &Key::name("parse")).unwrap();
    for line in include_str!("reference.expected.txt").lines() {
        let row = ops::call(&mut realm, &parse, json.clone(), &[Value::str(line)]).unwrap();
        let label = index(&mut realm, &row, 0);
        let Value::String(label) = label else {
            panic!("encoding")
        };
        let encoding = encoding::named(label.units()).unwrap();
        let vector = index(&mut realm, &row, 1);
        let vector = number(&mut realm, &vector);
        let split = index(&mut realm, &row, 2);
        let split = number(&mut realm, &split);
        let mut state = [0; 7];
        let first = decode(&mut state, &VECTORS[vector][..split], encoding);
        let expected = index(&mut realm, &row, 3);
        assert_eq!(first, numbers(&mut realm, &expected), "{line}");
        let expected = index(&mut realm, &row, 4);
        assert_state(&mut realm, &state, &expected);
        let second = decode(&mut state, &VECTORS[vector][split..], encoding);
        let expected = index(&mut realm, &row, 5);
        assert_eq!(second, numbers(&mut realm, &expected), "{line}");
        let expected = index(&mut realm, &row, 6);
        assert_state(&mut realm, &state, &expected);
        let end = flush(&mut state, encoding);
        let expected = index(&mut realm, &row, 7);
        assert_eq!(end, numbers(&mut realm, &expected), "{line}");
        let missing = index(&mut realm, &row, 8);
        let total = index(&mut realm, &row, 9);
        let bytes = index(&mut realm, &row, 10);
        assert_eq!(usize::from(state[MISSING]), number(&mut realm, &missing));
        assert_eq!(
            usize::from(state[MISSING]) + usize::from(state[BUFFERED]),
            number(&mut realm, &total)
        );
        assert_eq!(
            state[..4]
                .iter()
                .map(|byte| u16::from(*byte))
                .collect::<Vec<_>>(),
            numbers(&mut realm, &bytes)
        );
    }
}

fn view(realm: &Realm, data: &[u8]) -> Value {
    let value = Value::Object(typed_array_builtin::new_typed_array(
        realm,
        Element::Uint8,
        data.len(),
    ));
    typed_array_builtin::view_of(&value)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(data);
    value
}

#[test]
fn native_binding_updates_owned_state_and_validates_buffer_boundaries() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let decode = ops::get(&mut realm, &exports, &Key::name("decode")).unwrap();
    let flush = ops::get(&mut realm, &exports, &Key::name("flush")).unwrap();
    let state = view(&realm, &[0, 0, 0, 0, 0, 0, 1]);
    let first = view(&realm, &[0xe2]);
    let empty = ops::call(
        &mut realm,
        &decode,
        Value::Undefined,
        &[state.clone(), first],
    )
    .unwrap();
    assert!(ops::strict_equals(&empty, &Value::str("")));
    let second = view(&realm, &[0x82, 0xac]);
    let euro = ops::call(
        &mut realm,
        &decode,
        Value::Undefined,
        &[state.clone(), second],
    )
    .unwrap();
    assert!(ops::strict_equals(&euro, &Value::str("€")));
    let empty = ops::call(
        &mut realm,
        &flush,
        Value::Undefined,
        core::slice::from_ref(&state),
    )
    .unwrap();
    assert!(ops::strict_equals(&empty, &Value::str("")));
    assert!(
        ops::call(
            &mut realm,
            &decode,
            Value::Undefined,
            &[state.clone(), Value::Null]
        )
        .is_err()
    );
    for data in [
        &[][..],
        &[0, 0, 0, 0, 0, 0, 6][..],
        &[0, 0, 0, 0, 5, 0, 1][..],
        &[0, 0, 0, 0, 2, 3, 1][..],
    ] {
        let state = view(&realm, data);
        assert!(ops::call(&mut realm, &flush, Value::Undefined, &[state]).is_err());
    }
    assert!(ops::call(&mut realm, &flush, Value::Undefined, &[Value::Null]).is_err());
}
