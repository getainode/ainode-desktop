//! Application menu bar and the shared handler for menu and tray clicks.

use crate::state::AppState;
use crate::windows;
use tauri::menu::{Menu, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

pub const SITE_URL: &str = "https://ainode.dev";

/// Menu item ids shared by the app menu and the tray menu.
pub mod ids {
    pub const OPEN: &str = "open";
    pub const SETTINGS: &str = "settings";
    pub const ABOUT: &str = "about";
    pub const REFRESH: &str = "refresh";
    pub const RELOAD: &str = "reload";
    pub const SITE: &str = "site";
    pub const QUIT: &str = "quit";
}

/// The menu bar: AINode, Edit, View, Window.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = MenuItemBuilder::with_id(ids::ABOUT, "About AINode").build(app)?;
    let settings = MenuItemBuilder::with_id(ids::SETTINGS, "Settings...")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let site = MenuItemBuilder::with_id(ids::SITE, "Visit ainode.dev").build(app)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit AINode")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;
    let app_menu = SubmenuBuilder::new(app, "AINode")
        .item(&about)
        .separator()
        .item(&settings)
        .item(&site)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&quit)
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let open = MenuItemBuilder::with_id(ids::OPEN, "Open AINode")
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let reload = MenuItemBuilder::with_id(ids::RELOAD, "Reload")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let refresh = MenuItemBuilder::with_id(ids::REFRESH, "Refresh Node List")
        .accelerator("CmdOrCtrl+Shift+R")
        .build(app)?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&open)
        .item(&reload)
        .item(&refresh)
        .separator()
        .fullscreen()
        .build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    Menu::with_items(app, &[&app_menu, &edit, &view, &window])
}

/// One handler for both the menu bar and the tray menu.
pub fn handle(app: &AppHandle, id: &str) {
    match id {
        ids::OPEN => open_main(app),
        ids::SETTINGS => windows::show_settings(app),
        ids::ABOUT => windows::show_about(app),
        ids::REFRESH => app.state::<AppState>().kick.notify_one(),
        ids::RELOAD => windows::reload_main(app),
        ids::SITE => {
            let _ = app.opener().open_url(SITE_URL, None::<&str>);
        }
        ids::QUIT => app.exit(0),
        _ => {}
    }
}

/// "Open AINode": the main window, or Settings when nothing is saved yet.
pub fn open_main(app: &AppHandle) {
    let configured = app.state::<AppState>().settings().is_configured();
    if configured {
        windows::show_main(app);
    } else {
        windows::show_settings(app);
    }
}
