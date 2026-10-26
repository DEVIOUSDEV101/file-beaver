//! Compress: smaller files, same format where possible.
//! JPG/WEBP re-encode at quality 72, PNG is optimised losslessly (oxipng), TIFF gets
//! Deflate, BMP becomes PNG, PDFs have their embedded photos recompressed.
//! A result that isn't meaningfully smaller is not saved.

use std::path::Path;

use anyhow::{bail, Result};
use image::{DynamicImage, GenericImageView};
use lopdf::{Document, Object};

use crate::decode;
use crate::encode::encode;
use crate::formats::Format;

pub const QUALITY: u8 = 72;
/// Results must be at least this much smaller, both relatively and in bytes, to be worth saving.
const MIN_SAVING: f64 = 0.03;
const MIN_SAVING_BYTES: usize = 1024;
/// PDF photos larger than this (long side, px) are scaled down; ~200 dpi on A4/Letter.
const PDF_MAX_SIDE: u32 = 2200;

/// (compressed bytes, output format) or None when it's already as small as we can make it.
pub fn compress(path: &Path) -> Result<Option<(Vec<u8>, Format)>> {
    let original = std::fs::read(path)?;
    if original.is_empty() {
        bail!("the file is empty");
    }
    if let Some(why) = crate::formats::not_really(path) {
        bail!(why);
    }
    let fmt = crate::formats::detect(path);
    let (bytes, out_fmt) = match fmt {
        Some(Format::Jpg) | Some(Format::Webp) | Some(Format::Tiff) => {
            let d = decode::load(path)?;
            (encode(&d.image, fmt.unwrap(), d.icc.as_deref(), QUALITY)?, fmt.unwrap())
        }
        Some(Format::Png) => (optimise_png(&original)?, Format::Png),
        Some(Format::Bmp) => {
            let d = decode::load(path)?;
            (optimise_png(&encode(&d.image, Format::Png, None, 0)?)?, Format::Png)
        }
        Some(Format::Pdf) => (compress_pdf(&original)?, Format::Pdf),
        _ => bail!("this file type can't be compressed"),
    };
    let worth_it = (bytes.len() as f64) < original.len() as f64 * (1.0 - MIN_SAVING)
        && bytes.len() + MIN_SAVING_BYTES <= original.len();
    Ok(worth_it.then_some((bytes, out_fmt)))
}

fn optimise_png(data: &[u8]) -> Result<Vec<u8>> {
    let mut opts = oxipng::Options::from_preset(2);
    opts.strip = oxipng::StripChunks::Safe;
    Ok(oxipng::optimize_from_memory(data, &opts)?)
}

fn name_is(obj: Option<&Object>, want: &[u8]) -> bool {
    match obj {
        Some(Object::Name(n)) => n == want,
        Some(Object::Array(a)) if a.len() == 1 => matches!(&a[0], Object::Name(n) if n == want),
        _ => false,
    }
}

/// Recompresses embedded JPEG photos (RGB/greyscale only, the safe cases), scales very
/// large ones to ~200 dpi, Deflates uncompressed streams and drops unused objects.
pub fn compress_pdf(data: &[u8]) -> Result<Vec<u8>> {
    let mut doc = match Document::load_mem(data) {
        Ok(d) => d,
        Err(_) => bail!("this PDF is damaged or uses a structure File Beaver can't read"),
    };
    if doc.is_encrypted() {
        bail!("password-protected PDFs aren't supported");
    }
    // Find the photos worth recompressing (the safe cases only)...
    let mut jobs: Vec<(lopdf::ObjectId, Vec<u8>, bool)> = Vec::new();
    for (id, obj) in doc.objects.iter() {
        let Object::Stream(s) = obj else { continue };
        let d = &s.dict;
        if !name_is(d.get(b"Subtype").ok(), b"Image") || !name_is(d.get(b"Filter").ok(), b"DCTDecode") {
            continue;
        }
        let gray = name_is(d.get(b"ColorSpace").ok(), b"DeviceGray");
        let rgb = name_is(d.get(b"ColorSpace").ok(), b"DeviceRGB");
        let bpc8 = matches!(d.get(b"BitsPerComponent"), Ok(Object::Integer(8)));
        if !(gray || rgb) || !bpc8 || d.has(b"Decode") || s.content.len() < 40_000 {
            continue;
        }
        jobs.push((*id, s.content.clone(), gray));
    }
    // ...recompress them in parallel...
    use rayon::prelude::*;
    let redone: Vec<(lopdf::ObjectId, Option<(Vec<u8>, u32, u32)>)> = crate::pool::get().install(|| {
        jobs.par_iter()
            .map(|(id, content, gray)| {
                let attempt = || -> Option<(Vec<u8>, u32, u32)> {
                    let img = image::load_from_memory_with_format(content, image::ImageFormat::Jpeg).ok()?;
                    let (w, h) = img.dimensions();
                    let img = if w.max(h) > PDF_MAX_SIDE { img.resize(PDF_MAX_SIDE, PDF_MAX_SIDE, image::imageops::FilterType::Lanczos3) } else { img };
                    let img = if *gray { DynamicImage::ImageLuma8(img.to_luma8()) } else { img };
                    let bytes = encode(&img, Format::Jpg, None, QUALITY).ok()?;
                    (bytes.len() as f64 <= content.len() as f64 * 0.9).then(|| (bytes, img.width(), img.height()))
                };
                (*id, attempt())
            })
            .collect()
    });
    // ...and put the smaller ones back.
    for (id, result) in redone {
        let (Some((bytes, nw, nh)), Ok(Object::Stream(s))) = (result, doc.get_object_mut(id)) else { continue };
        s.dict.set("Width", nw as i64);
        s.dict.set("Height", nh as i64);
        s.dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
        s.dict.remove(b"DecodeParms");
        s.set_content(bytes);
    }
    doc.compress();
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
