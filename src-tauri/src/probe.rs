//! Decide which of the stored addresses to use.
//!
//! Both addresses are probed at the same time with a short timeout. The pick
//! is pure and small so it can be tested with canned results.

use crate::api::{self, Status};
use crate::config::{Settings, Which};

/// Outcome of one probe.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub which: Which,
    pub address: String,
    pub result: Result<Status, String>,
}

impl Probe {
    pub fn ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// Pick an address from probe results.
///
/// Rules, in order:
/// 1. If the preferred slot answered, keep it (no flapping between two live
///    addresses, and the one that worked last time wins on launch).
/// 2. Otherwise the first slot that answered, primary before alternate.
/// 3. Nothing answered: `None`.
pub fn choose(results: &[(Which, bool)], preferred: Option<Which>) -> Option<Which> {
    if let Some(p) = preferred {
        if results.iter().any(|(w, ok)| *w == p && *ok) {
            return Some(p);
        }
    }
    let order = [Which::Primary, Which::Alternate];
    order
        .into_iter()
        .find(|w| results.iter().any(|(r, ok)| r == w && *ok))
}

/// Probe every candidate address at once.
pub async fn probe_all(client: &reqwest::Client, settings: &Settings) -> Vec<Probe> {
    let candidates = settings.candidates();
    let futures = candidates.iter().map(|(which, address)| {
        let which = *which;
        let address = address.clone();
        // reqwest clients are cheap handles around a shared pool.
        let client = client.clone();
        async move {
            let result = api::fetch_status(&client, &address).await;
            Probe {
                which,
                address,
                result,
            }
        }
    });
    futures_join_all(futures).await
}

/// Probe everything and choose. `preferred` is the slot in use right now,
/// or the one that worked last time.
pub async fn find_master(
    client: &reqwest::Client,
    settings: &Settings,
    preferred: Option<Which>,
) -> Option<Probe> {
    let probes = probe_all(client, settings).await;
    let flags: Vec<(Which, bool)> = probes.iter().map(|p| (p.which, p.ok())).collect();
    let pick = choose(&flags, preferred)?;
    probes.into_iter().find(|p| p.which == pick)
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

    const P: Which = Which::Primary;
    const A: Which = Which::Alternate;

    #[test]
    fn nothing_answers_means_none() {
        assert_eq!(choose(&[(P, false), (A, false)], None), None);
        assert_eq!(choose(&[(P, false), (A, false)], Some(A)), None);
        assert_eq!(choose(&[], Some(P)), None);
    }

    #[test]
    fn only_one_answers() {
        assert_eq!(choose(&[(P, true), (A, false)], None), Some(P));
        assert_eq!(choose(&[(P, false), (A, true)], None), Some(A));
        // Preference does not override reality.
        assert_eq!(choose(&[(P, false), (A, true)], Some(P)), Some(A));
        assert_eq!(choose(&[(P, true), (A, false)], Some(A)), Some(P));
    }

    #[test]
    fn both_answer_prefers_last_good_then_primary() {
        assert_eq!(choose(&[(P, true), (A, true)], None), Some(P));
        assert_eq!(choose(&[(P, true), (A, true)], Some(P)), Some(P));
        assert_eq!(choose(&[(P, true), (A, true)], Some(A)), Some(A));
    }

    #[test]
    fn primary_only_configuration() {
        assert_eq!(choose(&[(P, true)], Some(A)), Some(P));
        assert_eq!(choose(&[(P, false)], Some(A)), None);
    }
}
