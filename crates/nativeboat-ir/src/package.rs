//! The compiled application package and its embedding in an executable.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nativeboat_bundler::Bundle;
use nativeboat_css::Stylesheet;
use nativeboat_html::Document;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::path::Path;

/// Package format version; the runtime refuses other versions.
pub const FORMAT: u32 = 1;

/// Marks the end of an executable that carries a package.
pub const MAGIC: &[u8; 8] = b"NBTAPP01";

/// Application metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Metadata {
    /// Package name.
    pub name: String,
    /// Display name.
    pub product_name: String,
    /// Version.
    pub version: String,
    /// Description.
    pub description: String,
    /// Application user model id.
    pub app_id: Option<String>,
}

/// A JavaScript bundle and where the application loads it from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Script {
    /// Paths relative to the application root that name this script.
    pub runtime_paths: Vec<String>,
    /// The bundled modules.
    pub bundle: Bundle,
    /// The `import.meta.env` object.
    pub env: BTreeMap<String, serde_json::Value>,
}

/// A compiled stylesheet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Style {
    /// Source path relative to the application root.
    pub path: String,
    /// The parsed rules.
    pub sheet: Stylesheet,
}

/// A renderer page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Paths relative to the application root that name this page.
    pub runtime_paths: Vec<String>,
    /// The parsed HTML with `<script>` and stylesheet links removed.
    pub document: Document,
    /// Stylesheets in cascade order: linked, inline and imported by scripts.
    pub styles: Vec<Style>,
    /// The page's scripts.
    pub scripts: Script,
    /// For each script entry, whether it is a module (`type="module"`);
    /// classic scripts run in the global scope.
    pub module_entries: Vec<bool>,
}

/// A file served to the application by URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// The URL path, such as `/src/logo.png`.
    pub url: String,
    /// File content.
    #[serde(serialize_with = "to_base64", deserialize_with = "from_base64")]
    pub data: Vec<u8>,
}

fn to_base64<S: Serializer>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&STANDARD.encode(data))
}

fn from_base64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text = String::deserialize(deserializer)?;
    STANDARD.decode(text).map_err(serde::de::Error::custom)
}

/// Everything the runtime needs to run the application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPackage {
    /// Format version, [`FORMAT`].
    pub format: u32,
    /// Metadata.
    pub metadata: Metadata,
    /// The main process script.
    pub main: Script,
    /// Preload scripts.
    pub preloads: Vec<Script>,
    /// Renderer pages.
    pub pages: Vec<Page>,
    /// Assets referenced by URL.
    pub assets: Vec<Asset>,
}

impl AppPackage {
    /// Creates an empty package.
    pub fn new(metadata: Metadata) -> Self {
        AppPackage {
            format: FORMAT,
            metadata,
            main: Script::default(),
            preloads: Vec::new(),
            pages: Vec::new(),
            assets: Vec::new(),
        }
    }

    /// Serializes the package.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserializes a package and checks its format version.
    pub fn from_bytes(bytes: &[u8]) -> Result<AppPackage, String> {
        let package: AppPackage = serde_json::from_slice(bytes)
            .map_err(|error| format!("corrupt application package: {error}"))?;
        if package.format != FORMAT {
            return Err(format!(
                "application package format {} is not supported (expected {FORMAT})",
                package.format
            ));
        }
        Ok(package)
    }

    /// Finds the page loaded from `path`, a path relative to the application root.
    pub fn page(&self, path: &str) -> Option<&Page> {
        let path = normalize_runtime_path(path);
        self.pages
            .iter()
            .find(|page| page.runtime_paths.contains(&path))
            .or_else(|| {
                let name = path.rsplit('/').next().unwrap_or(&path);
                let mut matches = self.pages.iter().filter(|page| {
                    page.runtime_paths
                        .iter()
                        .any(|candidate| candidate.rsplit('/').next() == Some(name))
                });
                let first = matches.next();
                if matches.next().is_some() {
                    None
                } else {
                    first
                }
            })
    }

    /// Finds the preload script loaded from `path`.
    pub fn preload(&self, path: &str) -> Option<&Script> {
        let path = normalize_runtime_path(path);
        self.preloads
            .iter()
            .find(|script| script.runtime_paths.contains(&path))
    }

    /// Finds an asset by URL path.
    pub fn asset(&self, url: &str) -> Option<&Asset> {
        let url = url.split(['?', '#']).next().unwrap_or(url);
        let url = if url.starts_with('/') {
            url.to_owned()
        } else {
            format!("/{url}")
        };
        self.assets.iter().find(|asset| asset.url == url)
    }
}

/// Normalizes a runtime path: `/` separators, no `.` segments, `..` applied.
pub fn normalize_runtime_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Appends a package to a runtime executable.
pub fn embed(stub: &[u8], package: &AppPackage) -> Vec<u8> {
    let payload = package.to_bytes();
    let mut output = Vec::with_capacity(stub.len() + payload.len() + 16);
    output.extend_from_slice(stub);
    output.extend_from_slice(&payload);
    output.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    output.extend_from_slice(MAGIC);
    output
}

/// Extracts the package appended to an executable, if there is one.
pub fn extract(executable: &[u8]) -> Result<Option<AppPackage>, String> {
    let Some(footer_start) = executable.len().checked_sub(16) else {
        return Ok(None);
    };
    if &executable[footer_start + 8..] != MAGIC {
        return Ok(None);
    }
    let mut length = [0u8; 8];
    length.copy_from_slice(&executable[footer_start..footer_start + 8]);
    let length = u64::from_le_bytes(length) as usize;
    let Some(start) = footer_start.checked_sub(length) else {
        return Err("corrupt application package: invalid length".to_owned());
    };
    AppPackage::from_bytes(&executable[start..footer_start]).map(Some)
}

/// Writes `stub` with the package appended to `output`.
pub fn write_executable(stub: &Path, output: &Path, package: &AppPackage) -> Result<(), String> {
    let stub_bytes = std::fs::read(stub)
        .map_err(|error| format!("cannot read runtime {}: {error}", stub.display()))?;
    if extract(&stub_bytes).ok().flatten().is_some() {
        return Err(format!(
            "{} already contains an application",
            stub.display()
        ));
    }
    if let Some(directory) = output.parent() {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    }
    std::fs::write(output, embed(&stub_bytes, package))
        .map_err(|error| format!("cannot write {}: {error}", output.display()))
}

/// Reads the package appended to an executable file.
pub fn read_executable(path: &Path) -> Result<Option<AppPackage>, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    extract(&bytes)
}

#[cfg(test)]
mod tests;
