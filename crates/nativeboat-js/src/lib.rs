//! JavaScript host on Boa: runs bundles in the runtime module format with
//! timers, microtasks, console output and uncaught error reporting.
//!
//! Most of the host is JavaScript (`prelude.js`); the Rust side provides
//! compilation with source paths, the clock, console output and the job
//! queue that runs promise reactions as microtasks, system randomness and
//! the channel that brings results of background work back to JavaScript.

mod background;
pub mod intl;
mod sources;
mod stack;
mod timers;

use boa_engine::builtins::promise::{OperationType, PromiseState};
use boa_engine::context::HostHooks;
use boa_engine::job::{GenericJob, Job, JobExecutor, PromiseJob};
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::object::builtins::{JsPromise, JsUint8Array};
use boa_engine::{
    Context, Finalize, JsArgs, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue,
    NativeFunction, Source, Trace, js_string,
};
use nativeboat_bundler::Resolution;
use nativeboat_ir::Script;
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};

pub use background::{Background, Payload};
pub use boa_engine;

const PRELUDE: &str = include_str!("prelude.js");
const INTL: &str = include_str!("intl.js");
const STACK: &str = include_str!("stack.js");

/// Console output levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `console.log`
    Log,
    /// `console.info`
    Info,
    /// `console.debug`
    Debug,
    /// `console.warn`
    Warn,
    /// `console.error` and uncaught errors.
    Error,
}

/// Receives console output.
pub type Sink = Rc<dyn Fn(Level, &str)>;

enum Task {
    Promise(PromiseJob),
    Generic(GenericJob),
}

/// The microtask queue and the promises rejected without a handler.
#[derive(Default)]
struct Queue {
    tasks: RefCell<VecDeque<Task>>,
    rejections: RefCell<Vec<JsObject>>,
}

impl JobExecutor for Queue {
    fn enqueue_job(self: Rc<Self>, job: Job, _: &mut Context) {
        let task = match job {
            Job::PromiseJob(job) => Task::Promise(job),
            Job::GenericJob(job) => Task::Generic(job),
            // Timers are run by the native timer store; Boa enqueues no other jobs
            // for scripts.
            _ => return,
        };
        self.tasks.borrow_mut().push_back(task);
    }

    fn run_jobs(self: Rc<Self>, context: &mut Context) -> JsResult<()> {
        loop {
            let task = self.tasks.borrow_mut().pop_front();
            match task {
                Some(Task::Promise(job)) => job.call(context)?,
                Some(Task::Generic(job)) => job.call(context)?,
                None => return Ok(()),
            };
        }
    }
}

impl HostHooks for Queue {
    fn promise_rejection_tracker(
        &self,
        promise: &JsObject<boa_engine::builtins::promise::Promise>,
        operation: OperationType,
        _: &mut Context,
    ) {
        let promise = promise.clone().upcast();
        let mut rejections = self.rejections.borrow_mut();
        match operation {
            OperationType::Reject => rejections.push(promise),
            OperationType::Handle => {
                rejections.retain(|rejected| !JsObject::equals(rejected, &promise));
            }
        }
    }
}

/// Rust state shared with the host's native functions. It holds no
/// garbage-collected values.
#[derive(Clone, Trace, Finalize)]
struct Shared {
    #[unsafe_ignore_trace]
    sink: Sink,
    #[unsafe_ignore_trace]
    started: Instant,
}

fn text(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get_or_undefined(index)
        .to_string(context)?
        .to_std_string_escaped())
}

fn compile(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let code = text(args, 0, context)?;
    let path = text(args, 1, context)?;
    // Extra parameter names, such as `process` for preload scripts.
    let extra = text(args, 2, context)?;
    compile_source(&code, &path, &extra, context)
}

fn compile_source(code: &str, path: &str, extra: &str, context: &mut Context) -> JsResult<JsValue> {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(", {extra}")
    };
    // The module runs in a function of its own, so that its top-level
    // declarations, such as `const __dirname` in an ES module, shadow the
    // wrapper's parameters instead of clashing with them.
    let prefix = format!(
        "(function (exports, require, module, __filename, __dirname, __nui_import_meta, __nui_require{extra}) {{ return (function () {{"
    );
    let wrapped = format!("{prefix}{code}\n}}).call(this); }})");
    // Avoid an additional whole-AST optimization walk for module factories.
    // Preserve host options for subsequent evaluations and parse failures.
    let options = context.optimizer_options();
    context.set_optimizer_options(boa_engine::optimizer::OptimizerOptions::empty());
    let result = context.eval(Source::from_bytes(&wrapped).with_path(Path::new(path)));
    context.set_optimizer_options(options);
    result
}

