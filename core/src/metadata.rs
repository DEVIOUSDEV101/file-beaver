//! Metadata: removes EXIF (camera, dates, GPS), XMP, IPTC, comments and PDF document info.
//! JPG, PNG and WEBP are edited byte-for-byte: the image data is copied untouched, so
//! there is no quality loss. A JPEG's rotation flag is kept so photos stay upright.
//! TIFF is re-encoded (pixels identical, tags gone). PDF loses its Info dictionary and XMP.

use std::io::Cursor;
use std::path::Path;

use anyhow::{bail, Context, Result};
use lopdf::{Document, Object};

use crate::decode;
use crate::encode::encode;
use crate::formats::Format;

pub struct Stripped {
    pub bytes: Vec<u8>,
    /// The file's real format (by content), which sets the output's extension.
    pub format: Format,
    /// Human-readable list of what went, e.g. "EXIF (23 fields)", "XMP".
    pub removed: Vec<String>,
    pub had_gps: bool,
}

/// None when there was nothing to remove.
pub fn strip(path: &Path) -> Result<Option<Stripped>> {
    let data = std::fs::read(path)?;
    let (exif_fields, had_gps) = exif_summary(&data);
    if data.is_empty() {
        bail!("the file is empty");
    }
    if let Some(why) = crate::formats::not_really(path) {
        bail!(why);
    }
    let format = crate::formats::detect(path).ok_or_else(|| anyhow::anyhow!("this isn't a file type File Beaver can clean"))?;
    if format == Format::Jpg && !crate::decode::jpeg_is_complete(&data) {
        bail!("this JPEG is incomplete (the file was cut off)");
    }
    let (bytes, mut removed) = match Some(format) {
        Some(Format::Jpg) => strip_jpeg(&data)?,
        Some(Format::Png) => strip_png(&data)?,
        Some(Format::Webp) => strip_webp(&data)?,
        Some(Format::Tiff) => {
            let d = decode::load(path)?;
            (encode(&d.image, Format::Tiff, None, 0)?, vec!["EXIF".to_string()])
        }
        Some(Format::Pdf) => strip_pdf(&data)?,
        _ => bail!("removing metadata isn't supported for this file type"),
    };
    // Put the field count on the EXIF entry when we know it.
    if exif_fields > 0 {
        for r in removed.iter_mut().filter(|r| r.as_str() == "EXIF") {
            *r = format!("EXIF ({exif_fields} fields)");
        }
    } else {
        removed.retain(|r| r != "EXIF" || format != Format::Tiff);
    }
    if removed.is_empty() {
        return Ok(None);
    }
    Ok(Some(Stripped { bytes, format, removed, had_gps }))
}

fn exif_summary(data: &[u8]) -> (usize, bool) {
    match exif::Reader::new().read_from_container(&mut Cursor::new(data)) {
        Ok(e) => (e.fields().count(), e.fields().any(|f| f.tag.context() == exif::Context::Gps)),
        Err(_) => (0, false),
    }
}

fn jpeg_orientation(data: &[u8]) -> u16 {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(data))
        .ok()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .map(|o| o as u16)
        .unwrap_or(1)
}

