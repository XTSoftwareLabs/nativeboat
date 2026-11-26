//! The realm: intrinsic prototypes, the global object, function creation,
//! errors and the native call depth limit.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{ArrayData, ConstructorCode, Key, Kind, NativeCode, Object, Property, Slot};
use crate::string::JsStr;
use crate::symbol::WellKnown;
use crate::table::HashMap;
use crate::value::{Completion, Thrown, Value};
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

/// A compiled project's entry loader. It may run again while it runs: a
/// module it evaluates can `require` another entry synchronously.
type DynamicImportFn = dyn Fn(&mut Realm, usize) -> Completion;

#[derive(Clone)]
struct DynamicImportLoader(Rc<DynamicImportFn>);

impl core::fmt::Debug for DynamicImportLoader {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DynamicImportLoader")
    }
}

/// The intrinsics and global state that compiled behavior runs against.
#[derive(Debug)]
pub struct Realm {
    /// `Object.prototype`.
    pub object_prototype: Object,
    /// `Function.prototype`.
    pub function_prototype: Object,
    /// The shared `%ThrowTypeError%` intrinsic used by restricted properties.
    pub throw_type_error: Object,
    /// The original `%Array.prototype.values%` intrinsic, independent of edits.
    pub array_values: Object,
    /// `Array.prototype`.
    pub array_prototype: Object,
    /// `String.prototype`.
    pub string_prototype: Object,
    /// `Number.prototype`.
    pub number_prototype: Object,
    /// `Boolean.prototype`.
    pub boolean_prototype: Object,
    /// `Symbol.prototype`.
    pub symbol_prototype: Object,
    /// `BigInt.prototype`.
    pub bigint_prototype: Object,
    /// `%IteratorPrototype%`.
    pub iterator_prototype: Object,
    /// `%ArrayIteratorPrototype%`.
    pub array_iterator_prototype: Object,
    /// The built-in `next` of array and then string iterators, which
    /// `for`-`of` steps without result objects.
    pub(crate) iterator_next: Vec<Object>,
    /// `%StringIteratorPrototype%`.
    pub string_iterator_prototype: Object,
    /// `RegExp.prototype`, an ordinary object.
    pub regexp_prototype: Object,
    /// Protected RegExp prototype for compiled Node library helpers.
    pub(crate) protected_regexp_prototype: Object,
    /// Its native constructor, installed before application execution.
    pub(crate) protected_regexp_constructor: Option<Object>,
    /// Regular expression patterns compiled for this realm.
    pub(crate) patterns: crate::regexp_builtin::Patterns,
    /// `Promise.prototype`.
    pub promise_prototype: Object,
    /// The intrinsic `Promise` constructor, once installed.
    pub(crate) promise_constructor: Option<Object>,
    /// The prototypes of timer (`Timeout`) and immediate handles.
    pub(crate) timeout_prototype: Object,
    pub(crate) immediate_prototype: Object,
    /// Promise jobs, timers and immediates.
    pub(crate) event_loop: crate::event_loop::EventLoop,
    /// The native current-directory function, retaining identity across reads.
    pub(crate) process_cwd: Option<Object>,
    /// A stable native view of this process's Windows environment.
    pub(crate) process_environment: Option<Object>,
    /// The mutable native application's argument array, initialized once.
    pub(crate) process_argv: Option<Object>,
    /// The `process` object, created when first read.
    pub(crate) process: Option<Object>,
    /// Whether `process` has become an EventEmitter.
    pub(crate) process_emitter: bool,
    /// Private identity keys used by native Proxy instances.
    /// WeakRef targets kept alive until the current job ends (`KeepDuringJob`).
    pub(crate) kept: Vec<Value>,
    /// FinalizationRegistries whose targets are watched.
    pub(crate) registries: Vec<alloc::rc::Weak<crate::weak_builtin::Registry>>,
    /// `process.stdout` and `process.stderr`, created when first read.
    pub(crate) process_streams: [Option<Object>; 3],
    /// `process.exitCode`: undefined or a safe integer.
    pub(crate) exit_code: Value,
    /// The async context frame: the value V8 calls continuation-preserved
    /// embedder data. Jobs, timers and I/O completions run with the frame
    /// that was current when they were queued, which carries
    /// `AsyncLocalStorage` stores across awaits and callbacks.
    pub(crate) async_context: Value,
    /// Whether a warning has shown Node's `--trace-warnings` hint.
    pub(crate) warning_hint_shown: bool,
    /// The prototypes of Node's coded errors, by kind and code.
    pub(crate) node_error_prototypes: Vec<(ErrorKind, String, Object)>,
    /// Native modules loaded in this realm, by name.
    pub(crate) native_modules: Vec<(String, Object)>,
    /// Main-realm callbacks for native keyboard layout changes.
    pub(crate) keyboard_layout_callbacks: Vec<Value>,
    /// Shared native descriptors for filesystem and terminal operations.
    pub(crate) native_files: Option<alloc::sync::Arc<crate::fs_binding::Files>>,
    /// The precompiled Node.js library's `require`, when the application
    /// links it; `node:` native modules come from it.
    pub node_library: Option<fn(&mut Realm, &str) -> Completion>,
    /// The compiled project's lazy module loader, installed by generated code.
    dynamic_import_loader: Option<DynamicImportLoader>,
    /// Promises of module bodies with top-level `await` that started
    /// evaluating; a dynamic import resolves once those it started settle.
    async_evaluations: Vec<Value>,
    /// Package scripts the realm may load while it runs, by their paths from
    /// the project root, with their entries: a script element or a computed
    /// `import()` whose URL a path ends loads that entry.
    scripts: &'static [(&'static str, usize)],
    /// The functions whose text the program read, by that text, latest
    /// first made: a script the program makes from such text, as a worker's
    /// blob script, runs the compiled function. The page's workers share
    /// them.
    pub(crate) source_functions: Rc<core::cell::RefCell<Vec<(JsStr, Value)>>>,
    /// `Buffer.prototype`, which the library registers when `buffer` loads,
    /// for buffers that native bindings create.
    pub(crate) buffer_prototype: Option<Object>,
    /// The Electron version an application was built for, reported in
    /// `process.versions`.
    pub electron_version: Option<&'static str>,
    /// The application version and name read from its package manifest.
    pub electron_app_version: Option<&'static str>,
    pub electron_app_name: Option<&'static str>,
    /// Paths overridden through Electron's `app.setPath` in this Realm.
    pub(crate) electron_app_paths: Vec<(String, String)>,
    /// Data exposed by a preload for cloning into its renderer context.
    pub(crate) electron_bridge_exports: Vec<(String, Value)>,
    /// Build-time stylesheet sources registered by the native application
    /// before renderer modules are initialized.
    native_stylesheet_sources: Vec<(String, String)>,
    /// Stylesheet module identifiers whose imports have executed in this Realm.
    native_stylesheet_imports: Vec<String>,
    /// Electron page zoom state for this execution context.
    pub(crate) electron_zoom_factor: f64,
    pub(crate) electron_zoom_level: f64,
    /// Main-realm address used by a preload's synchronous native IPC adapter.
    /// The host installs it only while the main realm is alive.
    pub(crate) electron_ipc_main: usize,
    /// The channel ends of message ports, which every realm of the process
    /// shares once connected to the main realm.
    pub(crate) message_ports: crate::realm_ports::HubHandle,
    /// Whether this realm runs a page, whose scripts end in a microtask
    /// checkpoint, and the address of the page's preload realm, if any.
    page_scripts: Option<usize>,
    /// The window whose page this realm runs, for IPC senders; 0 for main.
    pub(crate) electron_window: usize,
    /// Main realm windows: id, `BrowserWindow` and `webContents`.
    pub(crate) electron_windows: Vec<(usize, Object, Object)>,
    /// Main realm `webContents.send` messages awaiting delivery.
    pub(crate) electron_outgoing: Vec<crate::electron_window::Outgoing>,
    /// Events of pages' notifications for their windows: the window, the
    /// page's notification and the event's type.
    pub(crate) page_notification_events: Vec<(usize, u32, &'static str)>,
    /// Messages a window's preload realm has had, for its page's realm.
    pub(crate) electron_preloaded: Vec<crate::electron_window::Outgoing>,
    /// A preload realm's `ipcRenderer` listeners.
    pub(crate) electron_renderer_listeners: Option<crate::electron_window::RendererListeners>,
    /// The main realm's `app`, for application events.
    pub(crate) electron_app: Option<Object>,
    /// The main realm's default session, which every window's contents
    /// share, as in Electron.
    pub(crate) electron_session: Option<Object>,
    /// The realm's `electron` module: one per realm, as Electron has one per
    /// process, so every importer shares its `ipcMain` registry and `app`.
    pub(crate) electron_exports: Option<Object>,
    /// Each window's `webPreferences.additionalArguments`.
    electron_window_arguments: Vec<(usize, Vec<String>)>,
    /// A page or preload realm's renderer arguments, which its
    /// `process.argv` gives after the executable.
    pub(crate) page_arguments: Option<Vec<String>>,
    /// Whether `app.quit()` is closing the windows.
    pub(crate) electron_quitting: bool,
    /// Exceptions thrown by listeners called from native window messages,
    /// handled at the next host step.
    pub(crate) host_errors: Vec<Thrown>,
    /// Tray icons, shown notifications and global shortcuts, by the
    /// identifier the host's notification window reports them with.
    pub(crate) notify_targets: alloc::collections::BTreeMap<u32, crate::electron_notify::Target>,
    pub(crate) next_notify_id: u32,
    /// Main-process IPC registry shared with engine-free preload contexts.
    electron_ipc_registry: RefCell<Option<crate::electron_ipc::StateHandle>>,
    /// Preload paths attached to native Electron windows created by this realm.
    electron_window_preloads: Vec<(usize, String)>,
    /// Page targets passed to BrowserWindow.loadFile/loadURL.
    electron_window_loads: Vec<(usize, String)>,
    /// Whether the generated host runs this realm's windows' pages, so that
    /// their `loadFile` and `loadURL` promises settle once the pages load.
    pub electron_hosts_pages: bool,
    /// The resolve functions of `loadFile` and `loadURL` promises, by
    /// window, which settle when the page has loaded.
    pub(crate) electron_load_waiters: Vec<(usize, Value)>,
    /// Private key carrying a BrowserWindow's native HWND into Electron APIs.
    pub(crate) electron_window_key: crate::symbol::Symbol,
    /// Private key carrying an Electron native image's representations.
    pub(crate) electron_image_key: crate::symbol::Symbol,
    /// The WebAssembly API's prototypes, once its library loads.
    pub(crate) wasm: Option<crate::wasm_binding::State>,
    /// The prototype and methods native images share, once one is made.
    pub(crate) electron_images: Option<crate::electron_native_image::ImageClass>,
    /// Whether `getBitmap` has warned that it is deprecated, which Electron
    /// does once.
    pub(crate) electron_image_bitmap_warned: bool,
    /// The current application menu, retained for new native windows.
    pub(crate) electron_application_menu: Option<Object>,
    /// BrowserWindow-specific menu trees keyed by their native HWND.
    pub(crate) electron_window_menus: Vec<(usize, Object)>,
    /// Monotonic command ID for menu items in this Realm.
    pub(crate) electron_next_menu_id: u16,
    /// Whether menu items changed since the native menus were last built;
    /// the next host step rebuilds them once.
    pub(crate) electron_menus_changed: bool,
    /// The native menus the last rebuild installed, keyed by window (0 for
    /// the application menu), so unchanged menus are not installed again.
    pub(crate) electron_installed_menus: Vec<(usize, Vec<crate::app_host::NativeMenuItem>)>,
    /// The keys of property names fixed in the program, by the address and
    /// length of their text: compiled code reads `object.name` often.
    name_keys: crate::table::HashMap<(usize, usize), Key>,
    /// Private key for native menu command identifiers.
    pub(crate) electron_menu_command_key: crate::symbol::Symbol,
    /// `%GeneratorPrototype%`.
    pub generator_prototype: Object,
    /// `%AsyncIteratorPrototype%`, `%AsyncGeneratorPrototype%` and
    /// `%AsyncGeneratorFunction.prototype%`.
    pub async_iterator_prototype: Object,
    pub async_generator_prototype: Object,
    pub async_generator_function_prototype: Object,
    /// `Date.prototype`.
    pub date_prototype: Object,
    /// `ArrayBuffer.prototype`.
    pub array_buffer_prototype: Object,
    /// `%TypedArray%.prototype`.
    pub typed_array_prototype: Object,
    /// The typed array constructors and prototypes, by element type.
    pub(crate) typed_array_constructors: Vec<Object>,
    pub(crate) typed_array_prototypes: Vec<Object>,
    /// The local time zone, read from Windows when first needed.
    pub(crate) time_zone: Option<crate::date_builtin::TimeZone>,
    /// A null-terminated locale name for date strings instead of the
    /// Windows user locale.
    pub(crate) locale: Option<Vec<u16>>,
    /// The ICU collator of `localeCompare` without locales or options.
    pub(crate) default_collator: Option<usize>,
    /// Generator and async bodies running now, innermost last.
    pub(crate) coroutines: Vec<Rc<crate::coroutine::Coroutine>>,
    /// Fibers whose bodies finished, kept for the next ones.
    pub(crate) idle_fibers: crate::coroutine::FiberPool,
    /// The thrown value with which a generator's `return()` unwinds its body.
    pub(crate) return_signal: Object,
    /// The well-known symbols.
    pub symbols: WellKnown,
    /// The `Symbol.for` registry, shared like `symbols` by the agent's realms.
    pub(crate) registry: Rc<core::cell::RefCell<crate::symbol_builtin::Registry>>,
    /// `Error.prototype`.
    pub error_prototype: Object,
    /// The original Error constructor used by protected Node error helpers.
    pub(crate) error_constructor: Option<Object>,
    /// The page proxies of preload functions passed across the context
    /// bridge, by the function they stand for.
    pub(crate) bridged_functions: Vec<(Object, Object)>,
    /// The `stack` getter and setter every error shares.
    pub(crate) error_stack: Option<(Object, Object)>,
    /// The host object of a renderer page's Web API prelude.
    #[cfg(feature = "dom")]
    pub(crate) web_host: Option<Object>,
    /// What a worker's realm evaluates first, for a page that starts
    /// workers.
    #[cfg(feature = "dom")]
    pub(crate) worker_scope: Option<crate::worker::WorkerScope>,
    /// The workers this realm started, which it steps after itself.
    #[cfg(feature = "dom")]
    pub(crate) workers: Vec<crate::worker::Worker>,
    /// The id of the last worker this realm started.
    #[cfg(feature = "dom")]
    pub(crate) next_worker: u32,
    /// Whether this worker's realm closed itself, so its page ends it.
    #[cfg(feature = "dom")]
    pub(crate) worker_closing: bool,
    /// The documents the build compiled that this page's frames may show.
    #[cfg(feature = "dom")]
    pub(crate) frame_documents: &'static [crate::frame::FrameDocument],
    /// Makes the page of a frame's realm.
    #[cfg(feature = "dom")]
    pub(crate) frame_page: Option<crate::frame::FramePage>,
    /// This realm's id, which the functions it makes keep.
    pub(crate) id: u32,
    /// Whether functions made here run here when other realms call them.
    pub(crate) home: bool,
    /// Ends the page of a frame's realm.
    #[cfg(feature = "dom")]
    pub(crate) frame_end: Option<crate::frame::FrameEnd>,
    /// `TypeError.prototype`.
    pub type_error_prototype: Object,
    /// `RangeError.prototype`.
    pub range_error_prototype: Object,
    /// `ReferenceError.prototype`.
    pub reference_error_prototype: Object,
    /// `EvalError.prototype`.
    pub eval_error_prototype: Object,
    /// `SyntaxError.prototype`.
    pub syntax_error_prototype: Object,
    /// `URIError.prototype`.
    pub uri_error_prototype: Object,
    /// `AggregateError.prototype`.
    pub aggregate_error_prototype: Object,
    /// `SuppressedError.prototype`.
    pub suppressed_error_prototype: Object,
    /// The global object.
    pub global: Object,
    /// The deepest native call nesting allowed before a RangeError, so that
    /// unbounded recursion becomes a catchable error instead of a crash.
    pub max_depth: usize,
    depth: usize,
    /// Call counts and times by call site, when `NATIVEBOAT_PROFILE_CALLS`
    /// asks for them.
    pub(crate) call_profile: Option<crate::trace::CallProfile>,
    current_new_target: Value,
    /// Tagged template objects by call site, created once per realm.
    templates: HashMap<u64, Object>,
    /// Arrays being joined, so that cyclic arrays join as empty strings.
    pub(crate) joining: Vec<Object>,
}

