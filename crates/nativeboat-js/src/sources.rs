//! Source ownership follows the loader graph, with no eager JS string copies.

use super::{compile_source, text};
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{
    Context, Finalize, JsArgs, JsNativeError, JsObject, JsResult, JsString, JsValue,
    NativeFunction, Source, Trace,
};
use nativeboat_ir::Script;
use std::path::Path;
use std::rc::Rc;

#[derive(Clone, Trace, Finalize)]
struct Sources {
    // Native UTF-8 source text has no GC pointers. The functions below keep
    // it alive exactly as long as the bundle's existing loader closures.
    #[unsafe_ignore_trace]
    code: Rc<Vec<String>>,
}

impl Sources {
    fn get(&self, args: &[JsValue], context: &mut Context) -> JsResult<&str> {
        let index = args.get_or_undefined(0).to_index(context)?;
        usize::try_from(index)
            .ok()
            .and_then(|index| self.code.get(index))
            .map(String::as_str)
            .ok_or_else(|| {
                JsNativeError::range()
                    .with_message("invalid bundled module index")
                    .into()
            })
    }
}

fn compile(
    _: &JsValue,
    args: &[JsValue],
    sources: &Sources,
    context: &mut Context,
) -> JsResult<JsValue> {
    let code = sources.get(args, context)?;
    let path = text(args, 1, context)?;
    let extra = text(args, 2, context)?;
    compile_source(code, &path, &extra, context)
}

fn evaluate(
    _: &JsValue,
    args: &[JsValue],
    sources: &Sources,
    context: &mut Context,
) -> JsResult<JsValue> {
    let code = sources.get(args, context)?;
    let path = text(args, 1, context)?;
    context.eval(Source::from_bytes(code).with_path(Path::new(&path)))
}

pub(super) fn install(
    descriptor: &JsObject,
    script: &Script,
    context: &mut Context,
) -> JsResult<()> {
    let sources = Sources {
        code: Rc::new(
            script
                .bundle
                .modules
                .iter()
                .map(|module| module.code.clone())
                .collect(),
        ),
    };
    for (name, operation) in [
        (
            "compile",
            compile as fn(&JsValue, &[JsValue], &Sources, &mut Context) -> JsResult<JsValue>,
        ),
        ("evaluate", evaluate),
    ] {
        let function = FunctionObjectBuilder::new(
            context.realm(),
            NativeFunction::from_copy_closure_with_captures(operation, sources.clone()),
        )
        .build();
        descriptor.create_data_property_or_throw(JsString::from(name), function, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
