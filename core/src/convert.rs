//! Convert: images and PDF pages to another format.
//! - To an image format: one output per image / per PDF page.
//! - To PDF or DOCX: everything dropped goes into one document, in order.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};
use rayon::prelude::*;

use crate::actions::{Outcome, Progress};
use crate::decode;
use crate::docx::{self, Picture};
use crate::encode::{encode, CONVERT_QUALITY};
use crate::formats::Format;
use crate::naming::{self, OutputDir};
use crate::pdf::{self, Page, PageImage};
use crate::pool;

/// One page of input: a whole image file, or one page of a PDF.
struct Unit {
    path: PathBuf,
    /// Some((index, page count)) for PDF pages.
    page: Option<(u32, u32)>,
}

fn units(paths: &[PathBuf]) -> Result<(Vec<Unit>, Vec<String>)> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for p in paths {
        if Format::from_path(p) == Some(Format::Pdf) {
            match pdf::open(p).and_then(|d| d.page_count()) {
                Ok(n) => out.extend((0..n).map(|i| Unit { path: p.clone(), page: Some((i, n)) })),
                Err(e) => errors.push(format!("{}: {e:#}", name(p))),
            }
        } else {
            out.push(Unit { path: p.clone(), page: None });
        }
    }
    Ok((out, errors))
}

pub fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn progress_detail(done: usize, units: &[Unit], files: usize) -> String {
    match units[0].page {
        Some((_, n)) if files == 1 && n > 1 => format!("Page {done} of {n}"),
        _ => format!("{done} of {} file(s)", units.len()),
    }
}

/// Runs `f` on every unit on the worker pool, in parallel, keeping input order.
/// None if the user cancelled.
fn par_units<T: Send>(
    units: &[Unit],
    files: usize,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(Progress) + Send),
    f: impl Fn(&Unit) -> Result<T> + Sync,
) -> Option<Vec<Result<T>>> {
    let done = AtomicUsize::new(0);
    let progress = Mutex::new(progress);
    let results = pool::get().install(|| {
        units
            .par_iter()
            .map(|u| {
                if cancel.load(Ordering::SeqCst) {
                    return Err(anyhow!("cancelled"));
                }
                let r = f(u);
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                // Skip an update rather than wait if another worker is reporting.
                if let Ok(mut p) = progress.try_lock() {
                    p(Progress { fraction: n as f32 / units.len() as f32, detail: progress_detail(n, units, files) });
                }
                r
            })
            .collect::<Vec<_>>()
    });
    (!cancel.load(Ordering::SeqCst)).then_some(results)
}

pub fn run(paths: &[PathBuf], target: Format, out: &OutputDir, cancel: &AtomicBool, progress: &mut (dyn FnMut(Progress) + Send)) -> Outcome {
    let mut outcome = Outcome::default();
    let (units, errors) = match units(paths) {
        Ok(u) => u,
        Err(e) => return Outcome::failed(format!("{e:#}")),
    };
    outcome.errors = errors;
    if units.is_empty() {
        outcome.summary = "Nothing to convert".into();
        return outcome;
    }
    progress(Progress { fraction: 0.0, detail: progress_detail(0, &units, paths.len()).replacen("0 of", "1 of", 1) });

    if matches!(target, Format::Pdf | Format::Docx) {
        enum Item {
            Page(Page),
            Picture(Picture),
        }
        let Some(results) = par_units(&units, paths.len(), cancel, progress, |u| {
            Ok(if target == Format::Pdf {
                Item::Page(match u.page {
                    Some((idx, _)) => {
                        let img = pool::render_page(&u.path, idx, pdf::RENDER_DPI)?;
                        let (width, height) = (img.width(), img.height());
                        let image = PageImage::Jpeg { bytes: pdf::jpeg_bytes(&img, CONVERT_QUALITY)?, width, height, gray: false };
                        Page { image, dpi: pdf::RENDER_DPI }
                    }
                    None => {
                        let image = match PageImage::jpeg_passthrough(&u.path) {
                            Some(jpeg) => jpeg,
                            None => PageImage::Pixels(decode::load(&u.path)?.image),
                        };
                        Page { image, dpi: 96.0 }
                    }
                })
            } else {
                let (img, photo) = match u.page {
                    Some((idx, _)) => (pool::render_page(&u.path, idx, 150.0)?, true),
                    None => {
                        let photo = matches!(crate::formats::detect(&u.path), Some(Format::Jpg | Format::Webp | Format::Heic | Format::Avif));
                        (decode::load(&u.path)?.image, photo)
                    }
                };
                let bytes = if photo { pdf::jpeg_bytes(&img, CONVERT_QUALITY)? } else { encode(&img, Format::Png, None, 0)? };
                Item::Picture(Picture { bytes, png: !photo, width: img.width(), height: img.height() })
            })
        }) else {
            return Outcome::cancelled();
        };
        let mut pages = Vec::new();
        let mut pictures = Vec::new();
        for (u, r) in units.iter().zip(results) {
            match r {
                Ok(Item::Page(p)) => pages.push(p),
                Ok(Item::Picture(p)) => pictures.push(p),
                Err(e) => outcome.errors.push(format!("{}: {e:#}", name(&u.path))),
            }
        }
        if pages.is_empty() && pictures.is_empty() {
            outcome.summary = "Nothing could be converted".into();
            return outcome;
        }
        let bytes = if target == Format::Pdf { pdf::write_images(&pages) } else { docx::write(&pictures) };
        let stem = if paths.len() == 1 { naming::stem(&paths[0]) } else { format!("{} combined", naming::stem(&paths[0])) };
        match bytes.and_then(|b| naming::write_new(&naming::target_dir(&paths[0], out), &stem, target.ext(), &b)) {
            Ok(p) => outcome.outputs.push(p),
            Err(e) => outcome.errors.push(format!("{e:#}")),
        }
    } else {
        let Some(results) = par_units(&units, paths.len(), cancel, progress, |u| {
            let (img, icc, stem) = match u.page {
                Some((idx, n)) => {
                    let stem = if n == 1 { naming::stem(&u.path) } else { format!("{} page {}", naming::stem(&u.path), idx + 1) };
                    (pool::render_page(&u.path, idx, pdf::RENDER_DPI)?, None, stem)
                }
                None => {
                    // Refuse only a true no-op: a PNG named .jpg converted to PNG is a useful fix.
                    if Format::from_path(&u.path) == Some(target) && crate::formats::detect(&u.path) == Some(target) {
                        bail!("already a {} file", target.label());
                    }
                    let d = decode::load(&u.path)?;
                    (d.image, d.icc, naming::stem(&u.path))
                }
            };
            let bytes = encode(&img, target, icc.as_deref(), CONVERT_QUALITY)?;
            naming::write_new(&naming::target_dir(&u.path, out), &stem, target.ext(), &bytes)
        }) else {
            return Outcome::cancelled();
        };
        for (u, r) in units.iter().zip(results) {
            match r {
                Ok(p) => outcome.outputs.push(p),
                Err(e) => outcome.errors.push(format!("{}: {e:#}", name(&u.path))),
            }
        }
    }
    outcome.summary = match outcome.outputs.as_slice() {
        [] => "Nothing was converted".into(),
        [one] => format!("Saved {}", name(one)),
        many => format!("Saved {} files", many.len()),
    };
    outcome
}

/// For callers that need a typed error when the target id is unknown.
pub fn target(id: &str) -> Result<Format> {
    Format::from_id(id).ok_or_else(|| anyhow!("unknown format {id}"))
}