/// The built-in error kinds the realm creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// `Error`.
    Error,
    /// `TypeError`.
    Type,
    /// `RangeError`.
    Range,
    /// `ReferenceError`.
    Reference,
    /// `EvalError`.
    Eval,
    /// `SyntaxError`.
    Syntax,
    /// `URIError`.
    Uri,
}

impl Default for Realm {
    fn default() -> Realm {
        Realm::new()
    }
}

/// The id the next realm takes; 0 marks functions made by no realm.
static NEXT_REALM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);

/// The realms that keep their functions home, by id, with their addresses.
struct Homes(core::cell::RefCell<Vec<(u32, usize)>>);

// SAFETY: realms run on the thread that made them; only that thread, which
// runs the application's pages and frames, registers or reads them here.
unsafe impl Sync for Homes {}

static HOMES: Homes = Homes(core::cell::RefCell::new(Vec::new()));

impl Drop for Realm {
    fn drop(&mut self) {
        if self.home {
            let id = self.id;
            HOMES.0.borrow_mut().retain(|(known, _)| *known != id);
        }
    }
}

impl Realm {
    /// Registers a stylesheet compiled into the application executable.
    pub fn register_native_stylesheet(&mut self, identifier: &str, source: &str) {
        if let Some((_, current)) = self
            .native_stylesheet_sources
            .iter_mut()
            .find(|(name, _)| name == identifier)
        {
            current.clear();
            current.push_str(source);
        } else {
            self.native_stylesheet_sources
                .push((identifier.to_owned(), source.to_owned()));
        }
    }

