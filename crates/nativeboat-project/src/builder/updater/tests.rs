use super::*;

#[test]
fn publish_settings_become_the_updater_feed_without_publishing() {
    let package = json!({"name":"Example", "repository":"github:owner/repo"});
    let feed = |build: Value| updater_config(&build, &package, None, None).unwrap();
    assert_eq!(
        feed(json!({})).unwrap(),
        json!({"provider":"github","owner":"owner","repo":"repo","updaterCacheDirName":"example-updater"})
    );
    assert_eq!(feed(json!({"publish":[]})), feed(json!({})));
    assert_eq!(feed(json!({"publish":null})), None);
    assert_eq!(
        feed(json!({"publish":"github","win":{"publish":null}})),
        None
    );
    assert_eq!(feed(json!({"publish":"github"})), feed(json!({})));
    assert_eq!(feed(json!({"publish":[{"provider":"github","owner":"other","repo":"other","channel":"beta","private":true},{"provider":"generic","url":"https://ignored.example"}]})).unwrap(), json!({"provider":"github","owner":"other","repo":"other","channel":"beta","private":true,"updaterCacheDirName":"example-updater"}));
    assert_eq!(feed(json!({"publish":"github","win":{"publish":{"provider":"generic","url":"https://updates.example","requestHeaders":{"Accept":"custom"}}}})).unwrap(), json!({"provider":"generic","url":"https://updates.example","requestHeaders":{"Accept":"custom"},"updaterCacheDirName":"example-updater"}));
    assert_eq!(
        feed(json!({"publish":{"provider":"s3","bucket":"releases","region":"us-west-2"}}))
            .unwrap()["bucket"],
        "releases"
    );
    assert_eq!(
        updater_config(&json!({}), &json!({}), None, None).unwrap(),
        None
    );
    assert_eq!(
        updater_config(
            &json!({"publish":{"provider":"generic","url":"https://updates.example"}}),
            &json!({}),
            None,
            None
        )
        .unwrap()
        .unwrap()["updaterCacheDirName"],
        "app-updater"
    );
    assert_eq!(
        updater_config(
            &json!({"publish":{"provider":"generic","url":"https://updates.example"}}),
            &json!({"name":"@Scope/App:*\n"}),
            None,
            None
        )
        .unwrap()
        .unwrap()["updaterCacheDirName"],
        "@scope-app----updater"
    );
}

#[test]
fn repository_urls_and_missing_publish_fields_are_resolved_or_diagnosed() {
    for repository in [
        "https://github.com/a/b.git",
        "git+https://github.com/a/b.git",
        "http://github.com/a/b/",
        "git://github.com/a/b",
        "ssh://git@github.com/a/b.git",
        "git@github.com:a/b.git",
        "github:a/b",
        "a/b",
    ] {
        assert_eq!(
            github_repository(&json!({"repository":repository})),
            Some(("a", "b"))
        );
        assert_eq!(
            updater_config(
                &json!({"publish":{"provider":"github","owner":"override"}}),
                &json!({"repository":{"type":"git","url":repository}}),
                None,
                None
            )
            .unwrap()
            .unwrap()["repo"],
            "b"
        );
    }
    for repository in [
        "https://example.com/a/b",
        "github:/b",
        "github:a/",
        "github:a/b/c",
        "github:a",
        "",
    ] {
        assert_eq!(github_repository(&json!({"repository":repository})), None);
    }
    for config in [
        json!({"publish":42}),
        json!({"publish":{}}),
        json!({"publish":{"provider":""}}),
        json!({"publish":"github"}),
        json!({"publish":{"provider":"github","owner":"a"}}),
        json!({"publish":{"provider":"github","owner":false,"repo":"b"}}),
        json!({"publish":"generic"}),
        json!({"publish":{"provider":"generic","url":""}}),
    ] {
        assert!(
            updater_config(&config, &json!({}), None, None).is_err(),
            "{config}"
        );
    }
}

