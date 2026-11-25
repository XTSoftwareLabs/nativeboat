//! Scalar range reads avoid materializing temporary JavaScript boundary arrays.
use super::*;

pub(super) fn field(dom: &RefCell<Dom>, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let dom = dom.borrow();
    let range = args
        .get_or_undefined(0)
        .as_number()
        .filter(|id| id.fract() == 0.0 && *id >= 0.0 && *id <= f64::from(u32::MAX))
        .and_then(|id| dom.range(id as u32))
        .ok_or_else(|| JsNativeError::typ().with_message("not a range"))?;
    let field = args
        .get_or_undefined(1)
        .as_number()
        .filter(|field| field.fract() == 0.0 && *field >= 0.0 && *field <= 5.0)
        .ok_or_else(|| JsNativeError::range().with_message("invalid range field"))?
        as u32;
    let [(start, from), (end, to)] = range;
    Ok(match field {
        0 => start.0.into(),
        1 => from.into(),
        2 => end.0.into(),
        3 => to.into(),
        4 => (start == end && from == to).into(),
        _ => {
            let common = std::iter::successors(Some(start), |node| dom.parent(*node))
                .find(|node| dom.contains(*node, end));
            id(common)
        }
    })
}

#[cfg(test)]
mod tests;
