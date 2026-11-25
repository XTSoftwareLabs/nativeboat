//! Native Windows implementation of the Electron dialog calls used by apps.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;

#[repr(C)]
struct OpenFileNameW {
    size: u32,
    owner: Hwnd,
    instance: Hinstance,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    extension_offset: u16,
    default_extension: *const u16,
    custom_data: isize,
    hook: *mut c_void,
    template_name: *const u16,
    reserved: *mut c_void,
    reserved_dword: u32,
    flags_ex: u32,
}

#[repr(C)]
struct BrowseInfoW {
    owner: Hwnd,
    root: *mut c_void,
    display_name: *mut u16,
    title: *const u16,
    flags: u32,
    callback: *mut c_void,
    parameter: isize,
    image: i32,
}

#[repr(C, packed(1))]
struct TaskDialogButton {
    id: i32,
    text: *const u16,
}

#[repr(C, packed(1))]
struct TaskDialogConfig {
    size: u32,
    parent: Hwnd,
    instance: Hinstance,
    flags: u32,
    common_buttons: u32,
    window_title: *const u16,
    main_icon: *const c_void,
    main_instruction: *const u16,
    content: *const u16,
    button_count: u32,
    buttons: *const TaskDialogButton,
    default_button: i32,
    radio_count: u32,
    radios: *const TaskDialogButton,
    default_radio: i32,
    verification_text: *const u16,
    expanded_information: *const u16,
    expanded_control_text: *const u16,
    collapsed_control_text: *const u16,
    footer_icon: *const c_void,
    footer: *const u16,
    callback: *mut c_void,
    callback_data: isize,
    width: u32,
}

#[link(name = "comdlg32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetOpenFileNameW(options: *mut OpenFileNameW) -> i32;
    fn GetSaveFileNameW(options: *mut OpenFileNameW) -> i32;
    fn CommDlgExtendedError() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MessageBoxW(owner: Hwnd, text: *const u16, title: *const u16, kind: u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> Hinstance;
    fn GetProcAddress(module: Hinstance, name: *const u8) -> *const c_void;
    fn FreeLibrary(module: Hinstance) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHBrowseForFolderW(info: *mut BrowseInfoW) -> *mut c_void;
    fn SHGetPathFromIDListW(item: *mut c_void, path: *mut u16) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemFree(memory: *mut c_void);
}

type TaskDialogIndirectFn = unsafe extern "system" fn(
    config: *const TaskDialogConfig,
    button: *mut i32,
    radio: *mut i32,
    verification: *mut i32,
) -> i32;

