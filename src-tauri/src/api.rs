//! Read-only HTTP calls to a master AINode. Nothing in this app ever POSTs.
//!
//! Two things every call here does since 0.2.0:
//!
//! * **It carries the API key**, when one is configured, as `Authorization:
//!   Bearer`. On every request, `/api/cluster/endpoint` included: that route is
//!   exempt from the key rule on the node's side, but a client that sends the
//!   key anyway is simpler than a client that decides per route, and it costs a
//!   header.
//! * **It reports WHY it failed**, not just that it did. Three of the reasons
//!   need different words on screen and nothing else can tell them apart: a node
//!   that wants a key (401) is not offline, and a certificate this machine does
//!   not trust is not a network problem. See [`ProbeError`].
//!
//! Nothing here can be told to skip certificate verification. There is no flag
//! for it and no code path to it: an untrusted certificate is reported with the
//! fix, because a client that quietly accepts any certificate is a client whose
//! API key travels to whoever answered.

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
    /// What THIS node says about its own TLS. Absent before AINode 0.5.29.
    ///
    /// The authority on the scheme for the address the app just used, and the
    /// reason it is read at all: the fleet list names each node by the address
    /// that node knows itself by, which is often not the one this app dials. A
    /// node reached on its MagicDNS name reports its LAN address in the list, so
    /// matching the list by host would leave the tailnet address on http forever.
    /// This block is that node talking about itself, on the connection in hand.
    #[serde(default)]
    pub tls: Option<TlsStatus>,
}

/// The `tls` block of `/api/status`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TlsStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub port: Option<u16>,
    /// True for a certificate this node generated itself. Reported, not acted
    /// on: whether THIS machine trusts it is a different question, and the only
    /// one that decides anything here.
    #[serde(default)]
    pub self_signed: Option<bool>,
}

impl Status {
    /// The https port the node that answered serves on, if it serves one.
    pub fn https_port(&self) -> Option<u16> {
        let tls = self.tls.as_ref()?;
        if !tls.enabled {
            return None;
        }
        tls.port.filter(|p| *p > 0)
    }
}

