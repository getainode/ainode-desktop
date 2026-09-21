//! The one setting: where the master AINode lives.
//!
//! Two fields sit behind it, a primary address and an optional alternate
//! (for example the tailnet address of the same node). Both are stored as
//! plain `host:port` strings. We also remember which one answered last so
//! the next launch tries the likely winner first.
//!
//! Since 0.2.0 there is one more field a user can fill in, and it is optional:
//! an API key, for a node that requires one. It is sent as `Authorization:
//! Bearer` on every request this app makes.
//!
//! A third field is not a setting and nobody types it: `known` is the fleet's
//! own list of addresses, learned from whichever node answered last and kept on
//! disk. Every AINode routes every model the fleet serves, so any node in that
//! list can serve this app; without it, one address in one text box is a single
//! point of failure for a cluster that does not have one.
//!
//! ## Where the scheme comes from
//!
//! Nobody types `https`. A node that serves TLS says so in its own fleet list
//! (`tls` and `tls_port` on every `endpoint_hint` row, AINode 0.5.30 and later),
//! and that list is the only place this app learns a scheme from. So `known`
//! carries a `tls_port` per node and every address the app is about to dial goes
//! through [`Settings::upgrade`], which looks the host up in that list. That is
//! what makes the configured primary get upgraded too: the user typed
//! `spark-1:3000`, the fleet said Spark-1 also answers https on 3443, and the
//! next poll prefers it without anybody editing a text box.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// File name inside the app data directory.
pub const STORE_FILE: &str = "settings.json";

const KEY_PRIMARY: &str = "primary";
const KEY_ALTERNATE: &str = "alternate";
const KEY_LAST_GOOD: &str = "last_good";
const KEY_KNOWN: &str = "known";
const KEY_API_KEY: &str = "api_key";

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
    /// `host:port`, already normalized. Always the node's HTTP address, which is
    /// the one every AINode always serves and never moves.
    pub address: String,
    /// The port this node serves https on, when it said it serves one.
    ///
    /// Absent for a node with no TLS and for every node older than 0.5.30, so a
    /// store written by 0.1.x loads unchanged and an http-only fleet writes a
    /// file byte-identical to the one it wrote before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_port: Option<u16>,
}

/// An address the app is willing to try, with whatever is known about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The stored slot this came from, or None for a node learned from the fleet.
    pub slot: Option<Which>,
    /// The address to dial: the https port when `tls`, else the HTTP one.
    pub address: String,
    /// The node's name when the fleet list carried one.
    pub name: Option<String>,
    /// Dial this one over https.
    pub tls: bool,
}

/// The host part of `host:port`, brackets kept for IPv6, lowercased.
///
/// The key TLS state is remembered under: a node's https port belongs to the
/// machine, not to the `host:port` string the user happened to type, so the
/// tailnet address and the LAN address of one node each find their own entry
/// while the certificate rejection below is remembered for the host that
/// presented it.
pub fn host_of(address: &str) -> String {
    let s = address.trim();
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return format!("[{}]", &rest[..end]).to_ascii_lowercase();
        }
    }
    match s.rsplit_once(':') {
        Some((host, _)) if !host.is_empty() => host.to_ascii_lowercase(),
        _ => s.to_ascii_lowercase(),
    }
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
    /// API key for a node that requires one. Optional: a node with auth off
    /// answers without it, which is every node until an operator turns auth on.
    pub api_key: Option<String>,
}

impl Settings {
    pub fn is_configured(&self) -> bool {
        !self.primary.is_empty()
    }