    /// Records a stylesheet module's evaluation and returns its source.
    pub(crate) fn import_native_stylesheet(&mut self, identifier: &str) -> Option<String> {
        let (_, source) = self
            .native_stylesheet_sources
            .iter()
            .find(|(name, _)| name == identifier)?;
        if !self
            .native_stylesheet_imports
            .iter()
            .any(|name| name == identifier)
        {
            self.native_stylesheet_imports.push(identifier.to_owned());
        }
        Some(source.clone())
    }

    /// Takes imported stylesheet sources in first-import order for the native
    /// renderer to parse and apply to this page.
    pub fn take_native_stylesheets(&mut self) -> Vec<String> {
        let imports = core::mem::take(&mut self.native_stylesheet_imports);
        imports
            .iter()
            .filter_map(|identifier| {
                self.native_stylesheet_sources
                    .iter()
                    .find(|(name, _)| name == identifier)
                    .map(|(_, source)| source.clone())
            })
            .collect()
    }

    pub(crate) fn cache_electron_ipc(&self, state: crate::electron_ipc::StateHandle) {
        *self.electron_ipc_registry.borrow_mut() = Some(state);
    }

    pub(crate) fn electron_ipc_registry(&self) -> Option<crate::electron_ipc::StateHandle> {
        self.electron_ipc_registry.borrow().clone()
    }

