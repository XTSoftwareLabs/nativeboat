//! What the application's Node.js side runs and reads: the main process,
//! preloads and forked modules. Where a bundler builds the pages, as
//! electron-vite does, the packages only pages use are bundled into the
//! pages in an Electron build, and the native build embeds what its pages
//! name, so their files need not ship beside the executable; the Node.js
//! side reads the files its sources name, such as
//! `path.join(__dirname, "fonts.vbs")`, and its sources are compiled into
//! the executable.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

thread_local! {
    static PAGES: Cell<bool> = const { Cell::new(false) };
    static USED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    /// The file and folder names the Node.js side's string literals hold,
    /// by the package that holds them or that they start with (`None`: the
    /// application's own sources), in lower case.
    static NAMED: RefCell<BTreeMap<Option<String>, BTreeSet<String>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Marks the sources compiled meanwhile as the pages'.
pub(crate) fn pages(on: bool) {
    PAGES.with(|pages| pages.set(on));
}

/// Records the package of a source the build compiles and the names its
/// string literals hold, unless the source is the pages'.
pub(crate) fn compiled(path: &Path, text: &str) {
    if PAGES.with(Cell::get) {
        return;
    }
    let package = package_name(path);
    if let Some(name) = &package {
        USED.with_borrow_mut(|used| used.insert(name.clone()));
    }
    NAMED.with_borrow_mut(|named| {
        for literal in string_literals(text) {
            let parts: Vec<String> = literal
                .split(['/', '\\'])
                .filter(|part| !part.is_empty() && *part != "." && *part != "..")
                .map(str::to_ascii_lowercase)
                .collect();
            if parts.is_empty() {
                continue;
            }
            // `require.resolve("pkg/data/table.json")` names a file of
            // another package.
            if !literal.starts_with(['.', '/', '\\']) && parts.len() > 1 {
                let scoped = parts[0].starts_with('@') && parts.len() > 2;
                let (name, rest) = if scoped {
                    (format!("{}/{}", parts[0], parts[1]), &parts[2..])
                } else {
                    (parts[0].clone(), &parts[1..])
                };
                named
                    .entry(Some(name))
                    .or_default()
                    .extend(rest.iter().cloned());
            }
            named.entry(package.clone()).or_default().extend(parts);
        }
    });
}

/// Whether a file of the application folder, at `destination` in it, is
/// one the Node.js side reads: the application's manifest, or a file whose
/// name, or the name of a folder it is in, the sources of its package (of
/// the application, for its own files) name, in a package the Node.js side
/// runs. Folders of other systems' programs stay behind.
pub(crate) fn named(destination: &str) -> bool {
    if destination == "package.json" {
        return true;
    }
    let path = Path::new(destination);
    let package = package_name(path);
    if let Some(name) = &package
        && !USED.with_borrow(|used| used.contains(name))
    {
        return false;
    }
    let parts: Vec<String> = parts(path)
        .into_iter()
        .map(|part| part.to_ascii_lowercase())
        .collect();
    // The path inside the package, or the application's.
    let inner = match parts.iter().rposition(|part| part == "node_modules") {
        Some(at) => {
            let skip = if parts.get(at + 1).is_some_and(|part| part.starts_with('@')) {
                3
            } else {
                2
            };
            &parts[(at + skip).min(parts.len())..]
        }
        None => &parts[..],
    };
    let Some((_, folders)) = inner.split_last() else {
        return false;
    };
    if folders
        .iter()
        .any(|folder| OTHER_SYSTEMS.contains(&folder.as_str()))
    {
        return false;
    }
    NAMED.with_borrow(|named| {
        named
            .get(&package)
            .is_some_and(|names| inner.iter().any(|part| names.contains(part)))
    })
}

/// Folders of programs for other systems than Windows.
const OTHER_SYSTEMS: &[&str] = &["darwin", "macos", "osx", "linux"];

/// The contents of a source's string and template literals, outside its
/// comments.
fn string_literals(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = text[index + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| index + 2 + end + 2);
            }
            quote @ (b'\'' | b'"' | b'`') => {
                let start = index + 1;
                index = start;
                // Quoted strings end at their line; templates may not.
                while index < bytes.len()
                    && bytes[index] != quote
                    && (quote == b'`' || bytes[index] != b'\n')
                {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                let end = index.min(bytes.len());
                literals.push(&text[start..end]);
                index = end + 1;
            }
            _ => index += 1,
        }
    }
    literals
}

/// The name of the installed package a path is in: the folder after its
/// last `node_modules`, with its scope.
fn package_name(path: &Path) -> Option<String> {
    let parts = parts(path);
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

fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
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
    fn string_literals_are_read_outside_comments() {
        let text = "// don't 'skip'\nconst a = 'one/two.txt', b = \"it\\\"s\";\n/* `no` */ const c = `x${y}z`;";
        assert_eq!(string_literals(text), ["one/two.txt", "it\\\"s", "x${y}z"]);
    }

    #[test]
    fn only_files_the_node_side_names_ship() {
        pages(true);
        compiled(
            Path::new("C:/app/node_modules/element-plus/es/index.mjs"),
            "import './theme-chalk/index.css';",
        );
        pages(false);
        compiled(
            Path::new(
                "C:/app/node_modules/.pnpm/font-list@2/node_modules/font-list/libs/win32/getByVBS.js",
            ),
            "const fn = path.join(__dirname, \"fonts.vbs\"); require('./standardize')",
        );
        compiled(
            Path::new("C:/app/node_modules/ced/index.js"),
            "const data = require.resolve('@scope/tables/data/table.bin');",
        );
        compiled(
            Path::new("C:/app/src/main/index.ts"),
            "const icons = path.join(__dirname, '../../assets'); // 'notes.txt'",
        );
        assert!(named("node_modules/font-list/libs/win32/fonts.vbs"));
        assert!(!named("node_modules/font-list/package.json"));
        assert!(!named("node_modules/font-list/LICENSE"));
        assert!(!named("node_modules/element-plus/theme-chalk/index.css"));
        // Files of a package another names, when the Node.js side runs it.
        assert!(!named("node_modules/@scope/tables/data/table.bin"));
        compiled(
            Path::new("C:/app/node_modules/@scope/tables/index.js"),
            "module.exports = 1;",
        );
        assert!(named("node_modules/@scope/tables/data/table.bin"));
        assert!(named("assets/icon.png"));
        assert!(named("package.json"));
        assert!(!named("notes.txt"));
        assert!(!named("patches/dep+1.0.0.patch"));
        compiled(
            Path::new("C:/app/node_modules/font-list/libs/darwin/index.js"),
            "path.join(__dirname, 'fontlist')",
        );
        assert!(!named("node_modules/font-list/libs/darwin/fontlist"));
    }
}