/// An APP1 segment holding only the orientation tag.
fn orientation_app1(o: u16) -> Vec<u8> {
    let mut p = b"Exif\0\0II*\0".to_vec();
    p.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
    p.extend_from_slice(&1u16.to_le_bytes()); // one entry
    p.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
    p.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    p.extend_from_slice(&1u32.to_le_bytes()); // count
    p.extend_from_slice(&o.to_le_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((p.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(&p);
    seg
}

pub fn strip_jpeg(data: &[u8]) -> Result<(Vec<u8>, Vec<String>)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        bail!("not a valid JPEG");
    }
    let orientation = jpeg_orientation(data);
    let mut out = vec![0xFF, 0xD8];
    let mut removed: Vec<String> = Vec::new();
    let mut i = 2;
    let mut after_app0 = 2; // where to put the orientation segment
    loop {
        if i + 1 >= data.len() || data[i] != 0xFF {
            bail!("damaged JPEG");
        }
        let mut j = i;
        while j < data.len() && data[j] == 0xFF {
            j += 1;
        }
        let marker = *data.get(j).context("damaged JPEG")?;
        // Start of scan / end of image: the rest is image data, copied untouched.
        if marker == 0xDA || marker == 0xD9 {
            out.extend_from_slice(&data[i..]);
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&data[i..=j]);
            i = j + 1;
            continue;
        }
        let len = u16::from_be_bytes([*data.get(j + 1).context("damaged JPEG")?, *data.get(j + 2).context("damaged JPEG")?]) as usize;
        let end = j + 1 + len;
        if end > data.len() || len < 2 {
            bail!("damaged JPEG");
        }
        let payload = &data[j + 3..end];
        let label = match marker {
            0xE1 if payload.starts_with(b"Exif\0") => Some("EXIF"),
            0xE1 if payload.starts_with(b"http://ns.adobe.com/xap") => Some("XMP"),
            0xE1 => Some("XMP"),
            0xED => Some("IPTC"),
            0xFE => Some("Comment"),
            0xE2 if payload.starts_with(b"ICC_PROFILE") => None, // colour profile: keep
            0xE2 => Some("Extra data"),
            0xE0 | 0xEE => None, // JFIF header, Adobe colour transform: keep
            0xE3..=0xEF => Some("Extra data"),
            _ => None,
        };
        match label {
            Some(l) => {
                if !removed.iter().any(|r| r == l) {
                    removed.push(l.to_string());
                }
            }
            None => {
                out.extend_from_slice(&data[i..end]);
                if marker == 0xE0 {
                    after_app0 = out.len();
                }
            }
        }
        i = end;
    }
    if orientation > 1 && orientation <= 8 && !removed.is_empty() {
        let seg = orientation_app1(orientation);
        out.splice(after_app0..after_app0, seg);
    }
    Ok((out, removed))
}

pub fn strip_png(data: &[u8]) -> Result<(Vec<u8>, Vec<String>)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIG) {
        bail!("not a valid PNG");
    }
    let mut out = SIG.to_vec();
    let mut removed: Vec<String> = Vec::new();
    let mut i = 8;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        let end = i + 12 + len;
        if end > data.len() {
            bail!("damaged PNG");
        }
        let label = match &data[i + 4..i + 8] {
            b"tEXt" | b"zTXt" | b"iTXt" => Some("Text fields"),
            b"eXIf" => Some("EXIF"),
            b"tIME" => Some("Timestamp"),
            _ => None,
        };
        match label {
            Some(l) => {
                if !removed.iter().any(|r| r == l) {
                    removed.push(l.to_string());
                }
            }
            None => out.extend_from_slice(&data[i..end]),
        }
        i = end;
    }
    Ok((out, removed))
}

pub fn strip_webp(data: &[u8]) -> Result<(Vec<u8>, Vec<String>)> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        bail!("not a valid WEBP");
    }
    let mut out = data[0..12].to_vec();
    let mut removed: Vec<String> = Vec::new();
    let mut vp8x_flags_at = None;
    let mut i = 12;
    while i + 8 <= data.len() {
        let size = u32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap()) as usize;
        let end = (i + 8 + size + (size & 1)).min(data.len());
        match &data[i..i + 4] {
            b"EXIF" => removed.push("EXIF".into()),
            b"XMP " => removed.push("XMP".into()),
            fourcc => {
                if fourcc == b"VP8X" {
                    vp8x_flags_at = Some(out.len() + 8);
                }
                out.extend_from_slice(&data[i..end]);
            }
        }
        i = end;
    }
    if let Some(at) = vp8x_flags_at {
        out[at] &= !(0x08 | 0x04); // EXIF and XMP present flags
    }
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    Ok((out, removed))
}

pub fn strip_pdf(data: &[u8]) -> Result<(Vec<u8>, Vec<String>)> {
    let mut doc = Document::load_mem(data).map_err(|_| anyhow::anyhow!("this PDF is damaged or uses a structure File Beaver can't read"))?;
    if doc.is_encrypted() {
        bail!("password-protected PDFs aren't supported");
    }
    let mut removed = Vec::new();
    if let Ok(info) = doc.trailer.get(b"Info").cloned() {
        let keys: Vec<String> = match &info {
            Object::Reference(id) => doc.get_dictionary(*id).map(|d| d.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).collect()).unwrap_or_default(),
            Object::Dictionary(d) => d.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).collect(),
            _ => vec![],
        };
        doc.trailer.remove(b"Info");
        if !keys.is_empty() {
            removed.push(format!("Document info ({})", keys.join(", ")));
        }
    }
    if let Ok(catalog) = doc.catalog_mut() {
        if catalog.remove(b"Metadata").is_some() {
            removed.push("XMP".into());
        }
    }
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok((out, removed))
}
