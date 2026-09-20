//! The one setting: where the master AINode lives.
//!
//! Two fields sit behind it, a primary address and an optional alternate
//! (for example the tailnet address of the same node). Both are stored as
//! plain `host:port` strings. We also remember which one answered last so
//! the next launch tries the likely winner first.
//!
//! A third field is not a setting and nobody types it: `known` is the fleet's
//! own list of addresses, learned from whichever node answered last and kept on
//! disk. Every AINode routes every model the fleet serves, so any node in that
//! list can serve this app; without it, one address in one text box is a single
//! point of failure for a cluster that does not have one.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// File name inside the app data directory.
pub const STORE_FILE: &str = "settings.json";

const KEY_PRIMARY: &str = "primary";
const KEY_ALTERNATE: &str = "alternate";
const KEY_LAST_GOOD: &str = "last_good";
const KEY_KNOWN: &str = "known";

/// How many learned nodes to keep. A fleet this app would talk to is a handful
/// of machines; the cap keeps an outage from fanning out into a long probe and
/// keeps a stale list from growing without end.
pub const MAX_KNOWN: usize = 12;

/// Which of the two stored addresses is meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Which {
    Primary,
    Alternate,
}

/// One node this app learned from the fleet, kept so it can be tried when the
/// configured address stops answering. Not a setting: the node list comes from
/// whichever AINode answered last, and the app only remembers it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownNode {
    /// The node's own name, for saying "connected through Spark-2".
    pub name: String,
    /// `host:port`, already normalized.
    pub address: String,
}

/// An address the app is willing to try, with whatever is known about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The stored slot this came from, or None for a node learned from the fleet.
    pub slot: Option<Which>,
    pub address: String,
    /// The node's name when the fleet list carried one.
    pub name: Option<String>,
}

/// Everything the app remembers between launches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// `host:port` of the master node, for example `192.168.0.10:3000`.
    pub primary: String,
    /// Optional second address for the same master, for example its tailnet IP.
    pub alternate: Option<String>,
    /// The address that answered most recently.
    pub last_good: Option<Which>,
    /// The fleet's other nodes, as last reported by a node that answered.
    pub known: Vec<KnownNode>,
}

impl Settings {
    pub fn is_configured(&self) -> bool {
        !self.primary.is_empty()
    }

    /// The address stored under `which`, if any.
    pub fn address(&self, which: Which) -> Option<&str> {
        match which {
            Which::Primary if !self.primary.is_empty() => Some(self.primary.as_str()),
            Which::Alternate => self.alternate.as_deref().filter(|a| !a.is_empty()),
            _ => None,
        }
    }

    /// The two addresses the user configured, primary first.
    ///
    /// Probed on every poll, which is two requests and what this app has always
    /// cost. The fleet's other nodes are a separate list on purpose: they are
    /// only worth a request once these two have stopped answering.
    pub fn configured_candidates(&self) -> Vec<Candidate> {
        let mut out = Vec::new();
        if let Some(p) = self.address(Which::Primary) {
            out.push(Candidate {
                slot: Some(Which::Primary),
                address: p.to_string(),
                name: None,
            });
        }
        if let Some(a) = self.address(Which::Alternate) {
            if a != self.primary {
                out.push(Candidate {
                    slot: Some(Which::Alternate),
                    address: a.to_string(),
                    name: None,
                });
            }
        }
        out
    }

    /// The fleet's other nodes, in the order the fleet reported them (master
    /// first), with anything already configured left out.
    pub fn fleet_candidates(&self) -> Vec<Candidate> {
        let configured: Vec<String> = self
            .configured_candidates()
            .into_iter()
            .map(|c| c.address)
            .collect();
        let mut out: Vec<Candidate> = Vec::new();
        for node in &self.known {
            if node.address.is_empty()
                || configured.iter().any(|a| a == &node.address)
                || out.iter().any(|c| c.address == node.address)
            {
                continue;
            }
            out.push(Candidate {
                slot: None,
                address: node.address.clone(),
                name: Some(node.name.clone()).filter(|n| !n.is_empty()),
            });
        }
        out
    }

    /// Replace the learned node list. True when it actually changed, which is
    /// what decides whether the store is written: the poller runs every ten
    /// seconds and a fleet that has not changed must not mean a disk write.
    pub fn set_known(&mut self, nodes: Vec<KnownNode>) -> bool {
        let mut deduped: Vec<KnownNode> = Vec::new();
        for node in nodes {
            if node.address.is_empty() || deduped.iter().any(|k| k.address == node.address) {
                continue;
            }
            deduped.push(node);
            if deduped.len() >= MAX_KNOWN {
                break;
            }
        }
        if deduped == self.known {
            return false;
        }
        self.known = deduped;
        true
    }