/// Cryptographically strong random bytes from the system generator.
pub fn random_bytes(size: usize) -> Result<Vec<u8>, String> {
    let length = u32::try_from(size).map_err(|_| format!("{size} random bytes is too many"))?;
    let mut bytes = vec![0; size];
    // SAFETY: the buffer is valid for writes of `length` bytes.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            length,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!(
            "the system random generator failed (0x{status:08X})"
        ));
    }
    Ok(bytes)
}

/// A page's title changed; this host has no window to show it.
fn title(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn random(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let size = args.get_or_undefined(0).to_length(context)?;
    let bytes = random_bytes(size as usize)
        .map_err(|message| JsNativeError::error().with_message(message))?;
    Ok(JsUint8Array::from_iter(bytes, context)?.into())
}

/// Task ids for every host of the process.
static NEXT_TASK: AtomicU32 = AtomicU32::new(1);

fn task_id(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(NEXT_TASK.fetch_add(1, Ordering::Relaxed)))
}

fn locale(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsString::from(intl::default_locale()).into())
}

/// A Chromium user agent, as pages outside Electron applications read it.
fn user_agent(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsString::from(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    )
    .into())
}

fn processors(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(std::thread::available_parallelism()
        .map_or(1, |count| count.get() as u32)
        .into())
}

fn system_locale(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsString::from(intl::system_locale()).into())
}

fn set_locale(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let locale = args.get_or_undefined(0);
    let locale = if locale.is_null_or_undefined() {
        None
    } else {
        Some(locale.to_string(context)?.to_std_string_escaped())
    };
    intl::set_default_locale(locale);
    Ok(JsValue::undefined())
}

fn locale_data(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let tag = text(args, 0, context)?;
    match intl::locale_data(&tag) {
        Some(data) => JsValue::from_json(&data, context),
        None => Ok(JsValue::null()),
    }
}

fn time_zone(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsString::from(intl::local_time_zone()).into())
}

fn zone_offset(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = text(args, 0, context)?;
    let moment = args.get_or_undefined(1).to_number(context)?;
    Ok(intl::zone_offset(&key, moment).map_or(JsValue::null(), JsValue::from))
}

fn print(
    _: &JsValue,
    args: &[JsValue],
    shared: &Shared,
    context: &mut Context,
) -> JsResult<JsValue> {
    let level = match text(args, 0, context)?.as_str() {
        "info" => Level::Info,
        "debug" => Level::Debug,
        "warn" => Level::Warn,
        "error" => Level::Error,
        _ => Level::Log,
    };
    let message = text(args, 1, context)?;
    (shared.sink)(level, &message);
    Ok(JsValue::undefined())
}

fn now(_: &JsValue, _: &[JsValue], shared: &Shared, _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(
        shared.started.elapsed().as_secs_f64() * 1000.0,
    ))
}

fn call_method(
    context: &mut Context,
    object: &JsObject,
    name: &str,
    args: &[JsValue],
) -> JsResult<JsValue> {
    let function = object.get(JsString::from(name), context)?;
    let callable = function
        .as_callable()
        .ok_or_else(|| JsNativeError::typ().with_message(format!("{name} is not a function")))?;
    callable.call(&JsValue::from(object.clone()), args, context)
}

fn report(context: &mut Context, host: &JsObject, error: JsValue, promise: bool) {
    // The prelude's reporter prints and never throws.
    let _ = call_method(context, host, "report", &[error, JsValue::from(promise)]);
}

fn report_error(context: &mut Context, host: &JsObject, error: JsError) {
    match error.into_opaque(context) {
        Ok(value) => report(context, host, value, false),
        Err(error) => {
            // Engine failures are uncatchable and cannot become JS values.
            // Their Display includes the original bytecode backtrace.
            let _ = call_method(
                context,
                host,
                "print",
                &[
                    JsValue::from(js_string!("error")),
                    JsValue::from(JsString::from(format!("Uncaught {error}"))),
                ],
            );
        }
    }
}

