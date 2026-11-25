use super::*;
use nativeboat_bundler::{Bundle, Module, rewrite};
use nativeboat_ir::package::Style;
use nativeboat_ir::{DisplayItem, Surface};
use nativeboat_layout::FontMetrics;
use std::collections::BTreeMap;

#[test]
#[ignore = "run with CARGO_PROFILE_TEST_OPT_LEVEL=2 for native scene storage timing"]
fn native_scene_storage_and_diff_performance() {
    let html = (0..1000)
        .map(|index| format!("<p>Paragraph {index}: retained native scene storage.</p>"))
        .collect::<String>();
    let mut fixture = fixture(&html, "p { font: 12px/18px serif; margin: 0 }", &[]);
    fixture.renderer.render();
    let snapshot = fixture.renderer.submitted.as_ref().unwrap().clone();
    let sample = |copy: bool| {
        let mut previous = snapshot.tree.clone();
        let mut retained = snapshot.clone();
        let started = Instant::now();
        for _ in 0..100 {
            if copy {
                let next = std::hint::black_box(&snapshot.tree).clone();
                assert!(std::hint::black_box(diff(&previous, &next)).is_empty());
                previous = next;
            } else {
                let next = std::hint::black_box(&snapshot).clone();
                assert!(std::hint::black_box(diff(&retained.tree, &next.tree)).is_empty());
                retained = next;
            }
        }
        started.elapsed().as_secs_f64() * 1000.0
    };
    for trial in 1..=3 {
        let (copy, retain) = if trial == 2 {
            let retain = sample(false);
            (sample(true), retain)
        } else {
            (sample(true), sample(false))
        };
        println!(
            "NATIVE_SCENE_PERFORMANCE trial={trial} iterations=100 paragraphs=1000 copy_and_diff_ms={copy} retain_and_diff_ms={retain}"
        );
    }
}

#[test]
fn submitted_scenes_share_tree_storage_and_release_replaced_snapshots() {
    let mut fixture = fixture("<p id=p>Hello</p><input id=e value=abc>", "", &[]);
    assert_eq!(fixture.renderer.tree(), &NativeTree::default());
    assert!(!fixture.renderer.render().is_empty());
    let snapshot = fixture.renderer.submitted.as_ref().unwrap().clone();
    assert!(std::ptr::eq(fixture.renderer.tree(), &snapshot.tree));
    let storage = fixture.renderer.tree().root.items.as_ptr();
    for _ in 0..10 {
        assert!(fixture.renderer.render().is_empty());
        assert_eq!(fixture.renderer.tree().root.items.as_ptr(), storage);
        assert!(std::ptr::eq(fixture.renderer.tree(), &snapshot.tree));
    }
    let id = fixture.node("#e");
    fixture
        .renderer
        .frame
        .borrow_mut()
        .selections
        .insert(NodeId(id as u32), (1, 2));
    assert_eq!(
        fixture.renderer.render(),
        [Change::Select {
            id,
            start: 1,
            end: 2
        }]
    );
    assert!(std::ptr::eq(fixture.renderer.tree(), &snapshot.tree));
    assert!(fixture.renderer.render().is_empty());
    let old = Rc::downgrade(&snapshot);
    drop(snapshot);
    fixture.eval("p.textContent='Changed'");
    assert!(!fixture.renderer.render().is_empty());
    assert!(
        old.upgrade().is_none(),
        "replaced native snapshots are released"
    );
    let current = Rc::downgrade(fixture.renderer.submitted.as_ref().unwrap());
    assert_eq!(
        current.strong_count(),
        3,
        "frame, submitted tree and hit state"
    );
    assert!(!fixture.renderer.clear().is_empty());
    assert!(fixture.renderer.submitted.is_none());
    assert!(fixture.renderer.hits.borrow().scene.is_none());
    assert_eq!(fixture.renderer.tree(), &NativeTree::default());
    assert_eq!(
        current.strong_count(),
        1,
        "only the cached frame retains its scene"
    );
    assert!(fixture.renderer.clear().is_empty());
    assert!(
        !fixture.renderer.render().is_empty(),
        "clear does not suppress resubmission"
    );
    assert_eq!(current.strong_count(), 3);
}

#[test]
fn retained_scenes_keep_the_existing_diff_semantics_for_nonfinite_values() {
    let mut fixture = fixture("<p>Text</p>", "", &[]);
    fixture.renderer.render();
    let mut scene = (*fixture.renderer.submitted.as_ref().unwrap().as_ref()).clone();
    scene.tree.root.rect.x = f32::NAN;
    let scene = Rc::new(scene);
    fixture
        .renderer
        .frame
        .borrow_mut()
        .rendered
        .as_mut()
        .unwrap()
        .scene = scene.clone();
    assert!(!fixture.renderer.render().is_empty());
    let expected = diff(&scene.tree, &scene.tree);
    assert!(
        !expected.is_empty(),
        "NaN is not equal to itself in the existing diff"
    );
    let changes = fixture.renderer.render();
    assert_eq!(changes.len(), expected.len());
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, Change::UpdateSurface { rect, .. } if rect.x.is_nan()))
    );
    assert!(std::ptr::eq(fixture.renderer.tree(), &scene.tree));
}

#[path = "hit/tests.rs"]
mod hit_tests;

#[test]
fn computed_style_reads_refresh_css_without_forcing_layout_or_paint() {
    let mut fixture = fixture(
        "<style id=rules></style><p id=p>Hello</p><div hidden><span id=hiddenChild>Hidden</span></div>",
        "p {width:100px;color:red} .wide {width:200px} @media (min-width:500px) {p {width:300px}}",
        &[],
    );
    fixture.renderer.frame.borrow_mut().rendered = None;
    assert_eq!(fixture.eval("getComputedStyle(p).color"), "rgb(255, 0, 0)");
    assert!(fixture.renderer.frame.borrow().rendered.is_none());
    // Sizes resolve against layout, as in Chromium.
    assert_eq!(fixture.eval("getComputedStyle(p).width"), "100px");
    assert!(fixture.renderer.frame.borrow().rendered.is_some());
    fixture.renderer.render();
    let old = fixture
        .renderer
        .frame
        .borrow()
        .rendered
        .as_ref()
        .unwrap()
        .scene
        .clone();
    assert_eq!(
        fixture.eval("p.textContent='Changed';p.className='wide';getComputedStyle(p).color"),
        "rgb(255, 0, 0)"
    );
    assert!(Rc::ptr_eq(
        &old,
        &fixture
            .renderer
            .frame
            .borrow()
            .rendered
            .as_ref()
            .unwrap()
            .scene
    ));
    assert_eq!(fixture.eval("getComputedStyle(p).width"), "200px");
    assert_eq!(fixture.eval("p.getBoundingClientRect().width"), "200");
    assert!(!Rc::ptr_eq(
        &old,
        &fixture
            .renderer
            .frame
            .borrow()
            .rendered
            .as_ref()
            .unwrap()
            .scene
    ));
    assert_eq!(
        fixture.eval("rules.sheet.insertRule('p {color:blue}');getComputedStyle(p).color"),
        "rgb(0, 0, 255)"
    );
    assert_eq!(
        fixture
            .eval("hiddenChild.parentNode.style.color='green';getComputedStyle(hiddenChild).color"),
        "rgb(0, 128, 0)"
    );
    fixture.renderer.resize(600.0, 300.0);
    assert_eq!(
        fixture.eval("p.className='';getComputedStyle(p).width"),
        "300px"
    );
    assert_eq!(fixture.eval("p.getBoundingClientRect().width"), "300");
}

