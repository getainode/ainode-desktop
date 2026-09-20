//! Menu bar item: a title like `AINode · 5 nodes · 3 models` and a menu
//! with one line per node. On Windows it is a system tray icon: the title
//! becomes the tooltip (a Windows tray has no text next to the icon) and the
//! menu is the same.

use crate::menu::{self, ids};
use crate::nodes::{self, NodeRow};
use crate::state::Serving;
use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

pub const TRAY_ID: &str = "ainode-tray";

/// Create the tray item once, at startup.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &[], None, false)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon()?)
        // A template icon is tinted by the macOS menu bar for light and dark;
        // the flag means nothing elsewhere.
        .icon_as_template(cfg!(target_os = "macos"))
        .title("AINode")
        .tooltip("AINode")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| menu::handle(app, event.id().as_ref()))
        // Double-clicking a tray icon opens the app; that is the Windows
        // convention, and only Windows sends this event.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                menu::open_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// macOS: the monochrome template drawn for the menu bar.
#[cfg(target_os = "macos")]
fn icon() -> tauri::Result<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/tray@2x.png"))
}

/// Windows (and Linux): the colour app icon, the same `.ico` the executable
/// and the installer carry.
#[cfg(not(target_os = "macos"))]
fn icon() -> tauri::Result<Image<'static>> {
    Image::from_bytes(include_bytes!("../icons/icon.ico"))
}

/// Refresh title and menu after a poll.
///
/// `serving` is the node answering right now (None while unreachable); `rows` is
/// the last good snapshot; `configured` says whether any address is saved.
pub fn update(app: &AppHandle, rows: &[NodeRow], serving: Option<&Serving>, configured: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let title = match (configured, serving) {
        (false, _) => "AINode".to_string(),
        (true, None) => "AINode · offline".to_string(),
        (true, Some(_)) if rows.is_empty() => "AINode".to_string(),
        (true, Some(s)) => match s.name.as_deref().filter(|_| !s.is_primary) {
            // Working through a fallback is worth one word in the menu bar: the
            // fleet is up, the address the user typed is not, and nothing else
            // on screen would say so.
            Some(name) => format!("AINode · via {name}"),
            None => nodes::tray_title(rows),
        },
    };
    let _ = tray.set_title(Some(&title));
    let _ = tray.set_tooltip(Some(&title));
    if let Ok(menu) = build_menu(app, rows, serving, configured) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    rows: &[NodeRow],
    serving: Option<&Serving>,
    configured: bool,
) -> tauri::Result<Menu<R>> {
    let open = MenuItemBuilder::with_id(ids::OPEN, "Open AINode").build(app)?;
    let mut b = MenuBuilder::new(app).item(&open).separator();

    // The full sentence, above the node list: which node is serving this app,
    // and at what address, whenever that is not the one the user configured.
    if let Some(line) = serving.and_then(Serving::via_label) {
        let via = MenuItemBuilder::with_id("serving", line)
            .enabled(false)
            .build(app)?;
        b = b.item(&via).separator();
    }

    if !configured {
        let hint = MenuItemBuilder::with_id("hint", "Not set up yet: choose Settings...")
            .enabled(false)
            .build(app)?;
        b = b.item(&hint);
    } else if serving.is_none() {
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
        let fleet = settings.fleet_candidates().len();
        if fleet > 0 {
            let hint3 = MenuItemBuilder::with_id(
                "hint-fleet",
                format!(
                    "and {fleet} other node{} of the fleet",
                    if fleet == 1 { "" } else { "s" }
                ),
            )
            .enabled(false)
            .build(app)?;
            b = b.item(&hint3);
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