const OFN_EXPLORER: u32 = 0x0008_0000;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_ALLOWMULTISELECT: u32 = 0x0000_0200;
const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
const BIF_RETURNONLYFSDIRS: u32 = 0x0001;
const BIF_NEWDIALOGSTYLE: u32 = 0x0040;
const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 8;
const TDF_USE_COMMAND_LINKS: u32 = 0x10;
const TDF_VERIFICATION_FLAG_CHECKED: u32 = 0x100;
const TDF_SIZE_TO_CONTENT: u32 = 0x0100_0000;
/// Electron's labels for the task dialog's own buttons, case aside: their
/// flags and the IDs the dialog returns for them.
const COMMON_BUTTONS: [(&str, u32, i32); 6] = [
    ("ok", 0x01, 1),
    ("yes", 0x02, 6),
    ("no", 0x04, 7),
    ("cancel", 0x08, 2),
    ("retry", 0x10, 4),
    ("close", 0x20, 8),
];
/// Custom buttons' IDs start here, past the IDs Windows uses.
const CUSTOM_BUTTON_ID: i32 = 100;
const TASKDIALOG_ERROR_ICON: *const c_void = 65534usize as *const c_void;
const TASKDIALOG_WARNING_ICON: *const c_void = 65535usize as *const c_void;
const TASKDIALOG_INFORMATION_ICON: *const c_void = 65533usize as *const c_void;
const MB_OK: u32 = 0;
const MB_ICONERROR: u32 = 0x10;
const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
const COMMON_CONTROLS: [u16; 13] = [
    b'c' as u16,
    b'o' as u16,
    b'm' as u16,
    b'c' as u16,
    b't' as u16,
    b'l' as u16,
    b'3' as u16,
    b'2' as u16,
    b'.' as u16,
    b'd' as u16,
    b'l' as u16,
    b'l' as u16,
    0,
];
const TASK_DIALOG_SYMBOL: &[u8] = b"TaskDialogIndirect\0";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FileOptions {
    properties: Vec<String>,
    title: Option<String>,
    default_path: Option<String>,
    filters: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MessageOptions {
    title: Option<String>,
    message: String,
    detail: Option<String>,
    kind: String,
    buttons: Vec<String>,
    /// The button chosen when the dialog opens; negative for none.
    default_id: i64,
    /// The response when the dialog is closed without a button.
    cancel_id: i64,
    /// Custom buttons as push buttons rather than command links.
    no_link: bool,
    checkbox_label: Option<String>,
    checkbox_checked: bool,
}

trait DialogBackend {
    fn choose_files(
        &self,
        owner: usize,
        options: &FileOptions,
        save: bool,
    ) -> Result<Vec<String>, u32>;
    /// The button chosen, and whether the checkbox was checked.
    fn message(&self, owner: usize, options: &MessageOptions) -> Result<(i64, bool), u32>;
    fn error_box(&self, owner: usize, title: &str, message: &str) -> Result<(), u32>;
}

struct WindowsDialogBackend;

impl DialogBackend for WindowsDialogBackend {
    fn choose_files(
        &self,
        owner: usize,
        options: &FileOptions,
        save: bool,
    ) -> Result<Vec<String>, u32> {
        if options
            .properties
            .iter()
            .any(|item| item == "openDirectory")
        {
            return choose_directory(owner, options).map(|path| path.into_iter().collect());
        }
        choose_file(owner, options, save)
    }

    fn message(&self, owner: usize, options: &MessageOptions) -> Result<(i64, bool), u32> {
        task_message(owner, options)
    }

    fn error_box(&self, owner: usize, title: &str, message: &str) -> Result<(), u32> {
        let title = wide(title);
        let message = wide(message);
        // SAFETY: both strings are NUL terminated and remain alive for the call.
        unsafe {
            MessageBoxW(
                owner as Hwnd,
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            )
        };
        Ok(())
    }
}

/// Builds the native Electron `dialog` module.
pub fn module(realm: &Realm) -> Object {
    module_with(realm, alloc::rc::Rc::new(WindowsDialogBackend))
}

fn module_with(realm: &Realm, backend: alloc::rc::Rc<dyn DialogBackend>) -> Object {
    let dialog = realm.object();
    for (name, save, sync) in [
        ("showOpenDialog", false, false),
        ("showOpenDialogSync", false, true),
        ("showSaveDialog", true, false),
        ("showSaveDialogSync", true, true),
    ] {
        let backend = backend.clone();
        dialog.define(
            Key::name(name),
            Property::data(Value::Object(realm.function(
                name,
                2,
                true,
                move |realm, _, args| file_dialog(realm, &*backend, args, name, save, sync),
            ))),
        );
    }
    for (name, sync) in [("showMessageBox", false), ("showMessageBoxSync", true)] {
        let backend = backend.clone();
        dialog.define(
            Key::name(name),
            Property::data(Value::Object(realm.function(
                name,
                2,
                true,
                move |realm, _, args| message_box(realm, &*backend, args, name, sync),
            ))),
        );
    }
    let error_backend = backend;
    dialog.define(
        Key::name("showErrorBox"),
        Property::data(Value::Object(realm.function(
            "showErrorBox",
            2,
            false,
            move |realm, _, args| {
                let title = string_arg(realm, args, 0, "title")?;
                let message = string_arg(realm, args, 1, "content")?;
                error_backend
                    .error_box(0, &title, &message)
                    .map_err(|code| dialog_error(realm, "showErrorBox", code))?;
                Ok(Value::Undefined)
            },
        ))),
    );
    dialog
}

/// `showOpenDialog`, `showSaveDialog` and their synchronous forms, which
/// return the paths, or the path, where the others resolve an object.
fn file_dialog(
    realm: &mut Realm,
    backend: &dyn DialogBackend,
    args: &[Value],
    method: &str,
    save: bool,
    sync: bool,
) -> Result<Value, Thrown> {
    let (owner, options) = owner_and_options(realm, args)?;
    let parsed = file_options(realm, &options)?;
    let paths = match backend.choose_files(owner, &parsed, save) {
        Ok(paths) => paths,
        Err(code) if sync => return Err(dialog_error(realm, method, code)),
        Err(code) => return rejected_dialog(realm, method, code),
    };
    // A canceled save dialog gives an empty path, a canceled open dialog no
    // paths at all.
    let path = Value::str(paths.first().map_or("", String::as_str));
    if sync {
        return Ok(match (save, paths.is_empty()) {
            (true, _) => path,
            (false, true) => Value::Undefined,
            (false, false) => Value::Object(string_array(realm, &paths)?),
        });
    }
    let result = realm.object();
    result.define(
        Key::name("canceled"),
        Property::data(Value::Boolean(paths.is_empty())),
    );
    if save {
        result.define(Key::name("filePath"), Property::data(path));
    } else {
        result.define(
            Key::name("filePaths"),
            Property::data(Value::Object(string_array(realm, &paths)?)),
        );
    }
    resolved(realm, Value::Object(result))
}

/// `showMessageBox`, which resolves the response and the checkbox's state,
/// and `showMessageBoxSync`, which returns the response.
fn message_box(
    realm: &mut Realm,
    backend: &dyn DialogBackend,
    args: &[Value],
    method: &str,
    sync: bool,
) -> Result<Value, Thrown> {
    let (owner, value) = owner_and_options(realm, args)?;
    let parsed = message_options(realm, &value)?;
    let (response, checked) = match backend.message(owner, &parsed) {
        Ok(chosen) => chosen,
        Err(code) if sync => return Err(dialog_error(realm, method, code)),
        Err(code) => return rejected_dialog(realm, method, code),
    };
    if sync {
        return Ok(Value::Number(response as f64));
    }
    let result = realm.object();
    result.define(
        Key::name("response"),
        Property::data(Value::Number(response as f64)),
    );
    result.define(
        Key::name("checkboxChecked"),
        Property::data(Value::Boolean(checked)),
    );
    resolved(realm, Value::Object(result))
}

fn owner_and_options(realm: &mut Realm, args: &[Value]) -> Result<(usize, Value), Thrown> {
    if args.len() > 1 {
        let owner = args[0]
            .as_object()
            .map(|window| {
                match ops::get(
                    realm,
                    &Value::Object(window.clone()),
                    &Key::Symbol(realm.electron_window_key.clone()),
                ) {
                    Ok(Value::Number(handle)) if handle.is_finite() && handle > 0.0 => {
                        handle as usize
                    }
                    _ => 0,
                }
            })
            .unwrap_or(0);
        Ok((owner, args.get(1).cloned().unwrap_or(Value::Undefined)))
    } else {
        Ok((0, args.first().cloned().unwrap_or(Value::Undefined)))
    }
}

fn file_options(realm: &mut Realm, value: &Value) -> Result<FileOptions, Thrown> {
    let mut options = FileOptions::default();
    let empty;
    let value = if matches!(value, Value::Undefined | Value::Null) {
        empty = Value::Object(realm.object());
        &empty
    } else if value.as_object().is_some() {
        value
    } else {
        return Err(realm.throw(ErrorKind::Type, "dialog options must be an object"));
    };
    options.title = optional_string(realm, value, "title")?;
    options.default_path = optional_string(realm, value, "defaultPath")?;
    let properties = ops::get_named(realm, value, "properties")?;
    for item in array_strings(realm, &properties)? {
        options.properties.push(item);
    }
    let filters = ops::get_named(realm, value, "filters")?;
    if let Some(filters) = filters.as_object() {
        let count =
            crate::array_builtin::length_of(realm, &Value::Object(filters.clone()))?.min(128);
        for i in 0..count {
            let filter = ops::get(
                realm,
                &Value::Object(filters.clone()),
                &Key::Index(i as u32),
            )?;
            let name = optional_string(realm, &filter, "name")?.unwrap_or_default();
            let extensions = ops::get_named(realm, &filter, "extensions")?;
            options
                .filters
                .push((name, array_strings(realm, &extensions)?));
        }
    }
    Ok(options)
}

fn message_options(realm: &mut Realm, value: &Value) -> Result<MessageOptions, Thrown> {
    // Without options, an empty message, as Electron gives it.
    let empty;
    let value = match value {
        Value::Undefined | Value::Null => {
            empty = Value::Object(realm.object());
            &empty
        }
        Value::Object(_) => value,
        _ => {
            return Err(realm.throw(ErrorKind::Type, "message box options must be an object"));
        }
    };
    let kind = optional_string(realm, value, "type")?.unwrap_or_else(|| "none".to_owned());
    if !["none", "info", "warning", "error", "question"].contains(&kind.as_str()) {
        return Err(realm.throw(ErrorKind::Type, "Invalid message box type"));
    }
    // Without a title, the dialog shows the application's name.
    let title = optional_string(realm, value, "title")?
        .filter(|title| !title.is_empty())
        .or_else(|| realm.electron_app_name.map(str::to_owned));
    let message = optional_string(realm, value, "message")?.unwrap_or_default();
    let detail = optional_string(realm, value, "detail")?.filter(|detail| !detail.is_empty());
    let button_values = ops::get_named(realm, value, "buttons")?;
    let buttons = array_strings(realm, &button_values)?;
    let no_link = match ops::get_named(realm, value, "noLink")? {
        Value::Undefined => false,
        Value::Boolean(no_link) => no_link,
        _ => return Err(realm.throw(ErrorKind::Type, "noLink must be a boolean")),
    };
    let checkbox_label =
        optional_string(realm, value, "checkboxLabel")?.filter(|label| !label.is_empty());
    let checked = ops::get_named(realm, value, "checkboxChecked")?;
    let checkbox_checked = ops::to_boolean(&checked);
    if checkbox_checked && checkbox_label.is_none() {
        return Err(realm.throw(
            ErrorKind::Error,
            "checkboxChecked requires that checkboxLabel also be passed",
        ));
    }
    let default_id = integer_option(realm, value, "defaultId")?.unwrap_or(-1);
    // Without a cancel button named, a "Cancel" or "No" button cancels, or
    // else the first button other than the default.
    let cancel_id = match integer_option(realm, value, "cancelId")? {
        Some(cancel_id) => cancel_id,
        None => buttons
            .iter()
            .position(|button| {
                button.eq_ignore_ascii_case("cancel") || button.eq_ignore_ascii_case("no")
            })
            .map_or(i64::from(default_id == 0 && buttons.len() > 1), |index| {
                index as i64
            }),
    };
    Ok(MessageOptions {
        title,
        message,
        detail,
        kind,
        buttons,
        default_id,
        cancel_id,
        no_link,
        checkbox_label,
        checkbox_checked,
    })
}

fn integer_option(realm: &mut Realm, value: &Value, name: &str) -> Result<Option<i64>, Thrown> {
    match ops::get(realm, value, &Key::name(name))? {
        Value::Number(number) if number.is_finite() => Ok(Some(number as i64)),
        Value::Undefined | Value::Null => Ok(None),
        _ => Err(realm.throw(ErrorKind::Type, "dialog button index must be a number")),
    }
}

fn optional_string(
    realm: &mut Realm,
    object: &Value,
    name: &str,
) -> Result<Option<String>, Thrown> {
    match ops::get(realm, object, &Key::name(name))? {
        Value::Undefined | Value::Null => Ok(None),
        value => Ok(Some(ops::to_string(realm, &value)?.to_rust_lossy())),
    }
}

fn array_strings(realm: &mut Realm, value: &Value) -> Result<Vec<String>, Thrown> {
    let Some(array) = value.as_object() else {
        return Ok(Vec::new());
    };
    let count = crate::array_builtin::length_of(realm, value)?.min(256);
    let mut result = Vec::with_capacity(count as usize);
    for i in 0..count {
        let value = ops::get(realm, &Value::Object(array.clone()), &Key::Index(i as u32))?;
        result.push(ops::to_string(realm, &value)?.to_rust_lossy());
    }
    Ok(result)
}

fn string_array(realm: &Realm, values: &[String]) -> Result<Object, Thrown> {
    let array = crate::array_builtin::array_create(realm, values.len() as u64)?;
    for (index, value) in values.iter().enumerate() {
        array.define(Key::Index(index as u32), Property::data(Value::str(value)));
    }
    array.define(
        Key::name("length"),
        Property::data(Value::Number(values.len() as f64)),
    );
    Ok(array)
}

fn resolved(realm: &mut Realm, value: Value) -> Result<Value, Thrown> {
    let (promise, resolve, _) = crate::promise_builtin::capability(realm);
    ops::call(realm, &resolve, Value::Undefined, &[value])?;
    Ok(promise)
}

fn rejected_dialog(realm: &mut Realm, method: &str, code: u32) -> Result<Value, Thrown> {
    let error = realm.error(
        ErrorKind::Error,
        &alloc::format!("dialog.{method} failed (Windows error {code})"),
    );
    let (promise, _, reject) = crate::promise_builtin::capability(realm);
    ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
    Ok(promise)
}

fn string_arg(
    realm: &mut Realm,
    args: &[Value],
    index: usize,
    name: &str,
) -> Result<String, Thrown> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.to_rust_lossy()),
        _ => Err(realm.throw(ErrorKind::Type, &alloc::format!("{name} must be a string"))),
    }
}

