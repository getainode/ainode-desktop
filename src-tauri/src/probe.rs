//! Decide which address to use.
//!
//! The configured pair is probed together on every poll, which is what this app
//! has always cost. Only when neither answers does it fan out to the fleet list
//! the nodes themselves reported: those addresses are a fallback, not a
//! heartbeat, and probing six nodes every ten seconds would be rude for nothing.
//!
//! The pick is a pure function over `(address, answered)` pairs so every rule
//! below is tested against canned results.

use crate::api::{self, Status};
use crate::config::{Candidate, Settings, Which};

/// Outcome of one probe.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// The configured slot, or None for an address learned from the fleet.
    pub slot: Option<Which>,
    pub address: String,
    /// The node's name: its own word from `/api/status` once it answered,
    /// otherwise whatever the fleet list called it.
    pub name: Option<String>,
    pub result: Result<Status, String>,
}

impl Probe {
    pub fn ok(&self) -> bool {
        self.result.is_ok()
    }

    /// The name to show for the node that answered.
    pub fn node_name(&self) -> Option<String> {
        if let Ok(status) = &self.result {
            if let Some(name) = status.node_name.as_deref().filter(|n| !n.is_empty()) {
                return Some(name.to_string());
            }
        }
        self.name.clone().filter(|n| !n.is_empty())
    }
}

/// Pick an address from probe results.
///
/// Rules, in order:
/// 1. The primary, whenever it answered. It is the address the user chose, so
///    the app goes home as soon as home is back, rather than living on a
///    fallback until the next restart.
/// 2. Otherwise `preferred` (the address in use, or the one that worked last
///    time) when it answered: no flapping between two live fallbacks.
/// 3. Otherwise the first address that answered, in the order given: the
///    alternate, then the fleet with the master first.
/// 4. Nothing answered: `None`.
pub fn choose(
    results: &[(String, bool)],
    primary: &str,
    preferred: Option<&str>,
) -> Option<String> {
    let answered = |address: &str| results.iter().any(|(a, ok)| *ok && a.as_str() == address);
    if !primary.is_empty() && answered(primary) {
        return Some(primary.to_string());
    }
    if let Some(p) = preferred.filter(|p| !p.is_empty() && answered(p)) {
        return Some(p.to_string());
    }
    results
        .iter()
        .find(|(_, ok)| *ok)
        .map(|(a, _)| a.to_string())
}

/// Probe a set of candidates at once.
pub async fn probe_all(client: &reqwest::Client, candidates: &[Candidate]) -> Vec<Probe> {
    let futures = candidates.iter().map(|candidate| {
        let candidate = candidate.clone();
        // reqwest clients are cheap handles around a shared pool.
        let client = client.clone();
        async move {
            let result = api::fetch_status(&client, &candidate.address).await;
            Probe {
                slot: candidate.slot,
                address: candidate.address,
                name: candidate.name,
                result,
            }
        }
    });
    futures_join_all(futures).await
}

/// The addresses to try first: the configured pair, plus the one in use when it
/// is a fleet node (so a working fallback is not dropped for a wave two it was
/// never in).
fn first_wave(settings: &Settings, current: Option<&str>) -> Vec<Candidate> {
    let mut wave = settings.configured_candidates();
    if let Some(address) = current.filter(|a| !a.is_empty()) {
        if !wave.iter().any(|c| c.address == address) {
            wave.push(Candidate {
                slot: None,
                address: address.to_string(),
                name: settings.name_of(address).map(str::to_string),
            });
        }
    }
    wave
}

/// Probe and choose. `current` is the address in use right now.
///
/// Wave one is [`first_wave`]. Wave two, only if none of it answered, is every
/// remaining fleet node in the order the fleet reported them, master first.
pub async fn find_master(
    client: &reqwest::Client,
    settings: &Settings,
    current: Option<&str>,
) -> Option<Probe> {
    let mut probes = probe_all(client, &first_wave(settings, current)).await;
    if !probes.iter().any(Probe::ok) {
        let tried: Vec<String> = probes.iter().map(|p| p.address.clone()).collect();
        let rest: Vec<Candidate> = settings
            .fleet_candidates()
            .into_iter()
            .filter(|c| !tried.iter().any(|a| a == &c.address))
            .collect();
        if !rest.is_empty() {
            probes.extend(probe_all(client, &rest).await);
        }
    }
    let preferred = current.map(str::to_string).or_else(|| {
        settings
            .last_good
            .and_then(|w| settings.address(w).map(str::to_string))
    });
    let flags: Vec<(String, bool)> = probes.iter().map(|p| (p.address.clone(), p.ok())).collect();
    let pick = choose(&flags, &settings.primary, preferred.as_deref())?;
    probes.into_iter().find(|p| p.address == pick)
}