/// One row of `endpoint_hint`, or of `GET /api/cluster/endpoint`'s `nodes`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EndpointNode {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    /// The node's HTTP port. Always present, always the plain one: AINode never
    /// moves it, TLS or no TLS, because the whole fleet talks to it.
    #[serde(default)]
    pub port: Option<u16>,
    /// Whether this node serves https as well. Absent on AINode 0.5.29 and
    /// earlier, which is exactly the behaviour before it existed: http only.
    #[serde(default)]
    pub tls: Option<bool>,
    /// Where https is, when `tls`.
    #[serde(default)]
    pub tls_port: Option<u16>,
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
    /// `host:port` for a row worth keeping, else None. Always the HTTP port.
    ///
    /// The node's own `url` is not parsed for the address: this app speaks
    /// `host:port` everywhere, the fields are built from each other on the
    /// server, and a row with no usable host has a null url anyway.
    pub fn address(&self) -> Option<String> {
        let host = self.host.as_deref().unwrap_or("").trim();
        if !usable_host(host) {
            return None;
        }
        let port = self.port.filter(|p| *p > 0)?;
        Some(format!("{host}:{port}"))
    }

    /// The https port this row advertises, if any.
    ///
    /// The explicit fields first, since 0.5.30 carries both. The `url` is the
    /// fallback and not the source: a row whose `tls` is true but whose
    /// `tls_port` did not arrive still names the port in its url, and reading it
    /// there is cheaper than losing the scheme.
    pub fn https_port(&self) -> Option<u16> {
        if self.tls == Some(false) {
            return None;
        }
        if let Some(port) = self.tls_port.filter(|p| *p > 0) {
            return Some(port);
        }
        if self.tls != Some(true) {
            return None;
        }
        let url = self.url.as_deref()?.trim();
        let rest = url.strip_prefix("https://")?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let port = if let Some(after) = authority.rsplit_once(']') {
            after.1.strip_prefix(':')?
        } else {
            authority.rsplit_once(':')?.1
        };
        port.parse::<u16>().ok().filter(|p| *p > 0)
    }

    pub fn known(&self) -> Option<crate::config::KnownNode> {
        Some(crate::config::KnownNode {
            name: self.name.clone().unwrap_or_default(),
            address: self.address()?,
            tls_port: self.https_port(),
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

/// Why a request did not produce an answer.
///
/// Three cases, because they need three different sentences and the user can act
/// on each one differently:
///
/// * `Unauthorized`: the node is up and wants a key. Reporting this as "offline"
///   sent people to look at a machine that was working.
/// * `Certificate`: this machine does not trust what the node presented. Not a
///   network fault and not something to work around; the app falls back to the
///   node's http port for the session and says what to fix.
/// * `Unreachable`: everything else, in the words it already used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError {
    Unauthorized,
    Certificate(String),
    Unreachable(String),
}

/// What a viewer is told to do about an untrusted certificate. Both halves are
/// real fixes and neither of them is "ignore it".
pub const CERT_FIX: &str = "trust this certificate on this machine, \
or give the node a real one with `ainode tls enable --tailscale`";

impl ProbeError {
    /// One line, for the menu bar and the waiting page.
    pub fn message(&self) -> String {
        match self {
            ProbeError::Unauthorized => "this node wants an API key".to_string(),
            // "did not check out" rather than "is not trusted": the same branch
            // catches a certificate issued for a different name, which is a
            // perfectly trusted certificate on the wrong address.
            ProbeError::Certificate(detail) => {
                format!("its certificate did not check out here ({detail}): {CERT_FIX}")
            }
            ProbeError::Unreachable(detail) => detail.clone(),
        }
    }

    pub fn is_certificate(&self) -> bool {
        matches!(self, ProbeError::Certificate(_))
    }

    pub fn is_unauthorized(&self) -> bool {
        matches!(self, ProbeError::Unauthorized)
    }
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Markers that mean "the certificate did not check out", across TLS backends.
///
/// Security.framework on macOS, schannel on Windows and rustls all word it
/// differently, none of them gives reqwest a typed error for it, and reqwest
/// reports every one of them as a plain connect failure, so the source chain's
/// text is the only thing there is to read. Every string here is one a real
/// stack produced: macOS answers a wrong-name certificate with "A host name
/// mismatch has occurred." and nothing else in the chain mentions certificates
/// at all, which is why the list has to carry the wording and not just the word.
const CERT_MARKERS: [&str; 13] = [
    "certificate",
    "self-signed",
    "self signed",
    "unknownissuer",
    "not trusted",
    "untrusted",
    "certverificationerror",
    "hostnamemismatch",
    "host name mismatch",
    "hostname mismatch",
    "chain of trust",
    "verify failed",
    "unable to get local issuer",
];

/// Everything reqwest chained onto this error, lowercased, so the markers above
/// can be looked for in the real cause rather than in the top-level summary
/// ("error sending request", which says nothing).
fn error_chain(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(err) = source {
        text.push_str("; ");
        text.push_str(&err.to_string());
        source = err.source();
    }
    text.to_ascii_lowercase()
}

fn probe_error(e: reqwest::Error) -> ProbeError {
    if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
        return ProbeError::Unauthorized;
    }
    let chain = error_chain(&e);
    if CERT_MARKERS.iter().any(|m| chain.contains(m)) {
        // The innermost line is the one that names the problem.
        let detail = chain
            .rsplit("; ")
            .next()
            .unwrap_or("verification failed")
            .to_string();
        return ProbeError::Certificate(detail);
    }
    ProbeError::Unreachable(short_error(e))
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

/// `https://host:port` or `http://host:port`.
pub fn base_url(address: &str, tls: bool) -> String {
    let scheme = if tls { "https" } else { "http" };
    format!("{scheme}://{address}")
}

/// Where the main window goes once the master answers.
pub fn ui_url(address: &str, tls: bool) -> String {
    format!("{}/", base_url(address, tls))
}

/// A GET with the API key attached, when there is one to attach.
fn get(client: &reqwest::Client, url: &str, api_key: Option<&str>) -> reqwest::RequestBuilder {
    let req = client.get(url);
    match api_key {
        Some(key) => req.bearer_auth(key),
        None => req,
    }
}

/// `GET /api/status`.
pub async fn fetch_status(
    client: &reqwest::Client,
    address: &str,
    tls: bool,
    api_key: Option<&str>,
) -> Result<Status, ProbeError> {
    let url = format!("{}/api/status", base_url(address, tls));
    let resp = get(client, &url, api_key)
        .send()
        .await
        .map_err(probe_error)?;
    let resp = resp.error_for_status().map_err(probe_error)?;
    let status: Status = resp
        .json()
        .await
        .map_err(|e| ProbeError::Unreachable(format!("bad status JSON: {}", short_error(e))))?;
    Ok(status)
}

/// `GET /api/cluster/endpoint`.
///
/// The fallback for a node that answers `/api/status` without an
/// `endpoint_hint`. Deliberately a separate call and not the first choice: the
/// hint costs nothing, and this path 404s on a node too old to have either.
/// It needs no API key by design, which is the whole point of it: a client
/// looking for a live address cannot be asked for the key held by the node that
/// just went away. The key goes along anyway, because one rule for every request
/// is easier to be sure of than a per-route rule.
pub async fn fetch_endpoint(
    client: &reqwest::Client,
    address: &str,
    tls: bool,
    api_key: Option<&str>,
) -> Result<Vec<EndpointNode>, ProbeError> {
    let url = format!("{}/api/cluster/endpoint", base_url(address, tls));
    let resp = get(client, &url, api_key)
        .send()
        .await
        .map_err(probe_error)?;
    let resp = resp.error_for_status().map_err(probe_error)?;
    let doc: Endpoint = resp
        .json()
        .await
        .map_err(|e| ProbeError::Unreachable(format!("bad endpoint JSON: {}", short_error(e))))?;
    Ok(doc.nodes)
}

/// `GET /api/nodes`, either response shape.
pub async fn fetch_nodes(
    client: &reqwest::Client,
    address: &str,
    tls: bool,
    api_key: Option<&str>,
) -> Result<Vec<NodeRow>, ProbeError> {
    let url = format!("{}/api/nodes", base_url(address, tls));
    let resp = get(client, &url, api_key)
        .send()
        .await
        .map_err(probe_error)?;
    let resp = resp.error_for_status().map_err(probe_error)?;
    let body = resp.text().await.map_err(probe_error)?;
    parse_nodes(&body).map_err(ProbeError::Unreachable)
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

    // ======================================================================
    // The scheme on the wire, and the key on every request
    // ======================================================================

    #[test]
    fn the_url_carries_the_scheme_it_was_asked_for() {
        assert_eq!(base_url("10.0.0.5:3000", false), "http://10.0.0.5:3000");
        assert_eq!(base_url("10.0.0.5:3443", true), "https://10.0.0.5:3443");
        assert_eq!(ui_url("10.0.0.5:3443", true), "https://10.0.0.5:3443/");
        assert_eq!(ui_url("10.0.0.5:3000", false), "http://10.0.0.5:3000/");
    }

    /// A real answer from a node running this branch, over https. Pasted from the
    /// isolated instance on Spark-3, not invented: `port` is still the HTTP port,
    /// `url` is the one to prefer, and the peer row is http because a peer's TLS
    /// state is not on the discovery wire.
    const TLS_ENDPOINT_BODY: &str = r#"{
      "self": {"name": "Spark-3-DGX", "host": "192.168.0.13", "port": 3141,
               "tls": true, "tls_port": 3541, "url": "https://192.168.0.13:3541",
               "version": "0.5.29", "role": "master"},
      "nodes": [
        {"name": "Spark-3-DGX", "host": "192.168.0.13", "port": 3141,
         "tls": true, "tls_port": 3541, "url": "https://192.168.0.13:3541",
         "version": "0.5.29", "role": "master"},
        {"name": "Spark-2", "host": "192.168.0.12", "port": 3000,
         "tls": false, "tls_port": null, "url": "http://192.168.0.12:3000",
         "version": "0.5.29", "role": "worker"}
      ],
      "generated_at": 1789954130.9
    }"#;

    #[test]
    fn a_tls_row_is_remembered_with_its_https_port_and_its_http_address() {
        let doc: Endpoint = serde_json::from_str(TLS_ENDPOINT_BODY).expect("parses");
        let known = known_nodes(&doc.nodes);
        assert_eq!(known.len(), 2);
        // The stored address stays the HTTP one, which never moves; the scheme
        // rides beside it.
        assert_eq!(known[0].address, "192.168.0.13:3141");
        assert_eq!(known[0].tls_port, Some(3541));
        assert_eq!(known[1].address, "192.168.0.12:3000");
        assert_eq!(known[1].tls_port, None, "a peer row is http");
    }

    #[test]
    fn a_node_older_than_the_tls_fields_is_http_and_says_nothing_else() {
        // The exact body 0.5.28 answers. The fields are absent, not false.
        let doc: Endpoint = serde_json::from_str(ENDPOINT_BODY).expect("parses");
        for row in &doc.nodes {
            assert_eq!(row.tls, None);
            assert_eq!(row.https_port(), None);
        }
        assert!(known_nodes(&doc.nodes).iter().all(|k| k.tls_port.is_none()));
    }

    #[test]
    fn the_https_port_falls_back_to_the_url_when_only_the_flag_arrived() {
        let row = EndpointNode {
            host: Some("10.0.0.5".into()),
            port: Some(3000),
            tls: Some(true),
            tls_port: None,
            url: Some("https://10.0.0.5:3443".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.https_port(), Some(3443));

        // An http url with the flag set is not a port, and neither is a url with
        // no port in it at all.
        let row = EndpointNode {
            tls: Some(true),
            url: Some("http://10.0.0.5:3000".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.https_port(), None);
        let row = EndpointNode {
            tls: Some(true),
            url: Some("https://10.0.0.5".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.https_port(), None);
        // tls: false wins over anything in the url.
        let row = EndpointNode {
            tls: Some(false),
            tls_port: Some(3443),
            url: Some("https://10.0.0.5:3443".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.https_port(), None);
    }

    #[test]
    fn an_ipv6_url_gives_up_its_port_and_not_its_address() {
        let row = EndpointNode {
            tls: Some(true),
            url: Some("https://[fd00::1]:3443/".into()),
            ..EndpointNode::default()
        };
        assert_eq!(row.https_port(), Some(3443));
    }

    #[test]
    fn the_three_failures_read_differently() {
        // The whole reason ProbeError exists: a node that wants a key is UP, and
        // calling that offline sent people to look at a working machine.
        assert_eq!(
            ProbeError::Unauthorized.message(),
            "this node wants an API key"
        );
        assert!(ProbeError::Unauthorized.is_unauthorized());
        assert!(!ProbeError::Unauthorized.is_certificate());

        let cert = ProbeError::Certificate("self-signed certificate".into());
        assert!(cert.is_certificate());
        assert!(!cert.is_unauthorized());
        // The fix is in the sentence, and neither half of it is "ignore it".
        assert!(cert.message().contains("did not check out here"));
        assert!(cert.message().contains("trust this certificate"));
        assert!(cert.message().contains("ainode tls enable --tailscale"));
        assert!(!cert.message().to_lowercase().contains("ignore"));

        let down = ProbeError::Unreachable("no answer within 2 seconds".into());
        assert_eq!(down.message(), "no answer within 2 seconds");
        assert!(!down.is_certificate() && !down.is_unauthorized());
    }

    #[test]
    fn every_tls_backends_wording_is_recognised_as_a_certificate_problem() {
        // None of these is typed by reqwest, and the three backends word it
        // differently, so the text is all there is. Real strings: Security
        // framework on macOS, schannel on Windows, rustls everywhere.
        for text in [
            "The certificate for this server is invalid",
            "certificate verify failed: self-signed certificate",
            "invalid peer certificate: UnknownIssuer",
            "the certificate chain was issued by an authority that is not trusted",
            "invalid peer certificate: HostnameMismatch",
            // macOS, observed against a real tailscale certificate on the wrong
            // address: nothing else in the chain mentions a certificate at all.
            "A host name mismatch has occurred.",
        ] {
            let lowered = text.to_ascii_lowercase();
            assert!(
                CERT_MARKERS.iter().any(|m| lowered.contains(m)),
                "not recognised: {text}"
            );
        }
        // And a plain network failure is not one of them.
        for text in [
            "connection refused",
            "operation timed out",
            "dns error: failed to lookup address information",
        ] {
            let lowered = text.to_ascii_lowercase();
            assert!(
                !CERT_MARKERS.iter().any(|m| lowered.contains(m)),
                "wrongly recognised: {text}"
            );
        }
    }

    /// A stub that records the request line and the headers it was sent, then
    /// answers `body`. Real sockets, because the thing worth proving is what went
    /// out on the wire.
    struct Recorder {
        address: String,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Recorder {
        fn serving(status_line: &'static str, body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
            let address = listener.local_addr().expect("bound").to_string();
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (sink, flag) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                for stream in listener.incoming() {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let Ok(mut stream) = stream else { return };
                    let mut buf = [0u8; 2048];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if let Ok(mut s) = sink.lock() {
                        s.push(String::from_utf8_lossy(&buf[..n]).to_string());
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                }
            });
            Recorder {
                address,
                seen,
                stop,
            }
        }

        fn requests(&self) -> Vec<String> {
            self.seen.lock().map(|s| s.clone()).unwrap_or_default()
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn the_key_goes_on_every_request_including_the_endpoint_fetch() {
        let node = Recorder::serving("200 OK", r#"{"node_name": "Spark-1", "nodes": []}"#);
        let client = client();
        let key = Some("ainode_secret");

        let _ = fetch_status(&client, &node.address, false, key).await;
        let _ = fetch_endpoint(&client, &node.address, false, key).await;
        let _ = fetch_nodes(&client, &node.address, false, key).await;

        let reqs = node.requests();
        assert_eq!(reqs.len(), 3, "three calls, three requests");
        for req in &reqs {
            // Lowercased, because hyper normalises header names on the wire.
            assert!(
                req.to_ascii_lowercase()
                    .contains("authorization: bearer ainode_secret"),
                "no key on:\n{req}"
            );
        }
        // /api/cluster/endpoint needs no key on the node's side; it gets one
        // anyway, because one rule for every request is easier to be sure of.
        assert!(reqs
            .iter()
            .any(|r| r.starts_with("GET /api/cluster/endpoint")));
    }

    #[tokio::test]
    async fn no_key_configured_sends_no_authorization_header() {
        let node = Recorder::serving("200 OK", r#"{"node_name": "Spark-1"}"#);
        let _ = fetch_status(&client(), &node.address, false, None).await;
        let req = node.requests().remove(0);
        assert!(!req.to_lowercase().contains("authorization"), "{req}");
    }

    #[tokio::test]
    async fn a_401_is_reported_as_wanting_a_key_and_not_as_offline() {
        let node = Recorder::serving("401 Unauthorized", r#"{"error": "no"}"#);
        let err = fetch_status(&client(), &node.address, false, None)
            .await
            .expect_err("401 is an error");
        assert_eq!(err, ProbeError::Unauthorized);
        assert_eq!(err.message(), "this node wants an API key");
    }

    #[tokio::test]
    async fn https_against_a_plain_http_port_is_not_reported_as_a_certificate() {
        // A misconfigured port is a transport failure, and saying "certificate"
        // about it would send the user to fix the wrong thing.
        let node = Recorder::serving("200 OK", r#"{"node_name": "Spark-1"}"#);
        let err = fetch_status(&client(), &node.address, true, None)
            .await
            .expect_err("a plain socket cannot do TLS");
        assert!(!err.is_certificate(), "{err:?}");
        assert!(!err.is_unauthorized(), "{err:?}");
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