fn dialog_error(realm: &Realm, method: &str, code: u32) -> Thrown {
    realm.throw(
        ErrorKind::Error,
        &alloc::format!("dialog.{method} failed (Windows error {code})"),
    )
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn choose_file(owner: usize, options: &FileOptions, save: bool) -> Result<Vec<String>, u32> {
    let filter = filter_string(&options.filters);
    let mut file = vec![0u16; 32_768];
    if let Some(path) = &options.default_path {
        let units: Vec<u16> = path.encode_utf16().collect();
        let filename = units
            .iter()
            .rposition(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
            .map_or(0, |i| i + 1);
        let copy = &units[filename.min(units.len())..];
        let count = copy.len().min(file.len() - 1);
        file[..count].copy_from_slice(&copy[..count]);
    }
    let title = options.title.as_deref().map(wide);
    let initial = options
        .default_path
        .as_deref()
        .and_then(|path| path.rfind(['\\', '/']).map(|index| wide(&path[..index])));
    let mut dialog = OpenFileNameW {
        size: core::mem::size_of::<OpenFileNameW>() as u32,
        owner: owner as Hwnd,
        instance: core::ptr::null_mut(),
        filter: if filter.is_empty() {
            core::ptr::null()
        } else {
            filter.as_ptr()
        },
        custom_filter: core::ptr::null_mut(),
        max_custom_filter: 0,
        filter_index: 1,
        file: file.as_mut_ptr(),
        max_file: file.len() as u32,
        file_title: core::ptr::null_mut(),
        max_file_title: 0,
        initial_dir: initial.as_ref().map_or(core::ptr::null(), |s| s.as_ptr()),
        title: title.as_ref().map_or(core::ptr::null(), |s| s.as_ptr()),
        flags: OFN_EXPLORER | OFN_PATHMUSTEXIST,
        file_offset: 0,
        extension_offset: 0,
        default_extension: core::ptr::null(),
        custom_data: 0,
        hook: core::ptr::null_mut(),
        template_name: core::ptr::null(),
        reserved: core::ptr::null_mut(),
        reserved_dword: 0,
        flags_ex: 0,
    };
    let multi = !save
        && options
            .properties
            .iter()
            .any(|item| item == "multiSelections");
    dialog.flags |= if save {
        OFN_OVERWRITEPROMPT
    } else {
        OFN_FILEMUSTEXIST
    };
    if multi {
        dialog.flags |= OFN_ALLOWMULTISELECT;
    }
    // SAFETY: all buffers and strings remain alive while the common dialog is shown.
    let accepted = unsafe {
        if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        }
    };
    if accepted == 0 {
        // SAFETY: reports the common-dialog error associated with the last call.
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 {
            Ok(Vec::new())
        } else {
            Err(error)
        };
    }
    Ok(parse_multi_path(&file))
}

fn choose_directory(owner: usize, options: &FileOptions) -> Result<Option<String>, u32> {
    let title = options
        .title
        .as_deref()
        .map(wide)
        .unwrap_or_else(|| wide("Select a folder"));
    let mut display = vec![0u16; 32_768];
    let info = BrowseInfoW {
        owner: owner as Hwnd,
        root: core::ptr::null_mut(),
        display_name: display.as_mut_ptr(),
        title: title.as_ptr(),
        flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        callback: core::ptr::null_mut(),
        parameter: 0,
        image: 0,
    };
    // SAFETY: `info` and its buffers remain valid until the shell dialog returns.
    let item = unsafe { SHBrowseForFolderW((&info as *const BrowseInfoW).cast_mut()) };
    if item.is_null() {
        return Ok(None);
    }
    let mut path = vec![0u16; 32_768];
    // SAFETY: the PIDL is returned by the shell and the path buffer has ample capacity.
    let ok = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) };
    // SAFETY: the shell allocated this PIDL with the task allocator.
    unsafe { CoTaskMemFree(item) };
    if ok == 0 {
        return Err(87);
    }
    Ok(Some(from_wide(&path)))
}