    /// The key to present, or None when there is nothing to present.
    pub fn key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
    }

    /// The https port this fleet said *host* serves, if it said one.
    pub fn tls_port_for(&self, host: &str) -> Option<u16> {
        let host = host.to_ascii_lowercase();
        self.known
            .iter()
            .find(|k| host_of(&k.address) == host)
            .and_then(|k| k.tls_port)
            .filter(|p| *p > 0)
    }

    /// Turn a stored `host:port` into the address and scheme to actually dial.
    ///
    /// https whenever the fleet said this host serves it, which is how the
    /// configured primary gets upgraded without the user retyping anything.
    /// `rejected` holds the hosts whose certificate this session could not
    /// verify: those fall back to http rather than failing forever, so a node
    /// with a self-signed certificate stays usable while the app says what to fix.
    pub fn upgrade(&self, address: &str, rejected: &HashSet<String>) -> (String, bool) {
        let host = host_of(address);
        if rejected.contains(&host) {
            return (address.to_string(), false);
        }
        match self.tls_port_for(&host) {
            Some(port) => (format!("{host}:{port}"), true),
            None => (address.to_string(), false),
        }
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
    pub fn configured_candidates(&self, rejected: &HashSet<String>) -> Vec<Candidate> {
        let mut out = Vec::new();
        for (slot, stored) in [
            (Which::Primary, self.address(Which::Primary)),
            (Which::Alternate, self.address(Which::Alternate)),
        ] {
            let Some(stored) = stored else { continue };
            let (address, tls) = self.upgrade(stored, rejected);
            // The alternate is skipped when it names the same address as the
            // primary, before and after the upgrade: two spellings of one node
            // must not become two probes.
            if out.iter().any(|c: &Candidate| c.address == address) {
                continue;
            }
            out.push(Candidate {
                slot: Some(slot),
                address,
                name: None,
                tls,
            });
        }
        out
    }

    /// The fleet's other nodes, in the order the fleet reported them (master
    /// first), with anything already configured left out.
    pub fn fleet_candidates(&self, rejected: &HashSet<String>) -> Vec<Candidate> {
        let configured: Vec<String> = self
            .configured_candidates(rejected)
            .into_iter()
            .map(|c| c.address)
            .collect();
        let mut out: Vec<Candidate> = Vec::new();
        for node in &self.known {
            if node.address.is_empty() {
                continue;
            }
            let (address, tls) = self.upgrade(&node.address, rejected);
            if configured.iter().any(|a| a == &address) || out.iter().any(|c| c.address == address)
            {
                continue;
            }
            out.push(Candidate {
                slot: None,
                address,
                name: Some(node.name.clone()).filter(|n| !n.is_empty()),
                tls,
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

    /// Record what the node reached at *address* said about its own https port.
    ///
    /// True when it changed something, which is what decides whether the store is
    /// written: the poller runs every ten seconds and a fleet that has not
    /// changed must not mean a disk write.
    ///
    /// This is the half of the TLS story the fleet list cannot tell. The list
    /// names each node by the address it knows itself by; this is the node
    /// speaking about the address in the app's own hand, so a node reached on its
    /// tailnet name gets its scheme learned against that name.
    pub fn note_tls(&mut self, name: &str, address: &str, tls_port: Option<u16>) -> bool {
        if address.is_empty() {
            return false;
        }
        let host = host_of(address);
        let tls_port = tls_port.filter(|p| *p > 0);
        if let Some(row) = self.known.iter_mut().find(|k| host_of(&k.address) == host) {
            // A node that turned TLS off has to lose its port here, or the app
            // goes on dialling a closed one.
            if row.tls_port == tls_port {
                return false;
            }
            row.tls_port = tls_port;
            return true;
        }
        // Nothing known about this host. Only worth an entry when there is
        // actually a scheme to remember: an http-only fleet writes the same file
        // it wrote before this existed.
        if tls_port.is_none() {
            return false;
        }
        self.known.insert(
            0,
            KnownNode {
                name: name.to_string(),
                address: address.to_string(),
                tls_port,
            },
        );
        self.known.truncate(MAX_KNOWN);
        true
    }

    /// The name the fleet gave this address, if it gave one.
    ///
    /// Matched on the host, not on the whole `host:port`: the address in use may
    /// be the upgraded https one while the list remembers the HTTP port.
    pub fn name_of(&self, address: &str) -> Option<&str> {
        let host = host_of(address);
        self.known
            .iter()
            .find(|k| k.address == address || host_of(&k.address) == host)
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
        api_key: as_str(KEY_API_KEY),
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
    // An emptied field deletes the key rather than storing "", so a user who
    // clears the box is not left with a node that thinks it has a credential.
    match settings.key() {
        Some(k) => store.set(KEY_API_KEY, k.to_string()),
        None => {
            store.delete(KEY_API_KEY);
        }
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
            tls_port: None,
        }
    }

    fn https(name: &str, address: &str, tls_port: u16) -> KnownNode {
        KnownNode {
            name: name.into(),
            address: address.into(),
            tls_port: Some(tls_port),
        }
    }

    /// No host has had its certificate refused. What every test below means
    /// unless it says otherwise.
    fn trusted() -> HashSet<String> {
        HashSet::new()
    }

    /// Every address the app would try, in order: what the two probe waves add
    /// up to.
    fn all(s: &Settings) -> Vec<String> {
        let mut out = s.configured_candidates(&trusted());
        out.extend(s.fleet_candidates(&trusted()));
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
        assert_eq!(s.configured_candidates(&trusted()).len(), 2);
        assert_eq!(
            s.configured_candidates(&trusted())[1].slot,
            Some(Which::Alternate)
        );
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
        assert_eq!(
            s.fleet_candidates(&trusted())[0].name.as_deref(),
            Some("Spark-2")
        );
        assert_eq!(s.fleet_candidates(&trusted())[0].slot, None);
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

    // ======================================================================
    // Where the scheme comes from, and where the key is kept
    // ======================================================================

    #[test]
    fn nothing_is_upgraded_until_the_fleet_says_a_node_serves_https() {
        // Every node today, and every node running AINode 0.5.29 or earlier: no
        // tls_port in the list, so nothing changes at all.
        let s = Settings {
            primary: "a:3000".into(),
            known: vec![known("Spark-1", "a:3000")],
            ..Settings::default()
        };
        assert_eq!(s.upgrade("a:3000", &trusted()), ("a:3000".into(), false));
        assert_eq!(s.tls_port_for("a"), None);
        let c = s.configured_candidates(&trusted());
        assert_eq!(c[0].address, "a:3000");
        assert!(!c[0].tls);
    }

    #[test]
    fn the_configured_primary_is_upgraded_from_the_fleets_own_list() {
        // The point of the feature: the user typed the http address, the fleet
        // said that node also answers https, and nobody had to retype anything.
        let s = Settings {
            primary: "spark-1:3000".into(),
            known: vec![https("Spark-1", "spark-1:3000", 3443)],
            ..Settings::default()
        };
        assert_eq!(
            s.upgrade("spark-1:3000", &trusted()),
            ("spark-1:3443".into(), true)
        );
        let c = s.configured_candidates(&trusted());
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].address, "spark-1:3443");
        assert!(c[0].tls);
        assert_eq!(c[0].slot, Some(Which::Primary));
    }

    #[test]
    fn the_scheme_is_remembered_per_host_and_not_per_typed_string() {
        // One node reached two ways: its LAN address serves https, and nothing is
        // known about the tailnet one, so only the first is upgraded. The TLS
        // state belongs to a machine, not to a string in a text box.
        let s = Settings {
            primary: "192.168.0.10:3000".into(),
            alternate: Some("100.122.26.9:3000".into()),
            known: vec![https("Spark-1", "192.168.0.10:3000", 3443)],
            ..Settings::default()
        };
        let c = s.configured_candidates(&trusted());
        assert_eq!(c[0].address, "192.168.0.10:3443");
        assert!(c[0].tls);
        assert_eq!(c[1].address, "100.122.26.9:3000");
        assert!(!c[1].tls);
    }

    #[test]
    fn a_refused_certificate_drops_that_host_back_to_http() {
        // Never bypassed: the app stops dialling the https port and keeps using
        // the node, while the reason stays on screen.
        let s = Settings {
            primary: "spark-1:3000".into(),
            known: vec![https("Spark-1", "spark-1:3000", 3443)],
            ..Settings::default()
        };
        let mut rejected = HashSet::new();
        rejected.insert("spark-1".to_string());
        assert_eq!(
            s.upgrade("spark-1:3000", &rejected),
            ("spark-1:3000".into(), false)
        );
        let c = s.configured_candidates(&rejected);
        assert_eq!(c[0].address, "spark-1:3000");
        assert!(!c[0].tls);
    }

    #[test]
    fn two_spellings_of_one_upgraded_address_are_not_two_probes() {
        // The primary and the alternate can differ as typed and agree once
        // upgraded (the same host, one with the port left off).
        let s = Settings {
            primary: "spark-1:3000".into(),
            alternate: Some("spark-1:3001".into()),
            known: vec![https("Spark-1", "spark-1:3000", 3443)],
            ..Settings::default()
        };
        let c = s.configured_candidates(&trusted());
        assert_eq!(c.len(), 1, "one node, one probe");
        assert_eq!(c[0].address, "spark-1:3443");
    }

    #[test]
    fn a_fleet_node_that_serves_https_is_dialled_there_too() {
        let s = Settings {
            primary: "a:3000".into(),
            known: vec![known("Spark-1", "a:3000"), https("Spark-2", "b:3000", 3443)],
            ..Settings::default()
        };
        let fleet = s.fleet_candidates(&trusted());
        assert_eq!(fleet.len(), 1);
        assert_eq!(fleet[0].address, "b:3443");
        assert!(fleet[0].tls);
        assert_eq!(fleet[0].name.as_deref(), Some("Spark-2"));
        // And the name is still found by the upgraded address.
        assert_eq!(s.name_of("b:3443"), Some("Spark-2"));
    }

    #[test]
    fn a_zero_tls_port_is_not_a_tls_port() {
        let s = Settings {
            primary: "a:3000".into(),
            known: vec![https("Spark-1", "a:3000", 0)],
            ..Settings::default()
        };
        assert_eq!(s.tls_port_for("a"), None);
        assert!(!s.configured_candidates(&trusted())[0].tls);
    }

    #[test]
    fn the_host_is_read_out_of_every_address_shape() {
        assert_eq!(host_of("10.0.0.5:3000"), "10.0.0.5");
        assert_eq!(host_of("Spark-1:3000"), "spark-1");
        assert_eq!(host_of("spark-1.local"), "spark-1.local");
        assert_eq!(host_of("[fd00::1]:3443"), "[fd00::1]");
        assert_eq!(host_of("[fd00::1]"), "[fd00::1]");
        assert_eq!(host_of("  a:1  "), "a");
    }

    #[test]
    fn an_ipv6_node_is_upgraded_without_losing_its_brackets() {
        let s = Settings {
            primary: "[fd00::1]:3000".into(),
            known: vec![https("Spark-1", "[fd00::1]:3000", 3443)],
            ..Settings::default()
        };
        assert_eq!(
            s.upgrade("[fd00::1]:3000", &trusted()),
            ("[fd00::1]:3443".into(), true)
        );
    }

    #[test]
    fn the_key_is_optional_and_whitespace_is_not_a_key() {
        let mut s = Settings::default();
        assert_eq!(s.key(), None);
        s.api_key = Some(String::new());
        assert_eq!(s.key(), None);
        s.api_key = Some("   ".into());
        assert_eq!(s.key(), None, "a box with a space in it is an empty box");
        s.api_key = Some("  ainode_abc123  ".into());
        assert_eq!(
            s.key(),
            Some("ainode_abc123"),
            "trimmed, because paste adds space"
        );
    }

    #[test]
    fn an_empty_fleet_list_leaves_todays_behaviour_exactly() {
        // What a node older than /api/cluster/endpoint gives us: nothing.
        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("b:3000".into()),
            ..Settings::default()
        };
        assert!(s.fleet_candidates(&trusted()).is_empty());
        assert_eq!(all(&s), vec!["a:3000", "b:3000"]);
    }
}
