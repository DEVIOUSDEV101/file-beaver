//! End-to-end tests of Convert / Compress / Metadata on real files written to a temp folder.
//! PDFs are checked by rendering them back with Windows' PDF engine.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use image::{DynamicImage, Rgb, RgbImage};
use shiftdrop_core::actions::{self, Outcome};
use shiftdrop_core::formats::Format;
use shiftdrop_core::naming::OutputDir;
use shiftdrop_core::{metadata, pdf, wic};

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "shiftdrop-it-{name}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A colourful noisy photo-like image (noise makes JPEG sizes realistic).
fn photo(w: u32, h: u32) -> DynamicImage {
    let mut seed = 12345u32;
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let n = (seed >> 16) as u8 % 40;
        Rgb([((x * 255 / w) as u8).saturating_add(n), ((y * 255 / h) as u8).saturating_add(n), 120u8.saturating_add(n)])
    }))
}

fn save(dir: &Path, name: &str, img: &DynamicImage) -> PathBuf {
    let p = dir.join(name);
    img.save(&p).unwrap();
    p
}

fn run(action: &str, paths: &[PathBuf]) -> Outcome {
    wic::init_thread();
    let o = actions::run(action, paths, &OutputDir::Beside, &AtomicBool::new(false), &mut |_| {});
    assert!(o.errors.is_empty(), "{action} errors: {:?}", o.errors);
    o
}

fn hash(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

#[test]
fn convert_png_to_every_image_format_and_back() {
    let d = tmpdir("convert");
    let src = save(&d, "Shot.png", &photo(320, 200));
    let before = hash(&src);
    let caps = wic::caps();
    for f in [Format::Jpg, Format::Webp, Format::Tiff, Format::Bmp, Format::Avif] {
        let o = run(&format!("convert:{}", f.id()), &[src.clone()]);
        assert_eq!(o.outputs.len(), 1, "{f:?}");
        let out = &o.outputs[0];
        assert_eq!(out.file_name().unwrap().to_string_lossy(), format!("Shot.{}", f.ext()));
        if f != Format::Avif || caps.avif_decode {
            let back = shiftdrop_core::decode::load(out).unwrap().image;
            assert_eq!((back.width(), back.height()), (320, 200), "{f:?} size");
        }
    }
    assert_eq!(hash(&src), before, "source must be untouched");
}

#[test]
fn images_to_one_pdf_render_back() {
    let d = tmpdir("pdf");
    let a = save(&d, "A.jpg", &photo(400, 300)); // embedded as-is
    let b = save(&d, "B.png", &photo(200, 400)); // embedded lossless
    let o = run("convert:pdf", &[a.clone(), b]);
    assert_eq!(o.outputs[0].file_name().unwrap(), "A combined.pdf");
    let doc = pdf::open(&o.outputs[0]).unwrap();
    assert_eq!(doc.page_count().unwrap(), 2);
    let p1 = doc.render(0, 96.0).unwrap();
    let p2 = doc.render(1, 96.0).unwrap();
    assert_eq!((p1.width(), p1.height()), (400, 300));
    assert_eq!((p2.width(), p2.height()), (200, 400));
}

#[test]
fn pdf_pages_to_jpgs_with_page_names() {
    let d = tmpdir("pdf2jpg");
    let a = save(&d, "A.png", &photo(300, 300));
    let b = save(&d, "B.png", &photo(300, 300));
    let pdf_path = run("convert:pdf", &[a, b]).outputs[0].clone();
    let o = run("convert:jpg", &[pdf_path]);
    let names: Vec<String> = o.outputs.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["A combined page 1.jpg", "A combined page 2.jpg"]);
    // 300 px at 96 dpi rendered at 200 dpi
    let img = image::open(&o.outputs[0]).unwrap();
    assert_eq!(img.width(), 625);
}

#[test]
fn docx_contains_pictures() {
    let d = tmpdir("docx");
    let a = save(&d, "A.jpg", &photo(400, 300));
    let o = run("convert:docx", &[a]);
    let f = std::fs::File::open(&o.outputs[0]).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
    assert!(names.contains(&"word/document.xml".to_string()));
    assert!(names.contains(&"word/media/image1.jpeg".to_string()));
}

