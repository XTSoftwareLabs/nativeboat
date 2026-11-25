//! The scripts a renderer's workers may run, which the compiler finds by
//! the paths a program names them by (`RequestKind::Worker`): a path given
//! as `./worker.js` names a file next to the module that names it, as
//! `new URL('./worker.js', import.meta.url)` does; another path names a file
//! in one of the folders above the module, up to its package's folder, as
//! programs name scripts from the root their URLs are made from (VS Code's
//! `FileAccess.asBrowserUri('vs/editor/common/services/editorWebWorkerMain.js')`).

use std::path::{Path, PathBuf};

/// The file `name` names from the module `from`, or the TypeScript source it
/// is compiled from.
pub(crate) fn locate(from: &Path, name: &str) -> Option<PathBuf> {
    let found = if let Some(relative) = name.strip_prefix("./") {
        nativeboat_project::plugins::existing(&from.parent()?.join(relative))
    } else {
        let mut found = None;
        for folder in from.ancestors().skip(1) {
            found = nativeboat_project::plugins::existing(&folder.join(name));
            if found.is_some() || folder.join("package.json").is_file() {
                break;
            }
        }
        found
    }?;
    Some(nativeboat_bundler::resolve::realpath(&found))
}

#[cfg(test)]
mod tests;
