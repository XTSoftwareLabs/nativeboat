use super::*;
use crate::ops;

fn text(realm: &mut Realm, object: &Value, name: &str) -> String {
    let value = ops::get(realm, object, &Key::name(name)).unwrap();
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

fn location(realm: &mut Realm) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name("location")).unwrap()
}

#[test]
fn file_pages_expose_their_query_and_path() {
    let mut realm = Realm::new();
    let document = realm.object();
    realm.set_global("document", Value::Object(document.clone()));
    realm.set_renderer_location("file:///C:/app/out/renderer/index.html?wid=1&type=editor#top");
    let location = location(&mut realm);
    assert_eq!(
        text(&mut realm, &location, "href"),
        "file:///C:/app/out/renderer/index.html?wid=1&type=editor#top"
    );
    assert_eq!(text(&mut realm, &location, "origin"), "file://");
    assert_eq!(text(&mut realm, &location, "protocol"), "file:");
    assert_eq!(text(&mut realm, &location, "host"), "");
    assert_eq!(
        text(&mut realm, &location, "pathname"),
        "/C:/app/out/renderer/index.html"
    );
    assert_eq!(text(&mut realm, &location, "search"), "?wid=1&type=editor");
    assert_eq!(text(&mut realm, &location, "hash"), "#top");
    let to_string = ops::get(&mut realm, &location, &Key::name("toString")).unwrap();
    let href = ops::call(&mut realm, &to_string, location.clone(), &[]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &href).unwrap().to_rust_lossy(),
        text(&mut realm, &location, "href")
    );
    let from_document =
        ops::get(&mut realm, &Value::Object(document), &Key::name("location")).unwrap();
    assert!(ops::strict_equals(&from_document, &location));
}

#[test]
fn file_paths_and_web_urls_have_browser_components() {
    let mut realm = Realm::new();
    realm.set_renderer_location(r"C:\app\out\renderer\index.html");
    let location = location(&mut realm);
    assert_eq!(
        text(&mut realm, &location, "href"),
        "file:///C:/app/out/renderer/index.html"
    );
    assert_eq!(text(&mut realm, &location, "search"), "");
    assert_eq!(text(&mut realm, &location, "hash"), "");

    realm.set_renderer_location("http://localhost:5173/index.html?x=1");
    let location = self::location(&mut realm);
    assert_eq!(
        text(&mut realm, &location, "origin"),
        "http://localhost:5173"
    );
    assert_eq!(text(&mut realm, &location, "host"), "localhost:5173");
    assert_eq!(text(&mut realm, &location, "hostname"), "localhost");
    assert_eq!(text(&mut realm, &location, "port"), "5173");

    realm.set_renderer_location("https://example.com/");
    let location = self::location(&mut realm);
    assert_eq!(text(&mut realm, &location, "host"), "example.com");
    assert_eq!(text(&mut realm, &location, "port"), "");

    // An empty target is the file root, as a path.
    realm.set_renderer_location("");
    let location = self::location(&mut realm);
    assert_eq!(text(&mut realm, &location, "href"), "file:///");
}