    /// Connects an isolated preload realm to the main process realm for native
    /// Electron IPC. The host must keep `main` alive until the preload is dropped.
    ///
    /// # Safety
    /// `main` must point to a distinct, live `Realm` for the entire lifetime of
    /// this realm and every IPC callback it creates.
    pub unsafe fn connect_electron_ipc(&mut self, main: *mut Realm) {
        self.electron_ipc_main = main as usize;
        // SAFETY: the caller guarantees `main` is live and distinct.
        self.message_ports = unsafe { (*main).message_ports.clone() };
    }

    /// Gives a window's preload realm its `window`: its global object, whose
    /// `postMessage(message, targetOrigin, transfer)` posts to the window's
    /// page, with the message ports moved there, as a preload passes on the
    /// ports it receives over IPC.
    pub fn install_preload_window(&mut self) {
        let global = self.global_object();
        let post = self.function("postMessage", 2, false, |realm, _, args| {
            let message = args.first().cloned().unwrap_or_default();
            let transfer = args.get(2).cloned().unwrap_or_default();
            crate::realm_ports::post_to_page(realm, &message, &transfer)?;
            Ok(Value::Undefined)
        });
        global.define(
            Key::name("postMessage"),
            Property::data(Value::Object(post)),
        );
        self.set_global("window", Value::Object(global));
    }

    /// Makes this realm run a page: each of its scripts ends in a microtask
    /// checkpoint, as HTML performs one after every script.
    ///
    /// # Safety
    /// `preload`, when not null, must point to a distinct, live `Realm` for
    /// the lifetime of this realm, as the main realm of
    /// [`Realm::connect_electron_ipc`] must.
    pub unsafe fn run_page_scripts(&mut self, preload: *mut Realm) {
        self.page_scripts = Some(preload as usize);
        crate::realm_ports::register_page(self);
    }

    /// Ends a script of a page: runs its promise jobs. The main and preload
    /// realms run theirs too, so replies to IPC the script requested arrive
    /// before the next script, as they do in Electron while the page's
    /// remaining scripts load. Other realms' errors are left for the host.
    pub fn script_checkpoint(&mut self) -> Result<(), Thrown> {
        let Some(preload) = self.page_scripts else {
            return Ok(());
        };
        let others = [self.electron_ipc_main, preload];
        // Each round can only settle what the previous one sent.
        for _ in 0..8 {
            let mut idle = !self.has_queued_jobs();
            for other in others.into_iter().filter(|other| *other != 0) {
                // SAFETY: the host keeps the main and preload realms alive
                // and distinct from this one, and runs all realms on this
                // thread; see `connect_electron_ipc` and `run_page_scripts`.
                let other = unsafe { &mut *(other as *mut Realm) };
                if other.has_queued_jobs() {
                    idle = false;
                    // In the preload's Node environment, ticks the page's
                    // code queued wait for Electron's next Node turn, after
                    // the page renders; its promise jobs run now, as the
                    // page's microtasks do.
                    let preload_realm = other as *mut Realm as usize == preload;
                    other.event_loop.deferring_ticks = preload_realm;
                    let ran = other.run_jobs();
                    other.event_loop.deferring_ticks = false;
                    if let Err(thrown) = ran {
                        other.host_errors.push(thrown);
                    }
                }
            }
            if idle {
                break;
            }
            self.run_jobs()?;
        }
        Ok(())
    }

    /// Takes the preload scripts requested by windows in this main realm.
    pub fn take_electron_window_preloads(&mut self) -> Vec<(usize, String)> {
        core::mem::take(&mut self.electron_window_preloads)
    }

    /// Whether windows wait for their preloads or pages to start.
    pub fn has_pending_electron_windows(&self) -> bool {
        !self.electron_window_loads.is_empty() || !self.electron_window_preloads.is_empty()
    }

    /// Takes the page targets requested by windows in this realm.
    pub fn take_electron_window_loads(&mut self) -> Vec<(usize, String)> {
        core::mem::take(&mut self.electron_window_loads)
    }

    /// Globals exposed by preload `contextBridge` for transfer to its page.
    pub fn electron_bridge_exports(&self) -> &[(String, Value)] {
        &self.electron_bridge_exports
    }

    /// A window's `webPreferences.additionalArguments`.
    pub fn electron_window_arguments(&self, window: usize) -> Vec<String> {
        self.electron_window_arguments
            .iter()
            .find(|(candidate, _)| *candidate == window)
            .map(|(_, arguments)| arguments.clone())
            .unwrap_or_default()
    }

    pub(crate) fn register_electron_window_arguments(
        &mut self,
        window: usize,
        arguments: Vec<String>,
    ) {
        self.electron_window_arguments
            .retain(|(candidate, _)| *candidate != window);
        self.electron_window_arguments.push((window, arguments));
    }

    /// Makes this a renderer realm's `process.argv`: the executable, the
    /// renderer's type switch and the window's additional arguments, as
    /// Electron gives a renderer process.
    pub fn set_page_arguments(&mut self, additional: Vec<String>) {
        let mut arguments = alloc::vec![String::from("--type=renderer")];
        arguments.extend(additional);
        self.page_arguments = Some(arguments);
        self.process_argv = None;
    }

    pub(crate) fn register_electron_window_preload(&mut self, window: usize, path: String) {
        self.electron_window_preloads.push((window, path));
    }

    pub(crate) fn register_electron_window_load(&mut self, window: usize, target: String) {
        self.electron_window_loads.push((window, target));
    }

    /// Installs this execution context's compiled module loader for dynamic
    /// `import()` expressions. The loader runs in a Promise job after the
    /// current JavaScript task returns.
    pub fn set_dynamic_import_loader(
        &mut self,
        loader: impl Fn(&mut Realm, usize) -> Completion + 'static,
    ) {
        self.dynamic_import_loader = Some(DynamicImportLoader(Rc::new(loader)));
    }

