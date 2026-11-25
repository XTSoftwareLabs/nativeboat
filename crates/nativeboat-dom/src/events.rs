//! Event listeners and dispatch.
//!
//! [`dispatch`] implements the capture, target and bubble phases and calls
//! back into the binding layer for each listener without holding a borrow of
//! the DOM, so listeners may change the tree.

use crate::prelude::*;

use crate::{Dom, NodeId, NodeKind};
use core::cell::RefCell;

/// Options of `addEventListener`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListenerOptions {
    /// Listen in the capture phase.
    pub capture: bool,
    /// Remove the listener after its first call.
    pub once: bool,
    /// The listener does not cancel the event.
    pub passive: bool,
}

/// A registered listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    /// Registration id.
    pub id: u64,
    /// The binding layer's handle for the callback.
    pub callback: u64,
    /// The event type.
    pub event_type: String,
    /// Options.
    pub options: ListenerOptions,
}

/// `Event.eventPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// `NONE`
    #[default]
    None,
    /// `CAPTURING_PHASE`
    Capturing,
    /// `AT_TARGET`
    AtTarget,
    /// `BUBBLING_PHASE`
    Bubbling,
}

impl Phase {
    /// The numeric `eventPhase` value.
    pub fn value(self) -> u16 {
        match self {
            Phase::None => 0,
            Phase::Capturing => 1,
            Phase::AtTarget => 2,
            Phase::Bubbling => 3,
        }
    }
}

/// The state of an event.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Event {
    /// `type`
    pub event_type: String,
    /// `bubbles`
    pub bubbles: bool,
    /// `cancelable`
    pub cancelable: bool,
    /// `target`
    pub target: Option<NodeId>,
    /// `currentTarget`
    pub current_target: Option<NodeId>,
    /// `eventPhase`
    pub phase: Phase,
    /// `stopPropagation` was called.
    pub propagation_stopped: bool,
    /// `stopImmediatePropagation` was called.
    pub immediate_stopped: bool,
    /// `defaultPrevented`
    pub canceled: bool,
    /// Whether the current listener is passive.
    pub in_passive_listener: bool,
    /// Whether dispatch is in progress.
    pub dispatching: bool,
    /// `isTrusted`: created by the runtime rather than script.
    pub trusted: bool,
}

impl Event {
    /// Creates an event.
    pub fn new(event_type: &str, bubbles: bool, cancelable: bool) -> Event {
        Event {
            event_type: event_type.to_owned(),
            bubbles,
            cancelable,
            ..Event::default()
        }
    }

    /// `preventDefault`.
    pub fn prevent_default(&mut self) {
        if self.cancelable && !self.in_passive_listener {
            self.canceled = true;
        }
    }

    /// `stopPropagation`.
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    /// `stopImmediatePropagation`.
    pub fn stop_immediate_propagation(&mut self) {
        self.propagation_stopped = true;
        self.immediate_stopped = true;
    }
}

impl Dom {
    /// `addEventListener`; returns false when an equal listener exists.
    pub fn add_event_listener(
        &mut self,
        target: NodeId,
        event_type: &str,
        callback: u64,
        options: ListenerOptions,
    ) -> bool {
        let listeners = &self.node(target).listeners;
        let duplicate = listeners.iter().any(|listener| {
            listener.callback == callback
                && listener.event_type == event_type
                && listener.options.capture == options.capture
        });
        if duplicate {
            return false;
        }
        let id = self.next_listener;
        self.next_listener += 1;
        self.node_mut(target).listeners.push(Listener {
            id,
            callback,
            event_type: event_type.to_owned(),
            options,
        });
        true
    }

    /// `removeEventListener`; returns the callback handle that was removed.
    pub fn remove_event_listener(
        &mut self,
        target: NodeId,
        event_type: &str,
        callback: u64,
        capture: bool,
    ) -> Option<u64> {
        let listeners = &mut self.node_mut(target).listeners;
        let position = listeners.iter().position(|listener| {
            listener.callback == callback
                && listener.event_type == event_type
                && listener.options.capture == capture
        })?;
        Some(listeners.remove(position).callback)
    }

