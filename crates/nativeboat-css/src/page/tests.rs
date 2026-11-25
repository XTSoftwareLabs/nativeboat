use super::*;
use crate::parser::parse_stylesheet;

fn margins(css: &str, print: bool) -> [Option<f32>; 4] {
    let sheet = parse_stylesheet(css).0;
    let environment = MediaEnvironment {
        print,
        ..MediaEnvironment::default()
    };
    page_margins(&[&sheet], &environment)
}

#[test]
fn page_rules_give_printed_pages_their_margins() {
    let mm = |value: f32| value * 96.0 / 25.4;
    assert_eq!(
        margins(
            "@media print { @page { margin: 15mm 20mm 15mm 20mm; } }",
            true
        ),
        [
            Some(mm(15.0)),
            Some(mm(20.0)),
            Some(mm(15.0)),
            Some(mm(20.0))
        ]
    );
    // Not when the media does not match.
    assert_eq!(
        margins("@media print { @page { margin: 1in } }", false),
        [None; 4]
    );
    assert_eq!(margins("@page { margin: 1in }", false), [Some(96.0); 4]);
    assert_eq!(
        margins(
            "@page { margin: 1in 2in } @page { margin-left: 10px }",
            true
        ),
        [Some(96.0), Some(192.0), Some(96.0), Some(10.0)]
    );
    assert_eq!(
        margins("@page { margin: 1in 2in 3in }", true),
        [Some(96.0), Some(192.0), Some(288.0), Some(192.0)]
    );
    assert_eq!(
        margins(
            "@page { margin-top: 0; margin-right: 1cm; margin-bottom: 72pt; }",
            true
        ),
        [Some(0.0), Some(96.0 / 2.54), Some(96.0), None]
    );
    // Pages other than every page, and values that are not lengths.
    assert_eq!(margins("@page :first { margin: 1in }", true), [None; 4]);
    assert_eq!(margins("@page { margin: auto }", true), [None; 4]);
    assert_eq!(
        margins("@page { margin: 1in 1in 1in 1in 1in }", true),
        [None; 4]
    );
    assert_eq!(
        margins("@page { margin-top: 1in 2in; color: red }", true),
        [None; 4]
    );
    assert_eq!(
        margins(
            "@supports (display: grid) { @layer base { @page { margin: 2px } } }",
            true
        ),
        [Some(2.0); 4]
    );
}