    /// Dynamically loads one compiler-assigned project entry and returns a
    /// Promise for its namespace. The entry uses this realm's project cache.
    pub fn dynamic_import(&mut self, entry: usize) -> Completion {
        let (promise, resolve, reject) = crate::promise_builtin::capability(self);
        let loader = self.dynamic_import_loader.clone();
        self.enqueue(alloc::boxed::Box::new(move |realm| {
            let started = realm.async_evaluations.len();
            let loaded = match loader {
                Some(loader) => (loader.0)(realm, entry),
                None => Err(realm.throw(
                    ErrorKind::Type,
                    "dynamic import is unavailable outside a compiled project",
                )),
            };
            let pending = realm.async_evaluations.split_off(started);
            match loaded {
                Ok(namespace) => settle_after(realm, pending, namespace, resolve, reject),
                Err(thrown) => {
                    crate::ops::call(realm, &reject, Value::Undefined, &[thrown.0]).map(drop)
                }
            }
        }));
        Ok(promise)
    }

    /// Records the promise of a module body with top-level `await`, which
    /// its generated code starts as an async function.
    pub fn async_evaluation(&mut self, promise: Value) {
        self.async_evaluations.push(promise);
    }

    /// `import(specifier)` with a specifier computed at run time: loads the
    /// project entry whose key, a path the compiler resolved for this site,
    /// ends the specifier (a URL or path, compared without its query or
    /// fragment, with `/` separators and case-insensitively), or returns a
    /// promise rejected with Node's `ERR_MODULE_NOT_FOUND`.
    /// Registers the package scripts this realm may load while it runs.
    pub fn register_scripts(&mut self, scripts: &'static [(&'static str, usize)]) {
        self.scripts = scripts;
    }

    /// The package scripts this realm may load, as registered.
    #[cfg(feature = "dom")]
    pub(crate) fn registered_scripts(&self) -> &'static [(&'static str, usize)] {
        self.scripts
    }

    /// The registered script whose path ends `url`, compared as
    /// [`Realm::dynamic_import_matching`] compares them.
    pub(crate) fn script_entry(&self, url: &str) -> Option<usize> {
        let path = url
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .replace('\\', "/")
            .to_ascii_lowercase();
        self.scripts
            .iter()
            .find(|(key, _)| {
                path.strip_suffix(key)
                    .is_some_and(|before| before.is_empty() || before.ends_with('/'))
            })
            .map(|(_, entry)| *entry)
    }

    /// Evaluates a project entry now, as a classic script's `importScripts`
    /// runs a script before it returns: its namespace.
    pub(crate) fn load_entry_now(&mut self, entry: usize) -> Completion {
        let Some(loader) = self.dynamic_import_loader.clone() else {
            return Err(self.throw(ErrorKind::Type, "scripts load only in a compiled project"));
        };
        (loader.0)(self, entry)
    }

    /// Loads the registered package script whose path ends `url`, as a
    /// script element loads its source: a promise of its completion, or one
    /// rejected with `ERR_MODULE_NOT_FOUND` when no compiled script matches.
    pub fn load_script(&mut self, url: &Value) -> Completion {
        let scripts = self.scripts;
        self.dynamic_import_matching(url, scripts)
    }

    pub fn dynamic_import_matching(
        &mut self,
        specifier: &Value,
        candidates: &[(&str, usize)],
    ) -> Completion {
        let text = crate::ops::to_string(self, specifier)?.to_rust_lossy();
        let path = text
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let found = candidates.iter().find(|(key, _)| {
            let key = key.to_ascii_lowercase();
            path.strip_suffix(&key)
                .is_some_and(|before| before.is_empty() || before.ends_with('/'))
        });
        // A URL the site's candidates do not name may be a registered
        // package script, as loaders import the scripts they name.
        let registered = || {
            self.scripts.iter().find(|(key, _)| {
                path.strip_suffix(key)
                    .is_some_and(|before| before.is_empty() || before.ends_with('/'))
            })
        };
        if let Some((_, entry)) = found.or_else(registered) {
            return self.dynamic_import(*entry);
        }
        let (promise, _, reject) = crate::promise_builtin::capability(self);
        let error = self.error(
            ErrorKind::Error,
            &alloc::format!("Cannot find module '{text}'"),
        );
        error.define(
            crate::Key::name("code"),
            crate::Property::data(Value::str("ERR_MODULE_NOT_FOUND")),
        );
        crate::ops::call(self, &reject, Value::Undefined, &[Value::Object(error)])?;
        Ok(promise)
    }

    /// Lets the functions made in this realm run in it when another realm
    /// calls them, as functions keep their realm's globals in browsers:
    /// frames and their pages hand functions to each other. The realm must
    /// stay where it is until it is dropped.
    pub fn keep_functions_home(&mut self) {
        if self.home {
            return;
        }
        self.home = true;
        let address = self as *mut Realm as usize;
        HOMES.0.borrow_mut().push((self.id, address));
    }

    /// The realm to call a function in from this one: the realm it was made
    /// in when that one keeps its functions home, else this one.
    pub(crate) fn home_of(&self, function: &Object) -> Option<*mut Realm> {
        let home = function.home();
        if home == 0 || home == self.id {
            return None;
        }
        let homes = HOMES.0.borrow();
        homes
            .iter()
            .find(|(id, _)| *id == home)
            .map(|(_, address)| *address as *mut Realm)
    }

    /// A realm with the intrinsics compiled behavior relies on, in a new
    /// language agent.
    pub fn new() -> Realm {
        Realm::in_agent(WellKnown::create(), Rc::default())
    }

    /// Another realm of this realm's agent: it shares the protocol symbols
    /// and the `Symbol.for` registry but has its own intrinsics.
    pub fn sibling(&self) -> Realm {
        Realm::in_agent(self.symbols.clone(), self.registry.clone())
    }

    fn in_agent(
        symbols: WellKnown,
        registry: Rc<core::cell::RefCell<crate::symbol_builtin::Registry>>,
    ) -> Realm {
        let object_prototype = Object::new(None, Kind::Ordinary);
        let ordinary = || Object::new(Some(object_prototype.clone()), Kind::Ordinary);
        let function_prototype = Object::new(
            Some(object_prototype.clone()),
            Kind::Function {
                code: Rc::new(|_, _, _| Ok(Value::Undefined)),
                constructor: false,
                realm: 0,
            },
        );
        let array_prototype = Object::new(
            Some(object_prototype.clone()),
            Kind::Array(ArrayData::new(Vec::new())),
        );
        let error_prototype = ordinary();
        let derived = |name: &str| {
            let prototype = Object::new(Some(error_prototype.clone()), Kind::Ordinary);
            prototype.define(
                crate::string::static_key!("name"),
                Property::hidden(Value::str(name)),
            );
            prototype.define(
                crate::string::static_key!("message"),
                Property::hidden(Value::str("")),
            );
            prototype
        };
        let type_error_prototype = derived("TypeError");
        let range_error_prototype = derived("RangeError");
        let reference_error_prototype = derived("ReferenceError");
        let eval_error_prototype = derived("EvalError");
        let syntax_error_prototype = derived("SyntaxError");
        let uri_error_prototype = derived("URIError");
        let aggregate_error_prototype = derived("AggregateError");
        let suppressed_error_prototype = derived("SuppressedError");
        error_prototype.define(
            crate::string::static_key!("name"),
            Property::hidden(Value::str("Error")),
        );
        let iterators = ordinary();
        error_prototype.define(
            crate::string::static_key!("message"),
            Property::hidden(Value::str("")),
        );
        let mut realm = Realm {
            string_prototype: ordinary(),
            number_prototype: ordinary(),
            boolean_prototype: ordinary(),
            symbol_prototype: ordinary(),
            bigint_prototype: ordinary(),
            iterator_prototype: iterators.clone(),
            array_iterator_prototype: Object::new(Some(iterators.clone()), Kind::Ordinary),
            iterator_next: Vec::new(),
            string_iterator_prototype: Object::new(Some(iterators.clone()), Kind::Ordinary),
            regexp_prototype: ordinary(),
            protected_regexp_prototype: ordinary(),
            protected_regexp_constructor: None,
            patterns: crate::regexp_builtin::Patterns::default(),
            promise_prototype: ordinary(),
            promise_constructor: None,
            timeout_prototype: ordinary(),
            immediate_prototype: ordinary(),
            event_loop: crate::event_loop::EventLoop::default(),
            process_cwd: None,
            process_environment: None,
            process_argv: None,
            process: None,
            process_emitter: false,
            kept: Vec::new(),
            registries: Vec::new(),
            process_streams: [None, None, None],
            exit_code: Value::Undefined,
            async_context: Value::Undefined,
            warning_hint_shown: false,
            node_error_prototypes: Vec::new(),
            native_modules: Vec::new(),
            keyboard_layout_callbacks: Vec::new(),
            native_files: None,
            node_library: None,
            dynamic_import_loader: None,
            async_evaluations: Vec::new(),
            scripts: &[],
            source_functions: Rc::default(),
            buffer_prototype: None,
            electron_version: None,
            electron_app_version: None,
            electron_app_name: None,
            electron_app_paths: Vec::new(),
            electron_bridge_exports: Vec::new(),
            native_stylesheet_sources: Vec::new(),
            native_stylesheet_imports: Vec::new(),
            electron_zoom_factor: 1.0,
            electron_zoom_level: 0.0,
            electron_ipc_main: 0,
            message_ports: Rc::default(),
            page_scripts: None,
            electron_window: 0,
            electron_windows: Vec::new(),
            electron_outgoing: Vec::new(),
            page_notification_events: Vec::new(),
            electron_preloaded: Vec::new(),
            electron_renderer_listeners: None,
            electron_app: None,
            electron_session: None,
            electron_exports: None,
            electron_window_arguments: Vec::new(),
            page_arguments: None,
            electron_quitting: false,
            host_errors: Vec::new(),
            notify_targets: alloc::collections::BTreeMap::new(),
            next_notify_id: 0,
            electron_ipc_registry: RefCell::new(None),
            electron_window_preloads: Vec::new(),
            electron_window_loads: Vec::new(),
            electron_hosts_pages: false,
            electron_load_waiters: Vec::new(),
            electron_window_key: crate::symbol::Symbol::private("nativeboat.electron.window"),
            electron_image_key: crate::symbol::Symbol::private("nativeboat.electron.image"),
            wasm: None,
            electron_images: None,
            electron_image_bitmap_warned: false,
            electron_application_menu: None,
            electron_window_menus: Vec::new(),
            electron_next_menu_id: 1,
            electron_menus_changed: false,
            electron_installed_menus: Vec::new(),
            name_keys: crate::table::HashMap::new(),
            electron_menu_command_key: crate::symbol::Symbol::private(
                "nativeboat.electron.menu.command",
            ),
            generator_prototype: Object::new(Some(iterators), Kind::Ordinary),
            async_iterator_prototype: ordinary(),
            async_generator_prototype: ordinary(),
            async_generator_function_prototype: ordinary(),
            coroutines: Vec::new(),
            idle_fibers: crate::coroutine::FiberPool::default(),
            date_prototype: ordinary(),
            array_buffer_prototype: ordinary(),
            typed_array_prototype: ordinary(),
            typed_array_constructors: Vec::new(),
            typed_array_prototypes: Vec::new(),
            time_zone: None,
            locale: None,
            default_collator: None,
            return_signal: ordinary(),
            symbols,
            registry,
            global: ordinary(),
            throw_type_error: object_prototype.clone(),
            array_values: object_prototype.clone(),
            object_prototype,
            function_prototype,
            array_prototype,
            error_prototype,
            error_constructor: None,
            bridged_functions: Vec::new(),
            error_stack: None,
            #[cfg(feature = "dom")]
            web_host: None,
            #[cfg(feature = "dom")]
            worker_scope: None,
            #[cfg(feature = "dom")]
            workers: Vec::new(),
            #[cfg(feature = "dom")]
            next_worker: 0,
            #[cfg(feature = "dom")]
            worker_closing: false,
            #[cfg(feature = "dom")]
            frame_documents: &[],
            #[cfg(feature = "dom")]
            frame_page: None,
            id: NEXT_REALM.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            home: false,
            #[cfg(feature = "dom")]
            frame_end: None,
            type_error_prototype,
            range_error_prototype,
            reference_error_prototype,
            eval_error_prototype,
            syntax_error_prototype,
            uri_error_prototype,
            aggregate_error_prototype,
            suppressed_error_prototype,
            max_depth: 4_000,
            depth: 0,
            call_profile: crate::trace::calls_profiled().then(crate::trace::CallProfile::default),
            current_new_target: Value::Undefined,
            templates: HashMap::new(),
            joining: Vec::new(),
        };
        crate::builtins::install(&mut realm);
        let thrower = realm.function("", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Type, "Restricted arguments property"))
        });
        for (name, value) in [("length", Value::Number(0.0)), ("name", Value::str(""))] {
            thrower.define(
                Key::name(name),
                Property {
                    slot: Slot::Data {
                        value,
                        writable: false,
                    },
                    enumerable: false,
                    configurable: false,
                },
            );
        }
        thrower.data_mut().extensible = false;
        realm.throw_type_error = thrower;
        realm
    }

    /// A new plain object.
    pub fn object(&self) -> Object {
        Object::new(Some(self.object_prototype.clone()), Kind::Ordinary)
    }

    /// A plain object with room for `properties` properties.
    pub fn object_sized(&self, properties: usize) -> Object {
        Object::with_capacity(
            Some(self.object_prototype.clone()),
            Kind::Ordinary,
            properties,
        )
    }

    /// An unmapped strict arguments object. Indexed values do not alias
    /// parameters; its iterator retains the original array-values intrinsic.
    pub fn arguments(&self, values: &[Value]) -> Object {
        let object = Object::new(Some(self.object_prototype.clone()), Kind::Arguments);
        for (index, value) in values.iter().enumerate() {
            object.define(Key::Index(index as u32), Property::data(value.clone()));
        }
        object.define(
            crate::string::static_key!("length"),
            Property::hidden(Value::Number(values.len() as f64)),
        );
        object.define(
            crate::string::static_key!("callee"),
            Property {
                slot: Slot::Accessor {
                    get: Some(self.throw_type_error.clone()),
                    set: Some(self.throw_type_error.clone()),
                },
                enumerable: false,
                configurable: false,
            },
        );
        object.define(
            Key::Symbol(self.symbols.iterator.clone()),
            Property::hidden(Value::Object(self.array_values.clone())),
        );
        object
    }

    /// A sloppy arguments object whose listed indexed properties alias their
    /// function's simple formal-parameter bindings.
    pub fn mapped_arguments(
        &self,
        values: &[Value],
        mappings: &[(u32, crate::binding::Binding)],
    ) -> Object {
        let object = self.arguments(values);
        for (index, binding) in mappings {
            object.map_argument(*index, binding.clone());
        }
        object
    }

    /// A new array of the given elements.
    pub fn array(&self, elements: Vec<Value>) -> Object {
        Object::new(
            Some(self.array_prototype.clone()),
            Kind::Array(ArrayData::new(elements.into_iter().map(Some).collect())),
        )
    }

    /// A new array with holes where `elements` has `None`.
    pub fn sparse_array(&self, elements: Vec<Option<Value>>) -> Object {
        Object::new(
            Some(self.array_prototype.clone()),
            Kind::Array(ArrayData::new(elements)),
        )
    }

    /// A function object for compiled code. Constructors get a fresh
    /// `prototype` object whose `constructor` refers back to the function.
    pub fn function(
        &self,
        name: &str,
        length: u32,
        constructor: bool,
        code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
    ) -> Object {
        self.native_function(name, length, constructor, Rc::new(code))
    }

    /// `function` for boxed code, generated once for every closure type.
    fn native_function(
        &self,
        name: &str,
        length: u32,
        constructor: bool,
        code: NativeCode,
    ) -> Object {
        let function = Object::new(
            Some(self.function_prototype.clone()),
            Kind::Function {
                code,
                constructor,
                realm: self.id,
            },
        );
        let fixed = |value| Property {
            slot: Slot::Data {
                value,
                writable: false,
            },
            enumerable: false,
            configurable: true,
        };
        function.define(
            crate::string::static_key!("length"),
            fixed(Value::Number(f64::from(length))),
        );
        function.define(crate::string::static_key!("name"), fixed(Value::str(name)));
        if constructor {
            let prototype = self.object();
            prototype.define(
                crate::string::static_key!("constructor"),
                Property::hidden(Value::Object(function.clone())),
            );
            function.define(
                crate::string::static_key!("prototype"),
                Property {
                    slot: Slot::Data {
                        value: Value::Object(prototype),
                        writable: true,
                    },
                    enumerable: false,
                    configurable: false,
                },
            );
        }
        function
    }

    /// Creates a native built-in constructor with a fixed prototype property.
    pub fn builtin_constructor(
        &self,
        name: &str,
        length: u32,
        prototype: &Object,
        call: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
        construct: impl Fn(&mut Realm, Object, &[Value]) -> Result<Object, Thrown> + 'static,
    ) -> Object {
        let function = self.function(name, length, false, call);
        let (code, _) = function.code().expect("new function has native code");
        function.data_mut().kind = Kind::BuiltinConstructor {
            code,
            construct: Rc::new(construct),
        };
        function.define(
            crate::string::static_key!("prototype"),
            Property {
                slot: Slot::Data {
                    value: Value::Object(prototype.clone()),
                    writable: false,
                },
                enumerable: false,
                configurable: false,
            },
        );
        prototype.define(
            crate::string::static_key!("constructor"),
            Property::hidden(Value::Object(function.clone())),
        );
        function
    }

    /// Defines a built-in method on an object.
    pub fn method(
        &self,
        target: &Object,
        name: &str,
        length: u32,
        code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
    ) {
        let function = self.function(name, length, false, code);
        target.define(Key::name(name), Property::hidden(Value::Object(function)));
    }

    /// A new error object with a message.
    pub fn error(&self, kind: ErrorKind, message: &str) -> Object {
        let error = Object::new(Some(self.error_prototype_for(kind).clone()), Kind::Error);
        crate::errors::capture_stack(self, &error);
        error.define(
            crate::string::static_key!("message"),
            Property::hidden(Value::str(message)),
        );
        error
    }

    /// The intrinsic prototype of an error kind.
    pub(crate) fn error_prototype_for(&self, kind: ErrorKind) -> &Object {
        match kind {
            ErrorKind::Error => &self.error_prototype,
            ErrorKind::Type => &self.type_error_prototype,
            ErrorKind::Range => &self.range_error_prototype,
            ErrorKind::Reference => &self.reference_error_prototype,
            ErrorKind::Eval => &self.eval_error_prototype,
            ErrorKind::Syntax => &self.syntax_error_prototype,
            ErrorKind::Uri => &self.uri_error_prototype,
        }
    }

    /// Boxes a symbol without changing its primitive identity.
    ///
    /// Symbol wrappers use their intrinsic prototype and retain the symbol's
    /// identity. Source Object boxing will use this when its lowering is added.
    pub fn symbol_object(&self, symbol: crate::Symbol) -> Object {
        Object::new(
            Some(self.symbol_prototype.clone()),
            Kind::SymbolObject(symbol),
        )
    }

    /// A Boolean, Number or String wrapper object.
    pub fn primitive_object(&self, value: crate::Wrapped) -> Object {
        let prototype = match value {
            crate::Wrapped::Boolean(_) => &self.boolean_prototype,
            crate::Wrapped::Number(_) => &self.number_prototype,
            crate::Wrapped::String(_) => &self.string_prototype,
            crate::Wrapped::BigInt(_) => &self.bigint_prototype,
        };
        Object::new(Some(prototype.clone()), Kind::Primitive(value))
    }

    /// `GetTemplateObject`: the frozen strings array of a tagged template
    /// call site, with its frozen `raw` array. Each site gets one object per
    /// realm; `None` cooked strings (invalid escapes) are `undefined`.
    pub fn template_object(
        &mut self,
        site: u64,
        cooked: &[Option<&[u16]>],
        raw: &[&[u16]],
    ) -> Object {
        if let Some(template) = self.templates.get(&site) {
            return template.clone();
        }
        let strings = |units: Option<&[u16]>| {
            units.map_or(Value::Undefined, |units| {
                Value::String(JsStr::from_units(units))
            })
        };
        let template = self.array(cooked.iter().map(|units| strings(*units)).collect());
        let raw = self.array(raw.iter().map(|units| strings(Some(units))).collect());
        let fixed = Property {
            slot: Slot::Data {
                value: Value::Object(raw.clone()),
                writable: false,
            },
            enumerable: false,
            configurable: false,
        };
        template.define(crate::string::static_key!("raw"), fixed);
        for array in [&template, &raw] {
            freeze(array);
        }
        self.templates.insert(site, template.clone());
        template
    }

    /// The report of an exception nothing caught, as `Uncaught <ToString>`
    /// (for errors, `Uncaught TypeError: message`). A value whose
    /// conversion throws is shown in its native form.
    pub fn uncaught(&mut self, thrown: &Value) -> String {
        let text = match crate::ops::to_string(self, thrown) {
            Ok(text) => text.to_rust_lossy(),
            Err(_) => format!("{thrown:?}"),
        };
        format!("Uncaught {text}\n")
    }

    /// A thrown error of the given kind.
    pub fn throw(&self, kind: ErrorKind, message: &str) -> Thrown {
        let error = Value::Object(self.error(kind, message));
        crate::trace::thrown(&error);
        Thrown(error)
    }

    /// Runs compiled code one call level deeper, failing with a RangeError
    /// when the nesting limit is reached.
    pub fn enter(&mut self, code: &NativeCode, this: Value, args: &[Value]) -> Completion {
        self.enter_target(code, this, args, Value::Undefined)
    }

    /// The current invocation's new target. Generated closures snapshot it
    /// before parameter evaluation; arrows capture that lexical snapshot.
    pub fn new_target(&self) -> Value {
        self.current_new_target.clone()
    }

    pub(crate) fn enter_target(
        &mut self,
        code: &NativeCode,
        this: Value,
        args: &[Value],
        target: Value,
    ) -> Completion {
        self.with_depth(|realm| realm.with_target(target, |realm| code(realm, this, args)))
    }

    fn with_target<T>(
        &mut self,
        target: Value,
        code: impl FnOnce(&mut Realm) -> Result<T, Thrown>,
    ) -> Result<T, Thrown> {
        let previous = core::mem::replace(&mut self.current_new_target, target);
        let result = code(self);
        self.current_new_target = previous;
        result
    }

    /// Enters native construction under the same call-depth limit as calls.
    pub fn enter_constructor(
        &mut self,
        code: &ConstructorCode,
        new_target: Object,
        args: &[Value],
    ) -> Result<Object, Thrown> {
        self.with_depth(|realm| {
            realm.with_target(Value::Object(new_target.clone()), |realm| {
                code(realm, new_target, args)
            })
        })
    }

    /// Whether calls are within a few frames of the depth limit, where
    /// `NATIVEBOAT_TRACE_THROWS` names them to show what recursed.
    pub(crate) fn near_depth_limit(&self) -> bool {
        self.depth + 24 >= self.max_depth
    }

    pub(crate) fn with_depth<T>(
        &mut self,
        code: impl FnOnce(&mut Realm) -> Result<T, Thrown>,
    ) -> Result<T, Thrown> {
        if self.depth >= self.max_depth {
            return Err(self.throw(ErrorKind::Range, "Maximum call stack size exceeded"));
        }
        self.depth += 1;
        let result = code(self);
        self.depth -= 1;
        result
    }

    /// Defines a global binding as a writable, configurable data property.
    pub fn set_global(&self, name: &str, value: Value) {
        self.global
            .define(Key::from_js(&JsStr::from(name)), Property::hidden(value));
    }

    /// The global object, also used as the renderer's `window` object.
    pub fn global_object(&self) -> Object {
        self.global.clone()
    }

    /// The key of a property name fixed in the program, made once: later
    /// uses share it instead of converting the name again.
    pub fn name_key(&mut self, name: &'static str) -> Key {
        let identity = (name.as_ptr() as usize, name.len());
        if let Some(key) = self.name_keys.get(&identity) {
            return key.clone();
        }
        let key = Key::name(name);
        self.name_keys.insert(identity, key.clone());
        key
    }
}

