//! Taskbar buttons, which Electron's `skipTaskbar` removes through the
//! shell's taskbar list, leaving the window's styles alone.

use super::Hwnd;
use crate::com::{Com, Guid, Hresult, S_OK};
use core::ffi::c_void;

const CLSID_TASKBAR_LIST: Guid = Guid {
    data1: 0x56fd_f344,
    data2: 0xfd6d,
    data3: 0x11d0,
    data4: [0x95, 0x8a, 0x00, 0x60, 0x97, 0xc9, 0xa0, 0x90],
};
const IID_TASKBAR_LIST: Guid = Guid {
    data1: 0x56fd_f342,
    data2: 0xfd6d,
    data3: 0x11d0,
    data4: [0x95, 0x8a, 0x00, 0x60, 0x97, 0xc9, 0xa0, 0x90],
};

// `ITaskbarList` slots after IUnknown's three.
const HR_INIT: usize = 3;
const ADD_TAB: usize = 4;
const DELETE_TAB: usize = 5;

/// Adds a window's taskbar button, or removes it.
pub(super) fn set_button(window: Hwnd, shown: bool) {
    let Some(list) = Com::create(&CLSID_TASKBAR_LIST, &IID_TASKBAR_LIST) else {
        return;
    };
    // SAFETY: the list is live and the slots have these signatures; the
    // taskbar ignores windows it has no button for.
    unsafe {
        if list.method::<unsafe extern "system" fn(*mut c_void) -> Hresult>(HR_INIT)(list.0) != S_OK
        {
            return;
        }
        let slot = if shown { ADD_TAB } else { DELETE_TAB };
        list.method::<unsafe extern "system" fn(*mut c_void, Hwnd) -> Hresult>(slot)(
            list.0, window,
        );
    }
}
