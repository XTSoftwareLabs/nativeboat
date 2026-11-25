//! Read live child lists without allocating intermediate JavaScript arrays.
use super::*;

fn children(dom: &Dom, parent: NodeId, elements: bool) -> impl Iterator<Item = NodeId> + '_ {
    std::iter::successors(dom.first_child(parent), |node| dom.next_sibling(*node))
        .filter(move |node| !elements || dom.is_element(*node))
}

pub(super) fn count(dom: &RefCell<Dom>, args: &[JsValue], elements: bool) -> JsResult<JsValue> {
    let dom = dom.borrow();
    let parent = node_at(&dom, args, 0)?;
    Ok(JsValue::from(if elements {
        dom.element_child_count(parent)
    } else {
        dom.child_count(parent)
    }))
}

pub(super) fn at(
    dom: &RefCell<Dom>,
    args: &[JsValue],
    context: &mut Context,
    elements: bool,
) -> JsResult<JsValue> {
    let index = args.get_or_undefined(1).to_number(context)?;
    if !index.is_finite() || index < 0.0 || index > f64::from(u32::MAX) {
        return Ok(JsValue::null());
    }
    let dom = dom.borrow();
    let parent = node_at(&dom, args, 0)?;
    Ok(id(children(&dom, parent, elements).nth(index as usize)))
}