fn parse_multi_path(data: &[u16]) -> Vec<String> {
    let fields: Vec<String> = data
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .map(from_wide)
        .collect();
    if fields.len() <= 1 {
        return fields;
    }
    let folder = &fields[0];
    fields[1..]
        .iter()
        .map(|name| alloc::format!("{}\\{}", folder.trim_end_matches(['\\', '/']), name))
        .collect()
}

fn from_wide(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn filter_string(filters: &[(String, Vec<String>)]) -> Vec<u16> {
    if filters.is_empty() {
        return Vec::new();
    }
    let mut value = Vec::new();
    for (name, extensions) in filters {
        let label = if name.is_empty() { "Files" } else { name };
        value.extend(label.encode_utf16());
        value.push(0);
        let mut patterns = Vec::new();
        for extension in extensions {
            let extension = extension.trim_start_matches('.');
            if !extension.is_empty() {
                patterns.push(alloc::format!("*.{extension}"));
            }
        }
        if patterns.is_empty() {
            patterns.push("*".to_owned());
        }
        value.extend(patterns.join(";").encode_utf16());
        value.push(0);
    }
    value.push(0);
    value
}

/// The task dialog's buttons as Electron lays them out: buttons labelled as
/// the dialog's own ("OK", "Cancel"...) become those, unless `noLink`, and
/// the others custom buttons, with IDs from [`CUSTOM_BUTTON_ID`].
#[derive(Debug, Default, PartialEq, Eq)]
struct ButtonPlan {
    /// The dialog's own buttons' flags.
    common: u32,
    /// The dialog's own buttons' IDs, with their indexes.
    common_ids: Vec<(i32, usize)>,
    /// The custom buttons' indexes.
    custom: Vec<usize>,
}

fn button_plan(buttons: &[String], no_link: bool) -> ButtonPlan {
    let mut plan = ButtonPlan::default();
    for (index, label) in buttons.iter().enumerate() {
        let common = COMMON_BUTTONS
            .iter()
            .find(|(name, _, _)| !no_link && label.eq_ignore_ascii_case(name));
        match common {
            Some((_, flag, id)) => {
                plan.common |= flag;
                plan.common_ids.push((*id, index));
            }
            None => plan.custom.push(index),
        }
    }
    plan
}

/// The index of the button the dialog returned, or the cancel ID when it
/// was closed otherwise.
fn response(plan: &ButtonPlan, selected: i32, cancel_id: i64) -> i64 {
    if let Some((_, index)) = plan.common_ids.iter().find(|(id, _)| *id == selected) {
        return *index as i64;
    }
    if selected >= CUSTOM_BUTTON_ID {
        return i64::from(selected - CUSTOM_BUTTON_ID);
    }
    cancel_id
}

fn task_message(owner: usize, options: &MessageOptions) -> Result<(i64, bool), u32> {
    let title = wide(options.title.as_deref().unwrap_or_default());
    let message = wide(&options.message);
    let detail = options.detail.as_deref().map(wide);
    let verification = options.checkbox_label.as_deref().map(wide);
    let labels: Vec<Vec<u16>> = options.buttons.iter().map(|label| wide(label)).collect();
    let plan = button_plan(&options.buttons, options.no_link);
    let buttons: Vec<TaskDialogButton> = plan
        .custom
        .iter()
        .map(|index| TaskDialogButton {
            id: CUSTOM_BUTTON_ID + *index as i32,
            text: labels[*index].as_ptr(),
        })
        .collect();
    let icon = match options.kind.as_str() {
        "error" => TASKDIALOG_ERROR_ICON,
        "warning" => TASKDIALOG_WARNING_ICON,
        "info" | "question" => TASKDIALOG_INFORMATION_ICON,
        _ => core::ptr::null(),
    };
    let mut flags = TDF_SIZE_TO_CONTENT | TDF_ALLOW_DIALOG_CANCELLATION;
    if !buttons.is_empty() && !options.no_link {
        flags |= TDF_USE_COMMAND_LINKS;
    }
    if options.checkbox_checked {
        flags |= TDF_VERIFICATION_FLAG_CHECKED;
    }
    // A message alone is the dialog's content; with a detail, the message
    // is its main instruction, set off above the detail.
    let (main_instruction, content) = match &detail {
        Some(detail) => (message.as_ptr(), detail.as_ptr()),
        None => (core::ptr::null(), message.as_ptr()),
    };
    let config = TaskDialogConfig {
        size: core::mem::size_of::<TaskDialogConfig>() as u32,
        parent: owner as Hwnd,
        instance: core::ptr::null_mut(),
        flags,
        common_buttons: plan.common,
        window_title: title.as_ptr(),
        main_icon: icon,
        main_instruction,
        content,
        button_count: buttons.len() as u32,
        buttons: if buttons.is_empty() {
            core::ptr::null()
        } else {
            buttons.as_ptr()
        },
        // As in Electron, the default names a custom button's ID.
        default_button: if options.default_id > 0 {
            CUSTOM_BUTTON_ID + options.default_id as i32
        } else {
            0
        },
        radio_count: 0,
        radios: core::ptr::null(),
        default_radio: 0,
        verification_text: verification
            .as_ref()
            .map_or(core::ptr::null(), |text| text.as_ptr()),
        expanded_information: core::ptr::null(),
        expanded_control_text: core::ptr::null(),
        collapsed_control_text: core::ptr::null(),
        footer_icon: core::ptr::null(),
        footer: core::ptr::null(),
        callback: core::ptr::null_mut(),
        callback_data: 0,
        width: 0,
    };
    let mut selected = 0;
    let mut checked = 0;
    // The API is available from Common Controls v6; load it dynamically so
    // hosts without its activation manifest can still start and use fallback UI.
    // SAFETY: the module path and export name are NUL terminated.
    let controls = unsafe { LoadLibraryW(COMMON_CONTROLS.as_ptr()) };
    let task_dialog = if controls.is_null() {
        core::ptr::null()
    } else {
        // SAFETY: `controls` is a loaded module and the export string is NUL terminated.
        unsafe { GetProcAddress(controls, TASK_DIALOG_SYMBOL.as_ptr()) }
    };
    if task_dialog.is_null() {
        if !controls.is_null() {
            // SAFETY: this is the reference acquired by LoadLibraryW above.
            unsafe { FreeLibrary(controls) };
        }
        return Err(ERROR_CALL_NOT_IMPLEMENTED);
    }
    // SAFETY: GetProcAddress resolved the documented TaskDialogIndirect ABI.
    let call: TaskDialogIndirectFn = unsafe { core::mem::transmute(task_dialog) };
    // SAFETY: each config pointer targets storage retained through the modal call.
    let status = unsafe { call(&config, &mut selected, core::ptr::null_mut(), &mut checked) };
    // SAFETY: `controls` is the reference acquired by LoadLibraryW above.
    unsafe { FreeLibrary(controls) };
    if status < 0 {
        return Err(status as u32);
    }
    Ok((response(&plan, selected, options.cancel_id), checked != 0))
}

#[cfg(test)]
#[path = "electron_dialog/tests.rs"]
mod tests;
