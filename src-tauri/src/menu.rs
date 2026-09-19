//! Application menu bar and the shared handler for menu and tray clicks.
//!
//! macOS gets the usual application menu (AINode, Edit, View, Window). On
//! Windows the same items sit in a File, View, Help bar on the main window;
//! the macOS-only entries (Services, Hide, Show All, Full Screen, the Window
//! menu) have no meaning there and are left out.

use crate::state::AppState;
use crate::windows;
use tauri::menu::{Menu, MenuItem, MenuItemBuilder, SubmenuBuilder};
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

/// The items both menu bars are built from.
struct Items<R: Runtime> {
    about: MenuItem<R>,
    settings: MenuItem<R>,
    site: MenuItem<R>,
    quit: MenuItem<R>,
    open: MenuItem<R>,
    reload: MenuItem<R>,
    refresh: MenuItem<R>,
}

fn items<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Items<R>> {
    Ok(Items {
        about: MenuItemBuilder::with_id(ids::ABOUT, "About AINode").build(app)?,
        settings: MenuItemBuilder::with_id(ids::SETTINGS, "Settings...")
            .accelerator("CmdOrCtrl+,")
            .build(app)?,
        site: MenuItemBuilder::with_id(ids::SITE, "Visit ainode.dev").build(app)?,
        quit: MenuItemBuilder::with_id(ids::QUIT, "Quit AINode")
            .accelerator("CmdOrCtrl+Q")
            .build(app)?,
        open: MenuItemBuilder::with_id(ids::OPEN, "Open AINode")
            .accelerator("CmdOrCtrl+1")
            .build(app)?,
        reload: MenuItemBuilder::with_id(ids::RELOAD, "Reload")
            .accelerator("CmdOrCtrl+R")
            .build(app)?,
        refresh: MenuItemBuilder::with_id(ids::REFRESH, "Refresh Node List")
            .accelerator("CmdOrCtrl+Shift+R")
            .build(app)?,
    })
}

/// The macOS menu bar: AINode, Edit, View, Window.
#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let i = items(app)?;
    let app_menu = SubmenuBuilder::new(app, "AINode")
        .item(&i.about)
        .separator()
        .item(&i.settings)
        .item(&i.site)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&i.quit)
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

    let view = SubmenuBuilder::new(app, "View")
        .item(&i.open)
        .item(&i.reload)
        .item(&i.refresh)
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

/// The Windows (and Linux) menu bar: File, View, Help, shown on the main
/// window. Minimize, maximize and close live on the title bar there, and the
/// webview handles its own Edit shortcuts, so neither menu is repeated.
#[cfg(not(target_os = "macos"))]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let i = items(app)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&i.settings)
        .item(&i.site)
        .separator()
        .item(&i.quit)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&i.open)
        .item(&i.reload)
        .item(&i.refresh)
        .build()?;

    let help = SubmenuBuilder::new(app, "Help").item(&i.about).build()?;

    Menu::with_items(app, &[&file, &view, &help])
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
