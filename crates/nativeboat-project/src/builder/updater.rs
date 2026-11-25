//! The runtime update feed electron-builder writes into app-update.yml.

use serde_json::{Value, json};
use std::path::Path;

fn github_repository(package: &Value) -> Option<(&str, &str)> {
    let repository = &package["repository"];
    github_path(repository.as_str().or_else(|| repository["url"].as_str())?)
}

/// The owner and name of a GitHub repository URL.
fn github_path(url: &str) -> Option<(&str, &str)> {
    let url = url.strip_prefix("git+").unwrap_or(url);
    let path = [
        "https://github.com/",
        "http://github.com/",
        "git://github.com/",
        "ssh://git@github.com/",
        "git@github.com:",
        "github:",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))
    .or_else(|| (!url.contains([':', '@'])).then_some(url))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/')).then_some((owner, repo))
}

/// The URL of a project's `origin` remote, where electron-builder finds the
/// GitHub repository when package.json names none.
pub fn origin_remote(root: &Path) -> Option<String> {
    let dot_git = root.join(".git");
    let directory = if dot_git.is_dir() {
        dot_git
    } else {
        // A worktree's `.git` file names its git directory, whose
        // `commondir` holds the shared configuration.
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let git_dir = root.join(text.strip_prefix("gitdir:")?.trim());
        match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(common) => git_dir.join(common.trim()),
            Err(_) => git_dir,
        }
    };
    let config = std::fs::read_to_string(directory.join("config")).ok()?;
    let mut origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            origin = line.replace(char::is_whitespace, "") == "[remote\"origin\"]";
        } else if origin
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "url"
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

/// Resolves the first publish provider. A native build's own feed comes
/// first, since native builds may be published apart from Electron ones:
/// `native`, the settings file's `publish`, then package.json's
/// `nativeboat.publish`; then electron-builder's NSIS, Windows and
/// top-level settings. Explicit null disables updates; a missing/empty
/// publish list can infer GitHub from package.json's repository or the
/// `origin` remote. No publishing is performed.
pub fn updater_config(
    config: &Value,
    package: &Value,
    native: Option<&Value>,
    origin: Option<&str>,
) -> Result<Option<Value>, String> {
    let publish = native
        .or_else(|| package["nativeboat"].get("publish"))
        .or_else(|| config["nsis"].get("publish"))
        .or_else(|| config["win"].get("publish"))
        .or_else(|| config.get("publish"));
    let repository = github_repository(package).or_else(|| origin.and_then(github_path));
    if publish == Some(&Value::Null) {
        return Ok(None);
    }
    let publish = publish.and_then(|value| match value {
        Value::Array(providers) => providers.first(),
        _ => Some(value),
    });
    let mut resolved = match publish {
        Some(Value::String(provider)) => json!({"provider": provider}),
        Some(Value::Object(options)) => Value::Object(options.clone()),
        Some(_) => return Err("publish must be a provider, object or array".to_owned()),
        None if repository.is_some() => json!({"provider": "github"}),
        None => return Ok(None),
    };
    let provider = resolved["provider"]
        .as_str()
        .filter(|provider| !provider.is_empty())
        .ok_or("publish.provider must be a nonempty string")?;
    if provider == "github" {
        let (owner, repo) = repository.unwrap_or(("", ""));
        for (key, inferred) in [("owner", owner), ("repo", repo)] {
            if resolved[key].is_null() {
                resolved[key] = json!(inferred);
            }
            if resolved[key].as_str().is_none_or(str::is_empty) {
                return Err(format!(
                    "GitHub publish requires {key}, or a GitHub repository in package.json or the origin remote"
                ));
            }
        }
    } else if provider == "generic" && resolved["url"].as_str().is_none_or(str::is_empty) {
        return Err("generic publish requires url".to_owned());
    }
    let name = package["name"].as_str().unwrap_or("app");
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                '-'
            } else {
                ch
            }
        })
        .collect();
    resolved["updaterCacheDirName"] = json!(format!("{}-updater", sanitized.to_lowercase()));
    Ok(Some(resolved))
}

/// The text of `resources/app-update.yml`, which electron-updater reads.
pub fn app_update_yml(feed: &Value) -> String {
    serde_yaml_ng::to_string(feed).unwrap_or_default()
}

#[cfg(test)]
mod tests;
