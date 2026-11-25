use super::*;

pub(crate) mod support;

#[test]
fn errors_describe_themselves() {
    let error = Error {
        message: "boom".to_owned(),
    };
    assert_eq!(error.to_string(), "boom");
    let last = Error::last("Call");
    assert!(last.message.starts_with("Call failed with error "));
}
