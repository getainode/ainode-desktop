//! Shared state behind `tauri::State`.

use crate::config::Settings;
use crate::nodes::{FleetState, NodeRow};
use std::sync::Mutex;
use tokio::sync::Notify;

pub struct AppState {
    pub client: reqwest::Client,
    /// The one setting, as last loaded or saved.
    pub settings: Mutex<Settings>,
    /// Address currently in use, `None` while the master is unreachable.
    pub master: Mutex<Option<String>>,
    /// Memory for the notification state machine.
    pub fleet: Mutex<FleetState>,
    /// Last good `/api/nodes` snapshot, for the tray menu.
    pub rows: Mutex<Vec<NodeRow>>,
    /// Why the last probe failed, shown on the waiting page.
    pub last_error: Mutex<Option<String>>,
    /// Wakes the poller early (after Save, or Refresh, or Retry now).
    pub kick: Notify,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            client: crate::api::client(),
            settings: Mutex::new(settings),
            master: Mutex::new(None),
            fleet: Mutex::new(FleetState::new()),
            rows: Mutex::new(Vec::new()),
            last_error: Mutex::new(None),
            kick: Notify::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn master(&self) -> Option<String> {
        self.master.lock().ok().and_then(|m| m.clone())
    }

    pub fn set_master(&self, address: Option<String>) {
        if let Ok(mut m) = self.master.lock() {
            *m = address;
        }
    }

    /// Forget node memory; the next poll primes it silently.
    pub fn reset_fleet(&self) {
        if let Ok(mut f) = self.fleet.lock() {
            *f = FleetState::new();
        }
        self.set_rows(Vec::new());
    }

    pub fn rows(&self) -> Vec<NodeRow> {
        self.rows.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn set_rows(&self, rows: Vec<NodeRow>) {
        if let Ok(mut r) = self.rows.lock() {
            *r = rows;
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }

    pub fn set_last_error(&self, err: Option<String>) {
        if let Ok(mut e) = self.last_error.lock() {
            *e = err;
        }
    }
}