    /// The name the fleet gave this address, if it gave one.
    pub fn name_of(&self, address: &str) -> Option<&str> {
        self.known
            .iter()
            .find(|k| k.address == address)
            .map(|k| k.name.as_str())
            .filter(|n| !n.is_empty())
    }
}

/// Default AINode web port when the user types only a host.
pub const DEFAULT_PORT: u16 = 3000;

/// Turn whatever the user typed into a clean `host:port`.
///
/// Accepts `10.0.0.5`, `10.0.0.5:3000`, `http://10.0.0.5:3000/`, `spark-1:3000`
/// and bracketed IPv6 like `[fd00::1]:3000`. Rejects empty input, bad ports
/// and anything with spaces.
pub fn normalize_address(input: &str) -> Result<String, String> {
    let mut s = input.trim().to_string();
    if s.is_empty() {
        return Err("Enter the address of your master AINode.".into());
    }
    for scheme in ["http://", "https://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest.to_string();
        }
    }
    // Drop any path, query or fragment: we only want host and port.
    if let Some(idx) = s.find(['/', '?', '#']) {
        s.truncate(idx);
    }
    if s.contains(char::is_whitespace) {
        return Err("The address cannot contain spaces.".into());
    }
    if s.is_empty() {
        return Err("Enter the address of your master AINode.".into());
    }

    let (host, port) = split_host_port(&s)?;
    if host.is_empty() {
        return Err("Enter a host name or IP address.".into());
    }
    let port = match port {
        Some(p) => p
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| format!("'{p}' is not a valid port."))?,
        None => DEFAULT_PORT,
    };
    Ok(format!("{host}:{port}"))
}

/// Split `host:port` or `[v6]:port`. Returns the host as it should be
/// written back (brackets kept for IPv6) and the port text if present.
fn split_host_port(s: &str) -> Result<(String, Option<String>), String> {
    if let Some(rest) = s.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| "IPv6 addresses need a closing bracket.".to_string())?;
        let host = format!("[{}]", &rest[..end]);
        let after = &rest[end + 1..];
        if after.is_empty() {
            return Ok((host, None));
        }
        return match after.strip_prefix(':') {
            Some(p) if !p.is_empty() => Ok((host, Some(p.to_string()))),
            _ => Err("Expected ':port' after the IPv6 address.".into()),
        };
    }
    // A bare IPv6 without brackets has several colons; leave it alone but
    // it cannot carry a port.
    if s.matches(':').count() > 1 {
        return Ok((format!("[{s}]"), None));
    }
    match s.split_once(':') {
        Some((h, "")) => Err(format!("'{h}:' is missing a port.")),
        Some((h, p)) => Ok((h.to_string(), Some(p.to_string()))),
        None => Ok((s.to_string(), None)),
    }
}

/// Read settings from the store. Missing or unreadable store means defaults.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(_) => return Settings::default(),
    };
    let as_str = |key: &str| -> Option<String> {
        store
            .get(key)
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
    };
    let last_good = store
        .get(KEY_LAST_GOOD)
        .and_then(|v| serde_json::from_value::<Which>(v).ok());
    // A store written by an older build has no `known` key, and a partly
    // unreadable one must not cost the app its addresses: an unparsable list
    // reads as an empty one, which is exactly the behaviour before it existed.
    let known = store
        .get(KEY_KNOWN)
        .and_then(|v| serde_json::from_value::<Vec<KnownNode>>(v).ok())
        .unwrap_or_default();
    Settings {
        primary: as_str(KEY_PRIMARY).unwrap_or_default(),
        alternate: as_str(KEY_ALTERNATE),
        last_good,
        known,
    }
}

