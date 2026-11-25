//! Modal dialogs: opening and saving files, choosing folders and message
//! boxes. Each runs its own message loop until the user answers.

use crate::WindowId;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::{narrow, wide};
use crate::window::hwnd;
use core::ptr;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_FORCESHOWHIDDEN, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_ERROR_ICON, TD_INFORMATION_ICON,
    TD_WARNING_ICON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDF_USE_HICON_MAIN, TDF_VERIFICATION_FLAG_CHECKED,
};
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDCANCEL, IDI_QUESTION, IDNO, IDOK, IDYES, LoadIconW, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MB_YESNOCANCEL, MessageBoxW,
};
use windows_sys::core::{BOOL, HRESULT, s};

/// A file type choice of a file dialog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileFilter {
    /// What the user sees, such as `Images`.
    pub name: String,
    /// Extensions without dots; `*` matches every file.
    pub extensions: Vec<String>,
}

/// What a file dialog asks for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDialog {
    /// The dialog title; empty for the default.
    pub title: String,
    /// The folder or file the dialog starts at; empty for the default.
    pub default_path: String,
    /// File type choices.
    pub filters: Vec<FileFilter>,
    /// Whether several files may be chosen.
    pub multiple: bool,
    /// Whether a folder is chosen instead of files.
    pub directory: bool,
    /// Whether a file name to save to is asked for.
    pub save: bool,
    /// Whether hidden files are listed.
    pub show_hidden: bool,
}

fn owner(window: Option<WindowId>) -> HWND {
    window.and_then(hwnd).unwrap_or(ptr::null_mut())
}

/// `name\0*.a;*.b\0` pairs, ending with an empty string.
fn filter_list(filters: &[FileFilter]) -> Vec<u16> {
    let mut text = String::new();
    for filter in filters {
        let patterns: Vec<String> = filter
            .extensions
            .iter()
            .map(|extension| format!("*.{extension}"))
            .collect();
        text.push_str(&format!("{}\0{}\0", filter.name, patterns.join(";")));
    }
    text.push('\0');
    text.encode_utf16().collect()
}

/// The paths in a file dialog's buffer: one full path, or a folder followed
/// by file names.
fn chosen(buffer: &[u16]) -> Vec<String> {
    let parts: Vec<String> = buffer
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.split_first() {
        Some((folder, files)) if !files.is_empty() => files
            .iter()
            .map(|file| format!("{}\\{file}", folder.trim_end_matches('\\')))
            .collect(),
        _ => parts,
    }
}

fn choose_folder(owner: HWND, options: &FileDialog) -> Option<Vec<String>> {
    let title = wide(&options.title);
    let info = BROWSEINFOW {
        hwndOwner: owner,
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    // SAFETY: the structure and its title outlive the modal call.
    let list = unsafe { SHBrowseForFolderW(&info) };
    if list.is_null() {
        return None;
    }
    let mut path = vec![0_u16; 32768];
    // SAFETY: the item list came from the dialog and the buffer holds a path.
    let found = unsafe { SHGetPathFromIDListW(list, path.as_mut_ptr()) };
    // SAFETY: the dialog allocated the list with the COM allocator.
    unsafe { CoTaskMemFree(list.cast()) };
    (found != 0).then(|| vec![narrow(&path)])
}

/// Whether a path names an existing directory.
fn is_directory(path: &str) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
    };
    // SAFETY: the path is null-terminated.
    let attributes = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Shows a file or folder dialog; `None` when the user cancels.
pub fn file_dialog(window: Option<WindowId>, options: &FileDialog) -> Option<Vec<String>> {
    let owner = owner(window);
    if options.directory {
        return choose_folder(owner, options);
    }
    let is_folder = is_directory(&options.default_path);
    let mut buffer = vec![0_u16; 65536];
    if !is_folder {
        for (slot, unit) in buffer.iter_mut().zip(options.default_path.encode_utf16()) {
            *slot = unit;
        }
    }
    let folder = wide(if is_folder { &options.default_path } else { "" });
    // Without file types the dialog offers all files, as Electron's does.
    let all = [FileFilter {
        name: "All Files (*.*)".into(),
        extensions: vec!["*".into()],
    }];
    let filters = filter_list(if options.filters.is_empty() {
        &all
    } else {
        &options.filters
    });
    let title = wide(&options.title);
    let extension = options
        .filters
        .first()
        .and_then(|filter| filter.extensions.first())
        .filter(|extension| *extension != "*")
        .map(|extension| wide(extension));
    let mut flags = OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
    if options.save {
        flags |= OFN_OVERWRITEPROMPT;
    } else {
        flags |= OFN_FILEMUSTEXIST;
    }
    if options.multiple {
        flags |= OFN_ALLOWMULTISELECT;
    }
    if options.show_hidden {
        flags |= OFN_FORCESHOWHIDDEN;
    }
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filters.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrInitialDir: if is_folder {
            folder.as_ptr()
        } else {
            ptr::null()
        },
        lpstrTitle: if options.title.is_empty() {
            ptr::null()
        } else {
            title.as_ptr()
        },
        Flags: flags,
        lpstrDefExt: extension
            .as_ref()
            .map_or(ptr::null(), |extension| extension.as_ptr()),
        ..Default::default()
    };
    // SAFETY: the structure and every buffer it points at outlive the
    // modal call.
    let ok = unsafe {
        if options.save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    (ok != 0).then(|| chosen(&buffer))
}

/// The icon of a message box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageKind {
    /// No icon.
    #[default]
    None,
    /// Information.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
    /// A question.
    Question,
}

