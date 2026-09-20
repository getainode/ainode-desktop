//! Shared state behind `tauri::State`.

use crate::config::Settings;
use crate::nodes::{FleetState, NodeRow};
use std::collections::HashSet;
use std::sync::Mutex;
use tokio::sync::Notify;

/// Which node is serving this app right now.
///
/// The address alone is not enough for the interface: "connected through
/// Spark-2" is the sentence a user can act on, and whether that address is the
/// configured one decides whether it is worth saying at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Serving {
    pub address: String,
    /// The node's own name from `/api/status`, when it gave one.
    pub name: Option<String>,
    /// True when this is the primary address out of Settings.
    pub is_primary: bool,
}

impl Serving {
    /// What to show when the app is not on the primary, else None.
    pub fn via_label(&self) -> Option<String> {
        if self.is_primary {
            return None;
        }
        Some(match &self.name {
            Some(name) => format!("Connected through {name}"),
            None => format!("Connected through {}", self.address),
        })
    }
}

pub struct AppState {
    pub client: reqwest::Client,
    /// The one setting, as last loaded or saved.
    pub settings: Mutex<Settings>,
    /// Address currently in use, `None` while the master is unreachable.
    pub master: Mutex<Option<String>>,
    /// Who is serving, with the name for the interface. Set beside `master`.
    pub serving: Mutex<Option<Serving>>,
    /// Addresses that answered `/api/status` but have no
    /// `/api/cluster/endpoint`: a node older than the fleet list. Asked once
    /// per session, not every ten seconds.
    pub endpoint_absent: Mutex<HashSet<String>>,
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
            serving: Mutex::new(None),
            endpoint_absent: Mutex::new(HashSet::new()),
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

    pub fn serving(&self) -> Option<Serving> {
        self.serving.lock().ok().and_then(|s| s.clone())
    }

    /// Record who is serving. The address is kept in `master` as well so every
    /// existing caller (the windows, the commands) keeps working unchanged.
    pub fn set_serving(&self, serving: Option<Serving>) {
        self.set_master(serving.as_ref().map(|s| s.address.clone()));
        if let Ok(mut s) = self.serving.lock() {
            *s = serving;
        }
    }

    /// True the first time an address is asked about `/api/cluster/endpoint`,
    /// and after it has answered. False once it has 404'd: a node that does not
    /// serve the fleet list is not asked again this session.
    pub fn may_ask_endpoint(&self, address: &str) -> bool {
        self.endpoint_absent
            .lock()
            .map(|seen| !seen.contains(address))
            .unwrap_or(false)
    }

    pub fn endpoint_is_absent(&self, address: &str) {
        if let Ok(mut seen) = self.endpoint_absent.lock() {
            seen.insert(address.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_via_label_is_only_said_when_it_is_news() {
        let home = Serving {
            address: "a:3000".into(),
            name: Some("Spark-1".into()),
            is_primary: true,
        };
        assert_eq!(home.via_label(), None, "the primary needs no explanation");

        let away = Serving {
            address: "b:3000".into(),
            name: Some("Spark-2".into()),
            is_primary: false,
        };
        assert_eq!(
            away.via_label().as_deref(),
            Some("Connected through Spark-2")
        );

        let nameless = Serving {
            address: "b:3000".into(),
            name: None,
            is_primary: false,
        };
        assert_eq!(
            nameless.via_label().as_deref(),
            Some("Connected through b:3000")
        );
    }
}