#[test]
fn native_builds_take_their_own_feed_before_electron_builders() {
    let package = |publish: Value| json!({"name":"app","repository":"github:owner/repo","nativeboat":{"publish":publish}});
    let build = json!({"publish":"github","win":{"publish":{"provider":"generic","url":"https://win.example"}},"nsis":{"publish":{"provider":"generic","url":"https://nsis.example"}}});
    // NSIS settings come before Windows and top-level ones.
    assert_eq!(
        updater_config(&build, &json!({"name":"app"}), None, None)
            .unwrap()
            .unwrap()["url"],
        "https://nsis.example"
    );
    let native = updater_config(
        &build,
        &package(json!({"provider":"generic","url":"https://native.example/updates"})),
        None,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        native,
        json!({"provider":"generic","url":"https://native.example/updates","updaterCacheDirName":"app-updater"})
    );
    assert_eq!(
        updater_config(&build, &package(Value::Null), None, None).unwrap(),
        None
    );
    // A settings file's feed comes before package.json's, and its null
    // turns updates off.
    let settings = json!({"provider":"generic","url":"https://settings.example"});
    assert_eq!(
        updater_config(
            &build,
            &package(json!({"provider":"generic","url":"https://native.example"})),
            Some(&settings),
            None,
        )
        .unwrap()
        .unwrap()["url"],
        "https://settings.example"
    );
    assert_eq!(
        updater_config(&build, &json!({"name":"app"}), Some(&Value::Null), None).unwrap(),
        None
    );
    // A native GitHub feed may name only its repository's owner and name.
    let github = updater_config(
        &json!({}),
        &package(json!({"provider":"github","repo":"native"})),
        None,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (&github["owner"], &github["repo"]),
        (&json!("owner"), &json!("native"))
    );
    // Without a repository in package.json, the origin remote names it.
    let inferred = updater_config(
        &json!({}),
        &json!({"name":"app"}),
        None,
        Some("https://github.com/remote-owner/remote-repo.git"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (&inferred["owner"], &inferred["repo"]),
        (&json!("remote-owner"), &json!("remote-repo"))
    );
    assert_eq!(
        updater_config(&json!({}), &json!({}), None, Some("C:/elsewhere/clone")).unwrap(),
        None
    );
    assert_eq!(
        app_update_yml(&inferred),
        "provider: github\nowner: remote-owner\nrepo: remote-repo\nupdaterCacheDirName: app-updater\n"
    );
}

#[test]
fn origin_remotes_come_from_the_git_configuration() {
    let root = std::env::temp_dir().join(format!("nativeboat-origin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let clone = root.join("clone");
    std::fs::create_dir_all(clone.join(".git")).unwrap();
    assert_eq!(origin_remote(&clone), None);
    std::fs::write(
        clone.join(".git/config"),
        "[core]\n\turl = ignored\n[remote \"upstream\"]\n\turl = https://github.com/up/stream\n\
         [remote \"origin\"]\n\tfetch = +refs/heads/*\n\turl = https://github.com/own/repo.git\n",
    )
    .unwrap();
    assert_eq!(
        origin_remote(&clone).as_deref(),
        Some("https://github.com/own/repo.git")
    );
    // Worktrees name their git directory, whose common directory holds the
    // configuration; other git directories hold their own.
    let worktree = root.join("worktree");
    std::fs::create_dir_all(clone.join(".git/worktrees/w")).unwrap();
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(clone.join(".git/worktrees/w/commondir"), "../..\n").unwrap();
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", clone.join(".git/worktrees/w").display()),
    )
    .unwrap();
    assert_eq!(
        origin_remote(&worktree).as_deref(),
        Some("https://github.com/own/repo.git")
    );
    std::fs::remove_file(clone.join(".git/worktrees/w/commondir")).unwrap();
    std::fs::write(
        clone.join(".git/worktrees/w/config"),
        "[remote \"origin\"]\nurl=git@github.com:own/other.git\n",
    )
    .unwrap();
    assert_eq!(
        origin_remote(&worktree).as_deref(),
        Some("git@github.com:own/other.git")
    );
    std::fs::write(worktree.join(".git"), "not a git file").unwrap();
    assert_eq!(origin_remote(&worktree), None);
    assert_eq!(origin_remote(&root), None);
    std::fs::remove_dir_all(&root).unwrap();
}
