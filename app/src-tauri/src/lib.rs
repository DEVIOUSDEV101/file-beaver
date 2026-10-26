mod beaver;
mod capture;
mod crash;
mod editor;
mod jobs;
mod settings;
mod trigger;
mod updates;

use std::path::PathBuf;
use std::sync::Mutex;

use settings::{OutputLocation, Settings};
use shiftdrop_core::naming::OutputDir;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};

pub(crate) struct AppState {
    pub(crate) settings: Mutex<Settings>,
    enabled_item: CheckMenuItem<Wry>,
    beaver_item: CheckMenuItem<Wry>,
}

#[derive(Clone, serde::Serialize)]
struct DropEvent {
    action: String,
    paths: Vec<String>,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
    settings::save(&app, &settings).map_err(|e| e.to_string())?;
    settings::apply(&app, &settings);
    let _ = state.enabled_item.set_checked(settings.enabled);
    let _ = state.beaver_item.set_checked(settings.show_beaver);
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    log::info!("settings saved");
    Ok(())
}

/// "Show in folder" on the progress card: opens Explorer with the file selected.
#[tauri::command]
fn reveal_in_folder(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if !std::path::Path::new(&path).exists() {
        return Err("file no longer exists".into());
    }
    std::process::Command::new("explorer")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_toast(app: AppHandle) {
    if let Some(w) = app.get_webview_window("toast") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn cancel_job() {
    jobs::CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Errors from the web windows go to the same log as everything else.
#[tauri::command]
fn log_js_error(view: String, message: String) {
    log::error!("[{view} window] {message}");
}

/// Bottom-right corner of the main screen's work area, for notice cards.
pub(crate) fn primary_corner(app: &AppHandle) -> (i32, i32) {
    app.primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let wa = m.work_area();
            // Inside the screen: its right/bottom edges are exclusive.
            (wa.position.x + wa.size.width as i32 - 1, wa.position.y + wa.size.height as i32 - 1)
        })
        .unwrap_or((800, 600))
}

pub(crate) fn output_dir(s: &Settings) -> OutputDir {
    match s.output {
        OutputLocation::Folder if !s.output_folder.trim().is_empty() => OutputDir::Folder(PathBuf::from(s.output_folder.trim())),
        _ => OutputDir::Beside,
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn set_enabled(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    s.enabled = on;
    trigger::set_enabled(on);
    let _ = state.enabled_item.set_checked(on);
    if let Err(e) = settings::save(app, &s) {
        log::warn!("could not save settings: {e}");
    }
    let _ = app.emit("settings-changed", s.clone());
}

/// Shows or hides the floating beaver and remembers the choice.
fn set_beaver(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    s.show_beaver = on;
    beaver::set_visible(on);
    let _ = state.beaver_item.set_checked(on);
    if let Err(e) = settings::save(app, &s) {
        log::warn!("could not save settings: {e}");
    }
    let _ = app.emit("settings-changed", s.clone());
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // First thing, so even a crash during startup leaves a report.
    let crashed_last_time = crash::install();
    tauri::Builder::default()
        // A second launch just opens the running instance's settings.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .level(log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: Some("shiftdrop".into()) }))
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout))
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            reveal_in_folder,
            hide_toast,
            cancel_job,
            editor::editor_file,
            editor::editor_save_image,
            editor::editor_save_pdf,
            editor::editor_close,
            log_js_error,
            updates::check_for_updates,
            updates::install_update
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let s = settings::load(&handle);

            let enabled_item = CheckMenuItem::with_id(app, "enabled", "Enabled", true, s.enabled, None::<&str>)?;
            let beaver_item = CheckMenuItem::with_id(app, "beaver", "Show beaver", true, s.show_beaver, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit File Beaver", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&enabled_item, &beaver_item, &settings_item, &sep, &quit_item])?;

            TrayIconBuilder::with_id("tray")
                // Flat beaver: the glass app icon turns to mush at tray size.
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .tooltip("File Beaver: drag a file and tap Shift")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "enabled" => {
                        let on = !app.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()).enabled;
                        set_enabled(app, on);
                    }
                    "beaver" => {
                        let on = !app.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()).show_beaver;
                        set_beaver(app, on);
                    }
                    "settings" => show_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            app.manage(AppState { settings: Mutex::new(s.clone()), enabled_item, beaver_item });
            settings::apply(&handle, &s);

            let ring = app.get_webview_window("ring").expect("ring window in tauri.conf.json");
            let ring_hwnd = ring.hwnd()?.0 as isize;
            trigger::prepare_overlay_window(ring_hwnd);
            let toast = app.get_webview_window("toast").expect("toast window in tauri.conf.json");
            trigger::prepare_overlay_window(toast.hwnd()?.0 as isize);

            shiftdrop_core::wic::init_thread();
            let caps = shiftdrop_core::wic::caps();
            log::info!("codecs: {caps:?}");

            let jobs = jobs::Jobs::start(handle.clone());
            let ring_emitter = handle.clone();
            let drop_handle = handle.clone();
            let beaver_handle = handle.clone();
            trigger::start(trigger::Hooks {
                beaver_pos: s.beaver_pos,
                // Runs on the drag thread: quick settings updates only.
                on_beaver: Box::new(move |e| match e {
                    beaver::BeaverEvent::Moved(x, y) => {
                        let state = beaver_handle.state::<AppState>();
                        let mut s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
                        s.beaver_pos = Some((x, y));
                        if let Err(e) = settings::save(&beaver_handle, &s) {
                            log::warn!("could not save the beaver's position: {e}");
                        }
                    }
                    beaver::BeaverEvent::Hidden => set_beaver(&beaver_handle, false),
                    beaver::BeaverEvent::OpenSettings => show_settings(&beaver_handle),
                }),
                ring_hwnd,
                caps,
                on_ring: Box::new(move |e| {
                    let _ = ring_emitter.emit_to("ring", "ring", e);
                }),
                // Runs on the drag thread inside the drop: queue the work and return at once.
                on_drop: Box::new(move |action, paths| {
                    let out = output_dir(&drop_handle.state::<AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()));
                    jobs.submit(jobs::JobRequest {
                        action: action.clone(),
                        paths: paths.iter().map(PathBuf::from).collect(),
                        anchor: trigger::cursor_pos(),
                        out,
                    });
                    let _ = drop_handle.emit("drop", DropEvent { action, paths });
                }),
            });
            log::info!("File Beaver {} started", app.package_info().version);
            updates::start_background_checks(handle.clone());
            if crashed_last_time {
                log::warn!("previous session crashed (a crash report was written)");
                let h = handle.clone();
                std::thread::spawn(move || {
                    // Let the card window finish loading before talking to it.
                    std::thread::sleep(std::time::Duration::from_millis(2500));
                    jobs::card_message(
                        &h,
                        primary_corner(&h),
                        "File Beaver closed unexpectedly".into(),
                        "A crash report was saved on this PC. Nothing was sent.".into(),
                        crash::latest_report().into_iter().collect(),
                        None,
                    );
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing settings hides it; the app keeps running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building File Beaver")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                crash::clean_exit();
            }
        });
}
