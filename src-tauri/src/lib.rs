//! AINode Desktop: a thin native shell around the AINode web UI.
//!
//! One setting (where the master node lives), a menu bar item that knows
//! what the fleet is doing, and notifications when a model finishes
//! loading or a node drops. Made in Texas.

mod api;
mod commands;
mod config;
mod menu;
mod nodes;
mod poller;
mod probe;
mod state;
mod tray;
mod windows;

use state::AppState;
use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_status_view,
            commands::test_address,
            commands::save_settings,
            commands::retry_now,
            commands::open_settings,
            commands::close_self,
            commands::open_site,
            commands::app_info,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = config::load(&handle);
            let configured = settings.is_configured();
            app.manage(AppState::new(settings));

            let app_menu = menu::build(&handle)?;
            app.set_menu(app_menu)?;
            app.on_menu_event(|app, event| menu::handle(app, event.id().as_ref()));

            tray::create(&handle)?;

            if configured {
                windows::show_main(&handle);
            } else {
                windows::show_settings(&handle);
            }
            // Debug aid: AINODE_DESKTOP_TEST_URL=https://example.com sends the
            // main window there instead of the master.
            if cfg!(debug_assertions) {
                if let Ok(url) = std::env::var("AINODE_DESKTOP_TEST_URL") {
                    let h = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        if let Some(w) = h.get_webview_window(windows::MAIN) {
                            windows::debug_log(&format!("test navigate -> {url}"));
                            if let Ok(u) = tauri::Url::parse(&url) {
                                let r = w.navigate(u);
                                windows::debug_log(&format!("test navigate result {r:?}"));
                            }
                        }
                    });
                    return Ok(());
                }
            }
            poller::spawn(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window hides it; the app lives in the menu bar.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == windows::MAIN {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building AINode");

    app.run(|app, event| match event {
        // Dock icon clicked with no window showing: bring the app back.
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => menu::open_main(app),
        // Last window closed: stay alive in the menu bar. `app.exit(0)`
        // (the Quit items) sets a code and is allowed through.
        RunEvent::ExitRequested {
            api, code: None, ..
        } => api.prevent_exit(),
        _ => {}
    });
}
