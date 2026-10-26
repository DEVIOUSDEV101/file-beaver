//! Stress benchmark: big photos, long PDFs, 100-file batches. Run in release:
//!   cargo run --release --example bench -- <work_dir>
//! Prints wall time and the process's peak memory after each case.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use image::{DynamicImage, Rgb, RgbImage};
use shiftdrop_core::actions;
use shiftdrop_core::naming::OutputDir;
use shiftdrop_core::pdf::{self, Page, PageImage};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;

fn peak_mb() -> u64 {
    let mut c = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
    }
    c.PeakWorkingSetSize as u64 / 1_048_576
}

/// Photo-like pixels: gradients plus grain, so codecs work as hard as on real photos.
fn photo(w: u32, h: u32, seed: u32) -> DynamicImage {
    let mut s = seed;
    DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
        s = s.wrapping_mul(1103515245).wrapping_add(12345);
        let n = (s >> 16) as u8 % 24;
        Rgb([((x * 255 / w) as u8).saturating_add(n), ((y * 255 / h) as u8).saturating_add(n), (((x + y) * 127 / (w + h)) as u8).saturating_add(n)])
    }))
}

fn save_jpeg(img: &DynamicImage, p: &Path) {
    let bytes = shiftdrop_core::encode::encode(img, shiftdrop_core::formats::Format::Jpg, None, 92).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn case(label: &str, action: &str, paths: &[PathBuf]) {
    let t = Instant::now();
    let o = actions::run(action, paths, &OutputDir::Beside, &AtomicBool::new(false), &mut |_| {});
    let in_bytes: u64 = paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    let out_bytes: u64 = o.outputs.iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    println!(
        "{label:<38} {:>7.1} s   peak {:>5} MB   in {:>6.1} MB -> out {:>6.1} MB ({} files){}",
        t.elapsed().as_secs_f32(),
        peak_mb(),
        in_bytes as f64 / 1e6,
        out_bytes as f64 / 1e6,
        o.outputs.len(),
        if o.errors.is_empty() { String::new() } else { format!("   ERRORS: {:?}", &o.errors[..o.errors.len().min(3)]) }
    );
    for p in &o.outputs {
        let _ = std::fs::remove_file(p);
    }
}

fn main() {
    shiftdrop_core::wic::init_thread();
    let work = PathBuf::from(std::env::args().nth(1).expect("usage: bench <work_dir>"));
    let which = std::env::args().nth(2).unwrap_or_else(|| "all".into());
    std::fs::create_dir_all(&work).unwrap();
    println!("cores: {}", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));

    if which == "all" || which == "big" {
        let p = work.join("Big 50MP.jpg");
        if !p.exists() {
            save_jpeg(&photo(8160, 6120, 1), &p);
        }
        case("50 MP photo -> PNG", "convert:png", &[p.clone()]);
        case("50 MP photo -> WEBP", "convert:webp", &[p.clone()]);
        case("50 MP photo -> PDF", "convert:pdf", &[p.clone()]);
        case("50 MP photo compress", "tool:compress", &[p.clone()]);
        case("50 MP photo metadata", "tool:metadata", &[p.clone()]);
    }

    if which == "all" || which == "pdf" {
        let p = work.join("Scan 300 pages.pdf");
        if !p.exists() {
            // A4 at 150 dpi, like a scanner's output.
            let page = photo(1240, 1754, 7);
            let jpeg = pdf::jpeg_bytes(&page, 85).unwrap();
            let pages: Vec<Page> = (0..300).map(|_| Page { image: PageImage::Jpeg { bytes: jpeg.clone(), width: 1240, height: 1754, gray: false }, dpi: 150.0 }).collect();
            std::fs::write(&p, pdf::write_images(&pages).unwrap()).unwrap();
        }
        case("300-page PDF -> JPGs", "convert:jpg", &[p.clone()]);
        case("300-page PDF compress", "tool:compress", &[p.clone()]);
        let t = Instant::now();
        let boxes = std::collections::BTreeMap::new();
        let r = shiftdrop_core::pdf_edit::redact(&p, &boxes, 200.0, &mut |_, _| {});
        println!("{:<38} {:>7.1} s   peak {:>5} MB   ok={}", "300-page PDF redact (flatten)", t.elapsed().as_secs_f32(), peak_mb(), r.is_ok());
    }

    if which == "all" || which == "batch" {
        let dir = work.join("batch");
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("Photo 000.jpg");
        if !first.exists() {
            save_jpeg(&photo(4032, 3024, 3), &first);
        }
        let files: Vec<PathBuf> = (0..100)
            .map(|i| {
                let p = dir.join(format!("Photo {i:03}.jpg"));
                if !p.exists() {
                    std::fs::copy(&first, &p).unwrap();
                }
                p
            })
            .collect();
        case("100 x 12 MP photos -> WEBP", "convert:webp", &files);
        case("100 x 12 MP photos compress", "tool:compress", &files);
        case("100 x 12 MP photos -> one PDF", "convert:pdf", &files);
    }
}
