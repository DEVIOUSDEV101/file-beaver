//! The editor window (Crop, Add BG, Edit, Annotate, Redact).
//!
//! Images are edited on a canvas in the web view; the finished pixels come back here as
//! PNG and are saved in the source's format. PDFs are edited here in Rust (crop box,
//! vector annotations, flattening redaction), from the shapes the web view sends.
//! Only the file that was dropped can be read or written: the web view never sends paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use shiftdrop_core::encode::encode;
use shiftdrop_core::formats::{Caps, Format};
use shiftdrop_core::naming;
use shiftdrop_core::pdf_edit::{self, Rect, Redaction, Shape};
use shiftdrop_core::{decode, pdf, wic};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

use crate::{jobs, trigger, AppState};

/// Editor window size in logical px.
const WIN_W: f64 = 1040.0;
const WIN_H: f64 = 740.0;
/// PDF pages are shown at this resolution while editing.
const PREVIEW_DPI: f64 = 150.0;
/// Redacted PDFs are flattened at this resolution.
const REDACT_DPI: f64 = 200.0;
/// Browser canvas limits (Chromium): per side, and total pixels with some headroom.
const MAX_EDIT_SIDE: u32 = 16_384;
const MAX_EDIT_PIXELS: u64 = 200_000_000;

struct Session {
    tool: String,
    path: PathBuf,
    is_pdf: bool,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Serialize, Clone)]
struct OpenEvent {
    /// "crop", "add_bg", "edit", "annotate", "redact"
    tool: String,
    name: String,
    kind: &'static str,
    pages: u32,
}

fn tool_label(tool: &str) -> (&'static str, &'static str) {
    // (window title, file-name suffix)
    match tool {
        "crop" => ("Crop", "Cropped"),
        "add_bg" => ("Add Background", "With BG"),
        "edit" => ("Edit", "Edited"),
        "annotate" => ("Annotate", "Annotated"),
        "redact" => ("Redact", "Redacted"),
        _ => ("Edit", "Edited"),
    }
}

/// Opens the editor for `action` ("tool:crop") on `path`, centred on the drop's monitor.
pub fn open(app: &AppHandle, action: &str, path: &Path, anchor: (i32, i32)) {
    let tool = action.trim_start_matches("tool:").to_string();
    let is_pdf = Format::from_path(path) == Some(Format::Pdf);
    let pages = if is_pdf {
        match pdf::open(path).and_then(|d| d.page_count()) {
            Ok(n) => n,
            Err(e) => {
                let finish = jobs::card_start(app, anchor, "Can't open".into());
                finish("Can't open this PDF".into(), Err(format!("{e:#}")));
                return;
            }
        }
    } else {
        1
    };
    // The editor draws on a browser canvas, which can't go past 16,384 px a side or
    // ~268 million pixels. Check the header (cheap) before opening a broken editor.
    if !is_pdf {
        if let Ok((w, h)) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| e.to_string()).and_then(|r| r.into_dimensions().map_err(|e| e.to_string())) {
            if w > MAX_EDIT_SIDE || h > MAX_EDIT_SIDE || (w as u64 * h as u64) > MAX_EDIT_PIXELS {
                let finish = jobs::card_start(app, anchor, "Too large to edit".into());
                finish(
                    "Too large to edit".into(),
                    Err(format!("This image is {w} × {h}. The editor handles up to {MAX_EDIT_SIDE} px per side; Convert and Compress still work.")),
                );
                return;
            }
        }
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session { tool: tool.clone(), path: path.to_path_buf(), is_pdf });

    let Some(win) = app.get_webview_window("editor") else { return };
    let (title, _) = tool_label(&tool);
    let _ = win.set_title(&format!("{title} · {name}"));
    if let Ok(Some(m)) = app.monitor_from_point(anchor.0 as f64, anchor.1 as f64) {
        let s = m.scale_factor();
        let wa = m.work_area();
        // set_size sets the inside of the window; leave room for the title bar and borders
        // so the whole window (and its Save button) fits above the taskbar.
        let (fw, fh) = match (win.outer_size(), win.inner_size()) {
            (Ok(o), Ok(i)) => (o.width.saturating_sub(i.width), o.height.saturating_sub(i.height)),
            _ => ((16.0 * s) as u32, (40.0 * s) as u32),
        };
        let w = ((WIN_W * s) as u32).min(wa.size.width.saturating_sub(fw));
        let h = ((WIN_H * s) as u32).min(wa.size.height.saturating_sub(fh));
        let x = wa.position.x + (wa.size.width as i32 - (w + fw) as i32) / 2;
        let y = wa.position.y + (wa.size.height as i32 - (h + fh) as i32) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, y));
        let _ = win.set_size(PhysicalSize::new(w, h));
    }
    let _ = app.emit_to("editor", "editor:open", OpenEvent { tool, name, kind: if is_pdf { "pdf" } else { "image" }, pages });
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    log::info!("editor opened: {action} on {}", path.display());
}

fn session_path(tool_must_be: Option<&str>) -> Result<(PathBuf, bool, String), String> {
    let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    let s = s.as_ref().ok_or("no file is open in the editor")?;
    if tool_must_be.is_some_and(|t| t != s.tool) {
        return Err("the editor was reopened with another tool".into());
    }
    Ok((s.path.clone(), s.is_pdf, s.tool.clone()))
}

/// Can the web view decode this format itself? (Otherwise we send it as PNG.)
fn browser_can_show(f: Option<Format>) -> bool {
    matches!(f, Some(Format::Jpg | Format::Png | Format::Webp | Format::Gif | Format::Bmp | Format::Avif))
}