#[test]
fn disclosure_content_is_hidden_until_mouse_or_keyboard_activation() {
    let mut fixture = fixture(
        "<details id=d>hidden text<summary id=s>Manage types</summary><input id=input><summary id=second>Other</summary></details><button id=after>After</button>",
        "",
        &[],
    );
    assert_eq!(fixture.eval("input.getBoundingClientRect().height"), "0");
    fixture.eval("s.focus()");
    assert_eq!(fixture.eval("document.activeElement.id"), "s");
    fixture.renderer.handle(&Event::Key {
        down: true,
        key: "Tab".into(),
        code: "Tab".into(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    });
    assert_eq!(fixture.eval("document.activeElement.id"), "after");
    fixture.eval("s.focus()");
    fixture.renderer.handle(&Event::Key {
        down: true,
        key: "Enter".into(),
        code: "Enter".into(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    });
    assert_eq!(fixture.eval("d.open"), "true");
    assert_eq!(
        fixture.eval("input.getBoundingClientRect().height > 0"),
        "true"
    );
    fixture.eval("second.click()");
    assert_eq!(fixture.eval("d.open"), "true");
    fixture.renderer.handle(&Event::Key {
        down: false,
        key: " ".into(),
        code: "Space".into(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    });
    assert_eq!(fixture.eval("d.open"), "false");
    assert_eq!(fixture.eval("input.getBoundingClientRect().height"), "0");
    fixture.eval("s.click()");
    assert_eq!(fixture.eval("d.open"), "true");
}

struct Mono;

impl TextMeasurer for Mono {
    fn width(&self, text: &str, font: &nativeboat_ir::Font) -> f32 {
        text.chars().count() as f32 * font.size * 0.5
    }

    fn metrics(&self, font: &nativeboat_ir::Font) -> FontMetrics {
        FontMetrics {
            ascent: font.size * 0.8,
            descent: font.size * 0.2,
        }
    }
}

type Output = Rc<RefCell<Vec<String>>>;

#[test]
fn editing_commands_reach_the_focused_page_and_fire_user_input_events() {
    let mut fixture = fixture("<div id=e contenteditable>ab</div>", "", &[]);
    fixture.eval("e.focus();getSelection().collapse(e.firstChild,2);globalThis.edits=[];e.onbeforeinput=event=>edits.push([event.inputType,event.isTrusted,event.cancelable]);");
    fixture.renderer.handle(&Event::Text("x".into()));
    assert!(fixture.renderer.edit_command("undo"));
    assert_eq!(fixture.eval("e.textContent"), "ab");
    assert!(fixture.renderer.edit_command("redo"));
    assert_eq!(fixture.eval("e.textContent"), "abx");
    assert!(fixture.renderer.edit_command("selectAll"));
    assert_eq!(fixture.eval("getSelection().toString()"), "abx");
    assert!(!fixture.renderer.edit_command("unknown"));
    assert_eq!(
        fixture.eval("JSON.stringify(edits)"),
        r#"[["insertText",true,true],["historyUndo",true,true],["historyRedo",true,true]]"#
    );
}

#[test]
fn composition_phases_reach_the_focused_host_and_commit_once() {
    let mut fixture = fixture(
        "<div id=e contenteditable>ab</div><button id=b>Other</button>",
        "",
        &[],
    );
    fixture.eval("e.focus();getSelection().collapse(e.firstChild,1);globalThis.phases=[];for(const type of ['compositionstart','compositionupdate','compositionend'])e.addEventListener(type,event=>phases.push([type,event.data]));");
    for (kind, data, cursor) in [
        (CompositionKind::Start, "", 0),
        (CompositionKind::Update, "ni", 1),
        (CompositionKind::Update, "你😀", 3),
        (CompositionKind::End, "你😀", 3),
    ] {
        assert!(!fixture.renderer.handle(&Event::Composition {
            kind,
            data: data.into(),
            cursor
        }));
    }
    assert_eq!(fixture.eval("e.textContent"), "a你😀b");
    assert_eq!(fixture.eval("getSelection().anchorOffset"), "4");
    assert_eq!(
        fixture.eval("JSON.stringify(phases)"),
        r#"[["compositionstart",""],["compositionupdate","ni"],["compositionupdate","你😀"],["compositionend","你😀"]]"#
    );
    fixture
        .eval("e.addEventListener('compositionstart',event=>event.preventDefault(),{once:true});");
    assert!(fixture.renderer.handle(&Event::Composition {
        kind: CompositionKind::Start,
        data: String::new(),
        cursor: 0
    }));
    assert!(!fixture.renderer.handle(&Event::Composition {
        kind: CompositionKind::End,
        data: String::new(),
        cursor: 0
    }));
    fixture.eval("e.focus();getSelection().collapse(e.firstChild,1);");
    fixture.renderer.handle(&Event::Composition {
        kind: CompositionKind::Start,
        data: String::new(),
        cursor: 0,
    });
    fixture.renderer.handle(&Event::Composition {
        kind: CompositionKind::Update,
        data: "hao".into(),
        cursor: 3,
    });
    fixture.eval("b.focus()");
    assert_eq!(fixture.eval("e.textContent"), "ahao你😀b");
    assert!(
        fixture.output.borrow().is_empty(),
        "{:?}",
        fixture.output.borrow()
    );
}

#[test]
fn hover_scale_transitions_keep_client_bounds_and_pointer_hits_in_sync() {
    let mut fixture = fixture(
        "<button id=button>Docs</button>",
        "body{margin:0} button{appearance:none;position:absolute;left:50px;top:50px;width:100px;height:40px;transition:transform .1s linear} button:hover{transform:scale(1.05)}",
        &[],
    );
    let time = Rc::new(std::cell::Cell::new(0.0));
    let clock = time.clone();
    fixture.renderer.frame.borrow_mut().clock = Rc::new(move || clock.get());
    fixture.renderer.render();
    fixture.eval("globalThis.moves=[]; button.onpointermove=e=>moves.push([e.target.id,e.isTrusted,e.clientX]);");
    fixture.pointer(PointerKind::Move, (100.0, 70.0), 1);
    fixture.renderer.render();
    time.set(0.05);
    fixture.renderer.render();
    assert_eq!(
        fixture.eval("Math.round(button.getBoundingClientRect().width*100)/100"),
        "102.5"
    );
    fixture.pointer(PointerKind::Move, (49.0, 70.0), 1);
    assert_eq!(fixture.eval("button.matches(':hover')"), "true");
    assert_eq!(
        fixture.eval("JSON.stringify(moves.at(-1))"),
        r#"["button",true,49]"#
    );
    time.set(0.1);
    fixture.renderer.render();
    assert_eq!(
        fixture.eval("Math.round(button.getBoundingClientRect().width*100)/100"),
        "105"
    );
    let node = NodeId(fixture.node("#button") as u32);
    assert_eq!(fixture.renderer.hit(48.0, 70.0), Some(node));
    assert_ne!(fixture.renderer.hit(47.0, 70.0), Some(node));
    fixture.pointer(PointerKind::Move, (10.0, 10.0), 1);
    fixture.renderer.render();
    time.set(0.2);
    fixture.renderer.render();
    assert_eq!(fixture.eval("button.getBoundingClientRect().width"), "100");
    assert_eq!(fixture.eval("button.matches(':hover')"), "false");
}

#[test]
fn css_transitions_drive_computed_styles_layout_paint_and_native_controls() {
    let mut fixture = fixture(
        "<div id=a><input id=field value=hello></div>",
        "body {margin:0} #a {width:100px; height:40px; background:black; transition:width 1s linear, transform 1s linear, background-color 1s linear, box-shadow 1s linear} #a.on {width:200px; transform:translateX(100px); background:white; box-shadow:0 0 10px black} input {width:100%; box-sizing:border-box}",
        &[],
    );
    let time = Rc::new(std::cell::Cell::new(0.0));
    let clock = time.clone();
    fixture.renderer.frame.borrow_mut().clock = Rc::new(move || clock.get());
    fixture.renderer.render();
    fixture.eval("globalThis.events=[]; for(const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) document.addEventListener(type,e=>events.push([e.type,e.propertyName,e.elapsedTime,e.isTrusted,e.target.id])); a.className='on'");
    fixture.renderer.render();
    assert!(fixture.renderer.next_wake().is_some());
    fixture.renderer.tick();
    assert_eq!(
        fixture.eval("events.filter(e=>e[0]==='transitionrun').length"),
        "4"
    );
    time.set(0.5);
    let changes = fixture.renderer.render();
    assert_eq!(fixture.eval("getComputedStyle(a).width"), "150px");
    // Recompute after a DOM edit while the transition is sampled halfway.
    // Retained CSS targets must stay at 200px, rather than retain the sampled
    // 150px presentation as a new authored target and restart the transition.
    fixture.eval("field.value='edited'");
    fixture.renderer.render();
    assert_eq!(fixture.eval("getComputedStyle(a).width"), "150px");
    assert_eq!(
        fixture.eval("getComputedStyle(a).backgroundColor"),
        "rgb(128, 128, 128)"
    );
    assert!(changes.iter().any(|change| matches!(change, Change::Paint {items,..} if items.iter().any(|item| matches!(item, DisplayItem::Fill {color,..} if *color == nativeboat_css::Color::rgb(128,128,128))))));
    assert_eq!(fixture.eval("a.getBoundingClientRect().x"), "50");
    assert_eq!(fixture.eval("field.getBoundingClientRect().width"), "150");
    assert!(changes.iter().any(
        |change| matches!(change,Change::UpdateControl {control} if control.rect.width==150.0)
    ));
    let node = NodeId(fixture.node("#field") as u32);
    assert_eq!(fixture.renderer.hit(75.0, 10.0), Some(node));
    assert!(fixture.renderer.hit(10.0, 10.0) != Some(node));
    time.set(1.0);
    fixture.renderer.tick();
    assert_eq!(fixture.eval("getComputedStyle(a).width"), "200px");
    assert_eq!(
        fixture.eval(
            "events.filter(e=>e[0]==='transitionend' && e[2]===1 && e[3] && e[4]==='a').length"
        ),
        "4"
    );
    assert_eq!(fixture.renderer.next_wake(), None);
}

#[test]
fn transition_handlers_can_change_styles_without_borrowing_the_frame_twice() {
    let mut fixture = fixture(
        "<div id=a></div>",
        "#a {opacity:0; transition:opacity 1s linear}",
        &[],
    );
    let time = Rc::new(std::cell::Cell::new(0.0));
    let clock = time.clone();
    fixture.renderer.frame.borrow_mut().clock = Rc::new(move || clock.get());
    fixture.renderer.render();
    fixture.eval("globalThis.log=[]; a.ontransitionrun=e=>log.push(e.type); a.ontransitionstart=e=>log.push(e.type); a.ontransitioncancel=e=>{log.push(e.type);a.textContent='cancelled'}; a.ontransitionend=e=>log.push(e.type); a.style.opacity='1'");
    fixture.renderer.tick();
    assert_eq!(
        fixture.eval("log.join(',')"),
        "transitionrun,transitionstart"
    );
    time.set(0.25);
    fixture.eval("a.style.transition='none'");
    fixture.renderer.tick();
    assert_eq!(
        fixture.eval("log.join(',')"),
        "transitionrun,transitionstart,transitioncancel"
    );
    assert_eq!(fixture.eval("a.textContent"), "cancelled");
    fixture.renderer.render();
    assert_eq!(fixture.renderer.next_wake(), None);
}

struct Fixture {
    renderer: Renderer,
    page: Page,
    output: Output,
    saved: Rc<RefCell<Option<String>>>,
}

/// A page with a stylesheet and module scripts given as source.
fn page(html: &str, css: &str, scripts: &[&str]) -> Page {
    let modules: Vec<Module> = scripts
        .iter()
        .enumerate()
        .map(|(index, source)| Module {
            path: format!("src/script{index}.js"),
            runtime_path: None,
            code: rewrite(source).unwrap().code,
            dependencies: Vec::new(),
        })
        .collect();
    Page {
        runtime_paths: vec!["index.html".to_owned()],
        document: nativeboat_html::parse_document(html),
        styles: vec![Style {
            path: "style.css".to_owned(),
            sheet: nativeboat_css::parse_stylesheet(css).0,
        }],
        scripts: Script {
            runtime_paths: Vec::new(),
            bundle: Bundle {
                entries: (0..modules.len()).collect(),
                modules,
                ..Bundle::default()
            },
            env: BTreeMap::new(),
        },
        module_entries: vec![true; scripts.len()],
    }
}

fn fixture_with(page: Page, dark: bool, preload: Option<&Script>) -> Fixture {
    let output: Output = Rc::default();
    let sink = output.clone();
    let saved: Rc<RefCell<Option<String>>> =
        Rc::new(RefCell::new(Some(r#"[["k","v"]]"#.to_owned())));
    let (load, save) = (saved.clone(), saved.clone());
    let options = Options {
        url: "file:///nativeboat/dist/index.html".to_owned(),
        base: "file:///nativeboat".to_owned(),
        viewport: (400.0, 300.0),
        scale: 1.5,
        dark,
        measurer: Rc::new(Mono),
        images: Rc::new(|key: &str| (key == "/dist/assets/logo.png").then_some((40.0, 20.0))),
        load_storage: Rc::new(move || load.borrow().clone()),
        save_storage: Rc::new(move |data: &str| *save.borrow_mut() = Some(data.to_owned())),
        sink: Rc::new(move |_, text: &str| sink.borrow_mut().push(text.to_owned())),
        window: 0,
    };
    assert!(format!("{options:?}").contains("viewport"));
    let mut renderer = Renderer::new(&page, options);
    assert!(format!("{renderer:?}").starts_with("Renderer"));
    let scope = preload
        .map(|_| {
            renderer
                .host()
                .eval("({ injected: 'scope' })", "scope.js")
                .unwrap()
        })
        .and_then(|value| value.as_object());
    renderer.start(&page, preload, scope.as_ref());
    Fixture {
        renderer,
        page,
        output,
        saved,
    }
}

fn fixture(html: &str, css: &str, scripts: &[&str]) -> Fixture {
    fixture_with(page(html, css, scripts), false, None)
}

impl Fixture {
    fn eval(&mut self, code: &str) -> String {
        let host = self.renderer.host();
        let value = host
            .eval(code, "test.js")
            .unwrap_or_else(|error| panic!("{error}"));
        host.run_microtasks();
        value
            .to_string(host.context())
            .unwrap()
            .to_std_string_escaped()
    }

    /// The center of an element's box.
    fn center(&mut self, selector: &str) -> (f32, f32) {
        let text = self.eval(&format!(
            "(() => {{ const r = document.querySelector('{selector}').getBoundingClientRect(); return (r.x + r.width / 2) + ',' + (r.y + r.height / 2); }})()"
        ));
        let (x, y) = text.split_once(',').unwrap();
        (x.parse().unwrap(), y.parse().unwrap())
    }

    fn node(&mut self, selector: &str) -> u64 {
        let id = self.eval(&format!(
            "(() => {{ const element = document.querySelector('{selector}'); return element[Object.getOwnPropertySymbols(element).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')]; }})()"
        ));
        id.parse().unwrap()
    }

    fn pointer(&mut self, kind: PointerKind, (x, y): (f32, f32), count: u32) -> bool {
        self.renderer.handle(&Event::Pointer {
            kind,
            button: 0,
            buttons: 1,
            count,
            x,
            y,
            modifiers: Modifiers::default(),
        })
    }

    fn click(&mut self, at: (f32, f32), count: u32) {
        self.pointer(PointerKind::Down, at, count);
        self.pointer(PointerKind::Up, at, 1);
    }

    fn texts(&mut self) -> Vec<String> {
        fn collect(surface: &Surface, texts: &mut Vec<String>) {
            for item in &surface.items {
                if let DisplayItem::Text { text, .. } = item {
                    texts.push(text.clone());
                }
            }
            for child in &surface.children {
                collect(child, texts);
            }
        }
        let mut texts = Vec::new();
        collect(&self.renderer.tree().root, &mut texts);
        texts
    }
}

#[test]
fn styled_buttons_are_painted_and_keyboard_driven() {
    let mut fixture = fixture(
        "<button id=native>Native</button><button id=styled class=s>Go <b>now</b></button>\
         <input id=send type=submit value=Send class=s><a id=ghost href=x style='visibility: hidden'>g</a>\
         <div style='display: none'><a id=none href=y>n</a></div>",
        ".s { background: #1d4ed8; color: white; border: 0; height: 40px; padding: 0 8px; font-size: 10px; line-height: 10px }",
        &[
            "globalThis.clicks = []; document.addEventListener('click', (event) => clicks.push(event.target.id));",
        ],
    );
    let changes = fixture.renderer.render();
    let controls: Vec<u64> = changes
        .iter()
        .filter_map(|change| match change {
            Change::AddControl { control, .. } => Some(control.id),
            _ => None,
        })
        .collect();
    assert_eq!(controls, vec![fixture.node("#native")]);
    let texts = fixture.texts();
    assert!(texts.iter().any(|text| text.trim() == "Go"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "Send"), "{texts:?}");
    // The content of a button is centered vertically.
    assert_eq!(
        fixture.eval(
            "(() => { const b = document.getElementById('styled').getBoundingClientRect(), t = document.querySelector('#styled b').getBoundingClientRect(); return [b.height, t.y - b.y]; })()"
        ),
        "40,15"
    );
    let tab = Event::Key {
        down: true,
        key: "Tab".to_owned(),
        code: "Tab".to_owned(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    };
    let mut order = Vec::new();
    for _ in 0..4 {
        assert!(fixture.renderer.handle(&tab));
        order.push(fixture.eval("document.activeElement.id"));
        fixture.renderer.render();
        let focus = fixture.renderer.take_requests().focus;
        order.push(format!("{focus:?}"));
    }
    let native = fixture.node("#native");
    assert_eq!(
        order,
        vec![
            "native".to_owned(),
            format!("Some(Some({native}))"),
            "styled".to_owned(),
            "Some(None)".to_owned(),
            "send".to_owned(),
            "Some(None)".to_owned(),
            "native".to_owned(),
            format!("Some(Some({native}))"),
        ]
    );
    let enter = Event::Key {
        down: true,
        key: "Enter".to_owned(),
        code: "Enter".to_owned(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    };
    fixture.eval("document.getElementById('styled').focus()");
    assert!(fixture.renderer.handle(&enter));
    assert_eq!(fixture.eval("clicks.join()"), "styled");
}

#[test]
fn pages_render_into_changes() {
    let mut fixture = fixture(
        "<title> My   App </title><p id=p>Hello</p>",
        "p { color: red }",
        &["document.getElementById('p').textContent += ' world';"],
    );
    let changes = fixture.renderer.render();
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, Change::Paint { surface: 0, .. }))
    );
    assert_eq!(fixture.texts(), ["Hello world"]);
    assert!(fixture.renderer.render().is_empty());
    assert_eq!(fixture.renderer.title(), "My App");
    // The title is read again only after the document changes.
    assert_eq!(fixture.renderer.title(), "My App");
    fixture.eval("document.querySelector('title').textContent = ' Next   Title '");
    assert_eq!(fixture.renderer.title(), "Next Title");
    assert_eq!(fixture.renderer.take_requests(), Requests::default());
    assert_eq!(
        fixture.eval("devicePixelRatio + ' ' + matchMedia('(prefers-color-scheme: dark)').matches"),
        "1.5 false"
    );
    // A new scale lays the page out again.
    fixture.renderer.set_scale(1.5);
    assert!(fixture.renderer.frame.borrow().rendered.is_some());
    fixture.renderer.set_scale(2.0);
    assert!(fixture.renderer.frame.borrow().rendered.is_none());
    assert!(fixture.renderer.render().is_empty());
    assert_eq!(fixture.eval("devicePixelRatio"), "2");
    let mut untitled = self::fixture("<p>x</p>", "", &[]);
    assert_eq!(untitled.renderer.title(), "");
    assert_eq!(
        untitled.eval("localStorage.getItem('k') + location.href"),
        "vfile:///nativeboat/dist/index.html"
    );
    untitled.eval("localStorage.setItem('n', '1')");
    assert_eq!(
        untitled.saved.borrow().as_deref(),
        Some(r#"[["k","v"],["n","1"]]"#)
    );
    let dark = fixture_with(
        page(
            "<p>x</p>",
            "",
            &["globalThis.dark = matchMedia('(prefers-color-scheme: dark)').matches;"],
        ),
        true,
        None,
    );
    let mut dark = dark;
    assert_eq!(dark.eval("String(dark)"), "true");
    assert_eq!(fixture.page.runtime_paths, ["index.html"]);
    let cleared = fixture.renderer.clear();
    assert!(
        cleared
            .iter()
            .any(|change| matches!(change, Change::Paint { surface: 0, .. }))
    );
    assert!(fixture.renderer.clear().is_empty());
}

#[test]
fn drawn_selects_ask_for_their_options() {
    let mut fixture = fixture(
        "<select id=s class=s><option>one</option><option selected> two  words </option></select>\
         <select id=e class=s></select><select id=n><option>native</option></select>",
        ".s { border: 1px solid red; width: 100px; height: 20px }",
        &[],
    );
    fixture.renderer.render();
    let select = fixture.node("#s");
    let at = fixture.center("#s");
    fixture.click(at, 1);
    let request = fixture.renderer.take_requests().select.unwrap();
    assert_eq!(request.id, select);
    assert_eq!(request.options, vec!["one", "two words"]);
    assert_eq!(request.selected, Some(1));
    assert_eq!((request.rect.width, request.rect.height), (100.0, 20.0));
    assert_eq!(fixture.renderer.take_requests().select, None);
    // The choice comes back as the control's selection.
    fixture.renderer.handle(&Event::Control {
        id: select,
        event: ControlEvent::Select(vec![0]),
    });
    assert_eq!(fixture.eval("s.value"), "one");
    let at = fixture.center("#e");
    fixture.click(at, 1);
    let request = fixture.renderer.take_requests().select.unwrap();
    assert_eq!((request.options.len(), request.selected), (0, None));
    // Nothing shows for a select gone from the page.
    let at = fixture.center("#s");
    fixture.click(at, 1);
    fixture.eval("s.style.display = 'none'");
    fixture.renderer.render();
    assert_eq!(fixture.renderer.take_requests().select, None);
}

#[test]
fn clicks_reach_scripts_through_hit_testing() {
    let mut fixture = fixture(
        "<div id=box style='height:40px;cursor:pointer'>box</div><p id=out>0</p>",
        "",
        &[
            "const out = document.getElementById('out'); const box = document.getElementById('box');\
           box.addEventListener('click', (event) => { out.textContent = String(Number(out.textContent) + event.detail); });\
           box.addEventListener('dblclick', () => { out.textContent += '!'; });\
           document.addEventListener('mousedown', (event) => { if (event.target.id === 'out') event.preventDefault(); });",
        ],
    );
    fixture.renderer.render();
    let at = fixture.center("#box");
    fixture.click(at, 1);
    fixture.click(at, 2);
    fixture.renderer.render();
    assert_eq!(fixture.texts()[1], "3!");
    let requests = fixture.renderer.take_requests();
    assert_eq!(requests.cursor, None);
    fixture.pointer(PointerKind::Move, at, 0);
    assert_eq!(
        fixture.renderer.take_requests().cursor,
        Some(Cursor::Pointer)
    );
    fixture.pointer(PointerKind::Move, at, 0);
    assert_eq!(fixture.renderer.take_requests().cursor, None);
    fixture.pointer(PointerKind::Leave, (0.0, 0.0), 0);
    assert_eq!(fixture.renderer.take_requests().cursor, Some(Cursor::Auto));
    let out = fixture.center("#out");
    assert!(fixture.pointer(PointerKind::Down, out, 1));
    fixture.pointer(PointerKind::Up, at, 1);
    fixture.pointer(PointerKind::Down, at, 1);
    fixture.renderer.handle(&Event::Pointer {
        kind: PointerKind::Up,
        button: 2,
        buttons: 0,
        count: 1,
        x: at.0,
        y: at.1,
        modifiers: Modifiers::default(),
    });
    assert_eq!(
        fixture.eval("document.getElementById('out').textContent"),
        "3!"
    );
    fixture.pointer(PointerKind::Move, at, 0);
    assert_eq!(
        fixture.renderer.take_requests().cursor,
        Some(Cursor::Pointer)
    );
    fixture.pointer(PointerKind::Move, (399.0, 299.0), 0);
    assert_eq!(fixture.renderer.take_requests().cursor, Some(Cursor::Auto));
}

#[test]
fn native_controls_drive_form_state() {
    let mut fixture = fixture(
        "<input id=t><input id=c type=checkbox><select id=s><option>a</option><option>b</option></select>\
         <input id=r type=range><div id=d tabindex=0>d</div><p id=log></p>",
        "",
        &["const log = document.getElementById('log');\
           for (const type of ['input', 'change', 'focus', 'blur']) document.addEventListener(type, (event) => { log.textContent += type + ':' + event.target.id + ' '; }, true);"],
    );
    let text = fixture.node("#t");
    let checkbox = fixture.node("#c");
    let select = fixture.node("#s");
    let range = fixture.node("#r");
    let control = |id: u64, event: ControlEvent| Event::Control { id, event };
    for event in [
        control(text, ControlEvent::Focus),
        control(text, ControlEvent::Input("typed".to_owned())),
        control(text, ControlEvent::Blur),
        control(checkbox, ControlEvent::Click),
        control(select, ControlEvent::Select(vec![1])),
        control(range, ControlEvent::Value(70.0)),
        control(checkbox, ControlEvent::Blur),
    ] {
        assert!(!fixture.renderer.handle(&event));
    }
    assert_eq!(
        fixture.eval("log.textContent"),
        "focus:t input:t blur:t change:t input:c change:c input:s change:s input:r change:r "
    );
    assert_eq!(
        fixture.eval("[t.value, c.checked, s.value, r.value].join()"),
        "typed,true,b,70"
    );
    assert_eq!(fixture.renderer.take_requests().focus, Some(None));
    fixture.eval("document.getElementById('t').focus()");
    assert_eq!(fixture.renderer.take_requests().focus, Some(Some(text)));
    fixture.eval("document.getElementById('d').focus()");
    assert_eq!(fixture.renderer.take_requests().focus, Some(None));
    fixture.renderer.render();
    assert!(
        fixture
            .renderer
            .tree()
            .root
            .controls
            .iter()
            .any(|control| control.id == text && control.state.text == "typed")
    );
}

#[test]
fn moving_the_caret_or_typing_scrolls_it_into_view() {
    let lines: String = (0..20).map(|n| format!("<p>line {n}</p>")).collect();
    let mut fixture = fixture(
        &format!(
            "<div id=ed contenteditable style='overflow:auto;height:100px'>{lines}</div>\
             <div style='height:2000px'></div>"
        ),
        "body{margin:0} p{margin:0;height:20px;line-height:20px}",
        &[],
    );
    fixture.renderer.render();
    let press = |fixture: &mut Fixture, key: &str, ctrl: bool| {
        fixture.renderer.handle(&Event::Key {
            down: true,
            key: key.to_owned(),
            code: String::new(),
            repeat: false,
            modifiers: Modifiers {
                ctrl,
                ..Modifiers::default()
            },
            control: None,
        })
    };
    let tops = |fixture: &mut Fixture| {
        fixture.eval("[ed.scrollTop, document.documentElement.scrollTop].join()")
    };
    // How far the caret lies inside the scroller's top and bottom edges.
    let room = |fixture: &mut Fixture| {
        fixture.eval(
            "(() => { const box = ed.getBoundingClientRect();              const caret = getSelection().getRangeAt(0).getBoundingClientRect();              return [caret.top - box.top, box.bottom - caret.bottom].map(Math.round).join(); })()",
        )
    };
    fixture.eval("ed.focus(); getSelection().collapse(ed.firstChild.firstChild, 0)");
    // While the caret shows nothing scrolls; past the edge, the least that
    // shows it.
    for _ in 0..4 {
        assert!(press(&mut fixture, "ArrowDown", false));
    }
    assert_eq!(tops(&mut fixture), "0,0");
    assert!(press(&mut fixture, "ArrowDown", false));
    assert_ne!(tops(&mut fixture), "0,0");
    assert!(room(&mut fixture).ends_with(",0"));
    assert!(press(&mut fixture, "End", true));
    assert!(room(&mut fixture).ends_with(",0"));
    assert_eq!(fixture.eval("getSelection().anchorNode.data"), "line 19");
    // The caret's own box, not its line's, meets the edge, as in Chromium.
    assert!(press(&mut fixture, "Home", true));
    assert!(room(&mut fixture).starts_with("0,"));
    // Typing where the caret is out of view brings it back.
    fixture.eval("ed.scrollTop = 200");
    assert!(fixture.renderer.handle(&Event::Text("x".into())));
    assert!(room(&mut fixture).starts_with("0,"));
    assert_eq!(fixture.eval("ed.firstChild.textContent"), "xline 0");
    // Scripts moving the selection leave scrolling alone.
    fixture.eval("ed.scrollTop = 0; getSelection().collapse(ed.lastChild.firstChild, 0)");
    assert_eq!(tops(&mut fixture), "0,0");
}

#[test]
fn keys_the_page_leaves_alone_scroll_the_nearest_scroller() {
    let lines: String = (0..60).map(|n| format!("<p>line {n}</p>")).collect();
    let mut fixture = fixture(
        &format!(
            "<div id=box style='overflow:auto;height:100px'><div style='height:1000px'>tall</div></div>\
             <input id=field><div id=ed contenteditable style='overflow:auto;height:100px'>{lines}</div>\
             <div style='height:2000px'></div>"
        ),
        "body{margin:0} p{margin:0;height:20px}",
        &[],
    );
    fixture.renderer.render();
    let press = |fixture: &mut Fixture, key: &str, ctrl: bool, shift: bool| {
        fixture.renderer.handle(&Event::Key {
            down: true,
            key: key.to_owned(),
            code: String::new(),
            repeat: false,
            modifiers: Modifiers {
                ctrl,
                shift,
                ..Modifiers::default()
            },
            control: None,
        })
    };
    let tops = |fixture: &mut Fixture| {
        fixture.eval("[box.scrollTop, ed.scrollTop, document.documentElement.scrollTop].join()")
    };
    // Nothing focused: the scroller around the last press scrolls.
    fixture.click((10.0, 10.0), 1);
    assert!(press(&mut fixture, "ArrowDown", false, false));
    assert_eq!(tops(&mut fixture), "40,0,0");
    assert!(press(&mut fixture, "PageDown", false, false));
    assert_eq!(tops(&mut fixture), "127.5,0,0");
    assert!(press(&mut fixture, "End", true, false));
    assert_eq!(tops(&mut fixture), "900,0,0");
    // At its end the keys reach the viewport.
    assert!(press(&mut fixture, "ArrowDown", false, false));
    assert_eq!(tops(&mut fixture), "900,0,40");
    // Shift, and Control but with Home and End, leave scrolling alone.
    assert!(!press(&mut fixture, "ArrowUp", false, true));
    assert!(!press(&mut fixture, "ArrowUp", true, false));
    assert!(press(&mut fixture, "Home", false, false));
    assert_eq!(tops(&mut fixture), "0,0,40");
    // Space pages, Shift+Space back.
    assert!(press(&mut fixture, " ", false, false));
    assert_eq!(tops(&mut fixture), "87.5,0,40");
    assert!(press(&mut fixture, " ", false, true));
    assert_eq!(tops(&mut fixture), "0,0,40");
    // A focused text field keeps its keys.
    fixture.eval("document.documentElement.scrollTop = 0; field.focus()");
    assert!(!press(&mut fixture, "PageDown", false, false));
    assert_eq!(tops(&mut fixture), "0,0,0");
    // In editable content Page Down scrolls and moves the caret along.
    fixture.eval("ed.focus(); getSelection().collapse(ed.firstChild.firstChild, 2)");
    assert!(press(&mut fixture, "PageDown", false, false));
    assert_eq!(tops(&mut fixture), "0,87.5,0");
    assert_eq!(fixture.eval("getSelection().anchorNode.data"), "line 4");
}

#[test]
fn points_fall_to_the_line_box_they_are_in() {
    // The raised span makes the second line box start well above its plain
    // text; a point there is on the second line, nearest its first letter,
    // as in Chromium, not on whichever text is nearest.
    let mut fixture = fixture(
        "<p id=p>aaaa<br>bbbb <span id=up style='vertical-align:12px'>c</span></p>",
        "body{margin:0} p{margin:0;font:10px/20px monospace}",
        &[],
    );
    fixture.renderer.render();
    let at = |fixture: &mut Fixture, y: f32| {
        fixture.eval(&format!(
            "(caret => caret.offsetNode.data + ':' + caret.offset)(document.caretPositionFromPoint(1, {y}))"
        ))
    };
    // The second line runs from 20 to 52; its plain text from 32.
    assert_eq!(fixture.eval("p.offsetHeight"), "52");
    assert_eq!(at(&mut fixture, 21.0), "bbbb :0");
    // The edge between the lines belongs to the lower one.
    assert_eq!(at(&mut fixture, 20.0), "bbbb :0");
    assert_eq!(at(&mut fixture, 19.0), "aaaa:0");
}

#[test]
fn keys_wheels_scrolling_and_resizing() {
    let mut fixture = fixture(
        "<div id=box style='overflow:auto;height:50px'><div style='height:500px'>tall</div></div><div style='height:900px'></div>",
        "",
        &["globalThis.log = [];\
           document.addEventListener('keydown', (event) => { log.push(event.key + event.code + event.ctrlKey); if (event.key === 'x') event.preventDefault(); });\
           document.addEventListener('wheel', (event) => { log.push('wheel' + event.deltaY); event.preventDefault(); });\
           document.addEventListener('scroll', () => log.push('doc scroll'));\
           document.getElementById('box').addEventListener('scroll', () => log.push('box scroll'));\
           window.addEventListener('resize', () => log.push('resize ' + innerWidth));"],
    );
    fixture.renderer.render();
    let key = |key: &str| Event::Key {
        down: true,
        key: key.to_owned(),
        code: "KeyX".to_owned(),
        repeat: false,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
        control: None,
    };
    assert!(fixture.renderer.handle(&key("x")));
    assert!(!fixture.renderer.handle(&key("y")));
    assert!(fixture.renderer.handle(&Event::Wheel {
        x: 10.0,
        y: 10.0,
        delta_x: 0.0,
        delta_y: 100.0,
        modifiers: Modifiers::default(),
    }));
    let box_id = fixture.node("#box");
    fixture.renderer.handle(&Event::Scroll {
        surface: 0,
        x: 0.0,
        y: 120.0,
    });
    fixture.renderer.handle(&Event::Scroll {
        surface: box_id,
        x: 0.0,
        y: 30.0,
    });
    assert_eq!(
        fixture.eval(
            "document.documentElement.scrollTop + ',' + document.getElementById('box').scrollTop"
        ),
        "120,30"
    );
    fixture.renderer.handle(&Event::Resized {
        width: 500.0,
        height: 200.0,
    });
    assert_eq!(
        fixture.eval("log.join('|')"),
        "xKeyXtrue|yKeyXtrue|wheel100|doc scroll|box scroll|resize 500"
    );
    let changes = fixture.renderer.render();
    assert!(changes.iter().any(
        |change| matches!(change, Change::UpdateSurface { id: 0, rect, .. } if rect.width == 500.0)
    ));
    for event in [Event::Activated(true), Event::Close, Event::Destroyed] {
        assert!(!fixture.renderer.handle(&event));
    }
    let empty = self::fixture("<p>x</p>", "", &[]);
    let dom = empty.renderer.dom();
    let root = dom.borrow().document_element().unwrap();
    dom.borrow_mut().remove(root);
    let mut empty = empty;
    empty.renderer.handle(&Event::Scroll {
        surface: 0,
        x: 0.0,
        y: 5.0,
    });
}

#[test]
fn timers_and_animation_frames() {
    let mut fixture = fixture(
        "<p id=p>0</p>",
        "",
        &[
            "globalThis.frames = 0; const step = () => { frames++; if (frames < 2) requestAnimationFrame(step); }; requestAnimationFrame(step);\
           setTimeout(() => { document.getElementById('p').textContent = 'timer'; }, 0);",
        ],
    );
    assert_eq!(fixture.renderer.next_wake(), Some(Duration::ZERO));
    std::thread::sleep(Duration::from_millis(2));
    fixture.renderer.tick();
    assert_eq!(fixture.eval("String(frames)"), "1");
    let wait = fixture.renderer.next_wake().unwrap();
    assert!(wait <= FRAME);
    fixture.renderer.tick();
    assert_eq!(fixture.eval("String(frames)"), "1");
    std::thread::sleep(FRAME);
    fixture.renderer.tick();
    assert_eq!(fixture.eval("String(frames)"), "2");
    assert_eq!(fixture.renderer.next_wake(), None);
    fixture.renderer.render();
    assert_eq!(fixture.texts(), ["timer"]);
}

#[test]
fn intersection_observers_report_scrolling_and_moving_targets() {
    let mut fixture = fixture(
        "<div id=box style='height:100px;overflow:auto'><div style='height:150px'></div>\
         <p id=low style='height:20px;margin:0'>low</p><div style='height:300px'></div></div>\
         <p id=mark style='height:20px;margin:0'>mark</p>",
        "body{margin:0}",
        &[],
    );
    // A frame, then the task after it that reports intersections.
    let frame = |fixture: &mut Fixture| {
        std::thread::sleep(FRAME);
        fixture.renderer.tick();
        fixture.renderer.tick();
    };
    fixture.eval(
        "globalThis.seen = []; new IntersectionObserver((entries) => entries.forEach((entry) => \
         seen.push(entry.isIntersecting + ':' + entry.intersectionRatio)), { root: box, threshold: [0, 1] })\
         .observe(low)",
    );
    frame(&mut fixture);
    assert_eq!(fixture.eval("seen.join()"), "false:0");
    // Half of it scrolls into the box, then all of it.
    fixture.eval("box.scrollTop = 60");
    frame(&mut fixture);
    assert_eq!(fixture.eval("seen.join()"), "false:0,true:0.5");
    fixture.eval("box.scrollTop = 100");
    frame(&mut fixture);
    assert_eq!(fixture.eval("seen.join()"), "false:0,true:0.5,true:1");
    // As floating-ui watches for layout shifts: the root shrunk to the
    // target's own box, so that any move leaves part of it outside.
    fixture.eval(
        "globalThis.moves = []; const rect = mark.getBoundingClientRect(); \
         const margin = [-rect.top, -(document.documentElement.clientWidth - rect.right), \
         -(document.documentElement.clientHeight - rect.bottom), -rect.left].map((side) => side + 'px').join(' '); \
         new IntersectionObserver((entries) => moves.push(entries[0].intersectionRatio), \
         { root: document, rootMargin: margin, threshold: 1 }).observe(mark)",
    );
    frame(&mut fixture);
    assert_eq!(fixture.eval("moves.join()"), "1");
    fixture.eval("box.style.height = '110px'");
    frame(&mut fixture);
    assert_eq!(fixture.eval("moves.join()"), "1,0.5");
}

#[test]
fn images_and_style_sheets() {
    let mut fixture = fixture(
        "<img id=logo src='assets/logo.png'><img id=root src='/dist/assets/logo.png?v=1'><img id=missing src='none.png'>\
         <img id=far src='https://example.com/x.png'><img id=data src='data:image/png;base64,AA'><p id=p>styled</p>",
        "",
        &[
            "const style = document.createElement('style'); style.textContent = 'p { background: rgb(0, 0, 255) }'; document.head.appendChild(style);\
           style.sheet.insertRule('#p { color: rgb(255, 0, 0) }', 1);",
        ],
    );
    fixture.renderer.render();
    let images: Vec<String> = fixture
        .renderer
        .tree()
        .root
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Image { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        images,
        [
            "/dist/assets/logo.png",
            "/dist/assets/logo.png",
            "none.png",
            "https://example.com/x.png",
            "data:image/png;base64,AA"
        ]
    );
    assert_eq!(
        fixture.eval("logo.naturalWidth + ',' + missing.naturalWidth"),
        "40,0"
    );
    let fills = fixture.renderer.tree().root.items.iter().filter(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == nativeboat_css::Color::rgb(0, 0, 255))).count();
    assert_eq!(fills, 1);
    let red = fixture.renderer.tree().root.items.iter().any(|item| matches!(item, DisplayItem::Text { color, .. } if *color == nativeboat_css::Color::rgb(255, 0, 0)));
    assert!(red);
    fixture.eval("document.querySelector('style').sheet.deleteRule(1)");
    fixture.renderer.render();
    assert!(!fixture.renderer.tree().root.items.iter().any(|item| matches!(item, DisplayItem::Text { color, .. } if *color == nativeboat_css::Color::rgb(255, 0, 0))));
    fixture.eval("document.querySelector('style').textContent = 'p { color: rgb(0, 128, 0) }'");
    fixture.renderer.render();
    let frame = fixture.renderer.frame.borrow();
    assert_eq!(frame.asset_keys("file:///other/x.png"), ["/other/x.png"]);
    assert_eq!(frame.asset_keys("../up.png"), ["/up.png", "/up.png"]);
}

#[test]
fn caret_positions_and_range_rectangles_follow_the_painted_text() {
    let mut fixture = fixture(
        "<p id=p>hello world</p>",
        "body { margin: 0 } p { margin: 0; font: 10px/10px serif }",
        &[],
    );
    assert_eq!(
        fixture.eval(
            "const caret = document.caretPositionFromPoint(12, 5); \
             [caret.offsetNode === p.firstChild, caret.offset].join()"
        ),
        "true,2"
    );
    assert_eq!(
        fixture.eval(
            "const range = new Range(); range.setStart(p.firstChild, 1); range.setEnd(p.firstChild, 3); \
             JSON.stringify([...range.getClientRects()].map((r) => [r.x, r.y, r.width, r.height]))"
        ),
        "[[5,0,10,10]]"
    );
    // Geometry follows changes to the text.
    assert_eq!(
        fixture.eval("p.firstChild.data = 'xy'; document.caretPositionFromPoint(300, 5).offset"),
        "2"
    );
}

#[test]
fn layout_queries_and_scrolling_into_view() {
    let mut fixture = fixture(
        "<div id=outer style='overflow:auto;height:100px;border:2px solid black'><div style='height:400px'></div><p id=deep>deep</p></div>\
         <div style='height:2000px'></div><p id=bottom>bottom</p><span id=hidden style='display:none'>x<b id=within style='color: rgb(1, 2, 3)'>y</b></span><div id=clip style='overflow:hidden;height:10px'><p id=inside>i</p></div>",
        "",
        &[],
    );
    assert_eq!(
        fixture.eval("[outer.clientWidth > 0, outer.clientLeft, outer.clientTop, outer.scrollHeight, document.documentElement.scrollHeight > 2000, hidden.clientWidth, hidden.scrollWidth].join()"),
        "true,2,2,435,true,0,0"
    );
    assert_eq!(fixture.eval("getComputedStyle(outer).overflowY + getComputedStyle(hidden).display + getComputedStyle(document).length"), "autonone0");
    // Elements inside hidden ones are styled when asked.
    assert_eq!(
        fixture.eval("getComputedStyle(within).color + ' ' + getComputedStyle(within).display"),
        "rgb(1, 2, 3) inline"
    );
    assert_eq!(
        fixture.eval("document.elementFromPoint(9, 9) === outer"),
        "true"
    );
    fixture.eval("document.getElementById('deep').scrollIntoView({block:'nearest'})");
    assert_eq!(
        fixture.eval("outer.scrollTop > 300 && document.documentElement.scrollTop === 0"),
        "true"
    );
    fixture.eval("document.getElementById('bottom').scrollIntoView({block:'nearest'})");
    assert_eq!(
        fixture.eval("document.documentElement.scrollTop > 1700"),
        "true"
    );
    fixture.eval("document.getElementById('outer').scrollIntoView({block:'nearest'})");
    assert_eq!(fixture.eval("document.documentElement.scrollTop"), "8");
    fixture.eval("hidden.scrollIntoView({block:'nearest'})");
    assert_eq!(fixture.eval("document.documentElement.scrollTop"), "8");
    fixture.eval("document.getElementById('inside').scrollIntoView({block:'nearest'})");
    // The hidden-overflow container scrolls its content before the viewport.
    assert_eq!(fixture.eval("document.documentElement.scrollTop"), "1881");
}

#[test]
fn scroll_alignment_margin_padding_and_nested_limits() {
    let mut fixture = fixture(
        "<div id=outer><div style='height:300px;width:500px'></div><div id=target></div><div style='height:400px'></div></div><div style='height:1500px'></div><a id=link href='#target'>target</a>",
        "body{margin:0} #outer{overflow:hidden;height:100px;width:200px;scroll-padding:10px 20px} #target{height:20px;width:30px;scroll-margin:5px}",
        &[],
    );
    fixture.eval("target.scrollIntoView({block:'center',inline:'center'})");
    assert_eq!(fixture.eval("outer.scrollTop"), "260");
    fixture.eval("target.scrollIntoView({block:'end'})");
    assert_eq!(fixture.eval("outer.scrollTop"), "235");
    fixture.eval("target.scrollIntoView({block:'start'})");
    assert_eq!(fixture.eval("outer.scrollTop"), "285");
    fixture.eval("target.scrollIntoViewIfNeeded()");
    assert_eq!(fixture.eval("outer.scrollTop"), "285");
    fixture.eval("outer.scrollTop=400; target.scrollIntoView({block:'nearest'})");
    assert_eq!(fixture.eval("outer.scrollTop"), "285");
    fixture.eval("outer.scrollTop=0; target.scrollIntoView(false)");
    assert_eq!(fixture.eval("outer.scrollTop"), "235");
    fixture.eval("target.scrollIntoView(null)");
    assert_eq!(fixture.eval("outer.scrollTop"), "285");
    fixture.eval("outer.scrollTop=0;link.click()");
    assert_eq!(fixture.eval("outer.scrollTop"), "285");
    assert_eq!(fixture.eval("location.hash"), "#target");
    fixture.eval("location.hash='#top'");
    assert_eq!(fixture.eval("document.documentElement.scrollTop"), "0");
}

#[test]
fn document_rows_drag_to_folders_without_clicking() {
    let mut fixture = fixture(
        "<div id=row draggable=true><span id=label>Document</span></div><div id=folder>Folder</div>",
        "#row,#folder{height:40px}",
        &[r#"
        globalThis.log=[];
        row.addEventListener('dragstart', e=>{e.dataTransfer.effectAllowed='move';e.dataTransfer.setData('text/plain','doc-1');log.push('start')});
        folder.addEventListener('dragover',e=>{e.preventDefault();folder.classList.add('target')});
        folder.addEventListener('drop',e=>{e.preventDefault();log.push(e.dataTransfer.getData('text/plain'));folder.classList.remove('target')});
        row.addEventListener('dragend',e=>log.push('end:'+e.dataTransfer.dropEffect));
        row.addEventListener('click',()=>log.push('click'));
        "#],
    );
    let start = fixture.center("#label");
    let end = fixture.center("#folder");
    fixture.pointer(PointerKind::Down, start, 1);
    fixture.pointer(PointerKind::Move, end, 1);
    assert_eq!(fixture.eval("folder.classList.contains('target')"), "true");
    fixture.pointer(PointerKind::Up, end, 1);
    assert_eq!(fixture.eval("log.join('|')"), "start|doc-1|end:move");
}

#[test]
fn shell_drops_expose_readable_files_with_their_original_paths() {
    let path = std::env::temp_dir().join(format!("nativeboat-drop-{}.txt", std::process::id()));
    std::fs::write(&path, "hello drop").unwrap();
    let mut fixture = fixture(
        "<div id=zone>Import</div>",
        "#zone{height:100px}",
        &[r#"
        globalThis.files=[];globalThis.text='';
        zone.addEventListener('dragover',e=>e.preventDefault());
        zone.addEventListener('drop',e=>{ e.preventDefault();files=Array.from(e.dataTransfer.files);files[0].text().then(value=>text=value); });
        "#],
    );
    let (x, y) = fixture.center("#zone");
    assert!(fixture.renderer.handle(&Event::FilesDropped {
        paths: vec![
            path.to_string_lossy().into_owned(),
            "C:\\missing-nativeboat-drop.txt".into()
        ],
        strings: Vec::new(),
        x,
        y
    }));
    assert_eq!(fixture.eval("text"), "hello drop");
    assert_eq!(
        fixture.eval("files.length+','+files[0].size+','+files[0].type+','+files[1].size"),
        "2,10,text/plain,0"
    );
    assert_eq!(fixture.eval("files[0].path"), path.to_string_lossy());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn incoming_file_hover_updates_hit_targets_and_stays_protected_until_drop() {
    let mut fixture = fixture(
        "<div id=zone>Import</div><div id=outside>Outside</div>",
        "#zone,#outside{height:80px} .hover{background:blue}",
        &[r#"
        globalThis.hoverLog=[];
        for(const type of ['dragenter','dragover','dragleave'])zone.addEventListener(type,e=>hoverLog.push([type,e.dataTransfer.types.join(),e.dataTransfer.files.length,e.dataTransfer.items[0].getAsFile(),e.ctrlKey,e.clientX,e.clientY]));
        zone.addEventListener('dragover',e=>{e.preventDefault();zone.classList.add('hover')});
        zone.addEventListener('dragleave',()=>zone.classList.remove('hover'));
        "#],
    );
    let (x, y) = fixture.center("#zone");
    let paths = vec!["C:\\incoming\\not-read-until-drop.txt".into()];
    assert!(fixture.renderer.handle(&Event::FileDrag {
        kind: nativeboat_win32::FileDragKind::Enter,
        paths: paths.clone(),
        strings: Vec::new(),
        x,
        y,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    }));
    assert_eq!(fixture.eval("zone.classList.contains('hover')"), "true");
    assert_eq!(
        fixture.eval("JSON.stringify(hoverLog.map(e=>e.slice(0,5)))"),
        r#"[["dragenter","Files",0,null,true],["dragover","Files",0,null,true]]"#
    );
    assert_eq!(
        fixture.eval("hoverLog[0][5]+','+hoverLog[0][6]"),
        format!("{x},{y}")
    );
    assert!(fixture.renderer.handle(&Event::FileDrag {
        kind: nativeboat_win32::FileDragKind::Over,
        paths: paths.clone(),
        strings: Vec::new(),
        x,
        y,
        modifiers: Modifiers::default(),
    }));
    let (x, y) = fixture.center("#outside");
    assert!(!fixture.renderer.handle(&Event::FileDrag {
        kind: nativeboat_win32::FileDragKind::Over,
        paths: paths.clone(),
        strings: Vec::new(),
        x,
        y,
        modifiers: Modifiers::default(),
    }));
    assert_eq!(fixture.eval("zone.classList.contains('hover')"), "false");
    assert!(!fixture.renderer.handle(&Event::FileDrag {
        kind: nativeboat_win32::FileDragKind::Leave,
        paths,
        strings: Vec::new(),
        x,
        y,
        modifiers: Modifiers::default(),
    }));
    assert!(
        fixture.output.borrow().is_empty(),
        "renderer listeners reported an error"
    );
}

#[test]
fn script_errors_are_reported() {
    let preload = page(
        "<p>x</p>",
        "",
        &["throw new Error('preload failed: ' + injected);"],
    )
    .scripts;
    let mut fixture = fixture_with(
        page("<p>x</p>", "", &["window.ok = 1;", "missing.call();"]),
        false,
        Some(&preload),
    );
    let output = fixture.output.borrow().clone();
    assert!(
        output[0].starts_with("Uncaught Error: preload failed: scope"),
        "{output:?}"
    );
    assert!(
        output[1].starts_with("Uncaught ReferenceError"),
        "{output:?}"
    );
    assert_eq!(fixture.eval("String(window.ok)"), "1");
}

#[test]
fn interaction_states_restyle_pages() {
    let mut fixture = fixture(
        "<div id=box>box</div><input id=field><ul><li>one</li></ul><table><tr><td>cell</td></tr></table>",
        "#box { height: 20px } #box:hover { height: 40px } #box:active { height: 60px } #field { width: 100px } #field:focus { width: 300px }",
        &[],
    );
    fixture.renderer.render();
    let height = |fixture: &mut Fixture| {
        fixture.eval("String(document.getElementById('box').getBoundingClientRect().height)")
    };
    let at = fixture.center("#box");
    fixture.pointer(PointerKind::Move, at, 0);
    fixture.renderer.render();
    assert_eq!(height(&mut fixture), "40");
    fixture.pointer(PointerKind::Down, at, 1);
    fixture.renderer.render();
    assert_eq!(height(&mut fixture), "60");
    fixture.pointer(PointerKind::Up, at, 1);
    fixture.renderer.render();
    assert_eq!(height(&mut fixture), "40");
    fixture.pointer(PointerKind::Leave, (0.0, 0.0), 0);
    fixture.renderer.render();
    assert_eq!(height(&mut fixture), "20");
    fixture.eval("document.getElementById('field').focus()");
    fixture.renderer.render();
    // A text field's width is its content box's, as in Chromium.
    assert_eq!(
        fixture.eval("String(document.getElementById('field').offsetWidth)"),
        "308"
    );
    let texts = fixture.texts();
    // The item's disc marker is a shape, not a bullet glyph.
    assert!(texts.iter().any(|text| text == "one"), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text.contains('\u{2022}')),
        "{texts:?}"
    );
    assert!(texts.iter().any(|text| text == "cell"), "{texts:?}");
}

#[test]
fn selections_and_the_clipboard_reach_the_platform() {
    use nativeboat_win32::Error;
    use shimforge::{Session, mock};
    let mut fixture = fixture("<input id=t value=hello>", "", &[]);
    let text = fixture.node("#t");
    fixture.renderer.render();
    let selection = "[t.selectionStart, t.selectionEnd].join()";
    assert_eq!(fixture.eval(selection), "5,5");
    fixture.eval("t.setSelectionRange(1, 3)");
    assert_eq!(fixture.eval(selection), "1,3");
    assert!(fixture.renderer.render().contains(&Change::Select {
        id: text,
        start: 1,
        end: 3
    }));
    assert!(
        !fixture
            .renderer
            .render()
            .iter()
            .any(|change| matches!(change, Change::Select { .. }))
    );
    let mut session = Session::new();
    let shown = mock!(
        session,
        nativeboat_win32::text_selection,
        fn(u64, u64) -> Option<(usize, usize)>
    );
    shown.expect().returning(|_, _| Some((0, 2)));
    let read = mock!(session, nativeboat_win32::read_text, fn() -> String);
    read.expect().returning(|| "pasted".to_owned());
    let write = mock!(
        session,
        nativeboat_win32::write_clipboard,
        fn(Option<&str>, Option<&str>) -> Result<(), Error>
    );
    write.expect().returning(|text, _| {
        if text == Some("he") {
            Ok(())
        } else {
            Err(Error {
                message: "busy".to_owned(),
            })
        }
    });
    assert_eq!(fixture.eval(selection), "0,2");
    fixture.eval(
        "t.focus(); globalThis.copied = document.execCommand('copy');\
         navigator.clipboard.readText().then((text) => { globalThis.read = text; });\
         navigator.clipboard.writeText('other').catch((error) => { globalThis.failed = error.name; });",
    );
    assert_eq!(
        fixture.eval("[copied, read, failed].join()"),
        "true,pasted,NotAllowedError"
    );
    drop(session);
}

#[test]
fn page_clipboard_events_carry_native_html_and_write_formats_together() {
    use nativeboat_win32::Error;
    use shimforge::{Session, mock};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    let mut session = Session::new();
    let writes = Arc::new(Mutex::new(Vec::new()));
    let seen = writes.clone();
    mock!(
        session,
        nativeboat_win32::write_clipboard,
        fn(Option<&str>, Option<&str>) -> Result<(), Error>
    )
    .expect()
    .returning(move |text, html| {
        seen.lock()
            .unwrap()
            .push((text.map(str::to_owned), html.map(str::to_owned)));
        Ok(())
    });
    let available = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let read = available.clone();
    mock!(
        session,
        nativeboat_win32::read_clipboard_contents,
        fn() -> Option<Vec<(&'static str, String)>>
    )
    .expect()
    .returning(move || {
        read.load(Ordering::Relaxed).then(|| {
            vec![
                ("text/plain", "plain😀".into()),
                ("text/html", "<b>rich😀</b>".into()),
            ]
        })
    });
    let mut fixture = fixture("<div id=e contenteditable><b>bold</b> tail</div>", "", &[]);
    fixture.eval("e.focus();const range=new Range();range.selectNodeContents(e);getSelection().removeAllRanges();getSelection().addRange(range)");
    assert!(fixture.renderer.edit_command("copy"));
    assert_eq!(
        *writes.lock().unwrap(),
        vec![(Some("bold tail".into()), Some("<b>bold</b> tail".into()))]
    );
    fixture.eval("e.addEventListener('paste',event=>{globalThis.paste=[event.isTrusted,event.clipboardData.types,event.clipboardData.getData('text/plain'),event.clipboardData.getData('text/html')];globalThis.saved=event.clipboardData;event.preventDefault()});");
    assert!(fixture.renderer.edit_command("paste"));
    assert_eq!(
        fixture.eval("JSON.stringify(paste)"),
        r#"[true,["text/plain","text/html"],"plain😀","<b>rich😀</b>"]"#
    );
    assert_eq!(
        fixture.eval("saved.types.length+':'+saved.getData('text/html')"),
        "0:"
    );
    assert_eq!(fixture.eval("e.innerHTML"), "<b>bold</b> tail");
    available.store(false, Ordering::Relaxed);
    assert!(!fixture.renderer.edit_command("paste"));
    fixture.eval("e.oncopy=event=>{event.preventDefault();event.clipboardData.setData('text/html','<i>only</i>')}");
    assert!(fixture.renderer.edit_command("copy"));
    assert_eq!(
        writes.lock().unwrap().last(),
        Some(&(None, Some("<i>only</i>".into())))
    );
}

#[test]
fn scripted_svg_icons_are_drawn() {
    let mut fixture = fixture(
        "<div id=host style='color: rgb(0, 128, 0)'></div>",
        "",
        &[
            "const ns = 'http://www.w3.org/2000/svg'; const svg = document.createElementNS(ns, 'svg');\
           for (const [name, value] of [['viewBox', '0 0 24 24'], ['width', '24'], ['height', '24'], ['fill', 'none'], ['stroke', 'currentColor'], ['stroke-width', '2']]) svg.setAttribute(name, value);\
           const path = document.createElementNS(ns, 'path'); path.setAttribute('d', 'M5 12h14'); svg.appendChild(path);\
           document.getElementById('host').appendChild(svg);",
        ],
    );
    fixture.renderer.render();
    let stroke = fixture
        .renderer
        .tree()
        .root
        .items
        .iter()
        .find_map(|item| match item {
            DisplayItem::Path {
                fill: None, stroke, ..
            } => stroke.clone(),
            _ => None,
        })
        .expect("the icon's path");
    assert_eq!(stroke.color, nativeboat_css::Color::rgb(0, 128, 0));
    assert_eq!(stroke.width, 2.0);
}

#[test]
fn canvases_draw_into_platform_bitmaps() {
    use crate::frame::PageView;
    use nativeboat_web::Page as _;
    let mut fixture = fixture(
        "<canvas id=c width=20 height=10></canvas><canvas id=d width=4 height=4></canvas><img id=logo src='assets/logo.png'>",
        "",
        &[],
    );
    fixture.eval(
        "const ctx = c.getContext('2d'); ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 10, 10);\
         d.getContext('2d').fillRect(0, 0, 1, 1); ctx.drawImage(d, 12, 0); ctx.drawImage(logo, 0, 0, 1, 1);\
         ctx.font = 'bold 12px serif'; ctx.fillText('x', 0, 10); ctx.putImageData(ctx.createImageData(1, 1), 19, 9);",
    );
    assert_eq!(
        fixture.eval("Array.from(ctx.getImageData(5, 5, 1, 1).data).join()"),
        "255,0,0,255"
    );
    assert_eq!(
        fixture.eval("Array.from(ctx.getImageData(12, 0, 1, 1).data).join()"),
        "0,0,0,255",
        "the other canvas"
    );
    assert!(
        fixture
            .eval("c.toDataURL()")
            .starts_with("data:image/png;base64,iVBOR")
    );
    assert!(
        fixture
            .eval("c.toDataURL('image/jpeg')")
            .starts_with("data:image/jpeg;base64,/9j/")
    );
    fixture.renderer.render();
    let canvas = fixture.node("#c");
    assert!(
        fixture
            .renderer
            .tree()
            .root
            .items
            .iter()
            .any(|item| matches!(
                item,
                DisplayItem::Canvas { version, .. } if *version > 0
            )),
        "canvas {canvas} is painted"
    );
    let view = PageView(fixture.renderer.frame.clone());
    let unknown = NodeId(9999);
    assert_eq!(view.canvas_read(unknown, (0, 0, 1, 1)), [0, 0, 0, 0]);
    view.canvas_write(unknown, (0, 0, 1, 1), &[1, 2, 3, 4]);
    assert_eq!(view.canvas_url(unknown, false), "data:,");
    view.canvas_draw(
        unknown,
        2,
        2,
        r#"[{"Image":{"source":{"Url":"missing.png"},"from":{"x":0,"y":0,"width":1,"height":1},"to":{"x":0,"y":0,"width":1,"height":1},"transform":[1,0,0,1,0,0],"alpha":1}},
           {"Image":{"source":{"Canvas":12345},"from":{"x":0,"y":0,"width":1,"height":1},"to":{"x":0,"y":0,"width":1,"height":1},"transform":[1,0,0,1,0,0],"alpha":1}}]"#,
    );
    view.canvas_draw(unknown, 2, 2, "not json");
    assert_eq!(view.canvas_read(unknown, (0, 0, 1, 1)), [0, 0, 0, 0]);
    assert_eq!(view.parse_color("red"), Some([255.0, 0.0, 0.0, 1.0]));
    assert_eq!(view.parse_color("currentcolor"), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(view.parse_color("red blue"), None);
    assert_eq!(view.parse_color("bogus"), None);
    assert_eq!(view.parse_font("bogus"), None);
    assert!(
        view.parse_font("italic 12px serif")
            .unwrap()
            .contains("\"italic\":true")
    );
    assert!(view.measure_text("not json", "abc")[0] > 0.0);
    assert_eq!(view.path_data("M0 0 Z"), r#"[{"Move":[0.0,0.0]},"Close"]"#);
    drop(view);
    drop(fixture);
}

#[test]
fn editable_content_takes_clicks_typing_and_caret_keys() {
    let mut fixture = fixture(
        "<div id=e contenteditable><p id=a>hello world</p><p id=b>xy</p><p id=c>longer line</p></div>\
         <p id=n>no select</p><p id=t>plain</p>",
        "body { margin: 0 } p { margin: 0; font: 10px/10px serif } #n { user-select: none }",
        &[],
    );
    let key = |fixture: &mut Fixture, key: &str, down: bool| {
        fixture.renderer.handle(&Event::Key {
            down,
            key: key.into(),
            code: String::new(),
            repeat: false,
            modifiers: Modifiers::default(),
            control: None,
        })
    };
    let press = |fixture: &mut Fixture, name: &str| {
        let taken = key(fixture, name, true);
        key(fixture, name, false);
        taken
    };
    let at =
        "(() => { const s = getSelection(); return s.anchorNode.data + ':' + s.anchorOffset; })()";
    assert!(!fixture.renderer.handle(&Event::Text("q".into())));
    fixture.click((12.0, 5.0), 1);
    assert_eq!(fixture.eval(at), "hello world:2");
    assert_eq!(fixture.eval("document.activeElement.id"), "e");
    assert!(!key(&mut fixture, "Z", true));
    assert!(fixture.renderer.handle(&Event::Text("Z".into())));
    key(&mut fixture, "Z", false);
    assert_eq!(fixture.eval(at), "heZllo world:3");
    let mut moves = Vec::new();
    for name in [
        "ArrowDown",
        "ArrowDown",
        "ArrowUp",
        "ArrowUp",
        "End",
        "Home",
        "ArrowUp",
    ] {
        assert!(press(&mut fixture, name));
        moves.push(fixture.eval(at));
    }
    assert_eq!(
        moves,
        [
            "xy:2",
            "longer line:3",
            "xy:2",
            "heZllo world:3",
            "heZllo world:12",
            "heZllo world:0",
            "heZllo world:0"
        ]
    );
    assert!(press(&mut fixture, "Delete"));
    assert_eq!(fixture.eval("a.textContent"), "eZllo world");
    let none = fixture.center("#n");
    fixture.click(none, 1);
    assert_eq!(
        fixture.eval(&format!("[{at}, document.activeElement.localName].join()")),
        "eZllo world:0,body"
    );
    assert!(!fixture.renderer.handle(&Event::Text("q".into())));
    fixture.click((12.0, 45.0), 2);
    assert_eq!(fixture.eval("String(getSelection())"), "plain");
}

#[test]
fn carets_and_selections_repaint_without_changing_the_page() {
    let mut fixture = fixture(
        "<div id=e contenteditable><p id=a>hello world</p></div>",
        "body { margin: 0 } p { margin: 0; font: 10px/10px serif }",
        &[],
    );
    let painted = |fixture: &mut Fixture| -> Vec<String> {
        let changes = fixture.renderer.render();
        let mut found = Vec::new();
        for change in &changes {
            if let Change::Paint { items, .. } = change {
                for item in items {
                    match item {
                        DisplayItem::Caret { rect, .. } => found.push(format!("caret {}", rect.x)),
                        DisplayItem::Fill { rect, .. } if rect.height == 10.0 => {
                            found.push(format!("fill {} {}", rect.x, rect.width))
                        }
                        _ => {}
                    }
                }
            }
        }
        found
    };
    painted(&mut fixture);
    fixture.click((12.0, 5.0), 1);
    assert_eq!(painted(&mut fixture), ["caret 10"]);
    assert!(painted(&mut fixture).is_empty());
    fixture.eval("getSelection().setBaseAndExtent(a.firstChild, 2, a.firstChild, 5)");
    assert_eq!(painted(&mut fixture), ["fill 10 15"]);
    fixture.eval("getSelection().collapse(a.firstChild, 4)");
    assert_eq!(painted(&mut fixture), ["caret 20"]);
}

#[test]
fn computed_declarations_read_native_properties_lazily_and_stay_live() {
    let mut fixture = fixture(
        "<p id=p>x</p><span id=hidden style='display:none'>hidden</span>",
        "body {--brand:green} p {color:red; width:10px}",
        &[],
    );
    assert_eq!(
        fixture.eval("globalThis.style=getComputedStyle(p);style.width"),
        "10px"
    );
    // Custom properties inherit and read like the others.
    assert_eq!(
        fixture.eval("style.getPropertyValue('--brand').trim()+[...style].includes('--brand')"),
        "greentrue"
    );
    fixture.eval("p.style.width='25px';p.style.color='blue'");
    assert_eq!(
        fixture.eval("style.width+':'+style.color"),
        "25px:rgb(0, 0, 255)"
    );
    assert_eq!(
        fixture.eval("style.getPropertyValue('unknown-property')"),
        ""
    );
    assert_eq!(fixture.eval("getComputedStyle(document).color"), "");
    assert_eq!(fixture.eval("getComputedStyle(hidden).display"), "none");
    assert_eq!(
        fixture.eval("style.cssText.includes('width: 25px;') && [...style].includes('width')"),
        "true"
    );
}

#[test]
fn computed_styles_resolve_line_heights_and_box_sizes_to_pixels() {
    // As Chromium resolves them: line heights in pixels, and the sizes,
    // margins and paddings of boxes as layout used them.
    let mut fixture = fixture(
        "<div style='width:200px'><div id=box style='width:50%;padding-left:10%;margin:0 auto'>\
         <span id=word>x</span></div><div id=gone style='display:none;width:50%'></div>\
         <div id=bordered style='box-sizing:border-box;width:60px;padding:5px;border:2px solid'></div></div>",
        "body{margin:0;font-size:20px;line-height:1.5} #gone{line-height:normal}",
        &[],
    );
    let style = |property: &str| {
        format!(
            "['box','word','gone','bordered'].map((id) => getComputedStyle(document.getElementById(id)).{property}).join()"
        )
    };
    assert_eq!(fixture.eval(&style("lineHeight")), "30px,30px,normal,30px");
    assert_eq!(fixture.eval(&style("width")), "100px,auto,50%,60px");
    assert_eq!(fixture.eval(&style("paddingLeft")), "20px,0px,0px,5px");
    assert_eq!(fixture.eval(&style("marginLeft")), "40px,0px,0px,0px");
    assert_eq!(
        fixture.eval(
            "getComputedStyle(box).margin + ';' + getComputedStyle(box).getPropertyValue('padding')"
        ),
        "0px 40px;0px 0px 0px 20px"
    );
}

#[test]
fn inner_text_follows_rendering() {
    let mut fixture = fixture(
        "<div id=a><p>a<b>b</b></p><p>c</p></div>\
         <div id=b>  x\n  <i>y</i> <span style='display:none'>z</span><br>w <div>v</div></div>\
         <table id=t><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>\
         <pre id=p>  a\n b</pre><div id=h style='display:none'>hidden <b>x</b></div>",
        "",
        &[],
    );
    fixture.renderer.render();
    assert_eq!(
        fixture.eval(
            "JSON.stringify([a.innerText, b.innerText, t.innerText, p.innerText, h.innerText])"
        ),
        r#"["ab\n\nc","x y\nw\nv","1\t2\n3\t4","  a\n b","hidden x"]"#
    );
    fixture.eval("a.innerText = 'one\\ntwo'");
    assert_eq!(fixture.eval("a.innerHTML"), "one<br>two");
}

#[test]
fn clicks_between_blocks_put_the_caret_in_the_block_below() {
    let mut fixture = fixture(
        "<div id=e contenteditable=true><div id=c><h1 id=h1><span contenteditable=true>Heading one</span></h1><p id=p><span contenteditable=true>Paragraph</span></p><h2 id=h2><span contenteditable=true>Heading two</span></h2></div></div>",
        "body{margin:0} h1{margin:20px 0;font-size:32px;line-height:42px} p{margin:0;line-height:26px} h2{margin:16px 0;line-height:33px}",
        &[],
    );
    fixture.renderer.render();
    // The paragraph ends at 108 and the heading starts at 124.
    let gap = fixture.eval("const p=document.getElementById('p').getBoundingClientRect(),h=document.getElementById('h2').getBoundingClientRect();JSON.stringify([p.bottom,h.top])");
    assert_eq!(gap, "[108,124]");
    fixture.click((50.0, 115.0), 1);
    assert_eq!(
        fixture.eval("const s=getSelection();JSON.stringify([s.anchorNode.textContent,document.activeElement.id])"),
        r#"["Heading two","e"]"#
    );
}

#[test]
fn the_root_element_measures_the_viewport() {
    let mut fixture = fixture(
        "<div id=d style='height:40px'></div>",
        "body{position:fixed;top:0;left:0;width:100vw;height:100vh;margin:0}",
        &[],
    );
    fixture.renderer.render();
    // The root holds nothing in flow, yet its client area is the viewport,
    // as floating-ui and Popper read it to keep popups on screen.
    assert_eq!(
        fixture.eval("const e=document.documentElement;JSON.stringify([e.clientWidth,e.clientHeight,e.scrollWidth,e.scrollHeight,e.getBoundingClientRect().height,visualViewport.width,visualViewport.height,visualViewport.scale,visualViewport instanceof VisualViewport])"),
        "[400,300,400,300,0,400,300,1,true]"
    );
}

#[test]
fn focused_text_fields_take_the_selection_from_the_page() {
    let mut fixture = fixture(
        "<div id=bar><input id=i></div><div id=e contenteditable=true><p id=p>Some text</p></div>",
        "",
        &[],
    );
    fixture.renderer.render();
    // As in Chromium, the field's selection is the frame's: the document's
    // collapses beside the field, and editing the editor's content then
    // leaves the focus alone.
    assert_eq!(
        fixture.eval("const t=document.getElementById('p').firstChild;getSelection().collapse(t,4);document.getElementById('i').focus();const s=getSelection();JSON.stringify([s.anchorNode.id,s.anchorOffset,s.isCollapsed,document.activeElement.id])"),
        r#"["bar",0,true,"i"]"#
    );
    assert_eq!(
        fixture.eval("document.getElementById('p').innerHTML='<span>Some</span> text';document.activeElement.id"),
        "i"
    );
}

#[test]
fn selections_set_where_they_are_fire_no_selectionchange() {
    let mut fixture = fixture(
        "<input id=i><div id=e contenteditable=true><p id=p>Some text</p></div>",
        "",
        &[],
    );
    fixture.renderer.render();
    fixture.eval("globalThis.changes=0;document.addEventListener('selectionchange',()=>changes++);const t=document.getElementById('p').firstChild;getSelection().collapse(t,4);''");
    fixture.renderer.tick();
    assert_eq!(fixture.eval("String(changes)"), "1");
    fixture.eval("const u=document.getElementById('p').firstChild;getSelection().collapse(u,4);''");
    fixture.renderer.tick();
    assert_eq!(
        fixture.eval("JSON.stringify([changes,document.activeElement.id])"),
        r#"[1,"e"]"#
    );
}

#[test]
fn clicks_between_muya_blocks_put_the_caret_in_the_block_below() {
    let mut fixture = fixture(
        "<div id=e contenteditable=true><div id=c>\
         <p id=p><span class=content contenteditable=true><span>Paragraph</span></span></p>\
         <h2 id=h2><i class=icon contenteditable=false tabindex=0><img alt=''></i><span class=content contenteditable=true><span class=hide>##</span><span> </span><span>Heading two</span></span></h2>\
         </div></div>",
        "body{margin:0} #c{margin-left:100px} p{margin:8px 0;line-height:26px} h2{position:relative;margin:16px 0;line-height:33px}\
         .content{display:block} .hide{display:inline-block;width:0;overflow:hidden;vertical-align:bottom}\
         .icon{position:absolute;left:-40px;top:0;display:flex;opacity:0;width:18px;height:18px} img{width:18px;height:18px}",
        &[],
    );
    fixture.renderer.render();
    let gap = fixture.eval("const p=document.getElementById('p').getBoundingClientRect(),h=document.getElementById('h2').getBoundingClientRect();JSON.stringify([p.bottom,h.top])");
    assert_eq!(gap, "[34,50]");
    fixture.click((300.0, 42.0), 1);
    assert_eq!(
        fixture.eval("const s=getSelection();JSON.stringify([s.anchorNode.textContent,s.anchorOffset,document.activeElement.id])"),
        r#"["Heading two",11,"e"]"#
    );
}

#[test]
fn scripted_scrolls_fire_scroll_events_at_the_next_frame() {
    let mut fixture = fixture(
        "<div id=s style='height:50px;overflow:auto'><div style='height:500px'></div></div><div style='height:900px'></div>",
        "body{margin:0}",
        &[
            "globalThis.log=[];document.getElementById('s').addEventListener('scroll',()=>log.push('s'));\
           window.addEventListener('scroll',()=>log.push('window'));",
        ],
    );
    fixture.renderer.render();
    fixture.eval("const s=document.getElementById('s');s.scrollTop=100;s.scrollTop=120;document.documentElement.scrollTop=40;s.scrollTop=120;''");
    // Not while the script runs, and once per element.
    assert_eq!(fixture.eval("JSON.stringify(log)"), "[]");
    std::thread::sleep(FRAME);
    fixture.renderer.tick();
    assert_eq!(fixture.eval("JSON.stringify(log)"), r#"["s","window"]"#);
    std::thread::sleep(FRAME);
    fixture.renderer.tick();
    assert_eq!(fixture.eval("JSON.stringify(log)"), r#"["s","window"]"#);
}

#[test]
fn scroll_offsets_stay_within_the_scroll_range() {
    let mut fixture = fixture(
        "<div id=s style='width:100px;height:50px;overflow:auto'><div style='width:300px;height:500px'></div></div>",
        "body{margin:0}",
        &[],
    );
    fixture.renderer.render();
    assert_eq!(
        fixture.eval("const s=document.getElementById('s');s.scrollTop=99999;s.scrollLeft=-5;const a=[s.scrollTop,s.scrollLeft];s.scrollTo(1000,10);JSON.stringify(a.concat([s.scrollTop,s.scrollLeft]))"),
        // Both scrollbars take 17 pixels of the 100 by 50 box.
        "[467,0,10,217]"
    );
}

#[test]
fn style_sheet_links_report_load_once_their_href_is_set() {
    let mut fixture = fixture("<p>x</p>", "", &[]);
    fixture.renderer.render();
    fixture.eval("globalThis.loads=[];const a=document.createElement('link');a.rel='stylesheet';a.onload=()=>loads.push('sheet');                  a.href='theme.css';document.head.appendChild(a);const b=document.createElement('link');b.rel='icon';                  b.onload=()=>loads.push('icon');b.href='x.ico';''");
    assert_eq!(fixture.eval("JSON.stringify(loads)"), "[]");
    std::thread::sleep(Duration::from_millis(2));
    fixture.renderer.tick();
    assert_eq!(fixture.eval("JSON.stringify(loads)"), r#"["sheet"]"#);
}

#[test]
fn clicks_on_a_heading_marker_space_pulled_left_go_after_it() {
    // Muya's heading: `##` hidden at font size 0, then a space pulled left
    // by -0.3em, so that a click just left of the text lands on its end.
    let mut fixture = fixture(
        concat!(
            "<div id=e contenteditable=true><h2 id=h><span class=content contenteditable=true>",
            "<span class=hide>##</span><span class=space> </span><span id=t>Heading two</span></span></h2></div>",
        ),
        concat!(
            "body{margin:0} #e{padding-left:100px} h2{margin:0;font-size:24px;line-height:30px} ",
            ".content{display:block;white-space:pre-wrap} .hide{font-size:0;line-height:0;white-space:nowrap} ",
            ".space{margin-left:-0.3em}",
        ),
        &[],
    );
    fixture.renderer.render();
    assert_eq!(
        fixture.eval(
            "String(Math.round(document.getElementById('t').getBoundingClientRect().x*10)/10)"
        ),
        "104.8"
    );
    fixture.click((102.0, 15.0), 1);
    assert_eq!(
        fixture.eval(
            "const s=getSelection();JSON.stringify([s.anchorNode.textContent,s.anchorOffset])"
        ),
        r#"[" ",1]"#
    );
}

#[test]
fn carets_beside_line_breaks_have_rects_and_between_them_none() {
    // As Chromium reports them: at the end of a line's text and at the
    // start of the next line's, but not on the empty line between.
    let mut fixture = fixture(
        concat!(
            "<div id=s style='height:200px;overflow:auto'><div style='height:150px'></div>",
            "<pre id=p style='overflow:auto;margin:0;font-size:10px;line-height:20px'>&lt;div&gt;\n\n&lt;/div&gt;</pre>",
            "<div style='height:600px'></div></div>",
        ),
        "body{margin:0}",
        &[],
    );
    fixture.renderer.render();
    fixture.eval("document.getElementById('s').scrollTop=100;''");
    fixture.renderer.render();
    assert_eq!(
        fixture.eval(concat!(
            "const t=document.getElementById('p').firstChild;",
            "JSON.stringify([5,6,7].map(o=>{const r=document.createRange();r.setStart(t,o);r.collapse(true);",
            "return [...r.getClientRects()].map(x=>[x.x,x.y,x.height])}))"
        )),
        "[[[25,55,10]],[],[[0,95,10]]]"
    );
}

#[test]
fn content_scrolled_under_a_still_pointer_gets_boundary_events_but_no_move() {
    // Chromium updates the hover when content scrolls under a still
    // pointer, with over, out, enter and leave events, but sends no move
    // (measured in Electron).
    let mut fixture = fixture(
        "<div id=s style='height:100px;overflow:auto'><div id=a style='height:100px'></div><div id=b style='height:300px'></div></div>",
        "body{margin:0}",
        &[
            "globalThis.moves=[];for(const type of ['mousemove','pointermove','mouseover','mouseout'])document.addEventListener(type,e=>moves.push(type+':'+e.target.id));",
        ],
    );
    fixture.renderer.render();
    fixture.renderer.handle(&Event::Pointer {
        kind: PointerKind::Move,
        button: 0,
        buttons: 0,
        count: 0,
        x: 10.0,
        y: 50.0,
        modifiers: Modifiers::default(),
    });
    fixture.eval("document.getElementById('s').scrollTop=100;''");
    std::thread::sleep(FRAME);
    fixture.renderer.tick();
    assert_eq!(
        fixture.eval("moves.join()"),
        "mouseover:a,pointermove:a,mousemove:a,mouseout:a,mouseover:b"
    );
}

#[test]
fn carets_in_line_breaks_between_highlighted_lines_have_no_rects() {
    let mut fixture = fixture(
        concat!(
            "<div id=e contenteditable=true style='height:300px;overflow:auto'><div style='height:100px'></div>",
            "<figure style='margin:0'><div contenteditable=false style='display:none'>x</div>",
            "<pre style='white-space:pre;padding:14px;margin:0'><code style='font-size:14px;line-height:23px'>",
            "<a contenteditable=false style='position:absolute;right:0'><i>c</i></a>",
            "<span id=c contenteditable=true><span><span><span>&lt;</span>div</span><span>&gt;</span></span>\n\n",
            "<span><span><span>&lt;/</span>div</span><span>&gt;</span></span></span></code></pre></figure>",
            "<div style='height:900px'></div></div>",
        ),
        "body{margin:0}",
        &[],
    );
    fixture.renderer.render();
    // Muya's HTML block: highlighted tokens with a text node of line breaks
    // between them. Chromium gives carets in it no rectangles (measured).
    assert_eq!(
        fixture.eval(concat!(
            "const c=document.getElementById('c');const texts=[];const w=document.createTreeWalker(c,4);while(w.nextNode())texts.push(w.currentNode);",
            "const t=texts.find(t=>t.data===String.fromCharCode(10,10));JSON.stringify([0,1,2].map(o=>{const r=document.createRange();r.setStart(t,o);r.collapse(true);",
            "return [...r.getClientRects()].map(x=>[Math.round(x.x),Math.round(x.y),Math.round(x.height)])}))"
        )),
        "[[],[],[]]"
    );
}

#[test]
fn carets_between_children_have_no_rectangles() {
    let mut fixture = fixture(
        concat!(
            "<div id=e contenteditable=true style='height:300px;overflow:auto'><div style='height:100px'></div>",
            "<pre style='white-space:pre;padding:14px;margin:0'><code style='font-size:14px;line-height:23px'>",
            "<span id=c contenteditable=true><span><span><span>&lt;</span>div</span><span>&gt;</span></span>\n\n",
            "<span><span><span>&lt;/</span>div</span><span>&gt;</span></span></span></code></pre>",
            "<div style='height:900px'></div></div>",
        ),
        "body{margin:0}",
        &[],
    );
    fixture.renderer.render();
    // A caret between an element's children covers no text: Chromium gives
    // it no rectangles, wherever it is.
    assert_eq!(
        fixture.eval(concat!(
            "const c=document.getElementById('c');",
            "JSON.stringify([0,1,2].map(o=>{const r=document.createRange();r.setStart(c,o);r.collapse(true);",
            "return [...r.getClientRects()].map(x=>[Math.round(x.x),Math.round(x.y),Math.round(x.height)])}))"
        )),
        "[[],[],[]]"
    );
    // An empty element has no caret rectangle, as in Chromium.
    assert_eq!(
        fixture.eval("const r=document.createRange();r.setStart(document.querySelector('pre').nextElementSibling,0);String(r.getClientRects().length)"),
        "0"
    );
}

#[test]
fn home_and_end_take_in_text_without_width_at_the_line_edges() {
    // Muya's heading: markers hidden at font-size 0 before and after the
    // visible text. Chromium's line start is before them, its end after.
    let mut fixture = fixture(
        "<div id=e contenteditable><h2 id=h><span class=m>## </span>Heading<span class=m> ##</span></h2></div>",
        "body { margin: 0 } h2 { margin: 0; font: 10px/10px serif } .m { font-size: 0 }",
        &[],
    );
    let key = |fixture: &mut Fixture, key: &str| {
        for down in [true, false] {
            fixture.renderer.handle(&Event::Key {
                down,
                key: key.into(),
                code: String::new(),
                repeat: false,
                modifiers: Modifiers::default(),
                control: None,
            });
        }
    };
    let at =
        "(() => { const s = getSelection(); return s.anchorNode.data + ':' + s.anchorOffset; })()";
    fixture.click((20.0, 5.0), 1);
    key(&mut fixture, "Home");
    assert_eq!(fixture.eval(at), "## :0");
    key(&mut fixture, "End");
    assert_eq!(fixture.eval(at), " ##:3");
    // Muya's active heading: the space after the hidden marker is pulled
    // left over it by a negative margin; the line still starts before it.
    fixture.eval(
        "document.getElementById('h').innerHTML = '<span class=m>##</span><span style=\"margin-left:-0.3em\"> </span>Heading'",
    );
    fixture.renderer.render();
    fixture.click((30.0, 5.0), 1);
    key(&mut fixture, "Home");
    assert_eq!(fixture.eval(at), "##:0");
}

#[test]
fn text_dragged_from_other_applications_drops_into_editable_content() {
    let mut fixture = fixture(
        "<div id=editor contenteditable>ab</div><div id=plain>c</div>",
        "#editor,#plain{height:40px}",
        &[r#"
        globalThis.log=[];
        editor.addEventListener('beforeinput',e=>log.push(e.inputType+':'+e.dataTransfer.getData('text/plain')));
        "#],
    );
    let strings = vec![("text/plain".to_owned(), "XY".to_owned())];
    let drag = |kind, (x, y): (f32, f32)| Event::FileDrag {
        kind,
        paths: Vec::new(),
        strings: strings.clone(),
        x,
        y,
        modifiers: Modifiers::default(),
    };
    // Content that is not editable takes no text unless the page wants it.
    let plain = fixture.center("#plain");
    assert!(
        !fixture
            .renderer
            .handle(&drag(nativeboat_win32::FileDragKind::Enter, plain))
    );
    let editor = fixture.center("#editor");
    assert!(
        fixture
            .renderer
            .handle(&drag(nativeboat_win32::FileDragKind::Over, editor))
    );
    assert!(fixture.renderer.handle(&Event::FilesDropped {
        paths: Vec::new(),
        strings: strings.clone(),
        x: editor.0,
        y: editor.1,
    }));
    assert_eq!(fixture.eval("editor.textContent"), "abXY");
    assert_eq!(fixture.eval("log.join()"), "insertFromDrop:XY");
}

#[test]
fn selected_text_drags_within_editable_content_and_a_press_without_a_drag_collapses_it() {
    let mut fixture = fixture(
        "<div id=editor contenteditable><span id=one>hello</span> <span id=two>world</span></div>",
        "#editor{height:40px}",
        &[r#"
        globalThis.log=[];
        editor.addEventListener('beforeinput',e=>log.push(e.inputType));
        document.addEventListener('dragstart',e=>log.push('dragstart:'+e.dataTransfer.getData('text/plain')));
        globalThis.select=()=>{const t=two.firstChild;getSelection().setBaseAndExtent(t,0,t,5)};
        select();
        "#],
    );
    let world = fixture.center("#two");
    let hello = fixture.eval(
        "(() => { const r = one.getBoundingClientRect(); return (r.x + 1) + ',' + (r.y + r.height / 2); })()",
    );
    let (x, y) = hello.split_once(',').unwrap();
    let hello = (x.parse().unwrap(), y.parse().unwrap());
    // A press and release on the selection puts the caret there.
    fixture.pointer(PointerKind::Down, world, 1);
    assert_eq!(fixture.eval("String(getSelection())"), "world");
    fixture.pointer(PointerKind::Up, world, 1);
    assert_eq!(fixture.eval("getSelection().isCollapsed"), "true");
    // Dragged before "hello", the selection moves there and stays selected.
    fixture.eval("select()");
    fixture.pointer(PointerKind::Down, world, 1);
    fixture.pointer(PointerKind::Move, hello, 1);
    fixture.pointer(PointerKind::Up, hello, 1);
    assert_eq!(fixture.eval("editor.textContent"), "worldhello ");
    assert_eq!(fixture.eval("String(getSelection())"), "world");
    assert_eq!(
        fixture.eval("log.join()"),
        "dragstart:world,deleteByDrag,insertFromDrop"
    );
}