#[test]
fn compress_jpg_is_smaller_and_reported() {
    let d = tmpdir("compress");
    let src = d.join("Big.jpg");
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 98).encode_image(&photo(1200, 900)).unwrap();
    std::fs::write(&src, &bytes).unwrap();
    let o = run("tool:compress", &[src.clone()]);
    assert_eq!(o.outputs[0].file_name().unwrap(), "Big Compressed.jpg");
    assert!(std::fs::metadata(&o.outputs[0]).unwrap().len() < bytes.len() as u64);
    assert!(o.summary.contains('→'), "{}", o.summary);
    assert_eq!(hash(&src), bytes);
}

#[test]
fn compress_pdf_shrinks_embedded_photo_and_still_renders() {
    let d = tmpdir("compresspdf");
    let src = d.join("Scan.jpg");
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 98).encode_image(&photo(1600, 1200)).unwrap();
    std::fs::write(&src, &bytes).unwrap();
    let pdf_path = run("convert:pdf", &[src]).outputs[0].clone();
    let o = run("tool:compress", &[pdf_path.clone()]);
    let out = &o.outputs[0];
    assert!(std::fs::metadata(out).unwrap().len() < std::fs::metadata(&pdf_path).unwrap().len());
    let doc = pdf::open(out).unwrap();
    assert_eq!(doc.page_count().unwrap(), 1);
    doc.render(0, 48.0).unwrap();
}

#[test]
fn already_small_png_is_not_saved() {
    let d = tmpdir("tiny");
    let src = save(&d, "Flat.png", &DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, Rgb([10, 20, 30]))));
    let o = run("tool:compress", &[src]);
    assert!(o.outputs.is_empty());
    assert!(o.summary.starts_with("Already"), "{}", o.summary);
}

/// EXIF with GPS + camera + orientation 6 (rotated), as a JPEG APP1 segment.
fn exif_app1() -> Vec<u8> {
    use exif::experimental::Writer;
    use exif::{Field, In, Rational, Tag, Value};
    let make = Field { tag: Tag::Make, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"TestCam".to_vec()]) };
    let orient = Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) };
    let lat = Field { tag: Tag::GPSLatitude, ifd_num: In::PRIMARY, value: Value::Rational(vec![Rational { num: 51, denom: 1 }, Rational { num: 30, denom: 1 }, Rational { num: 0, denom: 1 }]) };
    let lat_ref = Field { tag: Tag::GPSLatitudeRef, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"N".to_vec()]) };
    let mut w = Writer::new();
    w.push_field(&make);
    w.push_field(&orient);
    w.push_field(&lat);
    w.push_field(&lat_ref);
    let mut tiff = Cursor::new(Vec::new());
    w.write(&mut tiff, false).unwrap();
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff.into_inner());
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(&payload);
    seg
}

#[test]
fn metadata_strip_jpeg_removes_gps_keeps_pixels_and_rotation() {
    let d = tmpdir("meta");
    let mut jpg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(&photo(64, 48)).unwrap();
    let mut with_exif = jpg[..2].to_vec();
    with_exif.extend_from_slice(&exif_app1());
    with_exif.extend_from_slice(&[0xFF, 0xFE, 0, 7, b'h', b'e', b'l', b'l', b'o']); // COM
    with_exif.extend_from_slice(&jpg[2..]);
    let src = d.join("Holiday.jpg");
    std::fs::write(&src, &with_exif).unwrap();

    let o = run("tool:metadata", &[src.clone()]);
    assert!(o.summary.contains("GPS"), "{}", o.summary);
    let out = std::fs::read(&o.outputs[0]).unwrap();
    assert_eq!(o.outputs[0].file_name().unwrap(), "Holiday Clean.jpg");

    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(&out)).unwrap();
    assert_eq!(exif.fields().count(), 1, "only orientation survives");
    assert_eq!(exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).unwrap().value.get_uint(0), Some(6));
    // Pixel data untouched: the scan from SOS onwards is byte-identical.
    let sos = |b: &[u8]| b.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    assert_eq!(&out[sos(&out)..], &with_exif[sos(&with_exif)..]);
    assert!(!out.windows(5).any(|w| w == b"hello"));
}

