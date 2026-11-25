use super::*;
use nativeboat_css::Color;
use nativeboat_ir::{Corner, DisplayItem, Font, Rect, Surface};

#[test]
fn standard_papers_have_windows_codes() {
    assert_eq!(paper_code((210.0, 297.0)), Some(9));
    assert_eq!(paper_code((215.9, 279.4)), Some(1));
    assert_eq!(paper_code((210.5, 296.6)), Some(9));
    assert_eq!(paper_code((100.0, 100.0)), None);
}

fn document() -> NativeTree {
    NativeTree {
        root: Surface {
            rect: Rect::new(0.0, 0.0, 600.0, 600.0),
            content_width: 600.0,
            content_height: 1600.0,
            items: vec![
                DisplayItem::Fill {
                    rect: Rect::new(0.0, 0.0, 100.0, 40.0),
                    color: Color::rgb(200, 0, 0),
                    radii: [Corner::SQUARE; 4],
                },
                DisplayItem::Text {
                    rect: Rect::new(0.0, 900.0, 300.0, 20.0),
                    text: "Printed text".into(),
                    font: Font::default(),
                    color: Color::rgb(0, 0, 0),
                    underline: false,
                    line_through: false,
                    letter_spacing: 0.0,
                    rtl: false,
                    grayscale: false,
                    underline_offset: None,
                },
            ],
            ..Surface::default()
        },
    }
}

#[test]
fn documents_print_to_pdf_files_through_the_pdf_printer() {
    crate::init().unwrap();
    let paper = Paper {
        size: Some((210.0, 297.0)),
        landscape: false,
    };
    let Ok(Some(job)) = open_printer(Printer::Named("Microsoft Print to PDF"), paper) else {
        // Windows images without the printer cannot run this.
        return;
    };
    // A4 is 793.7 by 1122.5 CSS pixels.
    assert!(
        (job.paper.0 - 793.7).abs() < 2.0 && (job.paper.1 - 1122.5).abs() < 2.0,
        "{:?}",
        job.paper
    );
    let output = std::env::temp_dir().join(format!("nativeboat-print-{}.pdf", std::process::id()));
    let _ = std::fs::remove_file(&output);
    job.print(
        "Native print test",
        Some(output.to_str().unwrap()),
        &document(),
        &[0.0, 800.0],
        (700.0, 1000.0),
        [40.0; 4],
    )
    .unwrap();
    let pdf = std::fs::read(&output).unwrap();
    assert!(pdf.starts_with(b"%PDF"), "{:?}", &pdf[..8.min(pdf.len())]);
    // Two pages.
    let pages = pdf
        .windows(10)
        .filter(|window| window == b"/Type /Pag")
        .count();
    assert!(pages >= 2, "{pages}");
    let _ = std::fs::remove_file(&output);
    // A landscape page turns the paper.
    let turned = Paper {
        landscape: true,
        ..paper
    };
    if let Ok(Some(job)) = open_printer(Printer::Named("Microsoft Print to PDF"), turned) {
        assert!(job.paper.0 > job.paper.1, "{:?}", job.paper);
        // An output nobody can write fails the document.
        assert!(
            job.print(
                "x",
                Some("Z:\\no\\such\\dir\\out.pdf"),
                &document(),
                &[0.0],
                (700.0, 1000.0),
                [0.0; 4]
            )
            .is_err()
        );
    }
}

#[test]
fn unknown_printers_cannot_be_opened() {
    crate::init().unwrap();
    let error =
        open_printer(Printer::Named("No Such Printer 12345"), Paper::default()).unwrap_err();
    assert!(error.message.contains("open the printer"), "{error:?}");
    assert!(settings(&wide("No Such Printer 12345"), Paper::default()).is_none());
}
