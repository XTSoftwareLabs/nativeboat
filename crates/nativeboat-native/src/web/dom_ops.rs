//! The document natives: tree, text, attributes, inline style, form
//! controls, markup, selectors, focus, scrolling, ranges and mutation
//! observers, by node id.

use super::{
    DomOp, arg, done, failure, flag, history, id, ids, node, node_at, number, numbers,
    optional_node, optional_string, pairs, string, strings, text, unit,
};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, ErrorKind, JsStr, Realm, Value};
use core::cell::RefCell;
use nativeboat_dom::{
    Boundary, Dom, MutationKind, Namespace, NodeId, ObserveOptions, ShadowRootMode,
};

fn count(value: u32) -> Value {
    Value::Number(f64::from(value))
}

/// Two boundary points given as node, offset, node, offset from `first` on.
fn boundaries(
    realm: &mut Realm,
    dom: &RefCell<Dom>,
    args: &[Value],
    first: usize,
) -> Result<[Boundary; 2], crate::Thrown> {
    let start = (
        node(realm, dom, args, first)?,
        number(realm, args, first + 1)? as u32,
    );
    let end = (
        node(realm, dom, args, first + 2)?,
        number(realm, args, first + 3)? as u32,
    );
    Ok([start, end])
}

/// The children of `parent`, or its element children.
fn children(dom: &Dom, parent: NodeId, elements: bool) -> impl Iterator<Item = NodeId> + '_ {
    core::iter::successors(dom.first_child(parent), |node| dom.next_sibling(*node))
        .filter(move |node| !elements || dom.is_element(*node))
}

/// The number of children, read without materializing them.
fn child_count(
    realm: &mut Realm,
    dom: &RefCell<Dom>,
    args: &[Value],
    elements: bool,
) -> Completion {
    let dom = dom.borrow();
    let parent = node_at(realm, &dom, args, 0)?;
    Ok(count(if elements {
        dom.element_child_count(parent)
    } else {
        dom.child_count(parent)
    }))
}

/// A child by index, read without materializing the list.
fn child_at(realm: &mut Realm, dom: &RefCell<Dom>, args: &[Value], elements: bool) -> Completion {
    let index = number(realm, args, 1)?;
    if !index.is_finite() || index < 0.0 || index > f64::from(u32::MAX) {
        return Ok(Value::Null);
    }
    let dom = dom.borrow();
    let parent = node_at(realm, &dom, args, 0)?;
    Ok(id(children(&dom, parent, elements).nth(index as usize)))
}

/// A field of a range: its boundaries, whether it is collapsed, or its
/// common ancestor.
fn range_field(dom: &RefCell<Dom>, realm: &mut Realm, args: &[Value]) -> Completion {
    let dom = dom.borrow();
    let range = match arg(args, 0) {
        Value::Number(id)
            if crate::float::fract(*id) == 0.0 && *id >= 0.0 && *id <= f64::from(u32::MAX) =>
        {
            dom.range(*id as u32)
        }
        _ => None,
    }
    .ok_or_else(|| realm.throw(ErrorKind::Type, "not a range"))?;
    let field = match arg(args, 1) {
        Value::Number(field)
            if crate::float::fract(*field) == 0.0 && (0.0..=5.0).contains(field) =>
        {
            *field as u32
        }
        _ => return Err(realm.throw(ErrorKind::Range, "invalid range field")),
    };
    let [(start, from), (end, to)] = range;
    Ok(match field {
        0 => Value::Number(f64::from(start.0)),
        1 => Value::Number(f64::from(from)),
        2 => Value::Number(f64::from(end.0)),
        3 => Value::Number(f64::from(to)),
        4 => Value::Boolean(start == end && from == to),
        _ => id(
            core::iter::successors(Some(start), |node| dom.parent(*node))
                .find(|node| dom.contains(*node, end)),
        ),
    })
}

