//! Runs dropped actions one at a time on a worker thread and drives the progress card
//! (the "toast" window) next to where the file was dropped.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use shiftdrop_core::actions;
use shiftdrop_core::naming::OutputDir;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

/// Toast window size in logical px; matches the card layout in src/Toast.tsx.
const TOAST_W: f64 = 340.0;
const TOAST_H: f64 = 156.0;
/// Card sits this far below-right of the drop point.
const TOAST_OFFSET: f64 = 28.0;

pub static CANCEL: AtomicBool = AtomicBool::new(false);
/// Card ids; shared with editor saves so the card always shows the latest event.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct JobRequest {
    pub action: String,
    pub paths: Vec<PathBuf>,
    /// Screen point (physical px) where the drop happened.
    pub anchor: (i32, i32),
    pub out: OutputDir,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JobEvent {
    Start { id: u64, title: String, detail: String, queued: usize },
    Progress { id: u64, fraction: f32, detail: String },
    Done {
        id: u64,
        title: String,
        summary: String,
        outputs: Vec<PathBuf>,
        errors: Vec<String>,
        cancelled: bool,
        /// Stays until closed instead of hiding after a few seconds.
        #[serde(default)]
        sticky: bool,
        /// Optional button, e.g. "Install & restart".
        action: Option<CardAction>,
    },
}

/// A button on a card that runs a Tauri command.
#[derive(Serialize, Clone)]
pub struct CardAction {
    pub label: String,
    pub command: String,
}

#[derive(Clone)]
pub struct Jobs {
    tx: Sender<JobRequest>,
    queued: Arc<AtomicUsize>,
}

impl Jobs {
    pub fn start(app: AppHandle) -> Jobs {
        let (tx, rx) = channel::<JobRequest>();
        let queued = Arc::new(AtomicUsize::new(0));
        let q = queued.clone();
        std::thread::Builder::new()
            .name("shiftdrop-jobs".into())
            .spawn(move || {
                shiftdrop_core::wic::init_thread();
                for req in rx {
                    let id = NEXT_ID.fetch_add(1, SeqCst);
                    let waiting = q.fetch_sub(1, SeqCst).saturating_sub(1);
                    // A crash in one job must not stop the queue.
                    crate::crash::guarded("job", || {
                        if !actions::is_quick(&req.action) {
                            crate::editor::open(&app, &req.action, &req.paths[0], req.anchor);
                            return;
                        }
                        place_toast(&app, req.anchor);
                        run_one(&app, id, req, waiting);
                    });
                }
            })
            .expect("spawn jobs thread");
        Jobs { tx, queued }
    }

    pub fn submit(&self, req: JobRequest) {
        self.queued.fetch_add(1, SeqCst);
        let _ = self.tx.send(req);
    }
}

fn emit(app: &AppHandle, e: JobEvent) {
    let _ = app.emit_to("toast", "job", e);
}

fn run_one(app: &AppHandle, id: u64, req: JobRequest, queued: usize) {
    CANCEL.store(false, SeqCst);
    let detail = format!("1 of {} file(s)", req.paths.len());
    emit(app, JobEvent::Start { id, title: actions::title(&req.action), detail, queued });
    let started = Instant::now();
    log::info!("job {id}: {} on {} file(s)", req.action, req.paths.len());

    let mut last = Instant::now() - Duration::from_secs(1);
    let outcome = crate::crash::guarded("action", || {
        crash_test("job");
        actions::run(&req.action, &req.paths, &req.out, &CANCEL, &mut |p| {
            // Throttle: the card only needs ~20 updates a second.
            if last.elapsed() >= Duration::from_millis(50) {
                last = Instant::now();
                emit(app, JobEvent::Progress { id, fraction: p.fraction, detail: p.detail });
            }
        })
    })
    .unwrap_or_else(|| actions::Outcome::failed("File Beaver hit an internal error on this file. A crash report was saved on this PC.".into()));
    log::info!(
        "job {id} finished in {:.1}s: {} | outputs {:?} | errors {:?}",
        started.elapsed().as_secs_f32(),
        outcome.summary,
        outcome.outputs,
        outcome.errors
    );
    emit(
        app,
        JobEvent::Done {
            id,
            title: actions::done_title(&req.action, &outcome),
            summary: outcome.summary.clone(),
            outputs: outcome.outputs,
            errors: outcome.errors,
            cancelled: outcome.cancelled,
            sticky: false,
            action: None,
        },
    );
}

