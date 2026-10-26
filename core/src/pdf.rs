//! PDF in and out.
//! - Rendering pages to images uses Windows' built-in PDF engine (Windows.Data.Pdf).
//! - Writing a PDF from images is a small hand-rolled writer: one image per page,
//!   JPEGs embedded as-is when possible (no quality loss), others lossless (Flate).

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{anyhow, Result};
use image::{DynamicImage, ImageDecoder, ImageReader};
use windows::core::HSTRING;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Graphics::Imaging::BitmapEncoder;
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

use crate::wic;

/// Resolution for PDF -> image. 200 dpi keeps small text crisp.
pub const RENDER_DPI: f64 = 200.0;

pub struct PdfDoc(PdfDocument);

pub fn open(path: &Path) -> Result<PdfDoc> {
    wic::init_thread();
    let abs = std::path::absolute(path)?;
    let loaded = if abs.as_os_str().len() < 250 {
        StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.as_os_str())).and_then(|f| f.get()).and_then(|file| PdfDocument::LoadFromFileAsync(&file)?.get())
    } else {
        // Windows' storage APIs can't open paths over 260 characters; Rust can, so
        // read the bytes here and hand the PDF engine a stream instead.
        let bytes = std::fs::read(&abs)?;
        from_bytes(&bytes)
    };
    loaded.map(PdfDoc).map_err(|e| {
        // ERROR_WRONG_PASSWORD and friends
        if matches!(e.code().0 as u32, 0x8007_052B | 0x8007_0056) {
            anyhow!("this PDF is password-protected, and File Beaver can't open protected PDFs")
        } else {
            anyhow!("this PDF is damaged or can't be opened")
        }
    })
}

fn from_bytes(bytes: &[u8]) -> windows::core::Result<PdfDocument> {
    use windows::Storage::Streams::DataWriter;
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.get()?;
    writer.FlushAsync()?.get()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    PdfDocument::LoadFromStreamAsync(&stream)?.get()
}

impl PdfDoc {
    pub fn page_count(&self) -> Result<u32> {
        Ok(self.0.PageCount()?)
    }

    /// Renders page `index` (0-based) on white at exactly `dpi`.
    pub fn render(&self, index: u32, dpi: f64) -> Result<DynamicImage> {
        let page = self.0.GetPage(index)?;
        let size = page.Size()?; // device-independent pixels (1/96 inch)
        let want_w = (size.Width as f64 * dpi / 96.0).round().max(1.0) as u32;
        let want_h = (size.Height as f64 * dpi / 96.0).round().max(1.0) as u32;

        // The renderer multiplies the requested size by the display's scaling, which
        // depends on the PC. Learn that factor from the first render and ask for less.
        let known = f32::from_bits(RENDER_SCALE.load(Ordering::Relaxed));
        let k = if known > 0.0 { known as f64 } else { 1.0 };
        let mut img = render_raw(&page, want_w as f64 / k, want_h as f64 / k)?;
        if img.width().abs_diff(want_w) > 1 {
            let k = img.width() as f64 / (want_w as f64 / k);
            RENDER_SCALE.store((k as f32).to_bits(), Ordering::Relaxed);
            img = render_raw(&page, want_w as f64 / k, want_h as f64 / k)?;
        }
        if (img.width(), img.height()) != (want_w, want_h) {
            img = img.resize_exact(want_w, want_h, image::imageops::FilterType::Lanczos3);
        }
        Ok(img)
    }
}

/// Output/requested size ratio of Windows' PDF renderer on this PC (f32 bits; 0 = unknown).
static RENDER_SCALE: AtomicU32 = AtomicU32::new(0);

fn render_raw(page: &windows::Data::Pdf::PdfPage, w: f64, h: f64) -> Result<DynamicImage> {
    let opts = PdfPageRenderOptions::new()?;
    opts.SetDestinationWidth(w.round().max(1.0) as u32)?;
    opts.SetDestinationHeight(h.round().max(1.0) as u32)?;
    // BMP: uncompressed, so producing and decoding it is near-instant (PNG cost seconds).
    opts.SetBitmapEncoderId(BitmapEncoder::BmpEncoderId()?)?;
    let stream = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&stream, &opts)?.get()?;
    let len = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(len)?.get()?;
    let mut bytes = vec![0u8; len as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(image::load_from_memory(&bytes)?)
}

