use super::*;

#[test]
fn adapted_native_packages_have_their_implemented_support_level() {
    assert!(NATIVE_PACKAGES.contains(&"ced"));
    let ced = native_package("ced");
    assert_eq!(ced.support, Supported);
    assert_eq!(ced.id, "native:ced");
    assert!(ced.note.contains("Google CED"));
    assert_eq!(native_package("keytar").support, Supported);
    assert!(native_package("keytar").note.contains("Credential Manager"));
    assert!(NATIVE_PACKAGES.contains(&"native-keymap"));
    assert_eq!(native_package("native-keymap").support, Supported);
    assert!(native_package("native-keymap").note.contains("TSF"));
    // The runtime's replacements for VS Code's addons.
    for package in [
        "@vscode/windows-registry",
        "@vscode/deviceid",
        "@vscode/windows-mutex",
        "@vscode/policy-watcher",
        "@vscode/windows-process-tree",
        "node-pty",
        "native-is-elevated",
        "@vscode/native-watchdog",
    ] {
        assert_eq!(native_package(package).support, Supported, "{package}");
    }
    for package in ["@vscode/spdlog", "@vscode/sqlite3", "@parcel/watcher"] {
        assert_eq!(native_package(package).support, Partial, "{package}");
    }
    assert!(
        native_package("@parcel/watcher")
            .note
            .contains("getEventsSince")
    );
    assert_eq!(native_package("kerberos").support, Unsupported);
}

#[test]
fn gradients_are_classified_by_their_supported_form() {
    for value in [
        "linear-gradient(red, blue)",
        "LINEAR-GRADIENT(180deg, #fffaf0, #fff4df)",
        "linear-gradient(0, red, blue)",
        "linear-gradient(-0.25turn, red 20%, green, blue 80%)",
        "linear-gradient(100grad, red, blue)",
        "linear-gradient(1rad, rgb(20 30 40), hsl(120 50% 50%))",
        "linear-gradient(to top, red, blue)",
        "linear-gradient(to right, red, blue)",
        "linear-gradient(to bottom, red, blue)",
        "linear-gradient(to left, red, blue)",
        "linear-gradient(red -10%, blue)",
        "linear-gradient(red, blue 110%)",
        "linear-gradient(200.96deg, #fedc2a -29.09%, #dd5789 51.77%, #7a2c9e 129.35%)",
        "linear-gradient(red -300%, blue -100%)",
        "linear-gradient(red 200%, blue 400%)",
    ] {
        assert_eq!(
            css_property("background", value).unwrap().support,
            Supported,
            "{value}"
        );
    }
    for value in [
        "linear-gradient()",
        "linear-gradient(90deg)",
        "linear-gradient(90deg, red)",
        "linear-gradient(red,, blue)",
        "linear-gradient(nope, blue)",
        "linear-gradient(to top right, red, blue)",
        "linear-gradient(to center, red, blue)",
        "linear-gradient(10px, red, blue)",
        "linear-gradient(3, red, blue)",
        "linear-gradient(red 10px, blue)",
        "linear-gradient(red calc(10% + 1px), blue)",
        "linear-gradient(red 80%, blue 20%)",
        "linear-gradient(red -1e300%, blue 1e300%)",
        "linear-gradient(red 50%, blue 50%)",
        "linear-gradient(red, green 100%, blue)",
        "linear-gradient(red 0% 20%, blue)",
        "linear-gradient(red, transparent)",
        "linear-gradient(red, rgb(0 0 255 / .5))",
        "linear-gradient(currentcolor, blue)",
        "linear-gradient(var(--brand), blue)",
        "repeating-linear-gradient(red, blue)",
        "-webkit-linear-gradient(red, blue)",
        "radial-gradient(red, blue)",
        "linear-gradient(red, blue), linear-gradient(blue, red)",
        "linear-gradient(red, blue) center/cover",
    ] {
        assert_eq!(
            css_property("background-image", value).unwrap().support,
            Partial,
            "{value}"
        );
    }
}

#[test]
fn canvas_and_editing_commands_are_judged_by_what_is_used() {
    assert_eq!(canvas_2d(None).support, Supported);
    let shadows = canvas_2d(Some("shadows are not drawn"));
    assert_eq!(
        (shadows.id.as_str(), shadows.support),
        ("web:canvas-2d", Partial)
    );
    assert_eq!(shadows.note, "shadows are not drawn");
    for command in ["undo", "selectAll", "insertText", "copy", "forwardDelete"] {
        assert_eq!(exec_command(Some(command)).support, Supported, "{command}");
    }
    let bold = exec_command(Some("bold"));
    assert_eq!(bold.support, Partial);
    assert!(bold.note.starts_with("the bold command is not available"));
    assert_eq!(exec_command(None).support, Partial);
}