/// Debug builds only: SHIFTDROP_CRASH_TEST=job makes the next job panic and
/// SHIFTDROP_CRASH_TEST=native makes it crash natively, to exercise the crash handling.
fn crash_test(point: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    match std::env::var("SHIFTDROP_CRASH_TEST").as_deref() {
        Ok("job") if point == "job" => panic!("crash test: deliberate panic in a job"),
        Ok("native") if point == "job" => unsafe { std::ptr::null_mut::<u8>().write_volatile(1) },
        _ => {}
    }
}

/// A card with a message that stays until closed, e.g. the crash notice at startup.
pub fn card_message(app: &AppHandle, anchor: (i32, i32), title: String, summary: String, outputs: Vec<PathBuf>, action: Option<CardAction>) {
    let id = NEXT_ID.fetch_add(1, SeqCst);
    place_toast(app, anchor);
    emit(app, JobEvent::Done { id, title, summary, outputs, errors: vec![], cancelled: false, sticky: true, action });
}

/// Card for work done outside the queue (editor saves): shows "running" now and returns
/// a closure that shows the result.
pub fn card_start(app: &AppHandle, anchor: (i32, i32), title: String) -> impl FnOnce(String, Result<Vec<PathBuf>, String>) + '_ {
    let id = NEXT_ID.fetch_add(1, SeqCst);
    place_toast(app, anchor);
    emit(app, JobEvent::Start { id, title, detail: "Saving…".into(), queued: 0 });
    move |done_title, result| {
        let (outputs, errors, summary) = match result {
            Ok(out) => {
                let name = out.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (out, vec![], format!("Saved {name}"))
            }
            Err(e) => (vec![], vec![e], "Failed".into()),
        };
        emit(app, JobEvent::Done { id, title: done_title, summary, outputs, errors, cancelled: false, sticky: false, action: None });
    }
}

/// Puts the card below-right of the drop point, kept inside that monitor's work area,
/// and shows it without taking focus.
fn place_toast(app: &AppHandle, (x, y): (i32, i32)) {
    let Some(win) = app.get_webview_window("toast") else { return };
    // Fall back to the main screen if the point is off every monitor.
    let mon = app.monitor_from_point(x as f64, y as f64).ok().flatten().or_else(|| app.primary_monitor().ok().flatten());
    let scale = mon.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let (w, h) = ((TOAST_W * scale) as i32, (TOAST_H * scale) as i32);
    let off = (TOAST_OFFSET * scale) as i32;
    let (mut px, mut py) = (x + off, y + off);
    if let Some(m) = &mon {
        let wa = m.work_area();
        let (l, t) = (wa.position.x, wa.position.y);
        let (r, b) = (l + wa.size.width as i32, t + wa.size.height as i32);
        if px + w > r {
            px = x - off - w;
        }
        if py + h > b {
            py = y - off - h;
        }
        px = px.clamp(l, (r - w).max(l));
        py = py.clamp(t, (b - h).max(t));
        // Never sit on the beaver (it lives in the same corner by default): go above it,
        // or beside it when there's no room above.
        if let Some((bl, bt, br, bb)) = crate::beaver::visible_rect() {
            if px < br && px + w > bl && py < bb && py + h > bt {
                if bt - off - h >= t {
                    py = bt - off - h;
                } else if bl - off - w >= l {
                    px = bl - off - w;
                } else {
                    px = (br + off).min((r - w).max(l));
                }
            }
        }
    }
    log::info!("card at ({px}, {py}) size {w}x{h}");
    let _ = win.set_position(PhysicalPosition::new(px, py));
    // Moving onto a monitor with another DPI can resize it; set the size after moving.
    let _ = win.set_size(PhysicalSize::new(w as u32, h as u32));
    if let Ok(hwnd) = win.hwnd() {
        crate::trigger::show_no_activate(hwnd.0 as isize);
    }
}
