//! PDF crop / annotate / redact, checked by rendering results with Windows' PDF engine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use image::{DynamicImage, Rgb, RgbImage};
use shiftdrop_core::pdf::{self, Page, PageImage};
use shiftdrop_core::pdf_edit::{self, Rect, RedactMode, Redaction, Shape, ShapeKind};
use shiftdrop_core::wic;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "shiftdrop-pe-{name}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d.join("doc.pdf")
}

/// A one-page PDF, 400 x 300 px at 96 dpi (300 x 225 pt): left half white, right half blue.
fn sample(rotate: Option<i64>) -> Vec<u8> {
    let img = RgbImage::from_fn(400, 300, |x, _| if x < 200 { Rgb([255, 255, 255]) } else { Rgb([30, 60, 200]) });
    let bytes = pdf::write_images(&[Page { image: PageImage::Pixels(DynamicImage::ImageRgb8(img)), dpi: 96.0 }]).unwrap();
    match rotate {
        None => bytes,
        Some(r) => {
            let mut doc = lopdf::Document::load_mem(&bytes).unwrap();
            let (_, id) = doc.get_pages().into_iter().next().unwrap();
            doc.get_dictionary_mut(id).unwrap().set("Rotate", r);
            let mut out = Vec::new();
            doc.save_to(&mut out).unwrap();
            out
        }
    }
}

fn render(bytes: &[u8], name: &str) -> image::RgbImage {
    wic::init_thread();
    let p = tmp(name);
    std::fs::write(&p, bytes).unwrap();
    pdf::open(&p).unwrap().render(0, 96.0).unwrap().to_rgb8()
}

#[test]
fn crop_right_half_keeps_only_blue() {
    let out = pdf_edit::crop(&sample(None), Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }, None).unwrap();
    let img = render(&out, "crop");
    assert_eq!(img.dimensions(), (200, 300));
    assert_eq!(img.get_pixel(100, 150)[2] > 150, true, "blue side kept");
}

#[test]
fn crop_on_rotated_page_uses_displayed_orientation() {
    // Rotated 90: displayed 300 x 400 px. Crop the displayed top half.
    let src = sample(Some(90));
    assert_eq!(render(&src, "rot-src").dimensions(), (300, 400));
    let out = pdf_edit::crop(&src, Rect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }, None).unwrap();
    let img = render(&out, "rot-crop");
    assert_eq!(img.dimensions(), (300, 200));
}

fn red_box() -> Shape {
    Shape {
        kind: ShapeKind::Rect,
        color: "#ff0000".into(),
        width: 0.02,
        points: vec![[0.1, 0.1], [0.4, 0.9]],
        text: String::new(),
        size: 0.0,
        n: 0,
    }
}

#[test]
fn annotate_draws_vector_shapes_where_the_editor_put_them() {
    for rot in [None, Some(90), Some(180), Some(270)] {
        let mut shapes = BTreeMap::new();
        let mut text = red_box();
        text.kind = ShapeKind::Text;
        text.text = "Hello (world)".into();
        text.size = 0.08;
        text.points = vec![[0.55, 0.05]];
        shapes.insert(0, vec![red_box(), text]);
        let out = pdf_edit::annotate(&sample(rot), &shapes).unwrap();
        let img = render(&out, "annot");
        let (w, h) = img.dimensions();
        // Left edge of the rectangle: 10% across, half way down the displayed page.
        let p = img.get_pixel((w as f64 * 0.1) as u32, h / 2);
        assert!(p[0] > 180 && p[1] < 80 && p[2] < 80, "rotation {rot:?}: expected red at left edge, got {p:?}");
        // Just outside the rectangle there's no red.
        let outside = img.get_pixel((w as f64 * 0.05) as u32, h / 2);
        assert!(!(outside[0] > 180 && outside[1] < 80), "rotation {rot:?}: red leaked outside, got {outside:?}");
    }
}

#[test]
fn redact_burns_boxes_into_every_page() {
    let src = sample(None);
    let p = tmp("redact");
    std::fs::write(&p, &src).unwrap();
    let mut boxes = BTreeMap::new();
    boxes.insert(0, vec![Redaction { rect: Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }, mode: RedactMode::Black }]);
    wic::init_thread();
    let out = pdf_edit::redact(&p, &boxes, 150.0, &mut |_, _| {}).unwrap();
    let img = render(&out, "redact-out");
    assert_eq!(img.dimensions(), (400, 300), "same page size as the original");
    let p = img.get_pixel(100, 150);
    assert!(p[0] < 20 && p[1] < 20 && p[2] < 20, "redacted area is black, got {p:?}");
    // No text/vector content survives: one image per page, nothing else.
    let doc = lopdf::Document::load_mem(&out).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
}

#[test]
fn pixelate_averages_blocks() {
    let mut img = RgbImage::from_fn(160, 160, |x, y| if (x + y) % 2 == 0 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) });
    pdf_edit::apply_redactions(&mut img, &[Redaction { rect: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, mode: RedactMode::Pixelate }]);
    let p = img.get_pixel(5, 5);
    assert!((120..=135).contains(&p[0]), "checkerboard averages to grey, got {p:?}");
}
