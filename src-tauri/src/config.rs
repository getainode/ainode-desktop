//! The one setting: where the master AINode lives.
//!
//! Two fields sit behind it, a primary address and an optional alternate
//! (for example the tailnet address of the same node). Both are stored as
//! plain `host:port` strings. We also remember which one answered last so
//! the next launch tries the likely winner first.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

/// File name inside the app data directory.
pub const STORE_FILE: &str = "settings.json";

const KEY_PRIMARY: &str = "primary";
const KEY_ALTERNATE: &str = "alternate";
const KEY_LAST_GOOD: &str = "last_good";

/// Which of the two stored addresses is meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Which {
    Primary,
    Alternate,
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

    /// Every address worth probing, primary first.
    pub fn candidates(&self) -> Vec<(Which, String)> {
        let mut out = Vec::new();
        if let Some(p) = self.address(Which::Primary) {
            out.push((Which::Primary, p.to_string()));
        }
        if let Some(a) = self.address(Which::Alternate) {
            if a != self.primary {
                out.push((Which::Alternate, a.to_string()));
            }
        }
        out
    }

    /// Reverse lookup: which slot holds `address`?
    pub fn which_of(&self, address: &str) -> Option<Which> {
        if self.address(Which::Primary) == Some(address) {
            Some(Which::Primary)
        } else if self.address(Which::Alternate) == Some(address) {
            Some(Which::Alternate)
        } else {
            None
        }
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
    Settings {
        primary: as_str(KEY_PRIMARY).unwrap_or_default(),
        alternate: as_str(KEY_ALTERNATE),
        last_good,
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

    #[test]
    fn candidates_skip_empty_and_duplicate_alternate() {
        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("a:3000".into()),
            last_good: None,
        };
        assert_eq!(s.candidates(), vec![(Which::Primary, "a:3000".to_string())]);

        let s = Settings {
            primary: "a:3000".into(),
            alternate: Some("b:3000".into()),
            last_good: None,
        };
        assert_eq!(s.candidates().len(), 2);
        assert_eq!(s.which_of("b:3000"), Some(Which::Alternate));
        assert_eq!(s.which_of("c:3000"), None);

        let s = Settings::default();
        assert!(!s.is_configured());
        assert!(s.candidates().is_empty());
    }
}
