//! The awkward files real people drop: unusual-but-valid images, broken files, odd paths.
//! Rule: nothing ever panics; valid files work; broken files give a readable error.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use image::{DynamicImage, ImageBuffer, Luma, LumaA, Rgb, Rgba, RgbImage};
use shiftdrop_core::actions::{self, Outcome};
use shiftdrop_core::naming::OutputDir;
use shiftdrop_core::wic;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "shiftdrop-odd-{name}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn gradient(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| Rgb([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 140]))
}

/// (path, valid?) for every odd file.
fn corpus(d: &Path) -> Vec<(PathBuf, bool)> {
    let mut v = Vec::new();
    let mut add = |name: &str, bytes: Vec<u8>, valid: bool| {
        let p = d.join(name);
        std::fs::write(&p, bytes).unwrap();
        v.push((p, valid));
    };
    let enc = |img: &DynamicImage, fmt: image::ImageFormat| {
        let mut c = std::io::Cursor::new(Vec::new());
        img.write_to(&mut c, fmt).unwrap();
        c.into_inner()
    };

    // CMYK JPEG (print workflows, some scanners).
    let cmyk: Vec<u8> = (0..64 * 48).flat_map(|i| [(i % 64 * 4) as u8, 40, (i / 64 * 5) as u8, 20]).collect();
    let mut b = Vec::new();
    jpeg_encoder::Encoder::new(&mut b, 85).encode(&cmyk, 64, 48, jpeg_encoder::ColorType::Cmyk).unwrap();
    add("cmyk.jpg", b, true);

    let mut b = Vec::new();
    let mut e = jpeg_encoder::Encoder::new(&mut b, 85);
    e.set_progressive(true);
    e.encode(gradient(120, 80).as_raw(), 120, 80, jpeg_encoder::ColorType::Rgb).unwrap();
    add("progressive.jpg", b, true);

    add("gray.jpg", enc(&DynamicImage::ImageLuma8(ImageBuffer::from_fn(80, 60, |x, _| Luma([x as u8 * 3]))), image::ImageFormat::Jpeg), true);
    add("gray_alpha.png", enc(&DynamicImage::ImageLumaA8(ImageBuffer::from_fn(80, 60, |x, y| LumaA([x as u8 * 3, y as u8 * 4]))), image::ImageFormat::Png), true);
    add("rgba16.png", enc(&DynamicImage::ImageRgba16(ImageBuffer::from_fn(80, 60, |x, y| Rgba([x as u16 * 800, y as u16 * 1000, 30000, 50000]))), image::ImageFormat::Png), true);

    // Palette PNG with a transparent entry.
    let mut b = Vec::new();
    {
        let mut pe = png::Encoder::new(&mut b, 4, 4);
        pe.set_color(png::ColorType::Indexed);
        pe.set_palette(vec![255, 0, 0, 0, 0, 255]);
        pe.set_trns(vec![255, 0]);
        pe.write_header().unwrap().write_image_data(&[0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0]).unwrap();
    }
    add("palette_trns.png", b, true);

    add("1x1.png", enc(&DynamicImage::ImageRgb8(gradient(1, 1)), image::ImageFormat::Png), true);
    add("tall.png", enc(&DynamicImage::ImageRgb8(gradient(10, 5000)), image::ImageFormat::Png), true);

    // Animated GIF: we use the first frame.
    let mut b = Vec::new();
    {
        let mut ge = image::codecs::gif::GifEncoder::new(&mut b);
        for i in 0..2u8 {
            let f = image::Frame::new(image::RgbaImage::from_pixel(40, 30, Rgba([i * 200, 50, 90, 255])));
            ge.encode_frame(f).unwrap();
        }
    }
    add("animated.gif", b, true);

    add("deep16.tif", enc(&DynamicImage::ImageRgb16(ImageBuffer::from_fn(60, 40, |x, _| Rgb([x as u16 * 900, 20000, 40000]))), image::ImageFormat::Tiff), true);

    // Multi-page TIFF: we use the first page.
    let mut c = std::io::Cursor::new(Vec::new());
    {
        let mut te = tiff::encoder::TiffEncoder::new(&mut c).unwrap();
        te.write_image::<tiff::encoder::colortype::RGB8>(30, 20, gradient(30, 20).as_raw()).unwrap();
        te.write_image::<tiff::encoder::colortype::RGB8>(30, 20, gradient(30, 20).as_raw()).unwrap();
    }
    add("multipage.tif", c.into_inner(), true);

    // A PNG saved with a .jpg name (common with web downloads).
    add("really_a_png.jpg", enc(&DynamicImage::ImageRgb8(gradient(50, 40)), image::ImageFormat::Png), true);

    // Broken files.
    add("empty.jpg", vec![], false);
    let good = enc(&DynamicImage::ImageRgb8(gradient(200, 150)), image::ImageFormat::Jpeg);
    add("truncated.jpg", good[..good.len() / 3].to_vec(), false);
    add("garbage.png", (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect(), false);
    add("corrupt.pdf", b"%PDF-1.7\n1 0 obj << /Type /Catalog >> nonsense".to_vec(), false);
    add(
        "encrypted.pdf",
        b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\n3 0 obj << /Filter /Standard /V 1 /R 2 /O (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) /U (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa) /P -4 >> endobj\ntrailer << /Root 1 0 R /Encrypt 3 0 R /Size 4 >>\n%%EOF\n".to_vec(),
        false,
    );
    v
}

fn run(action: &str, p: &Path) -> std::thread::Result<Outcome> {
    let p = p.to_path_buf();
    let action = action.to_string();
    std::panic::catch_unwind(move || {
        wic::init_thread();
        actions::run(&action, &[p], &OutputDir::Beside, &AtomicBool::new(false), &mut |_| {})
    })
}

#[test]
fn odd_files_never_crash_and_valid_ones_work() {
    let d = tmpdir("corpus");
    let actions_list = ["convert:png", "convert:jpg", "convert:webp", "convert:tiff", "convert:bmp", "convert:pdf", "convert:docx", "tool:compress", "tool:metadata"];
    let mut problems = Vec::new();
    for (path, valid) in corpus(&d) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        for a in actions_list {
            // Converting a file to its own format is refused on purpose; skip those pairs.
            let own = shiftdrop_core::formats::Format::from_path(&path).map(|f| f.id());
            if a.strip_prefix("convert:") == own {
                continue;
            }
            // The ring greys these out for GIFs, so they're never offered.
            if name.ends_with(".gif") && (a == "tool:compress" || a == "tool:metadata") {
                continue;
            }
            match run(a, &path) {
                Err(_) => problems.push(format!("PANIC  {name} {a}")),
                Ok(o) => {
                    let line = if o.outputs.is_empty() { format!("{} {}", o.summary, o.errors.join(" | ")) } else { "ok".into() };
                    println!("{:<18} {:<14} {}", name, a, line);
                    let quiet_no_op = o.errors.is_empty() && o.outputs.is_empty(); // "already compressed", "no metadata"
                    if valid && !o.errors.is_empty() {
                        problems.push(format!("FAILED {name} {a}: {:?}", o.errors));
                    }
                    if !valid && !o.outputs.is_empty() && a != "tool:compress" {
                        problems.push(format!("ACCEPTED BROKEN {name} {a}"));
                    }
                    if !valid && o.outputs.is_empty() && quiet_no_op {
                        problems.push(format!("SILENT {name} {a}: broken file but no error shown"));
                    }
                    if o.errors.iter().any(|e| e.trim().is_empty()) {
                        problems.push(format!("EMPTY ERROR {name} {a}"));
                    }
                    for out in &o.outputs {
                        let ext = out.extension().unwrap().to_string_lossy().to_lowercase();
                        if ["png", "jpg", "webp", "tif", "tiff", "bmp"].contains(&ext.as_str()) && image::open(out).is_err() {
                            problems.push(format!("UNREADABLE OUTPUT {name} {a} -> {}", out.display()));
                        }
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn unicode_and_very_long_paths() {
    let base = tmpdir("paths");
    // > 260 characters in total.
    let mut deep = base.clone();
    for i in 0..6 {
        deep = deep.join(format!("a fairly long folder name number {i} to push past MAX_PATH"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    let files = [base.join("фото 照片 🎉.png"), deep.join("deep photo.png"), deep.join("deep scan.pdf")];
    let page = shiftdrop_core::pdf::Page { image: shiftdrop_core::pdf::PageImage::Pixels(DynamicImage::ImageRgb8(gradient(64, 48))), dpi: 96.0 };
    for f in &files {
        if f.extension().unwrap() == "pdf" {
            std::fs::write(f, shiftdrop_core::pdf::write_images(&[page_clone(&page)]).unwrap()).unwrap();
        } else {
            DynamicImage::ImageRgb8(gradient(64, 48)).save(f).unwrap();
        }
        let is_pdf = f.extension().unwrap() == "pdf";
        for a in ["convert:jpg", "convert:pdf", "tool:compress", "tool:metadata"] {
            if is_pdf && a == "convert:pdf" {
                continue;
            }
            let o = run(a, f).expect("no panic");
            assert!(o.errors.is_empty(), "{a} on {}: {:?}", f.display(), o.errors);
        }
    }
    println!("deepest path is {} characters", files[2].as_os_str().len());
}

fn page_clone(p: &shiftdrop_core::pdf::Page) -> shiftdrop_core::pdf::Page {
    match &p.image {
        shiftdrop_core::pdf::PageImage::Pixels(img) => shiftdrop_core::pdf::Page { image: shiftdrop_core::pdf::PageImage::Pixels(img.clone()), dpi: p.dpi },
        _ => unreachable!(),
    }
}

#[test]
fn unwritable_folder_falls_back_to_downloads() {
    let d = tmpdir("readonly");
    let src = d.join("locked.png");
    DynamicImage::ImageRgb8(gradient(64, 48)).save(&src).unwrap();
    let user = std::env::var("USERNAME").unwrap();
    // Deny creating files in the folder (existing files stay readable).
    let deny = std::process::Command::new("icacls").arg(&d).args(["/deny", &format!("{user}:(WD,AD)")]).output().unwrap();
    assert!(deny.status.success(), "icacls failed: {}", String::from_utf8_lossy(&deny.stderr));
    let o = run("convert:jpg", &src).expect("no panic");
    let _ = std::process::Command::new("icacls").arg(&d).args(["/remove:d", &user]).output();
    assert!(o.errors.is_empty(), "{:?}", o.errors);
    let out = &o.outputs[0];
    println!("saved to {}", out.display());
    assert!(out.starts_with(shiftdrop_core::naming::fallback_dir()), "expected fallback folder, got {}", out.display());
    let _ = std::fs::remove_file(out);
}