#[test]
fn shadows_and_scroll_anchoring_are_judged_by_value() {
    assert!(css_property("text-shadow", "none").is_none());
    assert_eq!(support(css_property("text-shadow", "0 1px red")), Partial);
    assert!(css_property("overflow-anchor", "none").is_none());
    assert_eq!(support(css_property("overflow-anchor", "auto")), Partial);
}

fn support(feature: Option<Feature>) -> Support {
    feature.unwrap().support
}

#[test]
fn support_levels() {
    assert_eq!(Supported.label(), "Supported");
    assert_eq!(Partial.label(), "Partial");
    assert_eq!(Unsupported.label(), "Unsupported");
    assert_eq!(Supported.weight(), 1.0);
    assert_eq!(Partial.weight(), 0.5);
    assert_eq!(Unsupported.weight(), 0.0);
    assert!(Supported < Partial && Partial < Unsupported);
}

#[test]
fn frameworks() {
    for name in [
        "typescript",
        "vite",
        "electron-vite",
        "vite-plugin-electron",
        "@electron-forge/plugin-vite",
        "vue",
    ] {
        assert_eq!(support(framework(name)), Supported, "{name}");
    }
    for name in [
        "react",
        "react-dom",
        "webpack",
        "svelte",
        "preact",
        "solid-js",
        "jquery",
    ] {
        assert_eq!(support(framework(name)), Partial, "{name}");
    }
    for name in ["@angular/core", "sass", "node-sass", "less", "stylus"] {
        assert_eq!(support(framework(name)), Unsupported, "{name}");
    }
    assert_eq!(framework("lodash"), None);
    assert_eq!(framework("react").unwrap().id, "framework:react");
    let native = native_package("sharp");
    assert_eq!(native.id, "native:sharp");
    assert_eq!(native.support, Unsupported);
}

#[test]
fn electron_apis() {
    for api in [
        "app",
        "BrowserWindow",
        "ipcMain",
        "ipcRenderer",
        "contextBridge",
        "dialog",
        "Menu",
        "MenuItem",
        "clipboard",
        "shell",
        "nativeTheme",
        "screen",
        "webUtils",
        "Tray",
        "globalShortcut",
        "safeStorage",
        "powerSaveBlocker",
        "powerMonitor",
        "systemPreferences",
        "nativeImage",
        "MessageChannelMain",
    ] {
        assert_eq!(electron(api).support, Supported, "{api}");
    }
    for api in [
        "webFrame",
        "session",
        "net",
        "crashReporter",
        "Notification",
        "protocol",
        "autoUpdater",
        "webContents",
        "utilityProcess",
        "desktopCapturer",
        "contentTracing",
    ] {
        assert_eq!(electron(api).support, Partial, "{api}");
    }
    assert_eq!(
        electron_member("webContents", "getAllWebContents").support,
        Supported
    );
    assert_eq!(
        electron_member("webContents", "fromFrame").support,
        Unsupported
    );
    assert_eq!(electron_member("utilityProcess", "fork").support, Supported);
    for member in ["handle", "registerFileProtocol", "unhandle"] {
        assert_eq!(
            electron_member("protocol", member).support,
            Supported,
            "{member}"
        );
    }
    assert_eq!(
        electron_member("protocol", "interceptFileProtocol").support,
        Unsupported
    );
    for api in ["WebContentsView", "BaseWindow"] {
        assert_eq!(electron(api).support, Unsupported, "{api}");
    }
    let view = electron("BrowserView");
    assert_eq!(view.support, Unsupported);
    assert_eq!(view.name, "Electron BrowserView");
}

#[test]
fn node_modules() {
    for module in [
        "fs",
        "fs/promises",
        "path",
        "os",
        "events",
        "buffer",
        "util",
        "url",
        "process",
        "assert",
        "querystring",
        "string_decoder",
        "timers",
        "readline",
        "readline/promises",
        "stream",
        "stream/promises",
        "http",
    ] {
        assert_eq!(node(module).support, Supported, "{module}");
    }
    for module in [
        "child_process",
        "crypto",
        "net",
        "tls",
        "https",
        "http2",
        "module",
        "async_hooks",
        "zlib",
        "constants",
        "tty",
    ] {
        assert_eq!(node(module).support, Partial, "{module}");
    }
    for module in [
        "perf_hooks",
        "v8",
        "dns",
        "vm",
        "inspector",
        "worker_threads",
    ] {
        assert_eq!(node(module).support, Partial, "{module}");
    }
    for module in ["diagnostics_channel", "console", "util/types"] {
        assert_eq!(node(module).support, Supported, "{module}");
    }
    assert_eq!(node("sqlite").support, Unsupported);
    assert_eq!(
        node_export("worker_threads", "MessageChannel").support,
        Supported
    );
    assert_eq!(node_export("worker_threads", "Worker").support, Unsupported);
    assert_eq!(node("fs/promises").id, "node:fs/promises");
}

