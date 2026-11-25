use super::fixture;
use std::rc::Rc;

#[test]
fn native_queries_use_submitted_geometry_without_running_scripts_or_layout() {
    let mut fixture = fixture(
        "<input id=field><span id=option>Case</span>",
        "body{margin:0}input{position:absolute;left:20px;top:30px;width:200px;height:30px}span{position:absolute;left:180px;top:30px;width:20px;height:30px;z-index:1}",
        &["globalThis.calls=0;queueMicrotask(()=>calls++)"],
    );
    let field = fixture.node("#field");
    let probe = fixture.renderer.native_hit_test();
    let alias = probe.clone();
    assert!(format!("{probe:?}").starts_with("NativeHitTest"));
    assert!(!probe.above_control(field, 185.0, 40.0));
    fixture.renderer.render();
    assert!(probe.above_control(field, 185.0, 40.0));
    assert!(alias.above_control(field, 185.0, 40.0));
    assert!(!probe.above_control(field, 40.0, 40.0));
    assert!(!probe.above_control(field, -1.0, -1.0));
    assert!(!probe.above_control(field, f32::NAN, 40.0));
    assert!(!probe.above_control(u64::MAX, 185.0, 40.0));
    assert!(!probe.above_control(123456, 185.0, 40.0));
    let scene = fixture.renderer.hits.borrow().scene.clone().unwrap();
    assert!(Rc::ptr_eq(
        &scene,
        &fixture
            .renderer
            .frame
            .borrow()
            .rendered
            .as_ref()
            .unwrap()
            .scene
    ));
    // A geometry read can compute a newer frame without submitting it. Windows
    // still hit-tests the displayed frame until the native tree is updated.
    fixture.eval("option.style.display='none';option.getBoundingClientRect();");
    assert!(probe.above_control(field, 185.0, 40.0));
    fixture.renderer.render();
    assert!(!probe.above_control(field, 185.0, 40.0));
    fixture.eval("option.style.display='';");
    fixture.renderer.render();
    assert!(probe.above_control(field, 185.0, 40.0));
    let calls = fixture.eval("calls");
    for _ in 0..10 {
        assert!(probe.above_control(field, 185.0, 40.0));
    }
    assert_eq!(fixture.eval("calls"), calls);
    assert!(Rc::ptr_eq(
        fixture.renderer.hits.borrow().scene.as_ref().unwrap(),
        &fixture
            .renderer
            .frame
            .borrow()
            .rendered
            .as_ref()
            .unwrap()
            .scene
    ));
    fixture.renderer.clear();
    assert!(!probe.above_control(field, 185.0, 40.0));
    fixture.renderer.render();
    fixture.eval("field.remove()");
    assert!(!probe.above_control(field, 185.0, 40.0));
    drop(fixture);
    assert!(!probe.above_control(field, 185.0, 40.0));
    assert!(!alias.above_control(field, 185.0, 40.0));
}

#[test]
fn native_queries_preserve_ownership_and_reentry_defaults() {
    let mut fixture = fixture(
        "<button id=button><span id=caption>Go</span></button><input id=field><span id=option>Case</span>",
        "body{margin:0}button{appearance:none;position:absolute;left:0;top:0;width:100px;height:30px}input{position:absolute;left:20px;top:50px;width:200px;height:30px}input::after{content:'';position:absolute;left:150px;top:0;width:20px;height:20px;background:red}#option{position:absolute;left:180px;top:50px;width:20px;height:30px;z-index:1}",
        &[],
    );
    fixture.renderer.render();
    let field = fixture.node("#field");
    let button = fixture.node("#button");
    let probe = fixture.renderer.native_hit_test();
    assert!(!probe.above_control(button, 50.0, 15.0));
    assert!(!probe.above_control(field, 175.0, 55.0));
    assert!(probe.above_control(field, 185.0, 60.0));
    {
        let _borrow = fixture.renderer.hits.borrow_mut();
        assert!(!probe.above_control(field, 185.0, 60.0));
    }
    {
        let dom = fixture.renderer.dom();
        let _borrow = dom.borrow_mut();
        assert!(!probe.above_control(field, 185.0, 60.0));
    }
    assert!(probe.above_control(field, 185.0, 60.0));
}

#[test]
fn native_queries_share_transformed_clipped_and_scrolled_scene_hits() {
    let mut fixture = fixture(
        "<div id=clip><input id=field><span id=option>Case</span><div style='height:300px'></div></div>",
        "body{margin:0}#clip{position:absolute;left:40px;top:30px;width:200px;height:100px;overflow:auto}input{position:absolute;left:10px;top:10px;width:180px;height:30px}span{position:absolute;left:140px;top:10px;width:40px;height:30px;transform:translate(20px,0);z-index:1}",
        &[],
    );
    fixture.renderer.render();
    let field = fixture.node("#field");
    let probe = fixture.renderer.native_hit_test();
    assert!(!probe.above_control(field, 185.0, 50.0));
    assert!(probe.above_control(field, 205.0, 50.0));
    fixture.eval("option.style.width='100px';clip.scrollTop=20;");
    fixture.renderer.render();
    assert!(probe.above_control(field, 205.0, 35.0));
    assert!(!probe.above_control(field, 205.0, 25.0));
    assert!(!probe.above_control(field, 245.0, 35.0));
}
