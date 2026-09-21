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
    /// The saved key, or "". Round-tripped to the field so Save does not wipe it,
    /// and it is the user's own key on the user's own machine: masking it here
    /// would only stop them checking what they pasted.
    pub api_key: String,
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
    /// True when the connection in use is https. What the lock is drawn from.
    pub tls: bool,
    /// "this node wants an API key", or the certificate sentence with its fix,
    /// or null. Separate from `last_error` because it is actionable and that one
    /// is only ever "nothing answered".
    pub needs_attention: Option<String>,
    /// True when a key is saved, so a page can say so without showing it.
    pub has_api_key: bool,
}

#[derive(Serialize)]
pub struct TestResult {
    pub address: String,
    pub node_name: String,
    pub version: String,
    pub model: String,
    pub engine_ready: bool,
    /// True when this test reached the node over https.
    pub tls: bool,
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
        api_key: s.api_key.unwrap_or_default(),
    }
}

#[tauri::command]
pub fn get_status_view(state: State<'_, AppState>) -> StatusView {
    let s = state.settings();
    let serving = state.serving();
    let fleet_count = s.fleet_candidates(&state.tls_rejected()).len();
    let has_api_key = s.key().is_some();
    StatusView {
        configured: s.is_configured(),
        primary: s.primary.clone(),
        alternate: s.alternate.clone().unwrap_or_default(),
        master: state.master(),
        last_error: state.last_error(),
        node_count: state.rows().len(),
        serving_name: serving.as_ref().and_then(|v| v.name.clone()),
        serving_via: serving.as_ref().and_then(crate::state::Serving::via_label),
        fleet_count,
        tls: serving.as_ref().is_some_and(|v| v.tls),
        needs_attention: state.needs_attention().map(|a| a.message),
        has_api_key,
    }
}

/// Hit `/api/status` on one address and report who answered.
///
/// Over https when the fleet list says that host serves it, so the Test button
/// exercises the transport the app will actually use, and reports a certificate
/// this machine does not trust as exactly that.
#[tauri::command]
pub async fn test_address(
    state: State<'_, AppState>,
    address: String,
) -> Result<TestResult, String> {
    let typed = config::normalize_address(&address)?;
    let settings = state.settings();
    let (address, tls) = settings.upgrade(&typed, &state.tls_rejected());
    let client = state.client.clone();
    let status = api::fetch_status(&client, &address, tls, settings.key())
        .await
        .map_err(|e| e.message())?;
    Ok(TestResult {
        address,
        node_name: status.node_name.unwrap_or_else(|| "unnamed node".into()),
        version: status.version.unwrap_or_else(|| "unknown version".into()),
        model: status.model.unwrap_or_default(),
        engine_ready: status.engine_ready,
        tls,
    })
}

/// Save the one setting. Returns the normalized addresses.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    primary: String,
    alternate: String,
    api_key: String,
) -> Result<SettingsView, String> {
    let primary = config::normalize_address(&primary)?;
    let alternate = if alternate.trim().is_empty() {
        None
    } else {
        Some(config::normalize_address(&alternate)?)
    };
    let alternate = alternate.filter(|a| a != &primary);
    let api_key = Some(api_key.trim().to_string()).filter(|k| !k.is_empty());

    let previous = state.settings();
    let changed = previous.primary != primary || previous.alternate != alternate;
    let key_changed = previous.api_key != api_key;
    let settings = Settings {
        primary: primary.clone(),
        alternate: alternate.clone(),
        last_good: if changed { None } else { previous.last_good },
        // A new address can mean a different fleet, and the old fleet's nodes
        // may still be answering: keeping them could quietly connect the app to
        // the cluster the user just pointed it away from. The first successful
        // poll fills the list back in from whoever answers.
        known: if changed { Vec::new() } else { previous.known },
        api_key: api_key.clone(),
    };
    config::save(&app, &settings)?;
    if let Ok(mut s) = state.settings.lock() {
        *s = settings;
    }
    if changed {
        // Forget the old master so the poller re-probes and re-navigates.
        state.set_master(None);
        state.reset_fleet();
        // A different fleet has different certificates; last session's
        // rejections say nothing about them.
        state.clear_tls_rejected();
    }
    if changed || key_changed {
        // The reason on screen was about the old settings. Let the next poll
        // write a new one rather than leaving "wants a key" up after the key was
        // pasted in.
        state.set_needs_attention(None);
    }
    state.kick.notify_one();

    windows::show_main(&app);
    if changed {
        windows::show_waiting(&app);
    }
    Ok(SettingsView {
        primary,
        alternate: alternate.unwrap_or_default(),
        api_key: api_key.unwrap_or_default(),
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