#[test]
fn exports_of_partial_modules() {
    let spawn = node_export("child_process", "spawn");
    assert_eq!(
        (spawn.id.as_str(), spawn.name.as_str(), spawn.support),
        ("node:child_process", "Node child_process", Supported)
    );
    let fork = node_export("child_process", "fork");
    assert_eq!(
        (fork.id.as_str(), fork.name.as_str(), fork.support),
        (
            "node:child_process.fork",
            "Node child_process.fork",
            Partial
        )
    );
    assert!(fork.note.contains("literal paths"));
    assert_eq!(node_export("https", "request").support, Supported);
    assert_eq!(node_export("crypto", "createHash").support, Supported);
    assert_eq!(node_export("crypto", "randomUUID").support, Supported);
    assert_eq!(node_export("crypto", "createCipheriv").support, Supported);
    assert_eq!(node_export("crypto", "X509Certificate").support, Supported);
    let sign = node_export("crypto", "createSign");
    assert_eq!(
        (sign.id.as_str(), sign.support),
        ("node:crypto.createSign", Unsupported)
    );
    assert_eq!(node_export("net", "isIP").support, Supported);
    assert_eq!(node_export("net", "connect").support, Supported);
    assert_eq!(node_export("https", "createServer").support, Unsupported);
    assert_eq!(node_export("http2", "constants").support, Supported);
    assert_eq!(node_export("module", "registerHooks").support, Supported);
    assert_eq!(node_export("module", "isBuiltin").support, Supported);
    // Off-thread hooks are accepted and never run.
    assert_eq!(node_export("module", "register").support, Unsupported);
    assert_eq!(node_export("http2", "connect").support, Unsupported);
    assert_eq!(node_export("net", "BlockList").support, Unsupported);
    assert_eq!(
        node_export("net", "BlockList").note,
        "not available in the native runtime"
    );
    assert_eq!(node_export("zlib", "gzipSync").support, Supported);
    let brotli = node_export("zlib", "createBrotliCompress");
    assert_eq!(brotli.support, Unsupported);
    assert_eq!(brotli.note, "brotli compression is not available");
    // Other modules are judged as a whole.
    assert_eq!(node_export("fs", "readFile"), node("fs"));
    assert_eq!(node_export("perf_hooks", "performance"), node("perf_hooks"));
    assert!(is_partial_module("zlib") && is_partial_module("net"));
    assert!(!is_partial_module("fs") && !is_partial_module("perf_hooks"));
}

#[test]
fn members_of_partial_electron_modules() {
    let zoom = electron_member("webFrame", "setZoomFactor");
    assert_eq!(
        (zoom.id.as_str(), zoom.name.as_str(), zoom.support),
        ("electron:webFrame", "Electron webFrame", Supported)
    );
    let words = electron_member("webFrame", "getWordSuggestions");
    assert_eq!(
        (words.id.as_str(), words.support),
        ("electron:webFrame.getWordSuggestions", Unsupported)
    );
    assert_eq!(electron_member("net", "fetch").support, Supported);
    assert_eq!(electron_member("crashReporter", "start").support, Supported);
    assert_eq!(
        electron_member("session", "defaultSession.resolveProxy").support,
        Supported
    );
    assert_eq!(
        electron_member("session", "defaultSession"),
        electron("session")
    );
    // Other modules are judged as a whole.
    assert_eq!(electron_member("app", "quit"), electron("app"));
    assert!(is_partial_electron("webFrame") && !is_partial_electron("app"));
}

