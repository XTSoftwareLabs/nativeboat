//! Nested browsing contexts: the documents `<iframe>` elements show. A
//! frame's document is one the build compiled, as it compiles pages, found
//! by the path its URL names; it runs in a realm of its own on its page's
//! thread, which the page steps after itself as it steps its workers.
//!
//! A frame and its page talk over a channel of the process's message
//! ports: each side's prelude wraps its end as the other's window, so
//! `postMessage` between them carries data, the sender's origin and
//! transferred ports, and the frame tells its page when it loaded. A frame
//! of the page's own origin is a realm of the page's agent, and the two
//! reach each other's windows directly, as same-origin frames do.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::worker::{Worker, WorkerScope};
use crate::{ErrorKind, Key, Property, Realm, Thrown, Value, ops};
use alloc::boxed::Box;

/// The global through which a frame's prelude receives its end of the
/// channel to its page.
pub const PRELUDE_ARGUMENTS: &str = "__nativeboat_frame";

/// A document the build compiled that frames may show.
#[derive(Debug, Clone, Copy)]
pub struct FrameDocument {
    /// The paths its URLs end with, lowercase with `/` separators.
    pub paths: &'static [&'static str],
    /// Its HTML.
    pub html: &'static str,
    /// What a realm of the document evaluates first: the Web API prelude,
    /// which its workers also start with.
    pub scope: WorkerScope,
    /// Runs the document's scripts.
    pub initialize: WorkerScope,
}

/// Makes a frame's page in a realm: a document for its HTML, which the
/// realm's prelude adopts, laid out at a size, in the top-level window it
/// is shown in, at the `<iframe>` element of the page whose realm is given.
pub type FramePage = fn(&mut Realm, &str, &str, (f32, f32), usize, *mut Realm, u32);

/// Ends what [`FramePage`] made for a frame's realm before the realm goes
/// away.
pub type FrameEnd = fn(*mut Realm);

impl Realm {
    /// Lets this page's frames show the documents the build compiled, made
    /// with `page` and ended with `end`.
    pub fn set_frame_documents(
        &mut self,
        documents: &'static [FrameDocument],
        page: FramePage,
        end: FrameEnd,
    ) {
        self.frame_documents = documents;
        self.frame_page = Some(page);
        self.frame_end = Some(end);
    }
}

/// The compiled document a URL names: the one with a path its path ends
/// with, ignoring case, query and fragment.
fn document(documents: &[FrameDocument], url: &str) -> Option<FrameDocument> {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let path = path.replace('\\', "/").to_ascii_lowercase();
    documents
        .iter()
        .find(|document| document.paths.iter().any(|known| path.ends_with(known)))
        .copied()
}

/// Starts the frame of `page` that shows `url`, at a size, with the end
/// `port` of its channel, for the `<iframe>` element `node`; a frame of the
/// page's origin gets the page's window as its parent's. Returns the
/// frame's id and window, or 0 and undefined when the build compiled no
/// document for the URL, which leaves the frame empty.
pub(crate) fn start(
    page: &mut Realm,
    port: &Value,
    url: &Value,
    size: (f32, f32),
    node: u32,
    same_origin: bool,
) -> Result<(u32, Value), Thrown> {
    let Some(port) = crate::realm_ports::end_of(page, port) else {
        return Err(page.throw(ErrorKind::Type, "a frame's port must be a live channel end"));
    };
    let url = ops::to_string(page, url)?.to_rust_lossy();
    let (Some(make_page), Some(document)) = (page.frame_page, document(page.frame_documents, &url))
    else {
        return Ok((0, Value::Undefined));
    };
    crate::realm_ports::move_out(page, &[port]);
    let mut realm = Box::new(page.sibling());
    // A frame and its page call each other's functions, which run in the
    // realm that made them.
    page.keep_functions_home();
    realm.keep_functions_home();
    realm.message_ports = page.message_ports.clone();
    realm.node_library = page.node_library;
    realm.source_functions = page.source_functions.clone();
    realm.electron_version = page.electron_version;
    realm.frame_documents = page.frame_documents;
    realm.frame_page = page.frame_page;
    realm.frame_end = page.frame_end;
    realm.worker_scope = Some(document.scope);
    realm.set_global("window", Value::Object(realm.global_object()));
    realm.set_renderer_location(&url);
    let parent: *mut Realm = page;
    make_page(
        &mut realm,
        &url,
        document.html,
        size,
        page.electron_window,
        parent,
        node,
    );
    let port = crate::realm_ports::adopt_handle(&mut realm, port);
    let parent_window = if same_origin {
        Value::Object(page.global_object())
    } else {
        Value::Null
    };
    let arguments = realm.array(vec![Value::Object(port), parent_window]);
    realm.global_object().define(
        Key::name(PRELUDE_ARGUMENTS),
        Property::hidden(Value::Object(arguments)),
    );
    // A frame's exceptions reach its own handlers, as a page's do; they
    // never end its page. Its document loads in a later task, once the
    // page that made the frame can listen to it.
    if realm
        .start_host(document.initialize)
        .is_none_or(|status| status == 0)
    {
        let ready = realm.function("ready", 0, false, |realm, _, _| {
            realm.web_call("ready", &[])?;
            Ok(Value::Undefined)
        });
        realm.native_timeout(ready, 0.0, alloc::rc::Rc::new(core::cell::Cell::new(true)));
    }
    let window = Value::Object(realm.global_object());
    page.next_worker += 1;
    let id = page.next_worker;
    page.workers.push(Worker::new(id, realm));
    Ok((id, window))
}
