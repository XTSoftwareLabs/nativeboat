//! Frame documents: HTML documents the application shows in `<iframe>`
//! elements rather than in windows. Those a page's own `<iframe src>`
//! names are found by themselves; documents a program serves another way,
//! such as through a protocol handler, are named by file globs in the
//! `documents` setting (`"src/frames/*.html"`, with `!` before a glob that
//! leaves files out). Each is compiled as a page is, with its scripts, and
//! is found by its path when a frame loads it.

use crate::Entry;
use std::path::{Path, PathBuf};

/// The HTML files `patterns` name under `root`, outside `node_modules`.
pub fn files(root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let (exclude, include): (Vec<&String>, Vec<&String>) = patterns
        .iter()
        .partition(|pattern| pattern.starts_with('!'));
    let include: Vec<_> = include.iter().map(|p| crate::glob::segments(p)).collect();
    let exclude: Vec<_> = exclude
        .iter()
        .map(|p| crate::glob::segments(&p[1..]))
        .collect();
    // `**` reaches any depth; other patterns only as deep as they are long.
    let depth = if include
        .iter()
        .any(|segments| segments.iter().any(|segment| *segment == ['*', '*']))
    {
        usize::MAX
    } else {
        include.iter().map(Vec::len).max().unwrap_or(0)
    };
    let mut found = Vec::new();
    collect_files(root, depth, &mut found);
    found.retain(|file| {
        let segments = crate::glob::path_segments(file.strip_prefix(root).unwrap_or(file));
        crate::is_html(file)
            && include
                .iter()
                .any(|pattern| crate::glob::matches(pattern, &segments))
            && !exclude
                .iter()
                .any(|pattern| crate::glob::matches(pattern, &segments))
    });
    found.sort();
    found
}

fn collect_files(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(listing) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in listing.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "node_modules" && !name.to_string_lossy().starts_with('.') {
                collect_files(&path, depth - 1, found);
            }
        } else {
            found.push(path);
        }
    }
}

/// The local documents the `<iframe src>` elements of a page's HTML name,
/// resolved against the page's folder, that exist.
pub fn page_frames(page: &Path) -> Vec<PathBuf> {
    let Ok(source) = std::fs::read_to_string(page) else {
        return Vec::new();
    };
    let folder = page.parent().unwrap_or(Path::new("."));
    nativeboat_html::parse_document(&source)
        .elements()
        .into_iter()
        .filter(|element| element.name == "iframe")
        .filter_map(|element| {
            let src = element.attribute("src")?;
            let local = src.split(['?', '#']).next().unwrap_or(src);
            let remote = local.starts_with("//")
                || local
                    .split_once(':')
                    .is_some_and(|(scheme, _)| !scheme.contains(['/', '\\']));
            if local.is_empty() || remote {
                return None;
            }
            let path = folder.join(local.trim_start_matches('/'));
            (path.is_file() && crate::is_html(&path)).then_some(path)
        })
        .collect()
}

/// Adds the frame documents of `pages` and of `patterns` to `pages`, each
/// once.
pub fn add_frames(root: &Path, pages: &mut Vec<Entry>, patterns: Option<&[String]>) {
    let mut found: Vec<PathBuf> = pages
        .iter()
        .flat_map(|page| page_frames(&page.source))
        .collect();
    if let Some(patterns) = patterns {
        found.extend(files(root, patterns));
    }
    let key = |path: &Path| {
        nativeboat_bundler::resolve::normalize(path)
            .to_string_lossy()
            .to_ascii_lowercase()
    };
    for source in found {
        let source = nativeboat_bundler::resolve::normalize(&source);
        let known = pages.iter().any(|page| key(&page.source) == key(&source));
        if !known {
            pages.push(Entry {
                runtime_paths: vec![crate::relative(root, &source)],
                source,
            });
        }
    }
}

#[cfg(test)]
mod tests;
