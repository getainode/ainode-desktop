//! Read-only HTTP calls to a master AINode. Nothing in this app ever POSTs.

use crate::nodes::{parse_nodes, NodeRow};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long a probe may take before we call the address unreachable.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The parts of `/api/status` shown in Settings and used for probing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub node_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub engine_ready: bool,
    #[serde(default)]
    pub load_phase: Option<String>,
    /// Where else this fleet answers. AINode 0.5.28 and later put it on every
    /// status poll, so the app learns its fallbacks without a second request.
    /// Empty on an older node, which is simply the behaviour before this
    /// existed: the configured addresses and nothing more.
    #[serde(default)]
    pub endpoint_hint: Vec<EndpointNode>,
}

/// One row of `endpoint_hint`, or of `GET /api/cluster/endpoint`'s `nodes`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EndpointNode {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// `GET /api/cluster/endpoint`. Only `nodes` is used here: `self` and `master`
/// say the same thing about addresses this app already has in that list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(default)]
    pub nodes: Vec<EndpointNode>,
}

/// Is this a host another machine can dial?
///
/// The node already refuses to publish a loopback address, but this list comes
/// off the network and the cost of trusting it is an app that spends an outage
/// dialling itself. Also guards the placeholder a node with no address at all
/// reports.
fn usable_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    !(h.is_empty()
        || h.starts_with("127.")
        || matches!(
            h.as_str(),
            "localhost" | "localhost.localdomain" | "0.0.0.0" | "::" | "::1" | "unknown" | "none"
        ))
}

impl EndpointNode {
    /// `host:port` for a row worth keeping, else None.
    ///
    /// The node's own `url` is not parsed: this app speaks `host:port`
    /// everywhere, the two fields are built from each other on the server, and a
    /// row with no usable host has a null url anyway.
    pub fn address(&self) -> Option<String> {
        let host = self.host.as_deref().unwrap_or("").trim();
        if !usable_host(host) {
            return None;
        }
        let port = self.port.filter(|p| *p > 0)?;
        Some(format!("{host}:{port}"))
    }

    pub fn known(&self) -> Option<crate::config::KnownNode> {
        Some(crate::config::KnownNode {
            name: self.name.clone().unwrap_or_default(),
            address: self.address()?,
        })
    }
}

/// The rows worth remembering, in the order the fleet reported them.
pub fn known_nodes(rows: &[EndpointNode]) -> Vec<crate::config::KnownNode> {
    rows.iter().filter_map(EndpointNode::known).collect()
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .user_agent(concat!("AINode-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client with static config")
}

/// `http://host:port`
pub fn base_url(address: &str) -> String {
    format!("http://{address}")
}

/// Where the main window goes once the master answers.
pub fn ui_url(address: &str) -> String {
    format!("http://{address}/")
}

fn short_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "no answer within 2 seconds".to_string()
    } else if e.is_connect() {
        "connection refused or host unreachable".to_string()
    } else if let Some(status) = e.status() {
        format!("HTTP {status}")
    } else {
        // reqwest's Display chains the source; keep the first line.
        e.to_string()
            .lines()
            .next()
            .unwrap_or("request failed")
            .to_string()
    }
}

/// `GET /api/status` on `address`.
pub async fn fetch_status(client: &reqwest::Client, address: &str) -> Result<Status, String> {
    let url = format!("{}/api/status", base_url(address));
    let resp = client.get(&url).send().await.map_err(short_error)?;
    let resp = resp.error_for_status().map_err(short_error)?;
    let status: Status = resp
        .json()
        .await
        .map_err(|e| format!("bad status JSON: {}", short_error(e)))?;
    Ok(status)
}

/// `GET /api/cluster/endpoint` on `address`.
///
/// The fallback for a node that answers `/api/status` without an
/// `endpoint_hint`. Deliberately a separate call and not the first choice: the
/// hint costs nothing, and this path 404s on a node too old to have either.
/// It needs no API key by design, which is the whole point of it: a client
/// looking for a live address cannot be asked for the key held by the node that
/// just went away.
pub async fn fetch_endpoint(
    client: &reqwest::Client,
    address: &str,
) -> Result<Vec<EndpointNode>, String> {
    let url = format!("{}/api/cluster/endpoint", base_url(address));
    let resp = client.get(&url).send().await.map_err(short_error)?;
    let resp = resp.error_for_status().map_err(short_error)?;
    let doc: Endpoint = resp
        .json()
        .await
        .map_err(|e| format!("bad endpoint JSON: {}", short_error(e)))?;
    Ok(doc.nodes)
}