/// One page's image, ready to embed.
pub enum PageImage {
    /// Original JPEG bytes, embedded untouched.
    Jpeg { bytes: Vec<u8>, width: u32, height: u32, gray: bool },
    Pixels(DynamicImage),
}

impl PageImage {
    /// Uses the file's JPEG bytes directly when that's safe (upright RGB/greyscale JPEG).
    /// The file's own JPEG bytes when they can go into the PDF untouched (an upright,
    /// complete RGB/greyscale JPEG). Reads only the header, never the pixels.
    pub fn jpeg_passthrough(path: &Path) -> Option<PageImage> {
        if crate::formats::sniff(path) != Some(crate::formats::Format::Jpg) {
            return None;
        }
        let mut dec = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        dec.no_limits();
        let mut d = dec.into_decoder().ok()?;
        let upright = d.orientation().ok()? == image::metadata::Orientation::NoTransforms;
        let ct = d.color_type();
        let gray = ct == image::ColorType::L8;
        if !upright || !(gray || ct == image::ColorType::Rgb8) {
            return None;
        }
        let (width, height) = d.dimensions();
        let bytes = std::fs::read(path).ok()?;
        crate::decode::jpeg_is_complete(&bytes).then_some(PageImage::Jpeg { bytes, width, height, gray })
    }

    fn size(&self) -> (u32, u32) {
        match self {
            PageImage::Jpeg { width, height, .. } => (*width, *height),
            PageImage::Pixels(img) => (img.width(), img.height()),
        }
    }
}

/// Composites transparency onto white (PDF/JPEG/DOCX pages have no alpha here).
pub fn flatten(img: &DynamicImage) -> image::RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        for c in 0..3 {
            o[c] = ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

/// A page for [`write_images`]: the image and the resolution it was made at, which sets
/// the page size (96 dpi for ordinary images, the render dpi for rasterised PDF pages).
pub struct Page {
    pub image: PageImage,
    pub dpi: f64,
}

/// A PDF with one image per page, each page exactly the size of its image at its dpi.
pub fn write_images(pages: &[Page]) -> Result<Vec<u8>> {
    if pages.is_empty() {
        return Err(anyhow!("nothing to put in the PDF"));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

    // Object numbers: 1 catalog, 2 pages, then per page: page, contents, image.
    let page_obj = |i: usize| 3 + i * 3;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", page_obj(i))).collect();

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        write!(out, "{n} 0 obj\n").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(
        &mut out,
        &mut offsets,
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes(),
    );

    for (i, Page { image: page, dpi }) in pages.iter().enumerate() {
        let (w, h) = page.size();
        let (pw, ph) = (w as f64 * 72.0 / dpi, h as f64 * 72.0 / dpi);
        let n = page_obj(i);
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                n + 2,
                n + 1
            )
            .as_bytes(),
        );
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        obj(
            &mut out,
            &mut offsets,
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).as_bytes(),
        );

        let (data, filter, cs) = match page {
            PageImage::Jpeg { bytes, gray, .. } => (bytes.clone(), "DCTDecode", if *gray { "DeviceGray" } else { "DeviceRGB" }),
            PageImage::Pixels(img) => {
                let rgb = flatten(img);
                let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(rgb.as_raw())?;
                (z.finish()?, "FlateDecode", "DeviceRGB")
            }
        };
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /{cs} /BitsPerComponent 8 /Filter /{filter} /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &body);
    }

    let xref = out.len();
    write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1)?;
    for off in &offsets {
        write!(out, "{off:010} 00000 n \n")?;
    }
    write!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1)?;
    Ok(out)
}

/// JPEG bytes for embedding a rendered/decoded page somewhere lossy is fine.
pub fn jpeg_bytes(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    crate::encode::jpeg(flatten(img).as_raw(), img.width(), img.height(), false, quality, None)
}