/// What a message box shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageBox {
    /// The icon.
    pub kind: MessageKind,
    /// The window title.
    pub title: String,
    /// The main message.
    pub message: String,
    /// Further text under the message.
    pub detail: String,
    /// Button labels, left to right; empty for a single `OK`.
    pub buttons: Vec<String>,
    /// The button that Enter presses.
    pub default_button: Option<usize>,
    /// The button that Escape and closing the box choose.
    pub cancel_button: Option<usize>,
    /// The label of a checkbox, if one is shown.
    pub checkbox: Option<String>,
    /// Whether the checkbox starts checked.
    pub checked: bool,
}

/// Button ids start here, clear of the standard command ids.
const FIRST_BUTTON: i32 = 100;

type TaskDialogIndirect =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;

/// `TaskDialogIndirect` from Common Controls 6, which only applications with
/// that dependency in their manifest load; others get `None`.
fn task_dialog() -> Option<TaskDialogIndirect> {
    let name = wide("comctl32.dll");
    // SAFETY: loads a system library by name.
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    // SAFETY: looks up an export by its NUL-terminated name.
    let address = unsafe { GetProcAddress(module, s!("TaskDialogIndirect")) }?;
    // SAFETY: the export has the documented TaskDialogIndirect signature.
    Some(unsafe {
        core::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirect>(address)
    })
}

/// The index of the answer and whether the checkbox is checked.
fn answer(button: i32, options: &MessageBox) -> usize {
    let count = options.buttons.len().max(1);
    match usize::try_from(button - FIRST_BUTTON) {
        Ok(index) if index < count => index,
        _ => options.cancel_button.unwrap_or(0),
    }
}

/// Shows a message box and returns the index of the chosen button and
/// whether its checkbox was checked.
pub fn message_box(window: Option<WindowId>, options: &MessageBox) -> (usize, bool) {
    let labels: Vec<Vec<u16>> = if options.buttons.is_empty() {
        vec![wide("OK")]
    } else {
        options.buttons.iter().map(|label| wide(label)).collect()
    };
    let owner = owner(window);
    let Some(show) = task_dialog() else {
        return fallback(owner, options);
    };
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| TASKDIALOG_BUTTON {
            nButtonID: FIRST_BUTTON + index as i32,
            pszButtonText: label.as_ptr(),
        })
        .collect();
    let title = wide(&options.title);
    let message = wide(&options.message);
    let detail = wide(&options.detail);
    let checkbox = options.checkbox.as_deref().map(wide);
    let mut flags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW;
    if options.checked {
        flags |= TDF_VERIFICATION_FLAG_CHECKED;
    }
    let icon = match options.kind {
        MessageKind::None => TASKDIALOGCONFIG_0 {
            pszMainIcon: ptr::null(),
        },
        MessageKind::Info => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_INFORMATION_ICON,
        },
        MessageKind::Warning => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_WARNING_ICON,
        },
        MessageKind::Error => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_ERROR_ICON,
        },
        MessageKind::Question => {
            flags |= TDF_USE_HICON_MAIN;
            // SAFETY: loads a stock system icon.
            let icon = unsafe { LoadIconW(ptr::null_mut(), IDI_QUESTION) };
            TASKDIALOGCONFIG_0 { hMainIcon: icon }
        }
    };
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: flags,
        pszWindowTitle: title.as_ptr(),
        Anonymous1: icon,
        pszMainInstruction: message.as_ptr(),
        pszContent: if options.detail.is_empty() {
            ptr::null()
        } else {
            detail.as_ptr()
        },
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: FIRST_BUTTON + options.default_button.unwrap_or(0) as i32,
        pszVerificationText: checkbox.as_ref().map_or(ptr::null(), |text| text.as_ptr()),
        ..Default::default()
    };
    let mut button = 0;
    let mut checked: BOOL = 0;
    // SAFETY: the configuration and all strings and buttons it points at
    // outlive the modal call.
    let result = unsafe { show(&config, &mut button, ptr::null_mut(), &mut checked) };
    if result < 0 {
        return fallback(owner, options);
    }
    (answer(button, options), checked != 0)
}

/// A plain message box for applications without Common Controls 6: the
/// labels map onto OK, OK/Cancel or Yes/No/Cancel by their number.
fn fallback(owner: HWND, options: &MessageBox) -> (usize, bool) {
    let text = if options.detail.is_empty() {
        options.message.clone()
    } else {
        format!("{}\n\n{}", options.message, options.detail)
    };
    let icon = match options.kind {
        MessageKind::None => 0,
        MessageKind::Info => MB_ICONINFORMATION,
        MessageKind::Warning => MB_ICONWARNING,
        MessageKind::Error => MB_ICONERROR,
        MessageKind::Question => MB_ICONQUESTION,
    };
    let buttons = match options.buttons.len() {
        0 | 1 => MB_OK,
        2 => MB_OKCANCEL,
        _ => MB_YESNOCANCEL,
    };
    let text = wide(&text);
    let title = wide(&options.title);
    // SAFETY: the strings outlive the modal call.
    let result = unsafe { MessageBoxW(owner, text.as_ptr(), title.as_ptr(), buttons | icon) };
    let index = match result {
        IDOK | IDYES => Some(0),
        IDNO => Some(1),
        IDCANCEL if options.buttons.len() == 2 => Some(1),
        IDCANCEL if options.buttons.len() > 2 => Some(2),
        _ => options.cancel_button,
    };
    (index.unwrap_or(0), options.checked)
}

/// Shows an error message box without an owner.
pub fn error_box(title: &str, content: &str) {
    let title = wide(title);
    let content = wide(content);
    // SAFETY: the strings outlive the modal call.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            content.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

#[cfg(test)]
mod tests;