fn run_microtasks(context: &mut Context, host: &JsObject, queue: &Queue) {
    while let Err(error) = context.run_jobs() {
        report_error(context, host, error);
    }
    // The checkpoint ends the job, so `WeakRef` targets it read may now be
    // collected.
    context.clear_kept_objects();
    let rejected = queue.rejections.take();
    for promise in rejected {
        if let Ok(promise) = JsPromise::from_object(promise)
            && let PromiseState::Rejected(reason) = promise.state()
        {
            report(context, host, reason, true);
        }
    }
}

/// A JavaScript context with the nativeboat host installed.
pub struct Host {
    context: Context,
    host: JsObject,
    queue: Rc<Queue>,
    background: Background,
    events: Receiver<background::Message>,
}

impl fmt::Debug for Host {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Host").finish_non_exhaustive()
    }
}

impl Host {
    /// Creates a context; `node` selects Node.js flavoured globals (timer
    /// handles and `global`) for the main process.
    pub fn new(node: bool, sink: Sink) -> Host {
        let queue = Rc::new(Queue::default());
        let mut context = Context::builder()
            .job_executor(queue.clone())
            .host_hooks(queue.clone())
            .build()
            .expect("a default context builds");
        let (sender, events) = channel();
        let background = Background::new(sender);
        context.insert_data(background.clone());
        let host = JsObject::with_null_proto();
        let shared = Shared {
            sink,
            started: Instant::now(),
        };
        timers::install(&host, shared.started, queue.clone(), &mut context);
        let natives = [
            ("compile", NativeFunction::from_fn_ptr(compile)),
            ("stackFrames", NativeFunction::from_fn_ptr(stack::frames)),
            ("random", NativeFunction::from_fn_ptr(random)),
            ("title", NativeFunction::from_fn_ptr(title)),
            ("taskId", NativeFunction::from_fn_ptr(task_id)),
            ("locale", NativeFunction::from_fn_ptr(locale)),
            ("userAgent", NativeFunction::from_fn_ptr(user_agent)),
            ("processors", NativeFunction::from_fn_ptr(processors)),
            ("localeData", NativeFunction::from_fn_ptr(locale_data)),
            ("systemLocale", NativeFunction::from_fn_ptr(system_locale)),
            ("setLocale", NativeFunction::from_fn_ptr(set_locale)),
            ("timeZone", NativeFunction::from_fn_ptr(time_zone)),
            ("zoneOffset", NativeFunction::from_fn_ptr(zone_offset)),
            (
                "print",
                NativeFunction::from_copy_closure_with_captures(print, shared.clone()),
            ),
            (
                "now",
                NativeFunction::from_copy_closure_with_captures(now, shared),
            ),
        ];
        for (name, function) in natives {
            let function = FunctionObjectBuilder::new(context.realm(), function)
                .name(JsString::from(name))
                .build();
            host.set(JsString::from(name), function, false, &mut context)
                .expect("the host object is extensible");
        }
        let prelude = context
            .eval(Source::from_bytes(PRELUDE).with_path(Path::new("nativeboat:prelude")))
            .expect("the prelude compiles");
        prelude
            .as_callable()
            .expect("the prelude is a function")
            .call(
                &JsValue::undefined(),
                &[JsValue::from(host.clone()), JsValue::from(node)],
                &mut context,
            )
            .expect("the prelude runs");
        context
            .eval(Source::from_bytes(STACK).with_path(Path::new("nativeboat:stack")))
            .expect("stack trace support compiles")
            .as_callable()
            .expect("stack trace support is a function")
            .call(
                &JsValue::undefined(),
                &[JsValue::from(host.clone())],
                &mut context,
            )
            .expect("stack trace support installs");
        context
            .eval(Source::from_bytes(INTL).with_path(Path::new("nativeboat:intl")))
            .expect("Intl compiles")
            .as_callable()
            .expect("Intl is a function")
            .call(
                &JsValue::undefined(),
                &[JsValue::from(host.clone())],
                &mut context,
            )
            .expect("Intl installs");
        Host {
            context,
            host,
            queue,
            background,
            events,
        }
    }

