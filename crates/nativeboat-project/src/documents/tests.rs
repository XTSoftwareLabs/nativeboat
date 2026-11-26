use super::*;
use crate::tests::fixture::Fixture;

#[test]
fn frame_documents_come_from_page_iframes_and_the_documents_setting() {
    let fixture = Fixture::new(&[
        (
            "src/index.html",
            "<iframe src='frame.html?x=1'></iframe><iframe src='https://example.com/a.html'></iframe>\
             <iframe src='missing.html'></iframe><iframe src='index.html'></iframe>",
        ),
        ("src/frame.html", "<p>frame</p>"),
        ("src/pre/index.html", "<script>1</script>"),
        ("src/pre/fake.html", ""),
        ("src/pre/other.txt", ""),
        ("src/pre/deep/skip.html", ""),
        ("node_modules/pkg/pre/index.html", ""),
    ]);
    let root = fixture.root().to_path_buf();
    let mut pages = vec![Entry {
        source: fixture.path("src/index.html"),
        runtime_paths: vec!["src/index.html".to_owned()],
    }];
    let patterns = ["src/pre/*.html".to_owned(), "!src/pre/fake.html".to_owned()];
    add_frames(&root, &mut pages, Some(&patterns));
    let found: Vec<_> = pages
        .iter()
        .map(|page| page.runtime_paths.clone())
        .collect();
    // Remote, missing and already compiled documents are not added again.
    assert_eq!(
        found,
        vec![
            vec!["src/index.html".to_owned()],
            vec!["src/frame.html".to_owned()],
            vec!["src/pre/index.html".to_owned()],
        ]
    );
    let everything = files(&root, &["**/*.html".to_owned()]);
    assert_eq!(everything.len(), 5, "{everything:?}");
}
