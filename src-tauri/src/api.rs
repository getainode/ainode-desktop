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

/// `GET /api/nodes` on `address`, either response shape.
pub async fn fetch_nodes(client: &reqwest::Client, address: &str) -> Result<Vec<NodeRow>, String> {
    let url = format!("{}/api/nodes", base_url(address));
    let resp = client.get(&url).send().await.map_err(short_error)?;
    let resp = resp.error_for_status().map_err(short_error)?;
    let body = resp.text().await.map_err(short_error)?;
    parse_nodes(&body)
}
