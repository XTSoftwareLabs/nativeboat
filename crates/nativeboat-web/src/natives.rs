//! Native functions over the Rust DOM and the page, called by the web prelude.
//!
//! Nodes cross into JavaScript as numeric ids; the prelude wraps them in
//! node objects. DOM errors are thrown as `Error`s whose message starts with
//! the DOMException name, which the prelude turns into `DOMException`s.

mod collections;
mod history;
mod ranges;

use crate::{Page, ScrollAlign};
use boa_engine::object::builtins::{JsArray, JsUint8Array};
use boa_engine::{Context, JsArgs, JsError, JsNativeError, JsResult, JsString, JsValue, js_string};
use nativeboat_dom::Namespace;
use nativeboat_dom::{Boundary, Dom, DomError, NodeId, ShadowRootMode};
use nativeboat_dom::{MutationKind, ObserveOptions};
use nativeboat_ir::Rect;
use std::cell::RefCell;

/// A native over the DOM.
pub(crate) type DomOp = fn(&RefCell<Dom>, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// A native over the page.
pub(crate) type PageOp = fn(&dyn Page, &[JsValue], &mut Context) -> JsResult<JsValue>;

fn node_at(dom: &Dom, args: &[JsValue], index: usize) -> JsResult<NodeId> {
    let value = args.get_or_undefined(index);
    value
        .as_number()
        .filter(|number| number.fract() == 0.0 && *number >= 0.0 && *number <= f64::from(u32::MAX))
        .map(|number| NodeId(number as u32))
        .filter(|id| dom.exists(*id))
        .ok_or_else(|| JsNativeError::typ().with_message("not a node").into())
}

fn node(dom: &RefCell<Dom>, args: &[JsValue], index: usize) -> JsResult<NodeId> {
    node_at(&dom.borrow(), args, index)
}

fn optional_node(dom: &RefCell<Dom>, args: &[JsValue], index: usize) -> JsResult<Option<NodeId>> {
    if args.get_or_undefined(index).is_null_or_undefined() {
        return Ok(None);
    }
    node(dom, args, index).map(Some)
}

fn text(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get_or_undefined(index)
        .to_string(context)?
        .to_std_string_escaped())
}

fn number(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<f64> {
    args.get_or_undefined(index).to_number(context)
}

fn flag(args: &[JsValue], index: usize) -> bool {
    args.get_or_undefined(index).to_boolean()
}

fn string(value: impl Into<JsString>) -> JsValue {
    JsValue::from(value.into())
}

fn optional_string(value: Option<impl Into<JsString>>) -> JsValue {
    value.map_or(JsValue::null(), string)
}

fn id(value: Option<NodeId>) -> JsValue {
    value.map_or(JsValue::null(), |id| JsValue::from(id.0))
}

fn ids(values: Vec<NodeId>, context: &mut Context) -> JsValue {
    JsArray::from_iter(values.into_iter().map(|id| JsValue::from(id.0)), context).into()
}

fn numbers(values: &[f32], context: &mut Context) -> JsValue {
    JsArray::from_iter(
        values.iter().map(|value| JsValue::from(f64::from(*value))),
        context,
    )
    .into()
}

fn rect(value: Option<Rect>, context: &mut Context) -> JsValue {
    value.map_or(JsValue::null(), |rect| {
        numbers(&[rect.x, rect.y, rect.width, rect.height], context)
    })
}

fn failure(error: DomError) -> JsError {
    JsNativeError::error()
        .with_message(format!("{}: {}", error.name(), error.message()))
        .into()
}

fn done<T>(result: Result<T, DomError>) -> JsResult<JsValue> {
    result.map(|_| JsValue::undefined()).map_err(failure)
}

fn unit() -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

/// Two boundary points given as node, offset, node, offset from `first` on.
fn boundaries(
    dom: &RefCell<Dom>,
    args: &[JsValue],
    first: usize,
    context: &mut Context,
) -> JsResult<[Boundary; 2]> {
    let start = (
        node(dom, args, first)?,
        number(args, first + 1, context)? as u32,
    );
    let end = (
        node(dom, args, first + 2)?,
        number(args, first + 3, context)? as u32,
    );
    Ok([start, end])
}

fn strings(value: &JsValue, context: &mut Context) -> JsResult<Option<Vec<String>>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let length = object
        .get(js_string!("length"), context)?
        .to_length(context)?;
    let mut items = Vec::new();
    for index in 0..length {
        let item = object.get(index, context)?.to_string(context)?;
        items.push(item.to_std_string_escaped());
    }
    Ok(Some(items))
}

