//! The Web API prelude of renderer pages: `nativeboat-web`'s JavaScript
//! implementation of `window`, `document`, nodes, events, ranges, editing
//! and the other browser APIs, compiled into each renderer graph as its first
//! module. It runs over the runtime's document natives
//! (`nativeboat_native::web`), which the generated window code installs
//! before the page's scripts run. Without them, a build that has no packaged
//! renderer libraries, the module does nothing.

use nativeboat_compiler::{ModuleKind, ModuleSource};
use std::path::{Path, PathBuf};

/// The prelude's parts, in order.
const PARTS: &[crate::Text] = &[
    nativeboat_conceal::text!("../../nativeboat-web/src/js/core.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/events.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/nodes.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/custom-elements.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/ranges.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/elements.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/shadow.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/canvas.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/xhr.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/window.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/frames.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/editing.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/composition.js"),
    nativeboat_conceal::text!("../../nativeboat-web/src/js/editing-history.js"),
];

/// The global scope of a worker, which the prelude runs in a worker's realm
/// instead of its parts.
const WORKER: crate::Text = nativeboat_conceal::text!("../../nativeboat-web/src/js/worker.js");

/// The global through which the runtime passes a worker's prelude its
/// arguments, as `nativeboat_native::worker::PRELUDE_ARGUMENTS` names it.
const WORKER_ARGUMENTS: &str = "__nativeboat_worker";

/// The global through which the runtime passes the prelude its arguments,
/// as `nativeboat_native::web::PRELUDE_ARGUMENTS` names it.
const ARGUMENTS: &str = "__nativeboat_web";

/// The prelude's virtual module path in a project.
pub(crate) fn path(root: &Path) -> PathBuf {
    root.join(".nativeboat/generated/web-prelude.cjs")
}

/// The prelude module's source, if `path` is the prelude of a project.
pub(crate) fn source(path: &Path) -> Option<ModuleSource> {
    if !path.ends_with(".nativeboat/generated/web-prelude.cjs") {
        return None;
    }
    let mut text = format!(
        "'use strict';\nconst prelude = globalThis.{ARGUMENTS};\nif (prelude) {{\ndelete globalThis.{ARGUMENTS};\n(function (host, native, page) {{\n'use strict';\n"
    );
    for part in PARTS {
        text.push_str(part());
        text.push('\n');
    }
    // A worker's realm runs the worker's global scope instead.
    text.push_str(&format!(
        "}})(prelude[0], prelude[1], prelude[2]);\n}} else if (globalThis.{WORKER_ARGUMENTS}) {{\nconst worker = globalThis.{WORKER_ARGUMENTS};\ndelete globalThis.{WORKER_ARGUMENTS};\n(function (host, port, errors, url, name, script, service) {{\n'use strict';\n"
    ));
    text.push_str(WORKER());
    text.push_str(
        "\n})(worker[0], worker[1], worker[2], worker[3], worker[4], worker[5], worker[6]);\n}\n",
    );
    Some(ModuleSource {
        text,
        kind: ModuleKind::CommonJs,
    })
}

#[cfg(test)]
mod tests;
