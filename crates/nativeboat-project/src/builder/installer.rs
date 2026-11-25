//! The Windows installer electron-builder's NSIS target makes, which native
//! builds make in its place: where it is written, its shortcuts and what it
//! does when it ends, and the update channel its metadata describes.

use super::Value;

/// Installer settings from electron-builder's `nsis` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    /// The installer's file name, from `nsis.artifactName`, `win.artifactName`
    /// or `artifactName`, with electron-builder's file macros filled in
    /// (`${productName}`, `${version}`, `${name}`, `${ext}`, `${arch}`,
    /// `${os}`, `${platform}` and `${env.NAME}`) and spaces made dashes, as
    /// GitHub stores uploads.
    pub artifact_name: String,
    /// The name of the shortcuts.
    pub shortcut_name: String,
    /// Whether a desktop shortcut is made.
    pub desktop_shortcut: bool,
    /// Whether a Start menu shortcut is made.
    pub start_menu_shortcut: bool,
    /// Whether the application starts once an interactive install ends.
    pub run_after_finish: bool,
    /// Whether uninstalling deletes the application's data folder.
    pub delete_app_data: bool,
    /// The update channel, whose `<channel>.yml` describes the installer.
    pub channel: String,
    /// The publisher Add or Remove Programs shows: package.json's author,
    /// electron-builder's company name.
    pub publisher: String,
    /// The application id the install is registered under: `appId`, or
    /// electron-builder's `com.electron.<name>`.
    pub app_id: String,
}

fn flag(nsis: &Value, key: &str, default: bool) -> bool {
    match &nsis[key] {
        Value::Bool(value) => *value,
        // `createDesktopShortcut: "always"` recreates it on every update.
        Value::String(value) => value == "always",
        _ => default,
    }
}

/// The installer a project's packaging settings describe. `feed` is the
/// resolved update feed, whose `channel` names the update channel; without
/// one, a prerelease version's first identifier does, as electron-builder
/// derives it.
/// Fills in a name's `${env.NAME}` macros with environment variables, as
/// electron-builder does; an unset one is left out, with a warning.
fn macros(name: &str, warnings: &mut Vec<String>) -> String {
    let mut filled = String::new();
    let mut rest = name;
    while let Some(start) = rest.find("${env.") {
        filled.push_str(&rest[..start]);
        let after = &rest[start + 6..];
        let Some(end) = after.find('}') else {
            filled.push_str(&rest[start..]);
            return filled;
        };
        let variable = &after[..end];
        match std::env::var(variable) {
            Ok(value) => filled.push_str(&value),
            Err(_) => warnings.push(format!(
                "the installer's artifactName names ${{env.{variable}}}, which is not set"
            )),
        }
        rest = &after[end + 1..];
    }
    filled.push_str(rest);
    filled
}

pub fn installer(
    config: &Value,
    package: &Value,
    metadata: &crate::Metadata,
    feed: Option<&Value>,
    warnings: &mut Vec<String>,
) -> Installer {
    let (product_name, name, version) = (
        metadata.product_name.as_str(),
        metadata.name.as_str(),
        metadata.version.as_str(),
    );
    let nsis = &config["nsis"];
    let template = [
        &nsis["artifactName"],
        &config["win"]["artifactName"],
        &config["artifactName"],
    ]
    .into_iter()
    .find_map(Value::as_str)
    .unwrap_or("${productName} Setup ${version}.${ext}");
    let artifact_name = macros(
        &template
            .replace("${productName}", product_name)
            .replace("${version}", version)
            .replace("${name}", name)
            .replace("${ext}", "exe")
            .replace("${arch}", "x64")
            .replace("${os}", "win")
            .replace("${platform}", "win32"),
        warnings,
    )
    .replace(' ', "-");
    for (key, effect) in [
        ("oneClick", "installs in one click"),
        ("perMachine", "installs for the current user"),
        (
            "allowToChangeInstallationDirectory",
            "installs into the default folder",
        ),
        ("license", "shows no license page"),
        ("include", "runs no NSIS script"),
        ("script", "runs no NSIS script"),
    ] {
        let set = match key {
            "oneClick" => nsis[key] == Value::Bool(false),
            _ => !matches!(nsis[key], Value::Null | Value::Bool(false)),
        };
        if set {
            warnings.push(format!(
                "nsis.{key} is not used: the native installer {effect}"
            ));
        }
    }
    let prerelease = version
        .split_once('-')
        .and_then(|(_, tag)| tag.split('.').next())
        .and_then(|tag| {
            let letters: String = tag.chars().take_while(char::is_ascii_alphabetic).collect();
            (!letters.is_empty()).then_some(letters)
        });
    let channel = feed
        .and_then(|feed| feed["channel"].as_str())
        .map(str::to_owned)
        .or(prerelease)
        .unwrap_or_else(|| "latest".to_owned());
    Installer {
        artifact_name,
        shortcut_name: nsis["shortcutName"]
            .as_str()
            .unwrap_or(product_name)
            .to_owned(),
        desktop_shortcut: flag(nsis, "createDesktopShortcut", true),
        start_menu_shortcut: flag(nsis, "createStartMenuShortcut", true),
        run_after_finish: flag(nsis, "runAfterFinish", true),
        delete_app_data: flag(nsis, "deleteAppDataOnUninstall", false),
        channel,
        publisher: author(&package["author"])
            .unwrap_or(product_name)
            .to_owned(),
        app_id: metadata
            .app_id
            .clone()
            .unwrap_or_else(|| format!("com.electron.{name}")),
    }
}

/// The name of package.json's `author`: `"Name <email> (url)"` or
/// `{"name": ...}`.
fn author(value: &Value) -> Option<&str> {
    let text = value.as_str().or_else(|| value["name"].as_str())?;
    let name = text.split(['<', '(']).next().unwrap_or(text).trim();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests;
