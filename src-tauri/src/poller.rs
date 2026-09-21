//! Background loop: find the master, keep the main window pointed at it,
//! poll `/api/nodes`, refresh the tray, and send notifications.

use crate::config::{self, Which};
use crate::nodes::Event;
use crate::probe::{self, Probe};
use crate::state::{AppState, Attention, Serving};
use crate::{api, tray, windows};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// Poll cadence while the master answers.
const UP_INTERVAL: Duration = Duration::from_secs(10);
/// Retry cadence while it does not.
const DOWN_INTERVAL: Duration = Duration::from_secs(5);
/// A single missed probe gets a quick second look before we call it down.
const RECHECK_INTERVAL: Duration = Duration::from_secs(3);
/// Consecutive failed probes before the main window drops to the waiting page.
const FAILS_BEFORE_DOWN: u32 = 2;

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move { run(app).await });
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut fail_streak: u32 = 0;

    loop {
        let settings = state.settings();
        if !settings.is_configured() {
            // Nothing to do until Settings are saved.
            tray::update(&app, &[], None, false);
            state.kick.notified().await;
            continue;
        }

        let current = state.master();
        let attempt = probe::find_master(
            &state.client,
            &settings,
            current.as_deref(),
            &state.tls_rejected(),
        )
        .await;

        // A certificate this machine will not verify is remembered for the
        // session, so the next poll dials that node's http port instead of
        // failing at it every ten seconds. Never bypassed, only fallen back from.
        for host in attempt.rejected_hosts() {
            if state.reject_tls(&host) {
                windows::debug_log(&format!("{host}: certificate not trusted, using http"));
            }
        }
        // "This node wants an API key" has to survive as its own sentence: it is
        // the one failure the user can fix from the Settings window.
        state.set_needs_attention(attempt.explanation().map(|message| Attention {
            wants_key: attempt.wants_key(),
            message,
        }));

        let found = attempt.chosen;

        // Whoever answered knows where the rest of the fleet is. Learn it on
        // every successful poll, so the list is already on disk the next time
        // this address is the one that went away.
        if let Some(probe) = found.as_ref().filter(|p| p.ok()) {
            learn_fleet(&app, probe).await;
        }

        match (current.as_deref(), &found) {
            (None, Some(p)) => {
                fail_streak = 0;
                came_up(&app, p);
            }
            (Some(c), Some(p)) if c != p.address => {
                // The address we were on stopped answering but another one did:
                // follow it, whether it is the alternate or a node out of the
                // fleet list. Also the way home: the primary wins the pick as
                // soon as it answers again.
                fail_streak = 0;
                came_up(&app, p);
            }
            (Some(_), Some(_)) => {
                fail_streak = 0;
            }
            (Some(_), None) => {
                fail_streak += 1;
                if fail_streak >= FAILS_BEFORE_DOWN {
                    went_down(&app, &settings);
                }
            }
            (None, None) => {
                fail_streak += 1;
                state.set_last_error(Some(describe_failure(&settings)));
                tray::update(&app, &[], None, true);
            }
        }

        if let Some(address) = state.master() {
            let tls = state.serving().is_some_and(|s| s.tls);
            match api::fetch_nodes(&state.client, &address, tls, settings.key()).await {
                Ok(rows) => {
                    let events = match state.fleet.lock() {
                        Ok(mut fleet) => fleet.observe(&rows, unix_now()),
                        Err(_) => Vec::new(),
                    };
                    tray::update(&app, &rows, state.serving().as_ref(), true);
                    state.set_rows(rows);
                    for event in &events {
                        notify(&app, event);
                    }
                }
                Err(err) => {
                    // The master answered /api/status but not /api/nodes.
                    // Keep the last list; note the reason for the waiting page.
                    state.set_last_error(Some(format!("/api/nodes: {err}")));
                }
            }
        }

        let delay = match (state.master().is_some(), fail_streak) {
            (true, 0) => UP_INTERVAL,
            (true, _) => RECHECK_INTERVAL,
            (false, _) => DOWN_INTERVAL,
        };
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = state.kick.notified() => {}
        }
    }
}

/// Plain words for the waiting page when nothing answers.
fn describe_failure(settings: &config::Settings) -> String {
    let mut line = match settings.alternate.as_deref() {
        Some(alt) => format!("No answer from {} or {}", settings.primary, alt),
        None => format!("No answer from {}", settings.primary),
    };
    // The saved fleet, counted without the TLS upgrade: this is a sentence about
    // how many machines were asked, and a rejection set would not change that.
    let fleet = settings
        .fleet_candidates(&std::collections::HashSet::new())
        .len();
    if fleet > 0 {
        // Say that the fleet was tried too, or a whole cluster of live nodes
        // looks like it was never asked.
        line.push_str(&format!(
            ", or {} other node{} of the fleet",
            fleet,
            if fleet == 1 { "" } else { "s" }
        ));
    }
    line
}

