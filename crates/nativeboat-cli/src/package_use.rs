//! Which installed packages the application's Node.js side runs: the main
//! process, preloads and forked modules. Where a bundler builds the pages,
//! as electron-vite does, the packages only pages use are bundled into the
//! pages in an Electron build, and the native build embeds what its pages
//! name, so their files need not ship beside the executable; packages the
//! Node.js side runs may read theirs.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Component, Path};

thread_local! {
    static PAGES: Cell<bool> = const { Cell::new(false) };
    static USED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

/// Marks the sources compiled meanwhile as the pages'.
pub(crate) fn pages(on: bool) {
    PAGES.with(|pages| pages.set(on));
}

/// Records the package of a source the build compiles, unless the pages'.
pub(crate) fn compiled(path: &Path) {
    if PAGES.with(Cell::get) {
        return;
    }
    if let Some(name) = package_name(path) {
        USED.with_borrow_mut(|used| used.insert(name));
    }
}

/// Whether the Node.js side runs the package a file is in; true for files
/// in no package.
pub(crate) fn used(path: &Path) -> bool {
    package_name(path).is_none_or(|name| USED.with_borrow(|used| used.contains(&name)))
}

/// The name of the installed package a path is in: the folder after its
/// last `node_modules`, with its scope.
fn package_name(path: &Path) -> Option<String> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let at = parts
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("node_modules"))?;
    let first = parts.get(at + 1)?;
    // The file must be inside the package's folder.
    if first.starts_with('@') {
        let second = parts.get(at + 2)?;
        (parts.len() > at + 3).then(|| format!("{first}/{second}"))
    } else {
        (parts.len() > at + 2).then(|| first.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_are_named_by_their_folder_after_node_modules() {
        let name = |path: &str| package_name(Path::new(path));
        assert_eq!(
            name("C:/app/node_modules/katex/dist/katex.css").as_deref(),
            Some("katex")
        );
        assert_eq!(
            name("C:/app/node_modules/.pnpm/x@1/node_modules/@scope/pkg/lib/a.js").as_deref(),
            Some("@scope/pkg")
        );
        assert_eq!(name("C:/app/src/main.ts"), None);
        assert_eq!(name("C:/app/node_modules/katex"), None);
    }

    #[test]
    fn only_packages_compiled_outside_the_pages_are_used() {
        pages(true);
        compiled(Path::new("C:/app/node_modules/element-plus/es/index.mjs"));
        pages(false);
        compiled(Path::new("C:/app/node_modules/ced/index.js"));
        assert!(used(Path::new("C:/app/node_modules/ced/data/table.bin")));
        assert!(!used(Path::new(
            "C:/app/node_modules/element-plus/theme-chalk/index.css"
        )));
        assert!(used(Path::new("C:/app/static/logo.png")));
    }
}
