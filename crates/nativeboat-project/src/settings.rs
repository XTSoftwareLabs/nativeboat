//! NativeBoat's own settings for a build, from the file `--config` names:
//! what applies to the native build and not to the Electron one, such as
//! the feed native builds are published to. Projects need no change for
//! them.

use serde_json::Value;
use std::path::Path;

/// The settings a build is given.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// The update feed of native builds, as electron-builder's `publish`
    /// takes it; `Some(Value::Null)` turns updates off.
    pub publish: Option<Value>,
    /// Folder globs of the plugin packages the program loads by path while
    /// it runs (see [`crate::plugins`]).
    pub plugins: Option<Vec<String>>,
    /// File globs of the HTML documents the program shows in frames
    /// without naming them in its pages (see [`crate::documents`]).
    pub documents: Option<Vec<String>>,
    /// Paths of the modules the program starts with `fork()` by paths it
    /// builds while it runs (see [`crate::plugins::module_entry`]).
    pub forks: Option<Vec<String>>,
    /// Scripts the program loads by URLs it builds while it runs (see
    /// [`crate::Project::scripts`]): their paths, or objects with the
    /// `path` URLs end with and the `source` it is compiled from.
    pub scripts: Option<Vec<serde_json::Value>>,
    /// The settings the file gives that this NativeBoat does not know.
    pub unknown: Vec<String>,
}

impl Settings {
    /// Reads a settings file: a JSON object such as
    /// `{"publish": {"provider": "generic", "url": "https://example.com/"}}`.
    pub fn read(path: &Path) -> Result<Settings, String> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!("cannot read the settings file {}: {error}", path.display())
        })?;
        let value: Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|error| {
                format!("the settings file {} is not JSON: {error}", path.display())
            })?;
        let Value::Object(entries) = value else {
            return Err(format!(
                "the settings file {} must hold a JSON object",
                path.display()
            ));
        };
        let mut settings = Settings::default();
        for (key, value) in entries {
            match key.as_str() {
                "publish" => settings.publish = Some(value),
                "plugins" => {
                    settings.plugins = Some(crate::plugins::patterns(&value).ok_or_else(|| {
                        format!(
                            "\"plugins\" in the settings file {} must be a list of folder globs",
                            path.display()
                        )
                    })?)
                }
                "documents" => settings.documents =
                    Some(crate::plugins::patterns(&value).ok_or_else(|| {
                        format!(
                            "\"documents\" in the settings file {} must be a list of file globs",
                            path.display()
                        )
                    })?),
                "scripts" => {
                    settings.scripts = Some(crate::scripts_setting(&value).ok_or_else(|| {
                        format!(
                            "\"scripts\" in the settings file {} must be a list of paths or of objects with \"path\" and \"source\"",
                            path.display()
                        )
                    })?)
                }
                "forks" => {
                    settings.forks = Some(crate::plugins::patterns(&value).ok_or_else(|| {
                        format!(
                            "\"forks\" in the settings file {} must be a list of module paths",
                            path.display()
                        )
                    })?)
                }
                // Editors use it to find a schema for the file.
                "$schema" => {}
                _ => settings.unknown.push(key),
            }
        }
        Ok(settings)
    }
}

#[cfg(test)]
mod tests;