    fn remove_listener_by_id(&mut self, target: NodeId, id: u64) {
        self.node_mut(target)
            .listeners
            .retain(|listener| listener.id != id);
    }

    /// The listeners registered on a target.
    pub fn listeners(&self, target: NodeId) -> &[Listener] {
        &self.node(target).listeners
    }

    /// Whether any node has a listener for `event_type`.
    pub fn has_listeners(&self, event_type: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.listeners
                .iter()
                .any(|listener| listener.event_type == event_type)
        })
    }

    /// The propagation path: the target, its ancestors, and the window when
    /// the target is in the document.
    pub fn event_path(&self, target: NodeId) -> Vec<NodeId> {
        if matches!(self.kind(target), NodeKind::Window | NodeKind::EventTarget) {
            return vec![target];
        }
        let mut path = vec![target];
        let mut current = self.parent(target);
        while let Some(node) = current {
            path.push(node);
            current = self.parent(node);
        }
        if path.last() == Some(&Dom::DOCUMENT) {
            path.push(Dom::WINDOW);
        }
        path
    }
}

fn invoke_listeners(
    dom: &RefCell<Dom>,
    event: &RefCell<Event>,
    node: NodeId,
    phase: Phase,
    invoke: &mut dyn FnMut(&Listener, NodeId),
) {
    let event_type = event.borrow().event_type.clone();
    let snapshot: Vec<Listener> = dom
        .borrow()
        .listeners(node)
        .iter()
        .filter(|listener| {
            listener.event_type == event_type
                && match phase {
                    Phase::Capturing => listener.options.capture,
                    Phase::Bubbling => !listener.options.capture,
                    _ => true,
                }
        })
        .cloned()
        .collect();
    let mut ordered = snapshot;
    if phase == Phase::AtTarget {
        ordered.sort_by_key(|listener| !listener.options.capture);
    }
    for listener in ordered {
        let still_registered = dom
            .borrow()
            .listeners(node)
            .iter()
            .any(|current| current.id == listener.id);
        if !still_registered {
            continue;
        }
        if listener.options.once {
            dom.borrow_mut().remove_listener_by_id(node, listener.id);
        }
        {
            let mut state = event.borrow_mut();
            state.current_target = Some(node);
            state.phase = phase;
            state.in_passive_listener = listener.options.passive;
        }
        invoke(&listener, node);
        let mut state = event.borrow_mut();
        state.in_passive_listener = false;
        if state.immediate_stopped {
            return;
        }
    }
}

/// Dispatches an event; returns false when it was canceled.
pub fn dispatch(
    dom: &RefCell<Dom>,
    target: NodeId,
    event: &RefCell<Event>,
    invoke: &mut dyn FnMut(&Listener, NodeId),
) -> bool {
    let path = dom.borrow().event_path(target);
    {
        let mut state = event.borrow_mut();
        state.target = Some(target);
        state.dispatching = true;
    }
    let stopped = |event: &RefCell<Event>| event.borrow().propagation_stopped;
    for node in path.iter().skip(1).rev() {
        if stopped(event) {
            break;
        }
        invoke_listeners(dom, event, *node, Phase::Capturing, invoke);
    }
    if !stopped(event) {
        invoke_listeners(dom, event, target, Phase::AtTarget, invoke);
    }
    if event.borrow().bubbles {
        for node in path.iter().skip(1) {
            if stopped(event) {
                break;
            }
            invoke_listeners(dom, event, *node, Phase::Bubbling, invoke);
        }
    }
    let mut state = event.borrow_mut();
    state.phase = Phase::None;
    state.current_target = None;
    state.dispatching = false;
    !state.canceled
}

#[cfg(test)]
mod tests;
