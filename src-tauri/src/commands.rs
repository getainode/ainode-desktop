//! IPC commands for the app's own pages (settings, waiting, about).

use crate::config::{self, Settings};
use crate::menu::SITE_URL;
use crate::state::AppState;
use crate::{api, windows};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Serialize)]
pub struct SettingsView {
    pub primary: String,
    pub alternate: String,
}

#[derive(Serialize)]
pub struct StatusView {
    pub configured: bool,
    pub primary: String,
    pub alternate: String,
    pub master: Option<String>,
    pub last_error: Option<String>,
    pub node_count: usize,
    /// The name of the node serving right now, when it gave one.
    pub serving_name: Option<String>,
    /// "Connected through Spark-2", or null while on the primary: the pages
    /// show it verbatim rather than each writing the sentence itself.
    pub serving_via: Option<String>,
    /// How many fleet addresses are saved as fallbacks.
    pub fleet_count: usize,
}

#[derive(Serialize)]
pub struct TestResult {
    pub address: String,
    pub node_name: String,
    pub version: String,
    pub model: String,
    pub engine_ready: bool,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub site: String,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    let s = state.settings();
    SettingsView {
        primary: s.primary,
        alternate: s.alternate.unwrap_or_default(),
    }
}

#[tauri::command]
pub fn get_status_view(state: State<'_, AppState>) -> StatusView {
    let s = state.settings();
    let serving = state.serving();
    let fleet_count = s.fleet_candidates().len();
    StatusView {
        configured: s.is_configured(),
        primary: s.primary,
        alternate: s.alternate.unwrap_or_default(),
        master: state.master(),
        last_error: state.last_error(),
        node_count: state.rows().len(),
        serving_name: serving.as_ref().and_then(|v| v.name.clone()),
        serving_via: serving.as_ref().and_then(crate::state::Serving::via_label),
        fleet_count,
    }
}

/// Hit `/api/status` on one address and report who answered.
#[tauri::command]
pub async fn test_address(
    state: State<'_, AppState>,
    address: String,
) -> Result<TestResult, String> {
    let address = config::normalize_address(&address)?;
    let client = state.client.clone();
    let status = api::fetch_status(&client, &address).await?;
    Ok(TestResult {
        address,
        node_name: status.node_name.unwrap_or_else(|| "unnamed node".into()),
        version: status.version.unwrap_or_else(|| "unknown version".into()),
        model: status.model.unwrap_or_default(),
        engine_ready: status.engine_ready,
    })
}

/// Save the one setting. Returns the normalized addresses.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    primary: String,
    alternate: String,
) -> Result<SettingsView, String> {
    let primary = config::normalize_address(&primary)?;
    let alternate = if alternate.trim().is_empty() {
        None
    } else {
        Some(config::normalize_address(&alternate)?)
    };
    let alternate = alternate.filter(|a| a != &primary);

    let previous = state.settings();
    let changed = previous.primary != primary || previous.alternate != alternate;
    let settings = Settings {
        primary: primary.clone(),
        alternate: alternate.clone(),
        last_good: if changed { None } else { previous.last_good },
        // A new address can mean a different fleet, and the old fleet's nodes
        // may still be answering: keeping them could quietly connect the app to
        // the cluster the user just pointed it away from. The first successful
        // poll fills the list back in from whoever answers.
        known: if changed { Vec::new() } else { previous.known },
    };
    config::save(&app, &settings)?;
    if let Ok(mut s) = state.settings.lock() {
        *s = settings;
    }
    if changed {
        // Forget the old master so the poller re-probes and re-navigates.
        state.set_master(None);
        state.reset_fleet();
    }
    state.kick.notify_one();

    windows::show_main(&app);
    if changed {
        windows::show_waiting(&app);
    }
    Ok(SettingsView {
        primary,
        alternate: alternate.unwrap_or_default(),
    })
}

/// Ask the poller to try again right now.
#[tauri::command]
pub fn retry_now(state: State<'_, AppState>) {
    state.kick.notify_one();
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    windows::show_settings(&app);
}

/// Close the window that asked (settings or about).
#[tauri::command]
pub fn close_self(window: tauri::WebviewWindow) {
    if window.label() != windows::MAIN {
        let _ = window.close();
    }
}

#[tauri::command]
pub fn open_site(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(SITE_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    let info = app.package_info();
    AppInfo {
        name: info.name.clone(),
        version: info.version.to_string(),
        site: SITE_URL.to_string(),
    }
}
