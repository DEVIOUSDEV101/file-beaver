//! Encoding an image to each output format.

use std::io::Cursor;

use anyhow::{anyhow, Result};
use image::codecs::{avif::AvifEncoder, bmp::BmpEncoder, jpeg::JpegEncoder, png::PngEncoder};
use image::{DynamicImage, ExtendedColorType, ImageEncoder};

use crate::formats::Format;
use crate::pdf::flatten;
use crate::wic;

/// Quality used for lossy formats when converting (not compressing).
pub const CONVERT_QUALITY: u8 = 90;

/// JPEG with progressive scans and optimised Huffman tables (pure Rust, no C to crash):
/// smaller than a plain baseline encoder at the same quality setting.
/// `pixels` is RGB8, or L8 when `gray`.
///
/// mozjpeg was evaluated and rejected: built with MinGW it fails on noisy photos
/// ("Missing Huffman code table entry") and crashes without its SIMD code.
pub fn jpeg(pixels: &[u8], w: u32, h: u32, gray: bool, quality: u8, icc: Option<&[u8]>) -> Result<Vec<u8>> {
    let (w, h) = (u16::try_from(w).map_err(|_| anyhow!("JPEG is limited to 65535 px per side"))?, u16::try_from(h).map_err(|_| anyhow!("JPEG is limited to 65535 px per side"))?);
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
    enc.set_progressive(true);
    enc.set_optimized_huffman_tables(true);
    if let Some(p) = icc {
        enc.add_icc_profile(p).map_err(|e| anyhow!("JPEG colour profile: {e}"))?;
    }
    let color = if gray { jpeg_encoder::ColorType::Luma } else { jpeg_encoder::ColorType::Rgb };
    enc.encode(pixels, w, h, color).map_err(|e| anyhow!("JPEG encode: {e}"))?;
    Ok(out)
}

/// The `image` crate's plain JPEG encoder; kept only as the size baseline in tests.
pub fn jpeg_baseline(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, quality).encode_image(&DynamicImage::ImageRgb8(flatten(img)))?;
    Ok(out)
}

pub fn encode(img: &DynamicImage, fmt: Format, icc: Option<&[u8]>, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let (w, h) = (img.width(), img.height());
    match fmt {
        Format::Jpg => {
            let gray = img.color().channel_count() == 1 && !img.color().has_alpha();
            out = if gray {
                jpeg(img.to_luma8().as_raw(), w, h, true, quality, icc)?
            } else {
                jpeg(flatten(img).as_raw(), w, h, false, quality, icc)?
            };
        }
        Format::Png => {
            let mut enc = PngEncoder::new(&mut out);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p.to_vec());
            }
            if img.color().has_alpha() {
                enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
            } else {
                enc.write_image(img.to_rgb8().as_raw(), w, h, ExtendedColorType::Rgb8)?;
            }
        }
        Format::Webp => {
            let rgba = img.to_rgba8();
            // Effort 2 (of 0-6) + threads: measured 2.1x faster than the default 4 on a
            // 12 MP photo, 6% smaller, -0.56 dB at ~40 dB PSNR (not visible).
            // libwebp's SSE4.1/AVX2 builds were left off: they'd crash on older CPUs.
            let mut cfg = webp::WebPConfig::new().map_err(|_| anyhow!("WebP encoder setup failed"))?;
            cfg.quality = quality as f32;
            cfg.method = 2;
            cfg.thread_level = 1;
            let mem = webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode_advanced(&cfg).map_err(|e| anyhow!("WebP encode: {e:?}"))?;
            out = mem.to_vec();
        }
        Format::Tiff => {
            use tiff::encoder::{colortype, compression::Deflate, TiffEncoder};
            let mut cur = Cursor::new(Vec::new());
            let mut enc = TiffEncoder::new(&mut cur)?;
            if img.color().has_alpha() {
                enc.write_image_with_compression::<colortype::RGBA8, _>(w, h, Deflate::default(), img.to_rgba8().as_raw())?;
            } else {
                enc.write_image_with_compression::<colortype::RGB8, _>(w, h, Deflate::default(), img.to_rgb8().as_raw())?;
            }
            out = cur.into_inner();
        }
        Format::Bmp => {
            BmpEncoder::new(&mut out).write_image(flatten(img).as_raw(), w, h, ExtendedColorType::Rgb8)?;
        }
        Format::Avif => {
            // Speed 8 of 10: a few seconds for a phone photo, still far smaller than JPEG.
            AvifEncoder::new_with_speed_quality(&mut out, 8, quality).write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
        }
        Format::Heic => out = wic::encode_heic(img)?,
        Format::Gif | Format::Pdf | Format::Docx => return Err(anyhow!("{} isn't an image output", fmt.label())),
    }
    Ok(out)
}