/// The DOM natives by name.
pub(super) const OPERATIONS: &[(&str, DomOp)] = &[
    ("ownerDocument", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().owner_document(node)))
    }),
    ("setOwnerDocument", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let document = node_at(realm, &dom.borrow(), args, 1)?;
        let result = dom.borrow_mut().set_owner_document(node, document);
        done(realm, result)
    }),
    ("historyBegin", history::begin),
    ("historyEnd", history::end),
    ("historyChanges", history::changes),
    ("historyRevert", history::revert),
    ("historyJoin", history::join),
    ("parseDocument", |dom, realm, args| {
        let source = text(realm, args, 0)?;
        Ok(id(Some(dom.borrow_mut().parse_html_document(&source))))
    }),
    ("kind", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Number(f64::from(dom.borrow().node_type(node))))
    }),
    ("name", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().node_name(node).as_str()))
    }),
    ("localName", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(optional_string(dom.borrow().local_name(node)))
    }),
    ("namespace", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(optional_string(
            dom.borrow().namespace(node).map(Namespace::uri),
        ))
    }),
    ("parent", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().parent(node)))
    }),
    ("parentElement", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().parent_element(node)))
    }),
    ("root", |dom, realm, args| {
        let mut node = node(realm, dom, args, 0)?;
        let dom = dom.borrow();
        while let Some(parent) = dom.parent(node) {
            node = parent;
        }
        Ok(id(Some(node)))
    }),
    ("firstChild", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().first_child(node)))
    }),
    ("lastChild", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().last_child(node)))
    }),
    ("previous", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().previous_sibling(node)))
    }),
    ("next", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().next_sibling(node)))
    }),
    ("previousElement", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().previous_element_sibling(node)))
    }),
    ("nextElement", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().next_element_sibling(node)))
    }),
    ("children", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let children = dom.borrow().children(node);
        Ok(ids(realm, children))
    }),
    ("childCount", |dom, realm, args| {
        child_count(realm, dom, args, false)
    }),
    ("childAt", |dom, realm, args| {
        child_at(realm, dom, args, false)
    }),
    ("elementChildCount", |dom, realm, args| {
        child_count(realm, dom, args, true)
    }),
    ("elementChildAt", |dom, realm, args| {
        child_at(realm, dom, args, true)
    }),
    ("elementChildren", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let children = dom.borrow().child_elements(node);
        Ok(ids(realm, children))
    }),
    ("insertBefore", |dom, realm, args| {
        let parent = node(realm, dom, args, 0)?;
        let child = node(realm, dom, args, 1)?;
        let reference = optional_node(realm, dom, args, 2)?;
        let result = dom.borrow_mut().insert_before(parent, child, reference);
        done(realm, result)
    }),
    ("removeChild", |dom, realm, args| {
        let parent = node(realm, dom, args, 0)?;
        let child = node(realm, dom, args, 1)?;
        let result = dom.borrow_mut().remove_child(parent, child);
        done(realm, result)
    }),
    ("replaceChild", |dom, realm, args| {
        let parent = node(realm, dom, args, 0)?;
        let replacement = node(realm, dom, args, 1)?;
        let child = node(realm, dom, args, 2)?;
        let result = dom.borrow_mut().replace_child(parent, replacement, child);
        done(realm, result)
    }),
    ("remove", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().remove(node);
        unit()
    }),
    ("contains", |dom, realm, args| {
        let ancestor = node(realm, dom, args, 0)?;
        let other = node(realm, dom, args, 1)?;
        Ok(Value::Boolean(dom.borrow().contains(ancestor, other)))
    }),
    ("connected", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().is_connected(node)))
    }),
    ("attachShadow", |dom, realm, args| {
        let host = node(realm, dom, args, 0)?;
        let mode = if text(realm, args, 1)? == "closed" {
            ShadowRootMode::Closed
        } else {
            ShadowRootMode::Open
        };
        let root = dom.borrow_mut().attach_shadow(host, mode, flag(args, 2));
        let root = root.map_err(|error| failure(realm, error))?;
        Ok(id(Some(root)))
    }),
    ("shadowRoot", |dom, realm, args| {
        let host = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().shadow_root(host)))
    }),
    ("shadowHost", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().shadow_host(root)))
    }),
    ("shadowMode", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let data = dom.borrow().shadow_root_data(root);
        Ok(optional_string(data.map(|data| match data.mode {
            ShadowRootMode::Open => "open",
            ShadowRootMode::Closed => "closed",
        })))
    }),
    ("shadowDelegatesFocus", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let data = dom.borrow().shadow_root_data(root);
        Ok(Value::Boolean(
            data.is_some_and(|data| data.delegates_focus),
        ))
    }),
    ("rootNode", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(Some(dom.borrow().root_node(node, flag(args, 1)))))
    }),
    ("assignedSlot", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().assigned_slot(node)))
    }),
    ("assignedNodes", |dom, realm, args| {
        let slot = node(realm, dom, args, 0)?;
        let assigned = dom.borrow().assigned_nodes(slot, flag(args, 1));
        Ok(ids(realm, assigned))
    }),
    ("retarget", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let against = node_at(realm, &dom.borrow(), args, 1)?;
        Ok(id(Some(dom.borrow().retarget(node, against))))
    }),
    ("customCandidates", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let found = dom.borrow().custom_element_candidates(root);
        Ok(ids(realm, found))
    }),
    ("defineCustom", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().set_custom_defined(node);
        unit()
    }),
    ("clone", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let clone = dom.borrow_mut().clone_node(node, flag(args, 1));
        Ok(id(Some(clone)))
    }),
    ("text", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(optional_string(dom.borrow().text_content(node).as_deref()))
    }),
    ("setText", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let value = text(realm, args, 1)?;
        dom.borrow_mut().set_text_content(node, &value);
        unit()
    }),
    ("data", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(optional_string(dom.borrow().data(node)))
    }),
    ("setData", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let value = text(realm, args, 1)?;
        dom.borrow_mut().set_data(node, &value);
        unit()
    }),
    ("replaceData", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let offset = number(realm, args, 1)? as u32;
        let count = number(realm, args, 2)? as u32;
        let value = text(realm, args, 3)?;
        let result = dom.borrow_mut().replace_data(node, offset, count, &value);
        done(realm, result)
    }),
    ("splitText", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let offset = number(realm, args, 1)? as u32;
        let tail = dom.borrow_mut().split_text(node, offset);
        let tail = tail.map_err(|error| failure(realm, error))?;
        Ok(id(Some(tail)))
    }),
    ("normalize", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().normalize(node);
        unit()
    }),
    ("index", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(count(dom.borrow().index(node)))
    }),
    ("createRange", |dom, realm, args| {
        let [start, end] = boundaries(realm, dom, args, 0)?;
        Ok(Value::Number(f64::from(
            dom.borrow_mut().create_range(start, end),
        )))
    }),
    ("range", |dom, realm, args| {
        let range = number(realm, args, 0)? as u32;
        let range = dom.borrow().range(range);
        Ok(range.map_or(Value::Null, |[(start, from), (end, to)]| {
            let values = [start.0, from, end.0, to]
                .map(|value| Value::Number(f64::from(value)))
                .to_vec();
            Value::Object(realm.array(values))
        }))
    }),
    ("rangeField", range_field),
    ("setRange", |dom, realm, args| {
        let range = number(realm, args, 0)? as u32;
        let [start, end] = boundaries(realm, dom, args, 1)?;
        dom.borrow_mut().set_range(range, start, end);
        unit()
    }),
    ("dropRange", |dom, realm, args| {
        let range = number(realm, args, 0)? as u32;
        dom.borrow_mut().drop_range(range);
        unit()
    }),
    ("setSelection", |dom, realm, args| {
        let range = match arg(args, 0) {
            Value::Undefined | Value::Null => None,
            _ => Some(number(realm, args, 0)? as u32),
        };
        dom.borrow_mut().set_selection(range);
        unit()
    }),
    ("createElement", |dom, realm, args| {
        let name = text(realm, args, 0)?;
        let created = dom.borrow_mut().create_element(&name);
        let created = created.map_err(|error| failure(realm, error))?;
        Ok(id(Some(created)))
    }),
    ("createElementNS", |dom, realm, args| {
        let namespace = text(realm, args, 0)?;
        let name = text(realm, args, 1)?;
        let created = dom
            .borrow_mut()
            .create_element_ns(Namespace::from_uri(&namespace), &name);
        let created = created.map_err(|error| failure(realm, error))?;
        Ok(id(Some(created)))
    }),
    ("createText", |dom, realm, args| {
        let value = text(realm, args, 0)?;
        Ok(id(Some(dom.borrow_mut().create_text(&value))))
    }),
    ("createComment", |dom, realm, args| {
        let value = text(realm, args, 0)?;
        Ok(id(Some(dom.borrow_mut().create_comment(&value))))
    }),
    ("createFragment", |dom, _, _| {
        Ok(id(Some(dom.borrow_mut().create_fragment())))
    }),
    ("createTarget", |dom, _, _| {
        Ok(id(Some(dom.borrow_mut().create_event_target())))
    }),
    ("getAttribute", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        Ok(optional_string(dom.borrow().get_attribute(node, &name)))
    }),
    ("setAttribute", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        let value = text(realm, args, 2)?;
        let result = dom.borrow_mut().set_attribute(node, &name, &value);
        done(realm, result)
    }),
    ("removeAttribute", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        dom.borrow_mut().remove_attribute(node, &name);
        unit()
    }),
    ("toggleAttribute", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        let force = (!matches!(arg(args, 2), Value::Undefined)).then(|| flag(args, 2));
        let present = dom.borrow_mut().toggle_attribute(node, &name, force);
        let present = present.map_err(|error| failure(realm, error))?;
        Ok(Value::Boolean(present))
    }),
    ("attributes", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let attributes = dom.borrow().attributes(node);
        Ok(pairs(realm, attributes))
    }),
    ("style", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().inline_style(node).css_text().as_str()))
    }),
    ("styleGet", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        Ok(string(dom.borrow().inline_style(node).get(&name).as_str()))
    }),
    ("stylePriority", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        Ok(string(dom.borrow().inline_style(node).priority(&name)))
    }),
    ("styleSet", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        let value = text(realm, args, 2)?;
        let important = flag(args, 3);
        dom.borrow_mut()
            .update_inline_style(node, &mut |style| style.set(&name, &value, important));
        unit()
    }),
    ("styleRemove", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        let mut removed = String::new();
        dom.borrow_mut()
            .update_inline_style(node, &mut |style| removed = style.remove(&name));
        Ok(string(&removed))
    }),
    ("styleNames", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let style = dom.borrow().inline_style(node);
        let names = (0..style.len())
            .filter_map(|index| style.item(index))
            .map(string)
            .collect();
        Ok(Value::Object(realm.array(names)))
    }),
    ("value", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().value(node).as_str()))
    }),
    ("setValue", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let value = text(realm, args, 1)?;
        dom.borrow_mut().set_value(node, &value);
        unit()
    }),
    ("checked", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().checked(node)))
    }),
    ("setChecked", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().set_checked(node, flag(args, 1));
        unit()
    }),
    ("indeterminate", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().indeterminate(node)))
    }),
    ("setIndeterminate", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().set_indeterminate(node, flag(args, 1));
        unit()
    }),
    ("options", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let options = dom.borrow().options(node);
        Ok(ids(realm, options))
    }),
    ("optionSelected", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().option_selected(node)))
    }),
    ("setOptionSelected", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        dom.borrow_mut().set_option_selected(node, flag(args, 1));
        unit()
    }),
    ("selectedIndex", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Number(f64::from(dom.borrow().selected_index(node))))
    }),
    ("setSelectedIndex", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let index = number(realm, args, 1)?;
        dom.borrow_mut().set_selected_index(node, index as i32);
        unit()
    }),
    ("formOwner", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().form_owner(node)))
    }),
    ("validity", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Number(f64::from(dom.borrow().validity(node))))
    }),
    ("validationMessage", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().validation_message(node).as_str()))
    }),
    ("setCustomValidity", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let message = text(realm, args, 1)?;
        dom.borrow_mut().set_custom_validity(node, &message);
        unit()
    }),
    ("willValidate", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().will_validate(node)))
    }),
    ("innerHTML", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().inner_html(node).as_str()))
    }),
    ("setInnerHTML", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let html = text(realm, args, 1)?;
        dom.borrow_mut().set_inner_html(node, &html);
        unit()
    }),
    ("outerHTML", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(string(dom.borrow().outer_html(node).as_str()))
    }),
    ("parseFragment", |dom, realm, args| {
        let html = text(realm, args, 0)?;
        let context = node(realm, dom, args, 1)?;
        Ok(id(Some(
            dom.borrow_mut().parse_html_fragment(&html, context),
        )))
    }),
    ("querySelector", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let selectors = text(realm, args, 1)?;
        let found = dom.borrow().query_selector(root, &selectors);
        let found = found.map_err(|error| failure(realm, error))?;
        Ok(id(found))
    }),
    ("querySelectorAll", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let selectors = text(realm, args, 1)?;
        let found = dom.borrow().query_selector_all(root, &selectors);
        let found = found.map_err(|error| failure(realm, error))?;
        Ok(ids(realm, found))
    }),
    ("matches", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let selectors = text(realm, args, 1)?;
        let matched = dom.borrow().matches(node, &selectors);
        let matched = matched.map_err(|error| failure(realm, error))?;
        Ok(Value::Boolean(matched))
    }),
    ("closest", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let selectors = text(realm, args, 1)?;
        let found = dom.borrow().closest(node, &selectors);
        let found = found.map_err(|error| failure(realm, error))?;
        Ok(id(found))
    }),
    ("byId", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        Ok(id(dom.borrow().get_element_by_id(root, &name)))
    }),
    ("byTag", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let name = text(realm, args, 1)?;
        let found = dom.borrow().get_elements_by_tag_name(root, &name);
        Ok(ids(realm, found))
    }),
    ("byClass", |dom, realm, args| {
        let root = node(realm, dom, args, 0)?;
        let names = text(realm, args, 1)?;
        let found = dom.borrow().get_elements_by_class_name(root, &names);
        Ok(ids(realm, found))
    }),
    ("focused", |dom, _, _| Ok(id(dom.borrow().focused()))),
    // The page's own input tracing, as NATIVEBOAT_TRACE_INPUT asks for it.
    ("tracing", |_, _, _| {
        Ok(Value::Boolean(crate::platform::input_traced()))
    }),
    ("trace", |_, realm, args| {
        if crate::platform::input_traced() {
            let line = text(realm, args, 0)?;
            crate::platform::write_stderr(&alloc::format!("nativeboat: page {line}\n"));
        }
        unit()
    }),
    // The file NATIVEBOAT_DUMP_DOM names for the page's markup, for
    // comparing documents with Chromium's.
    ("dumpTarget", |_, _, _| {
        let name = JsStr::from("NATIVEBOAT_DUMP_DOM");
        Ok(crate::platform::environment_value(&name)
            .filter(|path| !path.is_empty())
            .map_or(Value::Null, Value::String))
    }),
    ("writeDump", |_, realm, args| {
        let name = JsStr::from("NATIVEBOAT_DUMP_DOM");
        if let Some(path) = crate::platform::environment_value(&name) {
            let markup = text(realm, args, 0)?;
            crate::platform::write_file(&path.to_rust_lossy(), markup.as_bytes());
        }
        unit()
    }),
    ("focusable", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(Value::Boolean(dom.borrow().is_focusable(node)))
    }),
    ("editingHost", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        Ok(id(dom.borrow().editing_host(node)))
    }),
    ("setFocus", |dom, realm, args| {
        let target = optional_node(realm, dom, args, 0)?;
        let changed = dom.borrow_mut().set_focus(target, flag(args, 1));
        if let Some(previous) = changed {
            crate::platform::trace_focus(|| {
                let dom = dom.borrow();
                let describe = |node: Option<nativeboat_dom::NodeId>| {
                    node.and_then(|node| {
                        let name = dom.local_name(node)?;
                        let mut text = alloc::format!("<{name}");
                        for attribute in ["id", "class"] {
                            if let Some(value) = dom.get_attribute(node, attribute) {
                                text.push_str(&alloc::format!(" {attribute}=\"{value}\""));
                            }
                        }
                        text.push('>');
                        Some(text)
                    })
                    .unwrap_or_else(|| "none".into())
                };
                alloc::format!("{} -> {}", describe(previous), describe(target))
            });
        }
        Ok(match changed {
            Some(previous) => Value::Object(realm.array(vec![id(previous)])),
            None => Value::Null,
        })
    }),
    ("scroll", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let (left, top) = dom.borrow().scroll(node);
        Ok(numbers(realm, &[left, top]))
    }),
    ("setScroll", |dom, realm, args| {
        let node = node(realm, dom, args, 0)?;
        let left = number(realm, args, 1)?;
        let top = number(realm, args, 2)?;
        dom.borrow_mut()
            .set_scroll(node, left.max(0.0) as f32, top.max(0.0) as f32);
        unit()
    }),
    ("takeScrolled", |dom, realm, _| {
        let scrolled = dom.borrow_mut().take_scrolled();
        if scrolled.is_empty() {
            return Ok(Value::Null);
        }
        let ids = scrolled.into_iter().map(|node| id(Some(node))).collect();
        Ok(Value::Object(realm.array(ids)))
    }),
    ("hasScrolled", |dom, _, _| {
        Ok(Value::Boolean(dom.borrow().has_scrolled()))
    }),
    ("takeDetailsToggles", |dom, realm, _| {
        let changes = dom.borrow_mut().take_details_toggles();
        if changes.is_empty() {
            return Ok(Value::Null);
        }
        let records = changes
            .into_iter()
            .map(|(node, old, new)| {
                let fields = vec![id(Some(node)), Value::Boolean(old), Value::Boolean(new)];
                Value::Object(realm.array(fields))
            })
            .collect();
        Ok(Value::Object(realm.array(records)))
    }),
    ("createObserver", |dom, _, _| {
        Ok(Value::Number(dom.borrow_mut().create_observer() as f64))
    }),
    ("observe", |dom, realm, args| {
        let observer = number(realm, args, 0)? as u64;
        let target = node(realm, dom, args, 1)?;
        let options = ObserveOptions {
            child_list: flag(args, 2),
            attributes: flag(args, 3),
            character_data: flag(args, 4),
            subtree: flag(args, 5),
            attribute_old_value: flag(args, 6),
            character_data_old_value: flag(args, 7),
            attribute_filter: strings(realm, arg(args, 8))?,
        };
        dom.borrow_mut().observe(observer, target, options);
        unit()
    }),
    ("disconnect", |dom, realm, args| {
        let observer = number(realm, args, 0)? as u64;
        dom.borrow_mut().disconnect(observer);
        unit()
    }),
    ("takeRecords", |dom, realm, args| {
        let observer = number(realm, args, 0)? as u64;
        let records = dom.borrow_mut().take_records(observer);
        let records = records
            .into_iter()
            .map(|record| {
                let kind = match record.kind {
                    MutationKind::ChildList => "childList",
                    MutationKind::Attributes => "attributes",
                    MutationKind::CharacterData => "characterData",
                };
                let fields = vec![
                    string(kind),
                    id(Some(record.target)),
                    ids(realm, record.added),
                    ids(realm, record.removed),
                    id(record.previous_sibling),
                    id(record.next_sibling),
                    optional_string(record.attribute_name.as_deref()),
                    optional_string(record.old_value.as_deref()),
                ];
                Value::Object(realm.array(fields))
            })
            .collect();
        Ok(Value::Object(realm.array(records)))
    }),
    ("pendingObservers", |dom, realm, _| {
        let pending = dom
            .borrow()
            .pending_observers()
            .into_iter()
            .map(|observer| Value::Number(observer as f64))
            .collect();
        Ok(Value::Object(realm.array(pending)))
    }),
    ("documentElement", |dom, _, _| {
        Ok(id(dom.borrow().document_element()))
    }),
    ("htmlChild", |dom, realm, args| {
        let name = text(realm, args, 0)?;
        Ok(id(dom.borrow().html_child(&name)))
    }),
];