#[test]
fn metadata_strip_png_text_chunks() {
    let png = {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 4, 4);
        enc.set_color(png::ColorType::Rgb);
        enc.add_text_chunk("Author".into(), "Someone".into()).unwrap();
        enc.write_header().unwrap().write_image_data(&[0u8; 48]).unwrap();
        out
    };
    let (out, removed) = metadata::strip_png(&png).unwrap();
    assert_eq!(removed, ["Text fields"]);
    assert!(!out.windows(7).any(|w| w == b"Someone"));
    image::load_from_memory(&out).unwrap();
}

#[test]
fn metadata_strip_pdf_info() {
    let d = tmpdir("metapdf");
    let a = save(&d, "A.png", &photo(100, 100));
    let pdf_path = run("convert:pdf", &[a]).outputs[0].clone();
    // add an Info dictionary
    let mut doc = lopdf::Document::load(&pdf_path).unwrap();
    let info = doc.add_object(lopdf::dictionary! { "Author" => lopdf::Object::string_literal("Jane"), "Producer" => lopdf::Object::string_literal("Scanner 3000") });
    doc.trailer.set("Info", info);
    doc.save(&pdf_path).unwrap();

    let o = run("tool:metadata", &[pdf_path]);
    assert!(o.summary.contains("Author"), "{}", o.summary);
    let clean = std::fs::read(&o.outputs[0]).unwrap();
    assert!(!clean.windows(4).any(|w| w == b"Jane"));
    pdf::open(&o.outputs[0]).unwrap().render(0, 48.0).unwrap();
}

#[test]
fn heic_export_when_this_pc_can() {
    if !wic::caps().heic_encode {
        println!("skipped: no HEIC encoder on this PC");
        return;
    }
    let d = tmpdir("heic");
    let src = save(&d, "Pic.png", &photo(320, 240));
    let o = run("convert:heic", &[src]);
    let bytes = std::fs::read(&o.outputs[0]).unwrap();
    // ISO-BMFF "ftyp" box with a HEIF brand
    assert_eq!(&bytes[4..8], b"ftyp");
    let brand = &bytes[8..12];
    assert!([&b"heic"[..], b"heix", b"mif1", b"msf1"].contains(&brand), "brand {:?}", String::from_utf8_lossy(brand));
    println!("HEIC export ok: {} bytes", bytes.len());
}

#[test]
fn caps_probe_runs() {
    let c = wic::caps();
    println!("codecs on this PC: {c:?}");
}

fn psnr(a: &image::RgbImage, b: &image::RgbImage) -> f64 {
    let mse: f64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.as_raw().len() as f64;
    10.0 * (255.0f64 * 255.0 / mse).log10()
}

/// The JPEG encoder must beat a plain baseline encoder on size without losing fidelity.
/// Uses a noisy photo on purpose: that is what broke mozjpeg.
#[test]
fn jpeg_is_smaller_at_equal_quality() {
    let d = tmpdir("moz");
    // Photo-like: smooth gradients + grain, saved by ffmpeg-like high quality first.
    let src = photo(2000, 1500);
    let orig = src.to_rgb8();
    for q in [72u8, 90] {
        let old = shiftdrop_core::encode::jpeg_baseline(&src, q).unwrap();
        let new = shiftdrop_core::encode::encode(&src, Format::Jpg, None, q).unwrap();
        let p_old = psnr(&orig, &image::load_from_memory(&old).unwrap().to_rgb8());
        let p_new = psnr(&orig, &image::load_from_memory(&new).unwrap().to_rgb8());
        println!(
            "q{q}: old {} KB / {p_old:.2} dB   new {} KB / {p_new:.2} dB   ({:+.0}% size)",
            old.len() / 1024,
            new.len() / 1024,
            100.0 * (new.len() as f64 / old.len() as f64 - 1.0)
        );
        assert!(new.len() < old.len(), "q{q}: not smaller");
        assert!(p_new > p_old - 0.5, "q{q}: lost fidelity");
    }
    let _ = d;
}