/// The http `host:port` this app uses for the node it just polled.
///
/// Over http it is the address in hand. Over https it is the stored row for the
/// same host, because that row holds the HTTP port the https one was upgraded
/// from and it is the address the scheme has to keep being remembered against.
fn http_address_of(probe: &Probe, settings: &config::Settings) -> String {
    if !probe.tls {
        return probe.address.clone();
    }
    let host = config::host_of(&probe.address);
    settings
        .known
        .iter()
        .find(|k| config::host_of(&k.address) == host)
        .map(|k| k.address.clone())
        .unwrap_or_else(|| probe.address.clone())
}

/// Save the fleet's addresses, and this node's own scheme, as reported by the
/// node that just answered.
///
/// `endpoint_hint` on `/api/status` is the free path and the usual one. A node
/// that answered status without one is asked `/api/cluster/endpoint` once, and
/// never again this session if it 404s: that is a node older than the fleet
/// list, and there is nothing to learn from it.
///
/// The `tls` block is learned separately and unconditionally, because the fleet
/// list cannot answer for the address this app is using: a node reached on its
/// MagicDNS name reports its LAN address in that list, so its own word about its
/// own https port is the only thing that upgrades the address in hand.
async fn learn_fleet(app: &AppHandle, probe: &Probe) {
    let state = app.state::<AppState>();
    let Ok(status) = probe.result.as_ref() else {
        return;
    };
    let settings = state.settings();
    let own_address = http_address_of(probe, &settings);
    let own_name = probe.node_name().unwrap_or_default();
    let own_tls = status.https_port();

    let mut rows = api::known_nodes(&status.endpoint_hint);
    if rows.is_empty() && state.may_ask_endpoint(&probe.address) {
        match api::fetch_endpoint(&state.client, &probe.address, probe.tls, settings.key()).await {
            Ok(nodes) => rows = api::known_nodes(&nodes),
            Err(err) => {
                windows::debug_log(&format!("no fleet list from {}: {err}", probe.address));
                state.endpoint_is_absent(&probe.address);
            }
        }
    }
    let updated = match state.settings.lock() {
        Ok(mut s) => {
            // Order matters: set_known replaces the whole list, so the node's own
            // word about its own scheme is recorded after it.
            let fleet = if rows.is_empty() {
                false
            } else {
                s.set_known(rows)
            };
            let scheme = s.note_tls(&own_name, &own_address, own_tls);
            (fleet || scheme).then(|| s.clone())
        }
        Err(_) => None,
    };
    if let Some(s) = updated {
        windows::debug_log(&format!(
            "fleet list: {} node(s) saved, {} with https",
            s.known.len(),
            s.known.iter().filter(|k| k.tls_port.is_some()).count()
        ));
        let _ = config::save(app, &s);
    }
}

fn came_up(app: &AppHandle, probe: &Probe) {
    let state = app.state::<AppState>();
    let switched = state.master().is_some_and(|m| m != probe.address);
    if switched {
        // A different address means a possibly different node list; start
        // the notification memory fresh rather than mourning the old one.
        state.reset_fleet();
    }
    let serving = Serving {
        address: probe.address.clone(),
        name: probe.node_name(),
        is_primary: probe.slot == Some(Which::Primary),
        tls: probe.tls,
    };
    if let Some(line) = serving.via_label() {
        windows::debug_log(&line);
    }
    windows::debug_log(&format!(
        "serving over {} at {}",
        if probe.tls { "https" } else { "http" },
        probe.address
    ));
    state.set_serving(Some(serving));
    state.set_last_error(None);
    if let Some(which) = probe.slot {
        remember_last_good(app, which);
    }
    windows::show_master(app, &probe.address, probe.tls);
    tray::update(app, &state.rows(), state.serving().as_ref(), true);
}

fn went_down(app: &AppHandle, settings: &config::Settings) {
    let state = app.state::<AppState>();
    state.set_serving(None);
    state.set_last_error(Some(describe_failure(settings)));
    windows::show_waiting(app);
    tray::update(app, &[], None, true);
}

/// Persist which slot answered so the next launch tries it first.
fn remember_last_good(app: &AppHandle, which: Which) {
    let state = app.state::<AppState>();
    let updated = match state.settings.lock() {
        Ok(mut s) => {
            if s.last_good == Some(which) {
                None
            } else {
                s.last_good = Some(which);
                Some(s.clone())
            }
        }
        Err(_) => None,
    };
    if let Some(s) = updated {
        let _ = config::save(app, &s);
    }
}

fn notify(app: &AppHandle, event: &Event) {
    windows::debug_log(&format!("notify: {}", event.message()));
    let title = match event {
        Event::ModelReady { .. } => "Model ready",
        Event::NodeOffline { .. } => "Node offline",
        Event::NodeOnline { .. } => "Node back online",
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(event.message())
        .show();
}
