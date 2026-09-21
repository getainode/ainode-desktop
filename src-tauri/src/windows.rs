//! The three windows: main (the AINode UI or the waiting page), settings, about.

use crate::api;
use crate::state::AppState;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const MAIN: &str = "main";
pub const SETTINGS: &str = "settings";
pub const ABOUT: &str = "about";

pub const WAITING_PAGE: &str = "waiting.html";
const SETTINGS_PAGE: &str = "settings.html";
const ABOUT_PAGE: &str = "about.html";

/// Where the bundled pages live. Tauri serves them from its own origin;
/// we read it back from a live window so dev and bundle both work.
fn app_base(app: &AppHandle) -> Url {
    for label in [MAIN, SETTINGS, ABOUT] {
        if let Some(w) = app.get_webview_window(label) {
            if let Ok(url) = w.url() {
                // A window sitting on the master's http origin does not tell
                // us the app origin; keep looking.
                if is_app_page(&url) {
                    if let Ok(base) = url.join("/") {
                        return base;
                    }
                }
            }
        }
    }
    let fallback = if cfg!(windows) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    };
    Url::parse(fallback).expect("static app origin")
}

/// Full URL of one of the bundled pages.
pub fn page_url(app: &AppHandle, page: &str) -> Url {
    app_base(app).join(page).expect("page name is a valid path")
}

/// Is this URL one of our own bundled pages (as opposed to the master's UI)?
pub fn is_app_page(url: &Url) -> bool {
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
}

/// Create the main window if it does not exist yet. It starts on the
/// waiting page; the poller moves it to the master as soon as one answers.
pub fn ensure_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(MAIN) {
        return Ok(w);
    }
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App(WAITING_PAGE.into()))
        .title("AINode")
        .inner_size(1280.0, 840.0)
        .min_inner_size(720.0, 480.0)
        .zoom_hotkeys_enabled(true)
        .center()
        .on_page_load(|_window, payload| {
            debug_log(&format!(
                "page load {:?} {}",
                payload.event(),
                payload.url()
            ));
        })
        .build()
}

/// Show and focus the main window, creating it if needed.
pub fn show_main(app: &AppHandle) {
    if let Ok(w) = ensure_main(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Open (or focus) the Settings window.
pub fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App(SETTINGS_PAGE.into()))
        .title("AINode Settings")
        .inner_size(560.0, 620.0)
        .min_inner_size(480.0, 520.0)
        .resizable(true)
        .center()
        .build();
    if let Ok(w) = built {
        without_menu_bar(&w);
        let _ = w.set_focus();
    }
}

/// Open (or focus) the About window.
pub fn show_about(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(ABOUT) {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, ABOUT, WebviewUrl::App(ABOUT_PAGE.into()))
        .title("About AINode")
        .inner_size(380.0, 420.0)
        .resizable(false)
        .minimizable(false)
        .center()
        .build();
    if let Ok(w) = built {
        without_menu_bar(&w);
        let _ = w.set_focus();
    }
}

/// Windows and Linux hand every new window the app-wide menu bar; the small
/// dialogs (Settings, About) do not want one. macOS has a single menu bar for
/// the whole app, so there is nothing to take away there.
fn without_menu_bar(window: &WebviewWindow) {
    #[cfg(not(target_os = "macos"))]
    let _ = window.remove_menu();
    #[cfg(target_os = "macos")]
    let _ = window;
}

/// Debug builds print navigation events to stderr.
pub fn debug_log(msg: &str) {
    if cfg!(debug_assertions) {
        eprintln!("[ainode] {msg}");
    }
}

/// Point the main window at `url` unless it is already there.
fn navigate_main(app: &AppHandle, url: Url) {
    let Ok(main) = ensure_main(app) else { return };
    if let Ok(current) = main.url() {
        if current == url {
            return;
        }
    }
    debug_log(&format!("navigate main -> {url}"));
    if let Err(e) = main.navigate(url) {
        debug_log(&format!("navigate failed: {e}"));
    }
}

/// Main window shows the AINode UI on `address`. If the window is already on
/// that master (maybe deep in some page), leave it alone.
///
/// `tls` decides the scheme, and it is the reason the origin comparison below
/// checks the scheme too: http and https on one node are two origins, and the
/// window has to actually move when a node gains a certificate.
pub fn show_master(app: &AppHandle, address: &str, tls: bool) {
    let target = Url::parse(&api::ui_url(address, tls)).expect("address was validated");
    let Ok(main) = ensure_main(app) else { return };
    if let Ok(current) = main.url() {
        let same_origin = current.scheme() == target.scheme()
            && current.host_str() == target.host_str()
            && current.port_or_known_default() == target.port_or_known_default();
        if same_origin {
            return;
        }
        debug_log(&format!("main is on {current}"));
    }
    debug_log(&format!("navigate main -> {target}"));
    if let Err(e) = main.navigate(target) {
        debug_log(&format!("navigate failed: {e}"));
    }
}

/// Main window shows the bundled waiting page.
pub fn show_waiting(app: &AppHandle) {
    navigate_main(app, page_url(app, WAITING_PAGE));
}

/// Reload: go back to the master's front page, or the waiting page.
pub fn reload_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state.master() {
        Some(address) => {
            if let Ok(main) = ensure_main(app) {
                let tls = state.serving().is_some_and(|s| s.tls);
                let target =
                    Url::parse(&api::ui_url(&address, tls)).expect("address was validated");
                let _ = main.navigate(target);
            }
        }
        None => show_waiting(app),
    }
}
