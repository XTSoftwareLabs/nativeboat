//! Synchronous Windows hit queries against the last submitted native scene.

use nativeboat_dom::{Dom, NodeId};
use nativeboat_layout::Scene;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub(super) struct State {
    pub scene: Option<Rc<Scene>>,
    pub dom: Rc<RefCell<Dom>>,
}

/// A native geometry probe. It runs no application code, recomputes no layout
/// and does not keep a renderer alive after its window closes.
#[derive(Debug, Clone)]
pub struct NativeHitTest(pub(super) Weak<RefCell<State>>);

impl NativeHitTest {
    /// Whether another painted owner covers a native control at a window point
    /// in CSS pixels. The control and its descendants retain native input.
    pub fn above_control(&self, control: u64, x: f32, y: f32) -> bool {
        let Some(state) = self.0.upgrade() else {
            return false;
        };
        let Ok(state) = state.try_borrow() else {
            return false;
        };
        let Some(scene) = &state.scene else {
            return false;
        };
        let Ok(dom) = state.dom.try_borrow() else {
            return false;
        };
        let Ok(control) = u32::try_from(control) else {
            return false;
        };
        let control = NodeId(control);
        if scene.client_rect(control).is_none() || !dom.is_connected(control) {
            return false;
        }
        // The control's resize grip is painted above it.
        nativeboat_layout::resize::resizer_at(scene, &dom, x, y) == Some(control)
            || scene
                .hit_test(x, y)
                .is_some_and(|target| !dom.contains(control, target))
    }
}
