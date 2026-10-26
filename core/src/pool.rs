//! The worker pool for batches and multi-page PDFs.
//!
//! Capped at half the cores, 2 to 6 threads: each worker can hold a decoded photo
//! (a 12 MP photo needs a few hundred MB while being processed), so using every core
//! on a big machine would trade a little speed for gigabytes of memory.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use image::DynamicImage;

use crate::pdf::{self, PdfDoc};
use crate::wic;

pub fn workers() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    cores.div_ceil(2).clamp(2, 6)
}

pub fn get() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers())
            .thread_name(|i| format!("shiftdrop-worker-{i}"))
            // Windows codecs and the PDF engine need COM on every thread.
            .start_handler(|_| wic::init_thread())
            .build()
            .expect("worker pool")
    })
}

thread_local! {
    /// Each worker keeps its own open copy of the PDF it's rendering.
    static OPEN_PDF: RefCell<Option<(PathBuf, PdfDoc)>> = const { RefCell::new(None) };
}

/// Renders a PDF page on the calling worker, reusing that worker's open document.
pub fn render_page(path: &Path, index: u32, dpi: f64) -> Result<DynamicImage> {
    OPEN_PDF.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().map(|(p, _)| p != path).unwrap_or(true) {
            *slot = Some((path.to_path_buf(), pdf::open(path)?));
        }
        slot.as_ref().unwrap().1.render(index, dpi)
    })
}
