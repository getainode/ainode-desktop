//! Menu bar item: a title like `AINode · 5 nodes · 3 models` and a menu
//! with one line per node.

use crate::menu::{self, ids};
use crate::nodes::{self, NodeRow};
use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

pub const TRAY_ID: &str = "ainode-tray";

/// Create the tray item once, at startup.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &[], None, false)?;
    let icon = Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .title("AINode")
        .tooltip("AINode")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| menu::handle(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

/// Refresh title and menu after a poll.
///
/// `master` is the address in use (None while unreachable); `rows` is the
/// last good snapshot; `configured` says whether any address is saved.
pub fn update(app: &AppHandle, rows: &[NodeRow], master: Option<&str>, configured: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let title = match (configured, master) {
        (false, _) => "AINode".to_string(),
        (true, None) => "AINode · offline".to_string(),
        (true, Some(_)) if rows.is_empty() => "AINode".to_string(),
        (true, Some(_)) => nodes::tray_title(rows),
    };
    let _ = tray.set_title(Some(&title));
    let _ = tray.set_tooltip(Some(&title));
    if let Ok(menu) = build_menu(app, rows, master, configured) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    rows: &[NodeRow],
    master: Option<&str>,
    configured: bool,
) -> tauri::Result<Menu<R>> {
    let open = MenuItemBuilder::with_id(ids::OPEN, "Open AINode").build(app)?;
    let mut b = MenuBuilder::new(app).item(&open).separator();

    if !configured {
        let hint = MenuItemBuilder::with_id("hint", "Not set up yet: choose Settings...")
            .enabled(false)
            .build(app)?;
        b = b.item(&hint);
    } else if master.is_none() {
        let settings = app.state::<crate::state::AppState>().settings();
        let hint = MenuItemBuilder::with_id("hint", format!("Waiting for {}", settings.primary))
            .enabled(false)
            .build(app)?;
        b = b.item(&hint);
        if let Some(alt) = settings.alternate.as_deref() {
            let hint2 = MenuItemBuilder::with_id("hint-alt", format!("and {alt}"))
                .enabled(false)
                .build(app)?;
            b = b.item(&hint2);
        }
    } else if rows.is_empty() {
        let hint = MenuItemBuilder::with_id("hint", "No nodes reported yet")
            .enabled(false)
            .build(app)?;
        b = b.item(&hint);
    } else {
        for (i, row) in rows.iter().enumerate() {
            let item = MenuItemBuilder::with_id(format!("node-{i}"), nodes::menu_label(row))
                .enabled(false)
                .build(app)?;
            b = b.item(&item);
        }
    }

    let settings = MenuItemBuilder::with_id(ids::SETTINGS, "Settings...").build(app)?;
    let refresh = MenuItemBuilder::with_id(ids::REFRESH, "Refresh").build(app)?;
    let about = MenuItemBuilder::with_id(ids::ABOUT, "About AINode").build(app)?;
    let quit = MenuItemBuilder::with_id(ids::QUIT, "Quit").build(app)?;
    b.separator()
        .item(&settings)
        .item(&refresh)
        .item(&about)
        .separator()
        .item(&quit)
        .build()
}