/// Run a handful of futures concurrently without pulling in the whole
/// `futures` crate: join them through the tokio runtime.
async fn futures_join_all<F, T>(iter: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = iter.map(tauri::async_runtime::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KnownNode;

    const P: &str = "primary:3000";
    const A: &str = "alternate:3000";
    const N2: &str = "spark2:3000";
    const N3: &str = "spark3:3000";

    fn flags(pairs: &[(&str, bool)]) -> Vec<(String, bool)> {
        pairs.iter().map(|(a, ok)| (a.to_string(), *ok)).collect()
    }

    fn settings() -> Settings {
        Settings {
            primary: P.into(),
            alternate: Some(A.into()),
            known: vec![
                KnownNode {
                    name: "Spark-1".into(),
                    address: P.into(),
                },
                KnownNode {
                    name: "Spark-2".into(),
                    address: N2.into(),
                },
                KnownNode {
                    name: "Spark-3".into(),
                    address: N3.into(),
                },
            ],
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_answers_means_none() {
        assert_eq!(choose(&flags(&[(P, false), (A, false)]), P, None), None);
        assert_eq!(choose(&flags(&[(P, false), (A, false)]), P, Some(A)), None);
        assert_eq!(choose(&[], P, Some(P)), None);
    }

    #[test]
    fn only_one_answers() {
        assert_eq!(
            choose(&flags(&[(P, true), (A, false)]), P, None).as_deref(),
            Some(P)
        );
        assert_eq!(
            choose(&flags(&[(P, false), (A, true)]), P, None).as_deref(),
            Some(A)
        );
        // Preference does not override reality.
        assert_eq!(
            choose(&flags(&[(P, false), (A, true)]), P, Some(P)).as_deref(),
            Some(A)
        );
    }

    #[test]
    fn the_primary_wins_whenever_it_answers() {
        // The rule that brings the app home. Before the fleet list existed the
        // last good address won this contest, which would have meant living on a
        // fallback node until the next restart.
        assert_eq!(
            choose(&flags(&[(P, true), (A, true)]), P, Some(A)).as_deref(),
            Some(P)
        );
        assert_eq!(
            choose(&flags(&[(P, true), (N2, true)]), P, Some(N2)).as_deref(),
            Some(P)
        );
    }

    #[test]
    fn a_working_fallback_is_kept_while_the_primary_is_down() {
        // Two fleet nodes are up and we are on the second: stay on it rather
        // than hopping to the first every ten seconds.
        assert_eq!(
            choose(
                &flags(&[(P, false), (A, false), (N2, true), (N3, true)]),
                P,
                Some(N3)
            )
            .as_deref(),
            Some(N3)
        );
        // With no preference, the order given wins: the fleet list is master
        // first, so that is who gets asked to serve.
        assert_eq!(
            choose(
                &flags(&[(P, false), (A, false), (N2, true), (N3, true)]),
                P,
                None
            )
            .as_deref(),
            Some(N2)
        );
    }

    #[test]
    fn a_preference_for_an_address_that_is_gone_is_ignored() {
        // The saved list changed under us: preferring an address nobody probed
        // must not make the app pick nothing.
        assert_eq!(
            choose(&flags(&[(P, false), (N2, true)]), P, Some("vanished:3000")).as_deref(),
            Some(N2)
        );
    }

    #[test]
    fn wave_one_is_the_configured_pair_plus_the_address_in_use() {
        let s = settings();
        let wave: Vec<String> = first_wave(&s, None)
            .into_iter()
            .map(|c| c.address)
            .collect();
        assert_eq!(wave, vec![P, A]);

        // Serving through a fleet node: it is probed in wave one, so a working
        // fallback never has to wait for a second round.
        let wave: Vec<Candidate> = first_wave(&s, Some(N3));
        assert_eq!(
            wave.iter().map(|c| c.address.clone()).collect::<Vec<_>>(),
            vec![P, A, N3]
        );
        assert_eq!(wave[2].name.as_deref(), Some("Spark-3"));

        // Already in the pair: not probed twice.
        let wave: Vec<String> = first_wave(&s, Some(A))
            .into_iter()
            .map(|c| c.address)
            .collect();
        assert_eq!(wave, vec![P, A]);
    }

    #[test]
    fn with_no_fleet_list_the_waves_are_just_the_configured_pair() {
        let s = Settings {
            primary: P.into(),
            alternate: Some(A.into()),
            ..Settings::default()
        };
        let wave: Vec<String> = first_wave(&s, None)
            .into_iter()
            .map(|c| c.address)
            .collect();
        assert_eq!(wave, vec![P, A]);
        assert!(s.fleet_candidates().is_empty());
    }

    /// A stub AINode: answers every request with `body` until dropped.
    ///
    /// Real sockets rather than a mocked client, because the thing worth proving
    /// is that a refused connection on the configured address ends with the app
    /// talking to a saved node, and that is the layer where it happens.
    struct Stub {
        address: String,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Stub {
        fn serving(body: &'static str) -> Self {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
            let address = listener.local_addr().expect("bound address").to_string();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = stop.clone();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                for stream in listener.incoming() {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let Ok(mut stream) = stream else { return };
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                }
            });
            Stub { address, stop }
        }

        /// An address nothing listens on: the port is bound and released, so a
        /// connection there is refused rather than hanging.
        fn dead() -> String {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("a free loopback port");
            listener.local_addr().expect("bound address").to_string()
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    const STATUS_WITH_HINT: &str = r#"{"node_name": "Spark-2-DGX", "version": "0.5.28",
        "engine_ready": true,
        "endpoint_hint": [{"name": "Spark-1", "host": "10.0.0.1", "port": 3000,
                           "role": "master", "url": "http://10.0.0.1:3000"}]}"#;

    #[tokio::test]
    async fn a_dead_primary_ends_up_on_a_saved_fleet_node() {
        let live = Stub::serving(STATUS_WITH_HINT);
        let dead = Stub::dead();
        let s = Settings {
            primary: dead.clone(),
            alternate: None,
            known: vec![KnownNode {
                name: "Spark-2".into(),
                address: live.address.clone(),
            }],
            ..Settings::default()
        };
        let client = api::client();

        let found = find_master(&client, &s, None)
            .await
            .expect("the fleet node answers");
        assert_eq!(found.address, live.address);
        assert!(found.ok());
        // The node's own name, not the one the list remembered.
        assert_eq!(found.node_name().as_deref(), Some("Spark-2-DGX"));
        // And its answer carries the fleet list, which is how the app keeps the
        // fallbacks current while it is working through one of them.
        let hint = api::known_nodes(&found.result.as_ref().unwrap().endpoint_hint);
        assert_eq!(hint[0].address, "10.0.0.1:3000");

        // Once we are on that node, it is probed in wave one: still chosen, and
        // the primary is still dead.
        let again = find_master(&client, &s, Some(&live.address))
            .await
            .expect("still serving");
        assert_eq!(again.address, live.address);
    }

    #[tokio::test]
    async fn the_primary_takes_over_again_as_soon_as_it_answers() {
        let home = Stub::serving(r#"{"node_name": "Spark-1", "engine_ready": true}"#);
        let fallback = Stub::serving(STATUS_WITH_HINT);
        let s = Settings {
            primary: home.address.clone(),
            alternate: None,
            known: vec![KnownNode {
                name: "Spark-2".into(),
                address: fallback.address.clone(),
            }],
            ..Settings::default()
        };
        let client = api::client();

        // Working through the fallback, with the primary back up: go home.
        let found = find_master(&client, &s, Some(&fallback.address))
            .await
            .expect("something answers");
        assert_eq!(found.address, home.address);
        assert_eq!(found.slot, Some(Which::Primary));
    }

    #[tokio::test]
    async fn nothing_answering_anywhere_is_reported_as_nothing() {
        let s = Settings {
            primary: Stub::dead(),
            alternate: Some(Stub::dead()),
            known: vec![KnownNode {
                name: "Spark-2".into(),
                address: Stub::dead(),
            }],
            ..Settings::default()
        };
        assert!(find_master(&api::client(), &s, None).await.is_none());
    }

    #[test]
    fn the_name_shown_is_the_nodes_own_word_when_it_answered() {
        let status = Status {
            node_name: Some("Spark-2-DGX".into()),
            ..Status::default()
        };
        let probe = Probe {
            slot: None,
            address: N2.into(),
            name: Some("Spark-2".into()),
            result: Ok(status),
        };
        assert_eq!(probe.node_name().as_deref(), Some("Spark-2-DGX"));

        // A node that did not answer keeps the name the fleet list gave it.
        let probe = Probe {
            slot: None,
            address: N2.into(),
            name: Some("Spark-2".into()),
            result: Err("no answer within 2 seconds".into()),
        };
        assert_eq!(probe.node_name().as_deref(), Some("Spark-2"));

        // And an unnamed one says nothing rather than something invented.
        let probe = Probe {
            slot: None,
            address: N2.into(),
            name: None,
            result: Ok(Status::default()),
        };
        assert_eq!(probe.node_name(), None);
    }
}