/// Bytes of the open image, or of PDF page `page` rendered to PNG.
#[tauri::command]
pub async fn editor_file(page: u32) -> Result<tauri::ipc::Response, String> {
    let (path, is_pdf, _) = session_path(None)?;
    let started = std::time::Instant::now();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        wic::init_thread();
        let png = |img: &image::DynamicImage| encode(img, Format::Png, None, 0).map_err(|e| format!("{e:#}"));
        if is_pdf {
            let img = pdf::open(&path).and_then(|d| d.render(page, PREVIEW_DPI)).map_err(|e| format!("{e:#}"))?;
            // Only a preview (saves work from the PDF itself), so a quick JPEG is plenty.
            pdf::jpeg_bytes(&img, 90).map_err(|e| format!("{e:#}"))
        } else if browser_can_show(Format::from_path(&path)) {
            std::fs::read(&path).map_err(|e| e.to_string())
        } else {
            png(&decode::load(&path).map_err(|e| format!("{e:#}"))?.image)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    log::info!("editor_file page {page}: {} ms", started.elapsed().as_millis());
    result.map(tauri::ipc::Response::new)
}

/// Output format for an edited image: the source's, when we can write it.
fn image_output_format(src: &Path, caps: &Caps) -> Format {
    match Format::from_path(src) {
        Some(f) if f.is_image() && caps.can_write(f) => f,
        Some(Format::Heic) => Format::Jpg,
        _ => Format::Png,
    }
}

fn out_dir(app: &AppHandle, src: &Path) -> PathBuf {
    let out = crate::output_dir(&app.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()));
    naming::target_dir(src, &out)
}

fn finish_editor(app: &AppHandle) {
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some(w) = app.get_webview_window("editor") {
        let _ = w.hide();
    }
}

/// Saves the edited image. Body: the finished pixels as PNG. Header x-tool: the tool.
#[tauri::command]
pub async fn editor_save_image(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tool = request.headers().get("x-tool").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (src, is_pdf, _) = session_path(Some(&tool))?;
    if is_pdf {
        return Err("PDFs are saved with editor_save_pdf".into());
    }
    let tauri::ipc::InvokeBody::Raw(png) = request.body() else { return Err("expected image bytes".into()) };
    let png = png.clone();
    let (title, suffix) = tool_label(&tool);
    let finish = jobs::card_start(&app, trigger::cursor_pos(), format!("Saving {}", title.to_lowercase()));
    let dir = out_dir(&app, &src);
    let caps = wic::caps();
    let src2 = src.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        wic::init_thread();
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let fmt = image_output_format(&src2, &caps);
        let quality = if fmt == Format::Jpg { 92 } else { 90 };
        let bytes = encode(&img, fmt, None, quality).map_err(|e| format!("{e:#}"))?;
        naming::write_new(&dir, &format!("{} {suffix}", naming::stem(&src2)), fmt.ext(), &bytes).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?;
    report(&app, finish, title, result)
}

#[derive(Deserialize)]
pub struct CropEdit {
    rect: Rect,
    /// None = every page.
    page: Option<u32>,
}

#[derive(Deserialize)]
pub struct PdfEdit {
    tool: String,
    crop: Option<CropEdit>,
    #[serde(default)]
    shapes: BTreeMap<u32, Vec<Shape>>,
    #[serde(default)]
    redactions: BTreeMap<u32, Vec<Redaction>>,
}

#[tauri::command]
pub async fn editor_save_pdf(app: AppHandle, edit: PdfEdit) -> Result<String, String> {
    let (src, is_pdf, _) = session_path(Some(&edit.tool))?;
    if !is_pdf {
        return Err("not a PDF".into());
    }
    let (title, suffix) = tool_label(&edit.tool);
    let finish = jobs::card_start(&app, trigger::cursor_pos(), format!("Saving {}", title.to_lowercase()));
    let dir = out_dir(&app, &src);
    let src2 = src.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        wic::init_thread();
        let e = |x: anyhow::Error| format!("{x:#}");
        let bytes = match edit.tool.as_str() {
            "crop" => {
                let c = edit.crop.ok_or("no crop area")?;
                pdf_edit::crop(&std::fs::read(&src2).map_err(|x| x.to_string())?, c.rect, c.page).map_err(e)?
            }
            "annotate" => pdf_edit::annotate(&std::fs::read(&src2).map_err(|x| x.to_string())?, &edit.shapes).map_err(e)?,
            "redact" => pdf_edit::redact(&src2, &edit.redactions, REDACT_DPI, &mut |_, _| {}).map_err(e)?,
            other => return Err(format!("{other} isn't available for PDFs")),
        };
        naming::write_new(&dir, &format!("{} {suffix}", naming::stem(&src2)), "pdf", &bytes).map_err(e)
    })
    .await
    .map_err(|e| e.to_string())?;
    report(&app, finish, title, result)
}

fn report(app: &AppHandle, finish: impl FnOnce(String, Result<Vec<PathBuf>, String>), title: &str, result: Result<PathBuf, String>) -> Result<String, String> {
    match result {
        Ok(p) => {
            log::info!("editor saved {}", p.display());
            finish(format!("{title}: saved"), Ok(vec![p.clone()]));
            finish_editor(app);
            Ok(p.to_string_lossy().into_owned())
        }
        Err(e) => {
            log::warn!("editor save failed: {e}");
            finish("Couldn't save".into(), Err(e.clone()));
            Err(e)
        }
    }
}

#[tauri::command]
pub fn editor_close(app: AppHandle) {
    finish_editor(&app);
}