    /// The Boa context.
    pub fn context(&mut self) -> &mut Context {
        &mut self.context
    }

    /// The object holding the host's functions, for later layers.
    pub fn host_object(&self) -> JsObject {
        self.host.clone()
    }

    /// Makes `require(name)` and `require("node:" + name)` return `exports`.
    pub fn define_builtin(&mut self, name: &str, exports: JsValue) {
        let builtins = self
            .host
            .get(js_string!("builtins"), &mut self.context)
            .expect("the prelude defines builtins");
        builtins
            .as_object()
            .expect("builtins is an object")
            .set(JsString::from(name), exports, false, &mut self.context)
            .expect("builtins is extensible");
    }

    /// Runs the entries of a bundled script. `modules` tells, per entry,
    /// whether it is a module; classic scripts run in the global scope.
    /// `root` prefixes module paths in `__filename` and `__dirname`.
    pub fn load(&mut self, script: &Script, modules: &[bool], root: &str) -> Result<(), String> {
        self.load_scoped(script, modules, root, None)
    }

    /// Like [`Host::load`], with extra names visible to every module: the
    /// enumerable properties of `scope`.
    pub fn load_scoped(
        &mut self,
        script: &Script,
        modules: &[bool],
        root: &str,
        scope: Option<&JsObject>,
    ) -> Result<(), String> {
        let records: Vec<Value> = script
            .bundle
            .modules
            .iter()
            .map(|module| {
                let dependencies: Map<String, Value> = module
                    .dependencies
                    .iter()
                    .map(|(specifier, resolution)| {
                        let target = match resolution {
                            Resolution::Module(index) => json!(index),
                            Resolution::Builtin(name) => json!({ "builtin": name }),
                            Resolution::Empty => json!({ "empty": true }),
                            Resolution::Missing(name) => json!({ "missing": name }),
                            Resolution::File(path) => json!({ "file": path }),
                        };
                        (specifier.clone(), target)
                    })
                    .collect();
                json!({
                    "path": module.path,
                    "runtimePath": module.runtime_path,
                    "dependencies": dependencies,
                })
            })
            .collect();
        let entries: Vec<Value> = script
            .bundle
            .entries
            .iter()
            .enumerate()
            .map(|(position, index)| {
                json!({ "index": index, "classic": modules.get(position) == Some(&false) })
            })
            .collect();
        // A bundler puts all modules of an entry into the file the application
        // loads; modules see that file as `__filename`.
        let entry = script
            .bundle
            .entries
            .first()
            .map(|index| script.bundle.modules[*index].path.as_str());
        let filename = script
            .runtime_paths
            .first()
            .filter(|runtime| Some(runtime.as_str()) != entry)
            .map(|runtime| format!("{root}{runtime}"));
        let descriptor = json!({
            "modules": records,
            "entries": entries,
            "env": script.env,
            "root": root,
            "filename": filename,
        });
        let descriptor =
            JsValue::from_json(&descriptor, &mut self.context).expect("JSON descriptors convert");
        sources::install(
            &descriptor.as_object().expect("bundle descriptor"),
            script,
            &mut self.context,
        )
        .expect("native source functions install on the descriptor");
        let scope = scope.map_or(JsValue::undefined(), |scope| JsValue::from(scope.clone()));
        let result = call_method(&mut self.context, &self.host, "load", &[descriptor, scope]);
        self.run_microtasks();
        result.map(|_| ()).map_err(|error| self.describe(error))
    }

    /// Evaluates a script in the global scope.
    pub fn eval(&mut self, code: &str, path: &str) -> Result<JsValue, String> {
        let result = self
            .context
            .eval(Source::from_bytes(code).with_path(Path::new(path)));
        result.map_err(|error| self.describe(error))
    }

    /// Formats an error with its stack, as uncaught errors are printed.
    pub fn describe(&mut self, error: JsError) -> String {
        let value = match error.into_opaque(&mut self.context) {
            Ok(value) => value,
            Err(error) => return error.to_string(),
        };
        call_method(&mut self.context, &self.host, "describe", &[value])
            .and_then(|text| text.to_string(&mut self.context))
            .map_or_else(
                |_| "unknown error".to_owned(),
                |text| text.to_std_string_escaped(),
            )
    }