#[test]
fn web_apis() {
    let supported = [
        "local-storage",
        "drag-drop",
        "resize-observer",
        "mutation-observer",
        "intersection-observer",
        "clipboard-text",
    ];
    let partial = [
        "drag-image",
        "fetch",
        "xhr",
        "window-open",
        "clipboard-api",
        "web-notifications",
        "webassembly",
        "web-crypto",
        "websocket",
        "worker",
        "custom-elements",
        "shadow-dom",
    ];
    let unsupported = [
        "webgl",
        "service-worker",
        "indexeddb",
        "web-audio",
        "media-devices",
    ];
    for api in supported {
        assert_eq!(support(web(api)), Supported, "{api}");
    }
    for api in partial {
        assert_eq!(support(web(api)), Partial, "{api}");
    }
    for api in unsupported {
        assert_eq!(support(web(api)), Unsupported, "{api}");
    }
    assert_eq!(web("teleport"), None);
}

#[test]
fn elements() {
    for tag in [
        "iframe", "frame", "frameset", "webview", "object", "embed", "applet",
    ] {
        assert_eq!(support(element(tag)), Unsupported, "{tag}");
    }
    assert_eq!(support(element("caption")), Supported);
    for tag in [
        // Media shows its first frame without playing.
        "video",
        "audio",
        "source",
        "track",
        "dialog",
        // SVG content that is not drawn.
        "linearGradient",
        "radialGradient",
        "pattern",
        "clipPath",
        "mask",
        "filter",
        "feGaussianBlur",
        "image",
        "textPath",
        "animate",
        "animateTransform",
        "set",
    ] {
        assert_eq!(support(element(tag)), Partial, "{tag}");
    }
    for tag in [
        "svg", "canvas", "input", "button", "textarea", "select", "option", "optgroup", "label",
        "form", "fieldset", "legend", "details", "summary", "table", "thead", "tbody", "tfoot",
        "tr", "td", "th", "colgroup", "col",
    ] {
        assert_eq!(support(element(tag)), Supported, "{tag}");
    }
    assert_eq!(element("div"), None);
    assert_eq!(html_basics().support, Supported);
}

