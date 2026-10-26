//! Editing PDFs from the editor window.
//! - Crop sets each page's CropBox: lossless, text stays selectable.
//! - Annotate appends vector drawing to each page: original content untouched.
//! - Redact re-renders every page to an image with the boxes burned in, so the
//!   hidden text and images are really gone (not just covered).
//!
//! The editor works in "view" coordinates: 0..1 across the page as displayed
//! (CropBox, after /Rotate), origin top-left. `PageGeom` maps that to PDF space.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use image::RgbImage;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::Deserialize;

use crate::pdf::{self, Page, PageImage};

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactMode {
    Black,
    Pixelate,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Redaction {
    #[serde(flatten)]
    pub rect: Rect,
    pub mode: RedactMode,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Arrow,
    Rect,
    Ellipse,
    Pen,
    Highlight,
    Text,
    Step,
}

/// One annotation. Must stay in step with src/editor/render.ts, which draws the same
/// shapes on images and for the preview.
#[derive(Deserialize, Clone, Debug)]
pub struct Shape {
    pub kind: ShapeKind,
    /// "#rrggbb"
    pub color: String,
    /// Stroke width as a fraction of the page diagonal.
    pub width: f64,
    /// View-normalised points: two corners for rect/ellipse, start+end for arrow,
    /// the path for pen/highlight, the anchor for text/step.
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub text: String,
    /// Text: font size as a fraction of page height. Step: badge radius, same unit.
    #[serde(default)]
    pub size: f64,
    #[serde(default)]
    pub n: u32,
}

/// Highlighter opacity, shared with the canvas renderer.
pub const HIGHLIGHT_ALPHA: f64 = 0.35;

// ---------- page geometry ----------

fn num(doc: &Document, o: &Object) -> Option<f64> {
    match doc.dereference(o).ok()?.1 {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

/// A page attribute, looked up through /Parent like PDF inheritance requires.
fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut id = page;
    for _ in 0..64 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(v) = dict.get(key) {
            return doc.dereference(v).ok().map(|(_, o)| o.clone());
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn rect_of(doc: &Document, o: &Object) -> Option<[f64; 4]> {
    let a = o.as_array().ok()?;
    if a.len() != 4 {
        return None;
    }
    let v: Vec<f64> = a.iter().filter_map(|x| num(doc, x)).collect();
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

#[derive(Clone, Copy, Debug)]
pub struct PageGeom {
    llx: f64,
    lly: f64,
    urx: f64,
    ury: f64,
    /// 0, 90, 180 or 270, clockwise as displayed.
    rot: i64,
}

impl PageGeom {
    pub fn of(doc: &Document, page: ObjectId) -> Result<PageGeom> {
        let media = inherited(doc, page, b"MediaBox").and_then(|o| rect_of(doc, &o)).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let crop = inherited(doc, page, b"CropBox").and_then(|o| rect_of(doc, &o)).unwrap_or(media);
        let b = [crop[0].max(media[0]), crop[1].max(media[1]), crop[2].min(media[2]), crop[3].min(media[3])];
        let rot = inherited(doc, page, b"Rotate").and_then(|o| num(doc, &o)).unwrap_or(0.0) as i64;
        Ok(PageGeom { llx: b[0], lly: b[1], urx: b[2], ury: b[3], rot: rot.rem_euclid(360) / 90 * 90 })
    }

    /// Page size as displayed, in points.
    pub fn view_size(&self) -> (f64, f64) {
        let (w, h) = (self.urx - self.llx, self.ury - self.lly);
        if self.rot % 180 == 0 { (w, h) } else { (h, w) }
    }

    /// [a b c d e f] mapping view points (top-left origin, y down) to PDF user space.
    pub fn matrix(&self) -> [f64; 6] {
        match self.rot {
            90 => [0.0, 1.0, 1.0, 0.0, self.llx, self.lly],
            180 => [-1.0, 0.0, 0.0, 1.0, self.urx, self.lly],
            270 => [0.0, -1.0, -1.0, 0.0, self.urx, self.ury],
            _ => [1.0, 0.0, 0.0, -1.0, self.llx, self.ury],
        }
    }

    pub fn to_pdf(&self, vx: f64, vy: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.matrix();
        (a * vx + c * vy + e, b * vx + d * vy + f)
    }
}

fn load(data: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(data).map_err(|_| anyhow::anyhow!("this PDF is damaged or uses a structure File Beaver can't read"))?;
    if doc.is_encrypted() {
        bail!("password-protected PDFs aren't supported");
    }
    Ok(doc)
}

fn save(mut doc: Document) -> Result<Vec<u8>> {
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

// ---------- crop ----------

/// Crops pages to `rect` (view-normalised). `page` = Some(0-based) for one page, None for all.
pub fn crop(data: &[u8], rect: Rect, page: Option<u32>) -> Result<Vec<u8>> {
    let mut doc = load(data)?;
    for (num, id) in doc.get_pages() {
        if page.is_some_and(|p| p + 1 != num) {
            continue;
        }
        let g = PageGeom::of(&doc, id)?;
        let (wv, hv) = g.view_size();
        let corners = [
            g.to_pdf(rect.x * wv, rect.y * hv),
            g.to_pdf((rect.x + rect.w) * wv, (rect.y + rect.h) * hv),
        ];
        let (x0, x1) = (corners[0].0.min(corners[1].0), corners[0].0.max(corners[1].0));
        let (y0, y1) = (corners[0].1.min(corners[1].1), corners[0].1.max(corners[1].1));
        let dict = doc.get_dictionary_mut(id)?;
        dict.set("CropBox", vec![Object::Real(x0 as f32), Object::Real(y0 as f32), Object::Real(x1 as f32), Object::Real(y1 as f32)]);
    }
    save(doc)
}

// ---------- annotate ----------

fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    (p(0), p(2), p(4))
}

/// Latin text for the standard Helvetica font; characters it can't show become '?'.
fn pdf_text(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            c if (c as u32) >= 32 && (c as u32) < 256 => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// Width of digits in Helvetica, used to centre step numbers.
const HELV_DIGIT_EM: f64 = 0.556;

/// PDF operators for `shapes`, in view points (the caller sets the view->PDF matrix).
pub fn shape_ops(shapes: &[Shape], wv: f64, hv: f64) -> String {
    let diag = (wv * wv + hv * hv).sqrt();
    let mut o = String::new();
    let pt = |p: &[f64; 2]| (p[0] * wv, p[1] * hv);
    for s in shapes {
        let (r, g, b) = rgb(&s.color);
        let lw = (s.width * diag).max(0.1);
        let _ = write!(o, "q {r:.3} {g:.3} {b:.3} RG {r:.3} {g:.3} {b:.3} rg {lw:.2} w 1 J 1 j\n");
        match s.kind {
            ShapeKind::Rect | ShapeKind::Ellipse if s.points.len() >= 2 => {
                let (x0, y0) = pt(&s.points[0]);
                let (x1, y1) = pt(&s.points[1]);
                let (x, y, w, h) = (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
                if s.kind == ShapeKind::Rect {
                    let _ = write!(o, "{x:.2} {y:.2} {w:.2} {h:.2} re S\n");
                } else {
                    // Four Bezier quarter arcs.
                    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
                    let k = 0.552_284_8;
                    let _ = write!(
                        o,
                        "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c S\n",
                        cx + rx,
                        cx + rx, cy + ry * k, cx + rx * k, cy + ry, cy + ry,
                        cx - rx * k, cy + ry, cx - rx, cy + ry * k, cx - rx,
                        cx - rx, cy - ry * k, cx - rx * k, cy - ry, cy - ry,
                        cx + rx * k, cy - ry, cx + rx, cy - ry * k, cx + rx,
                    );
                }
            }
            ShapeKind::Arrow if s.points.len() >= 2 => {
                let (x0, y0) = pt(&s.points[0]);
                let (x1, y1) = pt(&s.points[1]);
                let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt().max(0.001);
                let (dx, dy) = ((x1 - x0) / len, (y1 - y0) / len);
                let head = (lw * 4.0).min(len);
                let half = lw * 2.4;
                let (bx, by) = (x1 - dx * head, y1 - dy * head);
                let _ = write!(o, "{x0:.2} {y0:.2} m {:.2} {:.2} l S\n", x1 - dx * head * 0.9, y1 - dy * head * 0.9);
                let _ = write!(o, "{x1:.2} {y1:.2} m {:.2} {:.2} l {:.2} {:.2} l f\n", bx - dy * half, by + dx * half, bx + dy * half, by - dx * half);
            }
            ShapeKind::Pen | ShapeKind::Highlight if !s.points.is_empty() => {
                if s.kind == ShapeKind::Highlight {
                    o.push_str("/GsdHL gs\n");
                }
                let (x, y) = pt(&s.points[0]);
                let _ = write!(o, "{x:.2} {y:.2} m");
                if s.points.len() == 1 {
                    let _ = write!(o, " {x:.2} {y:.2} l");
                }
                for p in &s.points[1..] {
                    let (x, y) = pt(p);
                    let _ = write!(o, " {x:.2} {y:.2} l");
                }
                o.push_str(" S\n");
            }
            ShapeKind::Text if !s.points.is_empty() => {
                let fs = s.size * hv;
                let (x, y) = pt(&s.points[0]);
                for (i, line) in s.text.lines().enumerate() {
                    let base = y + fs * 0.85 + i as f64 * fs * 1.2;
                    // y is flipped in view space, so flip glyphs back.
                    let _ = write!(o, "BT /FsdH 1 Tf {fs:.2} 0 0 {:.2} {x:.2} {base:.2} Tm ({}) Tj ET\n", -fs, pdf_text(line));
                }
            }
            ShapeKind::Step if !s.points.is_empty() => {
                let rad = s.size * hv;
                let (cx, cy) = pt(&s.points[0]);
                let k = 0.552_284_8 * rad;
                let _ = write!(
                    o,
                    "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c f\n",
                    cx + rad,
                    cx + rad, cy + k, cx + k, cy + rad, cy + rad,
                    cx - k, cy + rad, cx - rad, cy + k, cx - rad,
                    cx - rad, cy - k, cx - k, cy - rad, cy - rad,
                    cx + k, cy - rad, cx + rad, cy - k, cx + rad,
                );
                let label = s.n.to_string();
                let fs = rad * 1.1;
                let tw = label.len() as f64 * HELV_DIGIT_EM * fs;
                let _ = write!(o, "1 1 1 rg BT /FsdH 1 Tf {fs:.2} 0 0 {:.2} {:.2} {:.2} Tm ({label}) Tj ET\n", -fs, cx - tw / 2.0, cy + fs * 0.35);
            }
            _ => {}
        }
        o.push_str("Q\n");
    }
    o
}

/// A dictionary-valued entry of `res`, resolved and cloned (or empty).
fn sub_dict(doc: &Document, res: &Dictionary, key: &[u8]) -> Dictionary {
    res.get(key)
        .ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok().cloned())
        .unwrap_or_default()
}

/// Adds annotations. `shapes` maps 0-based page index to that page's shapes.
pub fn annotate(data: &[u8], shapes: &BTreeMap<u32, Vec<Shape>>) -> Result<Vec<u8>> {
    let mut doc = load(data)?;
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let gs = doc.add_object(dictionary! { "Type" => "ExtGState", "CA" => Object::Real(HIGHLIGHT_ALPHA as f32), "ca" => Object::Real(HIGHLIGHT_ALPHA as f32) });

    for (num, id) in doc.get_pages() {
        let Some(page_shapes) = shapes.get(&(num - 1)).filter(|s| !s.is_empty()) else { continue };
        let g = PageGeom::of(&doc, id)?;
        let (wv, hv) = g.view_size();
        let [a, b, c, d, e, f] = g.matrix();
        let ops = format!("q {a} {b} {c} {d} {e:.3} {f:.3} cm\n{}Q\n", shape_ops(page_shapes, wv, hv));

        // Our own copy of the page's resources, plus our font and transparency state.
        let mut res = inherited(&doc, id, b"Resources").and_then(|o| o.as_dict().ok().cloned()).unwrap_or_default();
        let mut fonts = sub_dict(&doc, &res, b"Font");
        fonts.set("FsdH", Object::Reference(font));
        let mut states = sub_dict(&doc, &res, b"ExtGState");
        states.set("GsdHL", Object::Reference(gs));
        res.set("Font", fonts);
        res.set("ExtGState", states);

        // Wrap the original content in q/Q so its graphics state can't leak into ours.
        let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
        let close = doc.add_object(Stream::new(Dictionary::new(), b"\nQ\n".to_vec()));
        let ours = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
        let page = doc.get_dictionary(id)?;
        let mut contents = vec![Object::Reference(open)];
        match page.get(b"Contents") {
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(arr)) => contents.extend(arr.iter().cloned()),
                _ => contents.push(Object::Reference(*r)),
            },
            Ok(Object::Array(arr)) => contents.extend(arr.iter().cloned()),
            _ => {}
        }
        contents.push(Object::Reference(close));
        contents.push(Object::Reference(ours));
        let page = doc.get_dictionary_mut(id)?;
        page.set("Contents", contents);
        page.set("Resources", res);
    }
    save(doc)
}

// ---------- redact ----------

/// Pixelate block size for an image, shared with the canvas renderer.
pub fn pixel_block(w: u32, h: u32) -> u32 {
    ((w.max(h) as f64 / 80.0).round() as u32).max(8)
}

/// Burns redaction boxes into an image (view-normalised rects).
pub fn apply_redactions(img: &mut RgbImage, boxes: &[Redaction]) {
    let (iw, ih) = img.dimensions();
    let block = pixel_block(iw, ih);
    for r in boxes {
        let x0 = (r.rect.x * iw as f64).floor().clamp(0.0, iw as f64) as u32;
        let y0 = (r.rect.y * ih as f64).floor().clamp(0.0, ih as f64) as u32;
        let x1 = ((r.rect.x + r.rect.w) * iw as f64).ceil().clamp(0.0, iw as f64) as u32;
        let y1 = ((r.rect.y + r.rect.h) * ih as f64).ceil().clamp(0.0, ih as f64) as u32;
        match r.mode {
            RedactMode::Black => {
                for y in y0..y1 {
                    for x in x0..x1 {
                        img.put_pixel(x, y, image::Rgb([0, 0, 0]));
                    }
                }
            }
            RedactMode::Pixelate => {
                let mut by = y0;
                while by < y1 {
                    let mut bx = x0;
                    let bh = block.min(y1 - by);
                    while bx < x1 {
                        let bw = block.min(x1 - bx);
                        let mut sum = [0u64; 3];
                        for y in by..by + bh {
                            for x in bx..bx + bw {
                                let p = img.get_pixel(x, y);
                                for c in 0..3 {
                                    sum[c] += p[c] as u64;
                                }
                            }
                        }
                        let n = (bw * bh) as u64;
                        let avg = image::Rgb([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]);
                        for y in by..by + bh {
                            for x in bx..bx + bw {
                                img.put_pixel(x, y, avg);
                            }
                        }
                        bx += bw;
                    }
                    by += bh;
                }
            }
        }
    }
}

/// Re-renders every page at `dpi` with the boxes burned in. `boxes` maps 0-based page
/// index to that page's boxes. Every page is flattened so nothing survives as text.
/// Pages are rendered in parallel on the worker pool.
pub fn redact(path: &Path, boxes: &BTreeMap<u32, Vec<Redaction>>, dpi: f64, progress: &mut (dyn FnMut(u32, u32) + Send)) -> Result<Vec<u8>> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    let n = pdf::open(path)?.page_count()?;
    let done = AtomicU32::new(0);
    let progress = std::sync::Mutex::new(progress);
    let pages: Vec<Result<Page>> = crate::pool::get().install(|| {
        (0..n)
            .into_par_iter()
            .map(|i| {
                let mut img = crate::pool::render_page(path, i, dpi).with_context(|| format!("rendering page {}", i + 1))?.to_rgb8();
                if let Some(b) = boxes.get(&i) {
                    apply_redactions(&mut img, b);
                }
                let (width, height) = img.dimensions();
                let bytes = pdf::jpeg_bytes(&image::DynamicImage::ImageRgb8(img), 90)?;
                let k = done.fetch_add(1, Ordering::SeqCst) + 1;
                if let Ok(mut p) = progress.try_lock() {
                    p(k, n);
                }
                Ok(Page { image: PageImage::Jpeg { bytes, width, height, gray: false }, dpi })
            })
            .collect()
    });
    let pages = pages.into_iter().collect::<Result<Vec<_>>>()?;
    pdf::write_images(&pages)
}
