//! Structured locations from Boa's active bytecode frames, without parsing
//! formatted error strings or exposing strict-mode receiver/function objects.
use boa_engine::builtins::function::OrdinaryFunction;
use boa_engine::vm::SourcePath;
use boa_engine::{Context, JsArgs, JsResult, JsValue};
use serde_json::{Value, json};

pub(crate) fn frames(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let limit = args.get_or_undefined(1).to_number(context)?.max(0.0) as usize;
    // The JavaScript captureStackTrace wrapper is not an application frame.
    let frames: Vec<_> = context.stack_trace().skip(1).collect();
    let constructor = args.first().and_then(JsValue::as_callable);
    let start = match constructor {
        Some(constructor) => {
            let ordinary = constructor.downcast_ref::<OrdinaryFunction>();
            frames
                .iter()
                .position(|frame| {
                    ordinary.as_ref().is_some_and(|function| {
                        std::ptr::eq(&**frame.code_block(), function.codeblock())
                    })
                })
                .map_or(frames.len(), |index| index + 1)
        }
        None => 0,
    };
    let locations: Vec<_> = frames[start..].iter().take(limit).map(|frame| {
        let location = frame.position();
        let path = match location.path {
            SourcePath::None => Value::Null,
            path => Value::from(path.to_string()),
        };
        let (line, column) = location.position.map_or((Value::Null, Value::Null), |position| {
            (Value::from(position.line_number()), Value::from(position.column_number()))
        });
        json!({ "name": location.function_name.to_std_string_escaped(), "file": path, "line": line, "column": column })
    }).collect();
    JsValue::from_json(&Value::Array(locations), context)
}
