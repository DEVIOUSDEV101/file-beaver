//! Auto-update: checks the update feed shortly after startup and then daily. When a
//! newer version exists, a card offers "Install & restart". Updates are signed with the
//! project's private key; the app only accepts ones matching the public key in
//! tauri.conf.json, so a tampered download is refused.

use std::sync::Mutex;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::jobs::{self, CardAction};

/// First check after this delay (don't slow startup), then every day.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

static PENDING: Mutex<Option<Update>> = Mutex::new(None);

pub fn start_background_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST_CHECK).await;
        loop {
            if let Err(e) = check(&app, false).await {
                log::warn!("update check failed: {e}");
            }
            tokio_sleep(EVERY).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    // tauri's async runtime is tokio; avoid a direct tokio dependency for one call.
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// Checks for an update. Shows the card when one exists. Returns a message for Settings.
pub async fn check(app: &AppHandle, from_settings: bool) -> Result<String, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(update) => {
            let version = update.version.clone();
            log::info!("update available: {version}");
            *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(update);
            let anchor = crate::primary_corner(app);
            jobs::card_message(
                app,
                anchor,
                format!("File Beaver {version} is available"),
                "Install now; File Beaver restarts in a few seconds.".into(),
                vec![],
                Some(CardAction { label: "Install & restart".into(), command: "install_update".into() }),
            );
            Ok(format!("Version {version} is available"))
        }
        None => {
            if from_settings {
                log::info!("no update available");
            }
            Ok("You have the latest version".into())
        }
    }
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<String, String> {
    check(&app, true).await
}

/// Downloads, verifies and installs the pending update, then restarts.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take().ok_or("no update is waiting")?;
    log::info!("installing update {}", update.version);
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        log::error!("update install failed: {e}");
        e.to_string()
    })?;
    // On Windows the installer takes over and the app exits; elsewhere, restart.
    app.restart();
}