/// `GET /api/nodes` on `address`, either response shape.
pub async fn fetch_nodes(client: &reqwest::Client, address: &str) -> Result<Vec<NodeRow>, String> {
    let url = format!("{}/api/nodes", base_url(address));
    let resp = client.get(&url).send().await.map_err(short_error)?;
    let resp = resp.error_for_status().map_err(short_error)?;
    let body = resp.text().await.map_err(short_error)?;
    parse_nodes(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real answer from a node running this build, trimmed to the fields read
    /// here. Pasted from a live `GET /api/cluster/endpoint`, not invented.
    const ENDPOINT_BODY: &str = r#"{
      "self": {"name": "Spark-1", "host": "192.168.0.10", "port": 3000,
               "version": "0.5.28", "role": "master"},
      "master": {"name": "Spark-1", "host": "192.168.0.10", "port": 3000,
                 "url": "http://192.168.0.10:3000"},
      "nodes": [
        {"name": "Spark-1", "host": "192.168.0.10", "port": 3000,
         "version": "0.5.28", "role": "master", "url": "http://192.168.0.10:3000"},
        {"name": "Spark-2", "host": "192.168.0.11", "port": 3000,
         "version": null, "role": "worker", "url": "http://192.168.0.11:3000"},
        {"name": "unknown", "host": "", "port": 3000,
         "version": null, "role": "worker", "url": null}
      ],
      "generated_at": 1789871479.29
    }"#;

    #[test]
    fn the_endpoint_document_becomes_a_list_of_addresses() {
        let doc: Endpoint = serde_json::from_str(ENDPOINT_BODY).expect("endpoint parses");
        let known = known_nodes(&doc.nodes);
        assert_eq!(known.len(), 2, "the row with no host is not an address");
        assert_eq!(known[0].name, "Spark-1");
        assert_eq!(known[0].address, "192.168.0.10:3000");
        assert_eq!(known[1].address, "192.168.0.11:3000");
    }

    #[test]
    fn status_carries_the_hint_and_survives_a_node_without_one() {
        let with = r#"{"node_name": "Spark-1", "version": "0.5.28",
                       "endpoint_hint": [{"name": "Spark-2", "host": "10.0.0.2", "port": 3000}]}"#;
        let s: Status = serde_json::from_str(with).expect("status parses");
        assert_eq!(s.endpoint_hint.len(), 1);
        assert_eq!(
            known_nodes(&s.endpoint_hint)[0].address,
            "10.0.0.2:3000".to_string()
        );

        // 0.5.27 and earlier: no such field, and the app must not care.
        let without = r#"{"node_name": "Spark-1", "version": "0.5.27", "engine_ready": true}"#;
        let s: Status = serde_json::from_str(without).expect("older status parses");
        assert!(s.endpoint_hint.is_empty());
        assert_eq!(s.node_name.as_deref(), Some("Spark-1"));
    }

    #[test]
    fn a_loopback_or_empty_row_is_never_an_address() {
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.53.0.1",
            "0.0.0.0",
            "::1",
            "[::1]",
            "unknown",
            "",
            "   ",
        ] {
            let row = EndpointNode {
                host: Some(host.to_string()),
                port: Some(3000),
                ..EndpointNode::default()
            };
            assert_eq!(row.address(), None, "{host} should not be dialled");
        }
        // A port of zero or none is no address either.
        let row = EndpointNode {
            host: Some("10.0.0.5".into()),
            port: Some(0),
            ..EndpointNode::default()
        };
        assert_eq!(row.address(), None);
        let row = EndpointNode {
            host: Some("10.0.0.5".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.address(), None);
        // And the ordinary case still works.
        let row = EndpointNode {
            host: Some("10.0.0.5".into()),
            port: Some(3100),
            ..EndpointNode::default()
        };
        assert_eq!(row.address().as_deref(), Some("10.0.0.5:3100"));
    }
}