/// The DOM natives by name.
pub(crate) const DOM: &[(&str, DomOp)] = &[
    ("ownerDocument", |dom, args, _| {
        let node = node(dom, args, 0)?;
        Ok(id(dom.borrow().owner_document(node)))
    }),
    ("setOwnerDocument", |dom, args, _| {
        let node = node(dom, args, 0)?;
        let document = node_at(&dom.borrow(), args, 1)?;
        done(dom.borrow_mut().set_owner_document(node, document))
    }),
    ("historyBegin", history::begin),
    ("historyEnd", history::end),
    ("historyChanges", history::changes),
    ("historyRevert", history::revert),
    ("historyJoin", history::join),
    ("parseDocument", |dom, args, context| {
        let source = text(args, 0, context)?;
        Ok(JsValue::from(
            dom.borrow_mut().parse_html_document(&source).0,
        ))
    }),
    ("kind", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().node_type(id)))
    }),
    ("name", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().node_name(id).as_str()))
    }),
    ("localName", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(optional_string(dom.borrow().local_name(id)))
    }),
    ("namespace", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(optional_string(
            dom.borrow().namespace(id).map(Namespace::uri),
        ))
    }),
    ("parent", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().parent(id)))
    }),
    ("parentElement", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().parent_element(id)))
    }),
    ("root", |dom, args, _| {
        let mut id = node(dom, args, 0)?;
        let dom = dom.borrow();
        while let Some(parent) = dom.parent(id) {
            id = parent;
        }
        Ok(JsValue::from(id.0))
    }),
    ("firstChild", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().first_child(id)))
    }),
    ("lastChild", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().last_child(id)))
    }),
    ("previous", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().previous_sibling(id)))
    }),
    ("next", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().next_sibling(id)))
    }),
    ("previousElement", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().previous_element_sibling(id)))
    }),
    ("nextElement", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().next_element_sibling(id)))
    }),
    ("children", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let children = dom.borrow().children(id);
        Ok(ids(children, context))
    }),
    ("childCount", |dom, args, _| {
        collections::count(dom, args, false)
    }),
    ("childAt", |dom, args, context| {
        collections::at(dom, args, context, false)
    }),
    ("elementChildCount", |dom, args, _| {
        collections::count(dom, args, true)
    }),
    ("elementChildAt", |dom, args, context| {
        collections::at(dom, args, context, true)
    }),
    ("elementChildren", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let children = dom.borrow().child_elements(id);
        Ok(ids(children, context))
    }),
    ("insertBefore", |dom, args, _| {
        let parent = node(dom, args, 0)?;
        let child = node(dom, args, 1)?;
        let reference = optional_node(dom, args, 2)?;
        done(dom.borrow_mut().insert_before(parent, child, reference))
    }),
    ("removeChild", |dom, args, _| {
        let parent = node(dom, args, 0)?;
        let child = node(dom, args, 1)?;
        done(dom.borrow_mut().remove_child(parent, child))
    }),
    ("replaceChild", |dom, args, _| {
        let parent = node(dom, args, 0)?;
        let replacement = node(dom, args, 1)?;
        let child = node(dom, args, 2)?;
        done(dom.borrow_mut().replace_child(parent, replacement, child))
    }),
    ("remove", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().remove(id);
        unit()
    }),
    ("contains", |dom, args, _| {
        let ancestor = node(dom, args, 0)?;
        let other = node(dom, args, 1)?;
        Ok(JsValue::from(dom.borrow().contains(ancestor, other)))
    }),
    ("connected", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().is_connected(id)))
    }),
    ("attachShadow", |dom, args, context| {
        let host = node(dom, args, 0)?;
        let mode = if text(args, 1, context)? == "closed" {
            ShadowRootMode::Closed
        } else {
            ShadowRootMode::Open
        };
        let root = dom.borrow_mut().attach_shadow(host, mode, flag(args, 2));
        Ok(id(Some(root.map_err(failure)?)))
    }),
    ("shadowRoot", |dom, args, _| {
        let host = node(dom, args, 0)?;
        Ok(id(dom.borrow().shadow_root(host)))
    }),
    ("shadowHost", |dom, args, _| {
        let root = node(dom, args, 0)?;
        Ok(id(dom.borrow().shadow_host(root)))
    }),
    ("shadowMode", |dom, args, _| {
        let root = node(dom, args, 0)?;
        let data = dom.borrow().shadow_root_data(root);
        Ok(optional_string(data.map(|data| match data.mode {
            ShadowRootMode::Open => "open",
            ShadowRootMode::Closed => "closed",
        })))
    }),
    ("shadowDelegatesFocus", |dom, args, _| {
        let root = node(dom, args, 0)?;
        let data = dom.borrow().shadow_root_data(root);
        Ok(JsValue::from(data.is_some_and(|data| data.delegates_focus)))
    }),
    ("rootNode", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().root_node(id, flag(args, 1)).0))
    }),
    ("assignedSlot", |dom, args, _| {
        let id_ = node(dom, args, 0)?;
        Ok(id(dom.borrow().assigned_slot(id_)))
    }),
    ("assignedNodes", |dom, args, context| {
        let slot = node(dom, args, 0)?;
        let assigned = dom.borrow().assigned_nodes(slot, flag(args, 1));
        Ok(ids(assigned, context))
    }),
    ("retarget", |dom, args, _| {
        let target = node(dom, args, 0)?;
        let against = node(dom, args, 1)?;
        Ok(JsValue::from(dom.borrow().retarget(target, against).0))
    }),
    ("customCandidates", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let found = dom.borrow().custom_element_candidates(root);
        Ok(ids(found, context))
    }),
    ("defineCustom", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().set_custom_defined(id);
        Ok(JsValue::undefined())
    }),
    ("clone", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(
            dom.borrow_mut().clone_node(id, flag(args, 1)).0,
        ))
    }),
    ("text", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(optional_string(dom.borrow().text_content(id).as_deref()))
    }),
    ("setText", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let value = text(args, 1, context)?;
        dom.borrow_mut().set_text_content(id, &value);
        unit()
    }),
    ("data", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(optional_string(dom.borrow().data(id)))
    }),
    ("setData", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let value = text(args, 1, context)?;
        dom.borrow_mut().set_data(id, &value);
        unit()
    }),
    ("replaceData", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let offset = number(args, 1, context)? as u32;
        let count = number(args, 2, context)? as u32;
        let value = text(args, 3, context)?;
        done(dom.borrow_mut().replace_data(id, offset, count, &value))
    }),
    ("splitText", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let offset = number(args, 1, context)? as u32;
        let tail = dom.borrow_mut().split_text(id, offset).map_err(failure)?;
        Ok(JsValue::from(tail.0))
    }),
    ("normalize", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().normalize(id);
        unit()
    }),
    ("index", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().index(id)))
    }),
    ("createRange", |dom, args, context| {
        let [start, end] = boundaries(dom, args, 0, context)?;
        Ok(JsValue::from(dom.borrow_mut().create_range(start, end)))
    }),
    ("range", |dom, args, context| {
        let id = number(args, 0, context)? as u32;
        let range = dom.borrow().range(id);
        Ok(range.map_or(JsValue::null(), |[(start, from), (end, to)]| {
            let values = [start.0, from, end.0, to].map(JsValue::from);
            JsArray::from_iter(values, context).into()
        }))
    }),
    ("rangeField", ranges::field),
    ("setRange", |dom, args, context| {
        let id = number(args, 0, context)? as u32;
        let [start, end] = boundaries(dom, args, 1, context)?;
        dom.borrow_mut().set_range(id, start, end);
        unit()
    }),
    ("dropRange", |dom, args, context| {
        let id = number(args, 0, context)? as u32;
        dom.borrow_mut().drop_range(id);
        unit()
    }),
    ("setSelection", |dom, args, context| {
        let id = if args.get_or_undefined(0).is_null_or_undefined() {
            None
        } else {
            Some(number(args, 0, context)? as u32)
        };
        dom.borrow_mut().set_selection(id);
        unit()
    }),
    ("createElement", |dom, args, context| {
        let name = text(args, 0, context)?;
        let created = dom.borrow_mut().create_element(&name).map_err(failure)?;
        Ok(JsValue::from(created.0))
    }),
    ("createElementNS", |dom, args, context| {
        let namespace = text(args, 0, context)?;
        let name = text(args, 1, context)?;
        let created = dom
            .borrow_mut()
            .create_element_ns(Namespace::from_uri(&namespace), &name)
            .map_err(failure)?;
        Ok(JsValue::from(created.0))
    }),
    ("createText", |dom, args, context| {
        let value = text(args, 0, context)?;
        Ok(JsValue::from(dom.borrow_mut().create_text(&value).0))
    }),
    ("createComment", |dom, args, context| {
        let value = text(args, 0, context)?;
        Ok(JsValue::from(dom.borrow_mut().create_comment(&value).0))
    }),
    ("createFragment", |dom, _, _| {
        Ok(JsValue::from(dom.borrow_mut().create_fragment().0))
    }),
    ("createTarget", |dom, _, _| {
        Ok(JsValue::from(dom.borrow_mut().create_event_target().0))
    }),
    ("getAttribute", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        Ok(optional_string(dom.borrow().get_attribute(id, &name)))
    }),
    ("setAttribute", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        let value = text(args, 2, context)?;
        done(dom.borrow_mut().set_attribute(id, &name, &value))
    }),
    ("removeAttribute", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        dom.borrow_mut().remove_attribute(id, &name);
        unit()
    }),
    ("toggleAttribute", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        let force = (!args.get_or_undefined(2).is_undefined()).then(|| flag(args, 2));
        let present = dom
            .borrow_mut()
            .toggle_attribute(id, &name, force)
            .map_err(failure)?;
        Ok(JsValue::from(present))
    }),
    ("attributes", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let attributes = dom.borrow().attributes(id);
        let pairs: Vec<JsValue> = attributes
            .into_iter()
            .map(|(name, value)| {
                JsArray::from_iter([string(name.as_str()), string(value.as_str())], context).into()
            })
            .collect();
        Ok(JsArray::from_iter(pairs, context).into())
    }),
    ("style", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().inline_style(id).css_text().as_str()))
    }),
    ("styleGet", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        Ok(string(dom.borrow().inline_style(id).get(&name).as_str()))
    }),
    ("stylePriority", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        Ok(string(dom.borrow().inline_style(id).priority(&name)))
    }),
    ("styleSet", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        let value = text(args, 2, context)?;
        let important = flag(args, 3);
        dom.borrow_mut()
            .update_inline_style(id, &mut |style| style.set(&name, &value, important));
        unit()
    }),
    ("styleRemove", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        let mut removed = String::new();
        dom.borrow_mut()
            .update_inline_style(id, &mut |style| removed = style.remove(&name));
        Ok(string(removed.as_str()))
    }),
    ("styleNames", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let style = dom.borrow().inline_style(id);
        let names: Vec<JsValue> = (0..style.len())
            .filter_map(|index| style.item(index))
            .map(string)
            .collect();
        Ok(JsArray::from_iter(names, context).into())
    }),
    ("value", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().value(id).as_str()))
    }),
    ("setValue", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let value = text(args, 1, context)?;
        dom.borrow_mut().set_value(id, &value);
        unit()
    }),
    ("checked", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().checked(id)))
    }),
    ("setChecked", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().set_checked(id, flag(args, 1));
        unit()
    }),
    ("indeterminate", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().indeterminate(id)))
    }),
    ("setIndeterminate", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().set_indeterminate(id, flag(args, 1));
        unit()
    }),
    ("options", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let options = dom.borrow().options(id);
        Ok(ids(options, context))
    }),
    ("optionSelected", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().option_selected(id)))
    }),
    ("setOptionSelected", |dom, args, _| {
        let id = node(dom, args, 0)?;
        dom.borrow_mut().set_option_selected(id, flag(args, 1));
        unit()
    }),
    ("selectedIndex", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().selected_index(id)))
    }),
    ("setSelectedIndex", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let index = number(args, 1, context)?;
        dom.borrow_mut().set_selected_index(id, index as i32);
        unit()
    }),
    ("formOwner", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(self::id(dom.borrow().form_owner(id)))
    }),
    ("validity", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(u32::from(dom.borrow().validity(id))))
    }),
    ("validationMessage", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().validation_message(id).as_str()))
    }),
    ("setCustomValidity", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let message = text(args, 1, context)?;
        dom.borrow_mut().set_custom_validity(id, &message);
        unit()
    }),
    ("willValidate", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().will_validate(id)))
    }),
    ("innerHTML", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().inner_html(id).as_str()))
    }),
    ("setInnerHTML", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let html = text(args, 1, context)?;
        dom.borrow_mut().set_inner_html(id, &html);
        unit()
    }),
    ("outerHTML", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(string(dom.borrow().outer_html(id).as_str()))
    }),
    ("parseFragment", |dom, args, context| {
        let html = text(args, 0, context)?;
        let context_node = node(dom, args, 1)?;
        Ok(JsValue::from(
            dom.borrow_mut().parse_html_fragment(&html, context_node).0,
        ))
    }),
    ("querySelector", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let selectors = text(args, 1, context)?;
        let found = dom
            .borrow()
            .query_selector(root, &selectors)
            .map_err(failure)?;
        Ok(id(found))
    }),
    ("querySelectorAll", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let selectors = text(args, 1, context)?;
        let found = dom
            .borrow()
            .query_selector_all(root, &selectors)
            .map_err(failure)?;
        Ok(ids(found, context))
    }),
    ("matches", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let selectors = text(args, 1, context)?;
        let matched = dom.borrow().matches(id, &selectors).map_err(failure)?;
        Ok(JsValue::from(matched))
    }),
    ("closest", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let selectors = text(args, 1, context)?;
        let found = dom.borrow().closest(id, &selectors).map_err(failure)?;
        Ok(self::id(found))
    }),
    ("byId", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        Ok(id(dom.borrow().get_element_by_id(root, &name)))
    }),
    ("byTag", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let name = text(args, 1, context)?;
        let found = dom.borrow().get_elements_by_tag_name(root, &name);
        Ok(ids(found, context))
    }),
    ("byClass", |dom, args, context| {
        let root = node(dom, args, 0)?;
        let names = text(args, 1, context)?;
        let found = dom.borrow().get_elements_by_class_name(root, &names);
        Ok(ids(found, context))
    }),
    ("focused", |dom, _, _| Ok(id(dom.borrow().focused()))),
    ("tracing", |_, _, _| Ok(JsValue::from(false))),
    ("dumpTarget", |_, _, _| Ok(JsValue::null())),
    ("writeDump", |_, _, _| Ok(JsValue::undefined())),
    ("trace", |_, _, _| Ok(JsValue::undefined())),
    ("focusable", |dom, args, _| {
        let id = node(dom, args, 0)?;
        Ok(JsValue::from(dom.borrow().is_focusable(id)))
    }),
    ("editingHost", |dom, args, _| {
        let node = node(dom, args, 0)?;
        Ok(id(dom.borrow().editing_host(node)))
    }),
    ("setFocus", |dom, args, context| {
        let target = optional_node(dom, args, 0)?;
        let changed = dom.borrow_mut().set_focus(target, flag(args, 1));
        Ok(match changed {
            Some(previous) => JsArray::from_iter([id(previous)], context).into(),
            None => JsValue::null(),
        })
    }),
    ("scroll", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let (left, top) = dom.borrow().scroll(id);
        Ok(numbers(&[left, top], context))
    }),
    ("setScroll", |dom, args, context| {
        let id = node(dom, args, 0)?;
        let left = number(args, 1, context)?;
        let top = number(args, 2, context)?;
        dom.borrow_mut()
            .set_scroll(id, left.max(0.0) as f32, top.max(0.0) as f32);
        unit()
    }),
    ("takeScrolled", |dom, _, context| {
        let scrolled = dom.borrow_mut().take_scrolled();
        if scrolled.is_empty() {
            return Ok(JsValue::null());
        }
        let ids = scrolled.into_iter().map(|node| JsValue::from(node.0));
        Ok(JsArray::from_iter(ids, context).into())
    }),
    ("hasScrolled", |dom, _, _| {
        Ok(JsValue::from(dom.borrow().has_scrolled()))
    }),
    ("takeDetailsToggles", |dom, _, context| {
        let changes = dom.borrow_mut().take_details_toggles();
        if changes.is_empty() {
            return Ok(JsValue::null());
        }
        let mut records = Vec::new();
        for (node, old, new) in changes {
            let fields = [
                JsValue::from(node.0),
                JsValue::from(old),
                JsValue::from(new),
            ];
            records.push(JsValue::from(JsArray::from_iter(fields, context)));
        }
        Ok(JsArray::from_iter(records, context).into())
    }),
    ("createObserver", |dom, _, _| {
        Ok(JsValue::from(dom.borrow_mut().create_observer() as f64))
    }),
    ("observe", |dom, args, context| {
        let observer = number(args, 0, context)? as u64;
        let target = node(dom, args, 1)?;
        let options = ObserveOptions {
            child_list: flag(args, 2),
            attributes: flag(args, 3),
            character_data: flag(args, 4),
            subtree: flag(args, 5),
            attribute_old_value: flag(args, 6),
            character_data_old_value: flag(args, 7),
            attribute_filter: strings(args.get_or_undefined(8), context)?,
        };
        dom.borrow_mut().observe(observer, target, options);
        unit()
    }),
    ("disconnect", |dom, args, context| {
        let observer = number(args, 0, context)? as u64;
        dom.borrow_mut().disconnect(observer);
        unit()
    }),
    ("takeRecords", |dom, args, context| {
        let observer = number(args, 0, context)? as u64;
        let records = dom.borrow_mut().take_records(observer);
        let records: Vec<JsValue> = records
            .into_iter()
            .map(|record| {
                let kind = match record.kind {
                    MutationKind::ChildList => "childList",
                    MutationKind::Attributes => "attributes",
                    MutationKind::CharacterData => "characterData",
                };
                let fields = [
                    string(kind),
                    JsValue::from(record.target.0),
                    ids(record.added, context),
                    ids(record.removed, context),
                    id(record.previous_sibling),
                    id(record.next_sibling),
                    optional_string(record.attribute_name.as_deref()),
                    optional_string(record.old_value.as_deref()),
                ];
                JsArray::from_iter(fields, context).into()
            })
            .collect();
        Ok(JsArray::from_iter(records, context).into())
    }),
    ("pendingObservers", |dom, _, context| {
        let pending: Vec<JsValue> = dom
            .borrow()
            .pending_observers()
            .into_iter()
            .map(|observer| JsValue::from(observer as f64))
            .collect();
        Ok(JsArray::from_iter(pending, context).into())
    }),
    ("documentElement", |dom, _, _| {
        Ok(id(dom.borrow().document_element()))
    }),
    ("htmlChild", |dom, args, context| {
        let name = text(args, 0, context)?;
        Ok(id(dom.borrow().html_child(&name)))
    }),
];

