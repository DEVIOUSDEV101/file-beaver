//! User settings, stored as JSON in %APPDATA%\app.filebeaver.desktop\settings.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::trigger::{self, TriggerKey};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Shift,
    CtrlShift,
    Alt,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputLocation {
    /// Next to the original file (the reference app's behaviour).
    #[default]
    Beside,
    /// Always into `output_folder`.
    Folder,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub trigger: Trigger,
    pub output: OutputLocation,
    pub output_folder: String,
    pub launch_at_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: Trigger::Shift,
            output: OutputLocation::Beside,
            output_folder: String::new(),
            launch_at_startup: true,
        }
    }
}

fn path(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().expect("config dir").join("settings.json")
}

pub fn load(app: &AppHandle) -> Settings {
    std::fs::read_to_string(path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, s: &Settings) -> std::io::Result<()> {
    let p = path(app);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, serde_json::to_string_pretty(s).unwrap())
}

/// Pushes settings into the running trigger and the OS (autostart).
pub fn apply(app: &AppHandle, s: &Settings) {
    use tauri_plugin_autostart::ManagerExt;
    trigger::set_enabled(s.enabled);
    trigger::set_key(match s.trigger {
        Trigger::Shift => TriggerKey::Shift,
        Trigger::CtrlShift => TriggerKey::CtrlShift,
        Trigger::Alt => TriggerKey::Alt,
    });
    let auto = app.autolaunch();
    let res = if s.launch_at_startup { auto.enable() } else { auto.disable() };
    if let Err(e) = res {
        log::warn!("autostart update failed: {e}");
    }
}
