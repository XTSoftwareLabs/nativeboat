use super::*;
use serde_json::json;

fn metadata(version: &str) -> crate::Metadata {
    crate::Metadata {
        name: "my-app".into(),
        product_name: "My App".into(),
        app_name: "my-app".into(),
        version: version.into(),
        description: String::new(),
        app_id: None,
        icon: None,
        locales: Vec::new(),
    }
}

fn made(config: Value, version: &str, feed: Option<&Value>) -> (Installer, Vec<String>) {
    let mut warnings = Vec::new();
    let installer = installer(&config, &json!({}), &metadata(version), feed, &mut warnings);
    (installer, warnings)
}

#[test]
fn installers_follow_electron_builders_nsis_defaults() {
    let (installer, warnings) = made(json!({}), "1.2.3", None);
    assert_eq!(
        installer,
        Installer {
            artifact_name: "My-App-Setup-1.2.3.exe".into(),
            shortcut_name: "My App".into(),
            desktop_shortcut: true,
            start_menu_shortcut: true,
            run_after_finish: true,
            delete_app_data: false,
            channel: "latest".into(),
            publisher: "My App".into(),
            app_id: "com.electron.my-app".into(),
        }
    );
    assert!(warnings.is_empty());
}

#[test]
fn authors_publish_and_app_ids_register_installs() {
    let mut warnings = Vec::new();
    let mut metadata = metadata("1.0.0");
    metadata.app_id = Some("com.example.app".into());
    for (author, publisher) in [
        (
            json!("Ada Lovelace <ada@example.com> (https://example.com)"),
            "Ada Lovelace",
        ),
        (
            json!({"name": "Example Inc.", "email": "x@example.com"}),
            "Example Inc.",
        ),
        (json!("<only@example.com>"), "My App"),
        (json!(42), "My App"),
    ] {
        let made = installer(
            &json!({}),
            &json!({"author": author}),
            &metadata,
            None,
            &mut warnings,
        );
        assert_eq!(made.publisher, publisher);
        assert_eq!(made.app_id, "com.example.app");
    }
}

#[test]
fn nsis_settings_name_shortcuts_artifacts_and_channels() {
    let config = json!({
        "artifactName": "${name}.${ext}",
        "win": {"artifactName": "${productName}_Win.${ext}"},
        "nsis": {
            "shortcutName": "Shortcut",
            "createDesktopShortcut": "always",
            "createStartMenuShortcut": false,
            "runAfterFinish": false,
            "deleteAppDataOnUninstall": true,
            "oneClick": false,
            "perMachine": true,
            "allowToChangeInstallationDirectory": true,
            "license": "./LICENSE",
            "include": "build/installer.nsh",
            "script": "build/installer.nsi"
        }
    });
    let (installer, warnings) = made(config.clone(), "2.0.0-beta.4", None);
    assert_eq!(installer.artifact_name, "My-App_Win.exe");
    assert_eq!(installer.shortcut_name, "Shortcut");
    assert!(installer.desktop_shortcut && !installer.start_menu_shortcut);
    assert!(!installer.run_after_finish && installer.delete_app_data);
    assert_eq!(installer.channel, "beta");
    assert_eq!(warnings.len(), 6, "{warnings:?}");
    assert!(warnings[0].contains("nsis.oneClick"));
    // NSIS names win over Windows and top-level ones, and a feed's channel
    // over the version's.
    let config = json!({"artifactName": "${name}-${version}.${ext}", "nsis": {"artifactName": "${name} setup.${ext}", "oneClick": true, "license": false}});
    let feed = json!({"channel": "nightly"});
    let (installer, warnings) = made(config, "3.0.0-rc.1", Some(&feed));
    assert_eq!(installer.artifact_name, "my-app-setup.exe");
    assert_eq!(installer.channel, "nightly");
    assert!(warnings.is_empty(), "{warnings:?}");
    let (installer, _) = made(
        json!({"artifactName": "${name}-${version}.${ext}"}),
        "3.0.0-1",
        Some(&json!({})),
    );
    assert_eq!(installer.artifact_name, "my-app-3.0.0-1.exe");
    assert_eq!(installer.channel, "latest");
    // electron-builder's other file macros, as MarkText's names use them.
    // SAFETY: the variable is this test's own; tests read no other.
    unsafe { std::env::set_var("NATIVEBOAT_TEST_FLAVOR", "beta") };
    let (installer, warnings) = made(
        json!({"nsis": {"artifactName": "${name}-${os}-${arch}-${platform}-${env.NATIVEBOAT_TEST_FLAVOR}${env.NATIVEBOAT_TEST_UNSET}-${version}-setup.${ext}"}}),
        "0.20.0",
        Some(&json!({})),
    );
    assert_eq!(
        installer.artifact_name,
        "my-app-win-x64-win32-beta-0.20.0-setup.exe"
    );
    assert_eq!(
        warnings,
        ["the installer's artifactName names ${env.NATIVEBOAT_TEST_UNSET}, which is not set"]
    );
    // An unclosed macro stays as written.
    let (installer, _) = made(json!({"artifactName": "${env.OPEN.${ext}"}), "1.0.0", None);
    assert_eq!(installer.artifact_name, "${env.OPEN.exe");
}
