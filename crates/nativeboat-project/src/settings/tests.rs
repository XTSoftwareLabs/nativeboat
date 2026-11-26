use super::*;
use serde_json::json;

fn file(name: &str, text: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("nativeboat-settings-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("nativeboat.json");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn settings_files_give_the_native_feed_and_name_unknown_settings() {
    let path = file(
        "feed",
        "\u{feff}{\"$schema\": \"x\", \"publish\": {\"provider\": \"generic\", \"url\": \"https://example.com/\"}, \"later\": 1}",
    );
    let settings = Settings::read(&path).unwrap();
    assert_eq!(
        settings.publish,
        Some(json!({"provider": "generic", "url": "https://example.com/"}))
    );
    assert_eq!(settings.unknown, ["later"]);
    // Null is kept: it turns updates off.
    let off = Settings::read(&file("off", "{\"publish\": null}")).unwrap();
    assert_eq!(off.publish, Some(Value::Null));
    assert_eq!(
        Settings::read(&file("empty", "{}")).unwrap(),
        Settings::default()
    );
}

#[test]
fn unreadable_settings_files_are_errors() {
    let missing = std::env::temp_dir()
        .join("nativeboat-settings-missing")
        .join("none.json");
    assert!(
        Settings::read(&missing)
            .unwrap_err()
            .starts_with("cannot read the settings file")
    );
    assert!(
        Settings::read(&file("broken", "{publish:"))
            .unwrap_err()
            .contains("is not JSON")
    );
    assert!(
        Settings::read(&file("list", "[]"))
            .unwrap_err()
            .ends_with("must hold a JSON object")
    );
}

#[test]
fn settings_files_name_plugin_folders() {
    let path = file(
        "plugins",
        "{\"plugins\": [\"extensions/*\", \"!extensions/test-*\"]}",
    );
    assert_eq!(
        Settings::read(&path).unwrap().plugins,
        Some(vec![
            "extensions/*".to_owned(),
            "!extensions/test-*".to_owned()
        ])
    );
    let error = Settings::read(&file("bad-plugins", "{\"plugins\": \"extensions\"}")).unwrap_err();
    assert!(error.contains("must be a list of folder globs"), "{error}");
}

#[test]
fn settings_files_name_forked_modules() {
    let path = file("forks", "{\"forks\": [\"server/out/main\"]}");
    assert_eq!(
        Settings::read(&path).unwrap().forks,
        Some(vec!["server/out/main".to_owned()])
    );
    let error = Settings::read(&file("bad-forks", "{\"forks\": {}}")).unwrap_err();
    assert!(error.contains("must be a list of module paths"), "{error}");
}

#[test]
fn settings_files_name_scripts_pages_load() {
    let path = file(
        "scripts",
        "{\"scripts\": [\"media/a.js\", {\"path\": \"media/b.js\", \"source\": \"src/b.ts\"}]}",
    );
    assert_eq!(Settings::read(&path).unwrap().scripts.unwrap().len(), 2);
    let error = Settings::read(&file("bad-scripts", "{\"scripts\": [{\"path\": 1}]}")).unwrap_err();
    assert!(
        error.contains("must be a list of paths or of objects"),
        "{error}"
    );
}