#[test]
fn css_features() {
    let property = |name: &str, value: &str| {
        css_property(name, value).map(|feature| (feature.id, feature.support))
    };
    assert_eq!(
        property("clip", "rect(0,0,0,0)"),
        Some(("css:clip".to_owned(), Supported))
    );
    assert_eq!(
        property("scroll-margin-top", "1rem"),
        Some(("css:scroll-spacing".to_owned(), Supported))
    );
    assert_eq!(
        property("scroll-padding", "10%"),
        Some(("css:scroll-spacing".to_owned(), Supported))
    );
    assert_eq!(
        property("display", "inline-flex"),
        Some(("css:flexbox".to_owned(), Supported))
    );
    assert_eq!(
        property("display", "grid"),
        Some(("css:grid".to_owned(), Supported))
    );
    assert_eq!(
        property("display", "table-cell"),
        Some(("css:table".to_owned(), Supported))
    );
    assert_eq!(
        property("display", "table-caption"),
        Some(("css:table".to_owned(), Supported))
    );
    assert_eq!(
        property("display", "contents"),
        Some(("css:contents".to_owned(), Supported))
    );
    assert_eq!(property("display", "block"), None);
    assert_eq!(
        property("position", "sticky"),
        Some(("css:sticky".to_owned(), Supported))
    );
    assert_eq!(property("position", "absolute"), None);
    for name in ["float", "clear"] {
        assert_eq!(property(name, "left").unwrap().0, "css:float");
    }
    for name in [
        "transform",
        "translate",
        "rotate",
        "scale",
        "transform-origin",
    ] {
        assert_eq!(property(name, "x").unwrap().0, "css:transform");
    }
    for name in [
        "transition",
        "transition-property",
        "transition-duration",
        "transition-timing-function",
        "transition-delay",
    ] {
        assert_eq!(property(name, "x").unwrap().0, "css:transition");
    }
    for name in [
        "animation",
        "animation-name",
        "animation-duration",
        "animation-iteration-count",
        "animation-timing-function",
        "animation-delay",
        "animation-fill-mode",
        "animation-direction",
        "animation-play-state",
    ] {
        assert_eq!(property(name, "x").unwrap().0, "css:animation");
    }
    assert_eq!(
        property("filter", "drop-shadow(16px 0 currentColor)"),
        Some(("css:filter".to_owned(), Supported))
    );
    assert_eq!(
        property("filter", "URL(#shadow)"),
        Some(("css:filter-url".to_owned(), Unsupported))
    );
    for name in ["backdrop-filter", "-webkit-backdrop-filter"] {
        assert_eq!(
            property(name, "blur(5px)"),
            Some(("css:backdrop-filter".to_owned(), Supported))
        );
        assert_eq!(property(name, " none "), None);
    }
    assert_eq!(
        property("mix-blend-mode", "multiply"),
        Some(("css:blend".to_owned(), Unsupported))
    );
    assert_eq!(property("mix-blend-mode", "normal"), None);
    for (value, expected) in [
        ("inset(50%)", Supported),
        ("circle(4px at 0 0)", Supported),
        (" Padding-Box ", Supported),
        ("polygon(0 0, 1px 0, 0 1px)", Supported),
        ("ellipse()", Supported),
        ("path('M0 0 L1 1')", Supported),
        ("url(#shape)", Supported),
        ("url('#shape')", Supported),
        ("url(shapes.svg#shape)", Partial),
    ] {
        for name in ["clip-path", "-webkit-clip-path"] {
            assert_eq!(
                property(name, value),
                Some(("css:clip-path".to_owned(), expected)),
                "{name}: {value}"
            );
        }
    }
    assert_eq!(property("clip-path", " none "), None);
    for name in ["mask", "mask-image", "-webkit-mask", "-webkit-mask-image"] {
        for (value, expected) in [
            ("url(icon.svg) no-repeat", Supported),
            ("linear-gradient(black, transparent)", Supported),
            ("url('#shape')", Partial),
        ] {
            assert_eq!(
                property(name, value),
                Some(("css:mask".to_owned(), expected)),
                "{name}: {value}"
            );
        }
        assert_eq!(property(name, " none "), None);
    }
    for name in ["columns", "column-count", "column-width"] {
        assert_eq!(property(name, "2").unwrap().0, "css:columns");
    }
    for name in ["box-shadow", "text-shadow"] {
        assert_eq!(property(name, "x").unwrap().0, "css:shadow");
    }
    assert_eq!(
        property("box-shadow", "0 12px 30px rgb(15 23 42 / 8%)")
            .unwrap()
            .1,
        Supported
    );
    assert_eq!(
        property("text-shadow", "0 1px 2px black").unwrap().1,
        Partial
    );
    assert_eq!(
        property("background", "linear-gradient(red, blue)")
            .unwrap()
            .0,
        "css:gradient"
    );
    assert_eq!(
        property("background-image", "radial-gradient(red, blue)")
            .unwrap()
            .0,
        "css:gradient"
    );
    assert_eq!(property("background", "red"), None);
    for (name, expected) in [
        ("media", Supported),
        ("supports", Supported),
        ("keyframes", Supported),
        ("-webkit-keyframes", Supported),
        ("font-face", Partial),
        ("container", Partial),
        ("layer", Supported),
        ("property", Supported),
        ("import", Supported),
    ] {
        assert_eq!(support(css_at_rule(name)), expected, "{name}");
    }
    assert_eq!(css_at_rule("page"), None);
    assert_eq!(css_variables().support, Supported);
    let fixed = keyframes_with_static(&["clip-path", "display"]);
    assert_eq!(
        (fixed.id.as_str(), fixed.support),
        ("css:animation", Partial)
    );
    assert!(
        fixed
            .note
            .starts_with("clip-path, display keep their value")
    );
    assert_eq!(css_selector(":has").id, "css:selector:has");
    assert_eq!(css_unknown_property("zoom").support, Partial);
}

#[test]
fn language_features() {
    assert_eq!(support(language("decorators")), Unsupported);
    assert_eq!(support(language("top-level-await")), Partial);
    assert_eq!(support(language("dynamic-require")), Partial);
    assert_eq!(support(language("remote-script")), Unsupported);
    assert_eq!(language("goto"), None);
}

#[test]
fn members_of_window_contents() {
    // What works is part of the window.
    assert_eq!(web_contents_member("send"), electron("BrowserWindow"));
    assert_eq!(web_contents_member("on"), electron("BrowserWindow"));
    assert_eq!(
        web_contents_member("postMessage"),
        electron("BrowserWindow")
    );
    // A dictionary lookup only macOS shows: Electron does nothing on Windows.
    assert_eq!(
        web_contents_member("showDefinitionForSelection"),
        electron("BrowserWindow")
    );
    let script = web_contents_member("executeJavaScript");
    assert_eq!(
        (script.id.as_str(), script.name.as_str(), script.support),
        (
            "electron:webContents.executeJavaScript",
            "Electron webContents.executeJavaScript",
            Partial
        )
    );
    assert!(script.note.contains("without running"));
    let capture = web_contents_member("capturePage");
    assert_eq!(
        (capture.id.as_str(), capture.support),
        ("electron:webContents.capturePage", Unsupported)
    );
}