    /// Reports an uncaught error: `host.onerror` may handle it, otherwise it
    /// is printed.
    pub fn report(&mut self, error: JsError) {
        report_error(&mut self.context, &self.host, error);
    }

    /// Calls a function, reporting what it throws.
    pub fn call(
        &mut self,
        function: &JsValue,
        this: &JsValue,
        args: &[JsValue],
    ) -> Option<JsValue> {
        let Some(callable) = function.as_callable() else {
            self.report(
                JsNativeError::typ()
                    .with_message("the callback is not a function")
                    .into(),
            );
            return None;
        };
        match callable.call(this, args, &mut self.context) {
            Ok(value) => Some(value),
            Err(error) => {
                self.report(error);
                None
            }
        }
    }

    /// Runs promise reactions and reports promises rejected without a
    /// handler.
    pub fn run_microtasks(&mut self) {
        run_microtasks(&mut self.context, &self.host, &self.queue);
    }

    fn timers(&mut self) -> JsObject {
        self.host
            .get(js_string!("timers"), &mut self.context)
            .expect("the prelude defines timers")
            .as_object()
            .expect("timers is an object")
    }

    /// Runs the timers that are due, each followed by a microtask checkpoint.
    pub fn run_timers(&mut self) {
        let timers = self.timers();
        // Ordinary exceptions are reported by the prelude; engine failures
        // bypass its JavaScript try/catch and must be reported here.
        if let Err(error) = call_method(&mut self.context, &timers, "run", &[]) {
            self.report(error);
        }
        self.run_microtasks();
    }

    /// How long until the next timer is due, if any is scheduled.
    pub fn next_timer(&mut self) -> Option<Duration> {
        let timers = self.timers();
        let wait = call_method(&mut self.context, &timers, "next", &[])
            .ok()
            .and_then(|value| value.as_number())
            .unwrap_or(-1.0);
        (wait >= 0.0).then(|| Duration::from_secs_f64(wait / 1000.0))
    }

    /// Converts a value to JSON for passing between contexts, as
    /// `JSON.stringify` does; values without a JSON form become `null`.
    pub fn to_json(&mut self, value: &JsValue) -> Value {
        call_method(
            &mut self.context,
            &self.host,
            "toJSON",
            std::slice::from_ref(value),
        )
        .ok()
        .and_then(|text| text.as_string().map(|text| text.to_std_string_escaped()))
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
    }

    /// Creates a value from JSON.
    pub fn from_json(&mut self, value: &Value) -> JsValue {
        JsValue::from_json(value, &mut self.context).expect("JSON values convert")
    }

    /// Sets what wakes the host's thread when background work sends an
    /// event; it runs on the sending thread.
    pub fn set_waker(&self, wake: impl Fn() + Send + 'static) {
        self.background.set_wake(Box::new(wake));
    }

    fn deliver(&mut self, message: background::Message) {
        let payload = match message.payload {
            Payload::None => JsValue::undefined(),
            Payload::Number(number) => JsValue::from(number),
            Payload::Text(text) => JsValue::from(JsString::from(text)),
            Payload::Bytes(bytes) => JsUint8Array::from_iter(bytes, &mut self.context)
                .expect("byte arrays are created")
                .into(),
        };
        let args = [
            JsValue::from(message.task),
            JsValue::from(JsString::from(message.event)),
            payload,
        ];
        // The prelude reports what task handlers throw.
        let _ = call_method(&mut self.context, &self.host, "complete", &args);
        self.run_microtasks();
    }

    /// Delivers the events that background work sent; returns whether there
    /// were any.
    pub fn run_background(&mut self) -> bool {
        let mut delivered = false;
        while let Ok(message) = self.events.try_recv() {
            self.deliver(message);
            delivered = true;
        }
        delivered
    }

    /// Waits up to `timeout` for an event from background work, then
    /// delivers every event that arrived; returns whether there were any.
    pub fn wait_background(&mut self, timeout: Duration) -> bool {
        match self.events.recv_timeout(timeout) {
            Ok(message) => {
                self.deliver(message);
                self.run_background();
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests;