/// Write settings to the store and flush to disk.
pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(KEY_PRIMARY, settings.primary.clone());
    match &settings.alternate {
        Some(a) => store.set(KEY_ALTERNATE, a.clone()),
        None => {
            store.delete(KEY_ALTERNATE);
        }
    }
    match settings.last_good {
        Some(w) => store.set(KEY_LAST_GOOD, serde_json::to_value(w).unwrap_or_default()),
        None => {
            store.delete(KEY_LAST_GOOD);
        }
    }
    if settings.known.is_empty() {
        store.delete(KEY_KNOWN);
    } else if let Ok(v) = serde_json::to_value(&settings.known) {
        store.set(KEY_KNOWN, v);
    }
    store.save().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_inputs() {
        assert_eq!(
            normalize_address("192.168.0.10:3000").unwrap(),
            "192.168.0.10:3000"
        );
        assert_eq!(
            normalize_address("  192.168.0.10 ").unwrap(),
            "192.168.0.10:3000"
        );
        assert_eq!(
            normalize_address("http://192.168.0.10:3000/").unwrap(),
            "192.168.0.10:3000"
        );
        assert_eq!(
            normalize_address("https://spark-1:3000/chat?x=1").unwrap(),
            "spark-1:3000"
        );
        assert_eq!(
            normalize_address("spark-1.local").unwrap(),
            "spark-1.local:3000"
        );
        assert_eq!(
            normalize_address("[fd00::1]:3001").unwrap(),
            "[fd00::1]:3001"
        );
        assert_eq!(normalize_address("[fd00::1]").unwrap(), "[fd00::1]:3000");
        assert_eq!(normalize_address("fd00::1").unwrap(), "[fd00::1]:3000");
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(normalize_address("").is_err());
        assert!(normalize_address("   ").is_err());
        assert!(normalize_address("http://").is_err());
        assert!(normalize_address("10.0.0.1:").is_err());
        assert!(normalize_address("10.0.0.1:abc").is_err());
        assert!(normalize_address("10.0.0.1:70000").is_err());
        assert!(normalize_address("10.0.0.1:0").is_err());
        assert!(normalize_address("10.0.0.1 :3000").is_err());
        assert!(normalize_address("[fd00::1:3000").is_err());
    }

    fn known(name: &str, address: &str) -> KnownNode {
        KnownNode {
            name: name.into(),
            address: address.into(),
        }
    }

    /// Every address the app would try, in order: what the two probe waves add
    /// up to.
    fn all(s: &Settings) -> Vec<String> {
        let mut out = s.configured_candidates();
        out.extend(s.fleet_candidates());
        out.into_iter().map(|c| c.address).collect()
    }

    #[test]
    fn candidates_skip_empty_and_duplicate_alternate() {
        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("a:3000".into()),
            ..Settings::default()
        };
        assert_eq!(all(&s), vec!["a:3000".to_string()]);

        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("b:3000".into()),
            ..Settings::default()
        };
        assert_eq!(s.configured_candidates().len(), 2);
        assert_eq!(s.configured_candidates()[1].slot, Some(Which::Alternate));
        assert_eq!(s.address(Which::Alternate), Some("b:3000"));

        let s = Settings::default();
        assert!(!s.is_configured());
        assert!(all(&s).is_empty());
    }

    #[test]
    fn fleet_candidates_come_after_the_configured_pair_and_never_repeat_it() {
        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("b:3000".into()),
            known: vec![
                // The master is usually one of the two already configured.
                known("Spark-1", "a:3000"),
                known("Spark-2", "c:3000"),
                known("Spark-3", "d:3000"),
            ],
            ..Settings::default()
        };
        assert_eq!(all(&s), vec!["a:3000", "b:3000", "c:3000", "d:3000"]);
        // A learned node carries its name, so the app can say which one served.
        assert_eq!(s.fleet_candidates()[0].name.as_deref(), Some("Spark-2"));
        assert_eq!(s.fleet_candidates()[0].slot, None);
        assert_eq!(s.name_of("c:3000"), Some("Spark-2"));
        assert_eq!(s.name_of("a:3000"), Some("Spark-1"));
        assert_eq!(s.name_of("zz:3000"), None);
    }

    #[test]
    fn set_known_reports_change_dedupes_and_caps() {
        let mut s = Settings::default();
        assert!(s.set_known(vec![known("Spark-1", "a:3000")]));
        // Same list again: no change, so nothing is written to disk.
        assert!(!s.set_known(vec![known("Spark-1", "a:3000")]));
        // A renamed node IS a change.
        assert!(s.set_known(vec![known("Spark-1-DGX", "a:3000")]));

        // Duplicates by address collapse, and rows with no address are dropped.
        let mut s = Settings::default();
        s.set_known(vec![
            known("Spark-1", "a:3000"),
            known("Spark-1 again", "a:3000"),
            known("nowhere", ""),
        ]);
        assert_eq!(s.known, vec![known("Spark-1", "a:3000")]);

        let many: Vec<KnownNode> = (0..40)
            .map(|i| known(&format!("n{i}"), &format!("h{i}:3000")))
            .collect();
        let mut s = Settings::default();
        s.set_known(many);
        assert_eq!(s.known.len(), MAX_KNOWN);
    }

    #[test]
    fn an_empty_fleet_list_leaves_todays_behaviour_exactly() {
        // What a node older than /api/cluster/endpoint gives us: nothing.
        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("b:3000".into()),
            ..Settings::default()
        };
        assert!(s.fleet_candidates().is_empty());
        assert_eq!(all(&s), vec!["a:3000", "b:3000"]);
    }
}