/// Makes every own property read-only and non-configurable and the object
/// non-extensible.
pub(crate) fn freeze(object: &Object) {
    object.prevent_extensions();
    for key in object.own_keys() {
        let accessor = matches!(
            object.own_property(&key).map(|property| property.slot),
            Some(Slot::Accessor { .. })
        );
        let descriptor = crate::Descriptor {
            configurable: Some(false),
            writable: (!accessor).then_some(false),
            ..crate::Descriptor::default()
        };
        object.define_own_property(&key, &descriptor);
    }
}

#[cfg(test)]
mod tests;

/// Resolves a dynamic import with `namespace` once every module body it
/// started that awaits at top level has finished, or rejects it with the
/// first one's error, as an import waits for its module graph to evaluate.
fn settle_after(
    realm: &mut Realm,
    mut pending: Vec<Value>,
    namespace: Value,
    resolve: Value,
    reject: Value,
) -> Result<(), Thrown> {
    let Some(first) = (!pending.is_empty()).then(|| pending.remove(0)) else {
        return crate::ops::call(realm, &resolve, Value::Undefined, &[namespace]).map(drop);
    };
    let evaluation = crate::promise_builtin::resolved(realm, first)?;
    let rejected = reject.clone();
    let on_fulfilled = realm.function("", 1, false, move |realm, _, _| {
        settle_after(
            realm,
            pending.clone(),
            namespace.clone(),
            resolve.clone(),
            reject.clone(),
        )?;
        Ok(Value::Undefined)
    });
    let on_rejected = realm.function("", 1, false, move |realm, _, args| {
        let reason = args.first().cloned().unwrap_or_default();
        crate::ops::call(realm, &rejected, Value::Undefined, &[reason])?;
        Ok(Value::Undefined)
    });
    crate::promise_builtin::react(
        realm,
        &evaluation,
        Value::Object(on_fulfilled),
        Value::Object(on_rejected),
    );
    Ok(())
}