/// The page natives by name.
pub(crate) const PAGE: &[(&str, PageOp)] = &[
    ("url", |page, _, _| Ok(string(page.url().as_str()))),
    // Package scripts are compiled into native applications only.
    ("loadScript", |_, _, _| {
        Err(JsNativeError::error()
            .with_message("scripts are compiled into native applications only")
            .into())
    }),
    ("rect", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        Ok(rect(page.rect(NodeId(id)), context))
    }),
    ("client", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        Ok(rect(page.client(NodeId(id)), context))
    }),
    ("bbox", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        Ok(rect(page.bbox(NodeId(id)), context))
    }),
    ("scrollSize", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        Ok(page
            .scroll_size(NodeId(id))
            .map_or(JsValue::null(), |(width, height)| {
                numbers(&[width, height], context)
            }))
    }),
    ("computedStyleValue", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let name = text(args, 1, context)?;
        Ok(string(
            page.computed_style_value(NodeId(id), &name).as_str(),
        ))
    }),
    ("computedStyle", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let pairs: Vec<JsValue> = page
            .computed_style(NodeId(id))
            .into_iter()
            .map(|(name, value)| {
                JsArray::from_iter([string(name.as_str()), string(value.as_str())], context).into()
            })
            .collect();
        Ok(JsArray::from_iter(pairs, context).into())
    }),
    ("viewport", |page, _, context| {
        let (width, height) = page.viewport();
        Ok(numbers(
            &[width, height, page.device_pixel_ratio()],
            context,
        ))
    }),
    ("elementFromPoint", |page, args, context| {
        let x = number(args, 0, context)? as f32;
        let y = number(args, 1, context)? as f32;
        Ok(id(page.element_from_point(x, y)))
    }),
    ("caretFromPoint", |page, args, context| {
        let x = number(args, 0, context)? as f32;
        let y = number(args, 1, context)? as f32;
        Ok(page
            .caret_from_point(x, y)
            .map_or(JsValue::null(), |(node, offset)| {
                JsArray::from_iter([JsValue::from(node.0), JsValue::from(offset)], context).into()
            }))
    }),
    ("textRects", |page, args, context| {
        let node = number(args, 0, context)? as u32;
        let start = number(args, 1, context)? as u32;
        let end = number(args, 2, context)? as u32;
        let rects: Vec<JsValue> = page
            .text_rects(NodeId(node), start, end)
            .into_iter()
            .map(|value| rect(Some(value), context))
            .collect();
        Ok(JsArray::from_iter(rects, context).into())
    }),
    ("matchMedia", |page, args, context| {
        let query = text(args, 0, context)?;
        Ok(JsValue::from(page.match_media(&query)))
    }),
    ("completeComposition", |page, _, _| {
        Ok(page
            .complete_composition()
            .map_or(JsValue::null(), |text| JsString::from(text).into()))
    }),
    ("focus", |page, args, _| {
        let target = args
            .get_or_undefined(0)
            .as_number()
            .map(|number| NodeId(number as u32));
        page.focus(target);
        unit()
    }),
    ("scrollIntoView", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let block = ScrollAlign::parse(&text(args, 1, context)?);
        let inline = ScrollAlign::parse(&text(args, 2, context)?);
        page.scroll_into_view(NodeId(id), block, inline);
        unit()
    }),
    ("imageSize", |page, args, context| {
        let url = text(args, 0, context)?;
        Ok(page
            .image_size(&url)
            .map_or(JsValue::null(), |(width, height)| {
                numbers(&[width, height], context)
            }))
    }),
    // This environment loads no network images.
    ("takeWantedImages", |_, _, _| Ok(JsValue::null())),
    // It reads media as it is asked for, calling back at once.
    ("loadMedia", |page, args, context| {
        let url = text(args, 0, context)?;
        let value = match page.load_media(&url) {
            Some((duration, frame)) => {
                let mut items = vec![JsValue::from(duration)];
                if let Some((width, height, pixels)) = frame {
                    items.push(JsValue::from(width));
                    items.push(JsValue::from(height));
                    items.push(JsUint8Array::from_iter(pixels, context)?.into());
                }
                JsArray::from_iter(items, context).into()
            }
            None => JsValue::null(),
        };
        let callback = args.get_or_undefined(1);
        if let Some(callback) = callback.as_callable() {
            callback.call(&JsValue::undefined(), &[value], context)?;
        }
        unit()
    }),
    ("addFrame", |page, args, context| {
        let key = text(args, 0, context)?;
        let width = number(args, 1, context)? as u32;
        let height = number(args, 2, context)? as u32;
        let pixels = args.get_or_undefined(3).to_object(context)?;
        let pixels = JsUint8Array::from_object(pixels)?.to_vec(context)?;
        Ok(JsValue::from(page.add_frame(&key, width, height, &pixels)))
    }),
    // Nor does it show notifications.
    ("imagePixels", |_, _, _| Ok(JsValue::null())),
    ("loadStorage", |page, _, _| {
        Ok(optional_string(page.load_storage().as_deref()))
    }),
    ("saveStorage", |page, args, context| {
        let data = text(args, 0, context)?;
        page.save_storage(&data);
        unit()
    }),
    ("sheetRules", |page, args, context| {
        let owner = number(args, 0, context)? as u32;
        let rules = text(args, 1, context)?;
        page.sheet_rules(NodeId(owner), &rules);
        unit()
    }),
    ("textSelection", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        Ok(page
            .text_selection(NodeId(id))
            .map_or(JsValue::null(), |(start, end)| {
                numbers(&[start as f32, end as f32], context)
            }))
    }),
    ("selectText", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let start = number(args, 1, context)? as u32;
        let end = number(args, 2, context)? as u32;
        page.select_text(NodeId(id), start, end);
        unit()
    }),
    ("readClipboard", |page, _, _| {
        Ok(optional_string(page.read_clipboard().as_deref()))
    }),
    ("readClipboardContents", |page, _, context| {
        let Some(contents) = page.read_clipboard_contents() else {
            return Ok(JsValue::null());
        };
        let pairs: Vec<JsValue> = contents
            .into_iter()
            .map(|(name, data)| {
                JsArray::from_iter([string(name.as_str()), string(data.as_str())], context).into()
            })
            .collect();
        Ok(JsArray::from_iter(pairs, context).into())
    }),
    ("writeClipboardContents", |page, args, context| {
        let plain = if args.get_or_undefined(0).is_null_or_undefined() {
            None
        } else {
            Some(text(args, 0, context)?)
        };
        let html = if args.get_or_undefined(1).is_null_or_undefined() {
            None
        } else {
            Some(text(args, 1, context)?)
        };
        Ok(JsValue::from(page.write_clipboard_contents(
            plain.as_deref(),
            html.as_deref(),
        )))
    }),
    ("openSelect", |page, args, context| {
        page.open_select(NodeId(number(args, 0, context)? as u32));
        unit()
    }),
    ("writeClipboard", |page, args, context| {
        let data = text(args, 0, context)?;
        Ok(JsValue::from(page.write_clipboard(&data)))
    }),
    ("canvasDraw", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let width = number(args, 1, context)? as u32;
        let height = number(args, 2, context)? as u32;
        let commands = text(args, 3, context)?;
        page.canvas_draw(NodeId(id), width, height, &commands);
        unit()
    }),
    ("canvasRead", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let area = canvas_area(args, context)?;
        let pixels = page.canvas_read(NodeId(id), area);
        Ok(JsUint8Array::from_iter(pixels, context)?.into())
    }),
    ("canvasWrite", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let area = canvas_area(args, context)?;
        let pixels = args.get_or_undefined(5).to_object(context)?;
        let pixels = JsUint8Array::from_object(pixels)?.to_vec(context)?;
        page.canvas_write(NodeId(id), area, &pixels);
        unit()
    }),
    ("canvasURL", |page, args, context| {
        let id = number(args, 0, context)? as u32;
        let jpeg = args.get_or_undefined(1).to_boolean();
        Ok(string(page.canvas_url(NodeId(id), jpeg).as_str()))
    }),
    ("parseColor", |page, args, context| {
        let color = text(args, 0, context)?;
        Ok(page
            .parse_color(&color)
            .map_or(JsValue::null(), |rgba| numbers(&rgba, context)))
    }),
    ("parseFont", |page, args, context| {
        let font = text(args, 0, context)?;
        Ok(optional_string(page.parse_font(&font).as_deref()))
    }),
    ("measureText", |page, args, context| {
        let font = text(args, 0, context)?;
        let content = text(args, 1, context)?;
        Ok(numbers(&page.measure_text(&font, &content), context))
    }),
    ("pathData", |page, args, context| {
        let data = text(args, 0, context)?;
        Ok(string(page.path_data(&data).as_str()))
    }),
];

/// The `x, y, width, height` arguments of canvas pixel natives.
fn canvas_area(args: &[JsValue], context: &mut Context) -> JsResult<(i32, i32, u32, u32)> {
    Ok((
        number(args, 1, context)? as i32,
        number(args, 2, context)? as i32,
        number(args, 3, context)? as u32,
        number(args, 4, context)? as u32,
    ))
}
