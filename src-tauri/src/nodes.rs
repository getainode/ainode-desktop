//! `/api/nodes` parsing, fleet summary, and the small state machine that
//! decides which notifications to send between two polls.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A row of `/api/nodes`. Only the fields the desktop app cares about;
/// everything else is ignored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeRow {
    #[serde(default, alias = "name")]
    pub node_name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub engine_ready: bool,
    #[serde(default)]
    pub load_phase: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub load_elapsed_seconds: Option<f64>,
    #[serde(default)]
    pub expected_ready_minutes: Option<f64>,
    #[serde(default)]
    pub gpu_name: Option<String>,
    #[serde(default)]
    pub instances: Vec<Instance>,
}

/// One served model on a node (newer masters report several per node).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Phases that mean "a model is on its way".
const LOADING_PHASES: [&str; 4] = [
    "starting",
    "loading_weights",
    "distributed_init",
    "profiling",
];

impl NodeRow {
    pub fn name(&self) -> &str {
        if self.node_name.is_empty() {
            "unnamed node"
        } else {
            &self.node_name
        }
    }

    pub fn is_online(&self) -> bool {
        match self.status.as_deref() {
            Some(s) => s.eq_ignore_ascii_case("online"),
            // Older masters omit the field; they only list nodes they can reach.
            None => true,
        }
    }

    pub fn phase(&self) -> &str {
        self.load_phase.as_deref().unwrap_or("idle")
    }

    pub fn is_loading(&self) -> bool {
        LOADING_PHASES.contains(&self.phase())
    }

    /// The model this node is serving, if it has one.
    pub fn model_name(&self) -> Option<&str> {
        self.model.as_deref().filter(|m| !m.is_empty())
    }

    /// Ready means the engine says so and there is a model behind it.
    pub fn is_ready(&self) -> bool {
        (self.engine_ready || self.phase() == "ready") && self.model_name().is_some()
    }

    /// Every distinct model this node is actually serving: the headline one
    /// once it is ready, plus any extra instances that report as serving.
    /// A model that is still loading does not count yet.
    pub fn served_models(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if self.is_ready() {
            if let Some(m) = self.model_name() {
                out.push(m.to_string());
            }
        }
        for inst in &self.instances {
            let serving = inst
                .status
                .as_deref()
                .map(|s| matches!(s, "serving" | "ready" | "online"))
                .unwrap_or(true);
            if let Some(m) = inst.model.as_deref().filter(|m| !m.is_empty()) {
                if serving && !out.iter().any(|x| x == m) {
                    out.push(m.to_string());
                }
            }
        }
        out
    }
}

/// Parse either shape of `/api/nodes`: a bare list, or `{"nodes": [...]}`.
pub fn parse_nodes(body: &str) -> Result<Vec<NodeRow>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
    let list = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(mut o) => match o.remove("nodes") {
            Some(serde_json::Value::Array(a)) => a,
            Some(_) => return Err("'nodes' is not a list".into()),
            None => return Err("no 'nodes' key in response".into()),
        },
        _ => return Err("unexpected JSON shape".into()),
    };
    list.into_iter()
        .map(|v| serde_json::from_value::<NodeRow>(v).map_err(|e| format!("bad node row: {e}")))
        .collect()
}

/// Node and distinct-model counts for the menu bar title.
pub fn summary(rows: &[NodeRow]) -> (usize, usize) {
    let mut models: HashSet<String> = HashSet::new();
    for row in rows {
        for m in row.served_models() {
            models.insert(m);
        }
    }
    (rows.len(), models.len())
}

/// `AINode · 5 nodes · 3 models`
pub fn tray_title(rows: &[NodeRow]) -> String {
    let (nodes, models) = summary(rows);
    format!(
        "AINode · {} · {}",
        plural(nodes, "node"),
        plural(models, "model")
    )
}

/// Model names come as `org/Model-Name`; people read the last part.
pub fn short_model(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// `m:ss` for elapsed load time.
fn mmss(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The middle part of a node's menu line: what it is doing right now.
pub fn activity_label(row: &NodeRow) -> String {
    if row.is_loading() {
        let phase = match row.phase() {
            "starting" => "starting",
            "loading_weights" => "loading weights",
            "distributed_init" => "joining the cluster",
            "profiling" => "profiling",
            other => other,
        };
        let mut s = phase.to_string();
        if let Some(elapsed) = row.load_elapsed_seconds {
            s.push(' ');
            s.push_str(&mmss(elapsed));
            if let Some(expected) = row.expected_ready_minutes {
                s.push_str(&format!(" of ~{} min", expected.round() as u64));
            }
        }
        return s;
    }
    let models = row.served_models();
    match models.len() {
        0 => "no model".to_string(),
        1 => short_model(&models[0]).to_string(),
        n => format!("{} +{} more", short_model(&models[0]), n - 1),
    }
}

/// One tray menu line: `name · model or activity · online/offline`.
pub fn menu_label(row: &NodeRow) -> String {
    if !row.is_online() {
        return format!("{} · offline", row.name());
    }
    format!("{} · {} · online", row.name(), activity_label(row))
}

/// Something worth telling the user about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ModelReady {
        node: String,
        model: String,
        loaded_in_secs: Option<u64>,
    },
    NodeOffline {
        node: String,
    },
    NodeOnline {
        node: String,
    },
}

impl Event {
    /// The sentence that goes in the notification.
    pub fn message(&self) -> String {
        match self {
            Event::ModelReady {
                node,
                model,
                loaded_in_secs,
            } => {
                let how_long = match loaded_in_secs {
                    Some(s) if *s < 60 => " (loaded in under a minute)".to_string(),
                    Some(s) => format!(" (loaded in {} min)", s.div_ceil(60)),
                    None => String::new(),
                };
                format!("{} is ready on {}{}", short_model(model), node, how_long)
            }
            Event::NodeOffline { node } => format!("{node} went offline"),
            Event::NodeOnline { node } => format!("{node} is back online"),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct NodeMemory {
    online: bool,
    loading: bool,
    /// Unix seconds when the current load started, best estimate.
    loading_since: Option<u64>,
}

/// Remembers the last snapshot so the next one can be diffed.
#[derive(Clone, Debug, Default)]
pub struct FleetState {
    primed: bool,
    nodes: HashMap<String, NodeMemory>,
}

impl FleetState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one `/api/nodes` snapshot taken at `now` (unix seconds) and get
    /// back the events it implies. The first snapshot never produces events;
    /// it only primes the memory.
    pub fn observe(&mut self, rows: &[NodeRow], now: u64) -> Vec<Event> {
        let mut events = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for row in rows {
            let name = row.name().to_string();
            seen.insert(name.clone());
            let online = row.is_online();
            let loading = online && row.is_loading();
            let ready = online && row.is_ready();
            let prev = self.nodes.get(&name).cloned();

            if self.primed {
                if let Some(prev) = &prev {
                    if prev.online && !online {
                        events.push(Event::NodeOffline { node: name.clone() });
                    } else if !prev.online && online {
                        events.push(Event::NodeOnline { node: name.clone() });
                    }
                    if prev.loading && ready {
                        let loaded_in_secs = prev.loading_since.map(|t| now.saturating_sub(t));
                        events.push(Event::ModelReady {
                            node: name.clone(),
                            model: row.model_name().unwrap_or_default().to_string(),
                            loaded_in_secs,
                        });
                    }
                }
            }

            let loading_since = if loading {
                match row.load_elapsed_seconds {
                    // The master knows exactly how long this load has run.
                    Some(elapsed) => Some(now.saturating_sub(elapsed.max(0.0).round() as u64)),
                    None => prev.as_ref().and_then(|p| p.loading_since).or(Some(now)),
                }
            } else {
                None
            };

            self.nodes.insert(
                name,
                NodeMemory {
                    online,
                    loading,
                    loading_since,
                },
            );
        }

        // Nodes that vanished from the list count as offline, once.
        for (name, mem) in self.nodes.iter_mut() {
            if !seen.contains(name) && mem.online {
                if self.primed {
                    events.push(Event::NodeOffline { node: name.clone() });
                }
                mem.online = false;
                mem.loading = false;
                mem.loading_since = None;
            }
        }

        self.primed = true;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, status: &str, phase: &str, model: &str, ready: bool) -> NodeRow {
        NodeRow {
            node_name: name.into(),
            status: Some(status.into()),
            engine_ready: ready,
            load_phase: Some(phase.into()),
            model: if model.is_empty() {
                None
            } else {
                Some(model.into())
            },
            ..Default::default()
        }
    }

    fn loading(name: &str, model: &str, elapsed: f64, expected: f64) -> NodeRow {
        NodeRow {
            load_elapsed_seconds: Some(elapsed),
            expected_ready_minutes: Some(expected),
            ..row(name, "online", "loading_weights", model, false)
        }
    }

    // (c) parsing both shapes

    #[test]
    fn parses_bare_list() {
        let body = r#"[{"node_name":"a","status":"online","engine_ready":true,"load_phase":"ready","model":"org/m"}]"#;
        let rows = parse_nodes(body).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name(), "a");
        assert!(rows[0].is_ready());
        assert_eq!(rows[0].model_name(), Some("org/m"));
    }

    #[test]
    fn parses_wrapped_object_and_name_alias() {
        let body = r#"{"nodes":[{"name":"b","status":"offline"},{"node_name":"c","status":"online","engine_ready":false,"load_phase":"idle","model":"","gpu_name":"NVIDIA GB10"}]}"#;
        let rows = parse_nodes(body).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name(), "b");
        assert!(!rows[0].is_online());
        assert_eq!(rows[1].name(), "c");
        assert!(rows[1].is_online());
        assert!(!rows[1].is_ready());
        assert!(!rows[1].is_loading());
        assert_eq!(rows[1].model_name(), None);
        assert_eq!(rows[1].gpu_name.as_deref(), Some("NVIDIA GB10"));
    }

    #[test]
    fn parses_instances_and_ignores_extra_fields() {
        let body = r#"{"nodes":[{"node_name":"s1","node_id":"x","fabric_ip":"10.0.0.1","status":"online","engine_ready":true,"load_phase":"ready","model":"org/one","instances":[{"model":"org/one","api_port":8000,"status":"serving"},{"model":"org/two","api_port":8001,"status":"serving"},{"model":"org/dead","status":"stopped"}]}]}"#;
        let rows = parse_nodes(body).unwrap();
        assert_eq!(rows[0].served_models(), vec!["org/one", "org/two"]);
        assert_eq!(summary(&rows), (1, 2));
        assert_eq!(tray_title(&rows), "AINode · 1 node · 2 models");
        assert_eq!(menu_label(&rows[0]), "s1 · one +1 more · online");
    }

    #[test]
    fn rejects_bad_shapes() {
        assert!(parse_nodes("not json").is_err());
        assert!(parse_nodes(r#"{"nodes": 5}"#).is_err());
        assert!(parse_nodes(r#"{"other": []}"#).is_err());
        assert!(parse_nodes("42").is_err());
        assert!(parse_nodes(r#"[{"node_name": 5}]"#).is_err());
    }

    #[test]
    fn labels_read_well() {
        let rows = vec![
            row(
                "Spark-1",
                "online",
                "ready",
                "unsloth/Qwen3.8-27B-NVFP4",
                true,
            ),
            loading("Spark-2", "org/DeepSeek", 192.0, 12.0),
            row("castor", "online", "idle", "", false),
            row("pollux", "offline", "idle", "", false),
        ];
        assert_eq!(menu_label(&rows[0]), "Spark-1 · Qwen3.8-27B-NVFP4 · online");
        assert_eq!(
            menu_label(&rows[1]),
            "Spark-2 · loading weights 3:12 of ~12 min · online"
        );
        assert_eq!(menu_label(&rows[2]), "castor · no model · online");
        assert_eq!(menu_label(&rows[3]), "pollux · offline");
        assert_eq!(tray_title(&rows), "AINode · 4 nodes · 1 model");

        let mut no_estimate = loading("s", "m", 65.0, 0.0);
        no_estimate.expected_ready_minutes = None;
        assert_eq!(activity_label(&no_estimate), "loading weights 1:05");
        let mut no_elapsed = loading("s", "m", 0.0, 0.0);
        no_elapsed.load_elapsed_seconds = None;
        no_elapsed.load_phase = Some("distributed_init".into());
        assert_eq!(activity_label(&no_elapsed), "joining the cluster");
    }

    // (b) node-state transitions

    #[test]
    fn first_poll_is_silent() {
        let mut fleet = FleetState::new();
        let rows = vec![
            row("a", "online", "ready", "org/m", true),
            row("b", "offline", "idle", "", false),
            loading("c", "org/x", 10.0, 5.0),
        ];
        assert!(fleet.observe(&rows, 1_000).is_empty());
    }

    #[test]
    fn loading_to_ready_notifies_with_duration() {
        let mut fleet = FleetState::new();
        // At t=1000 the master says the load has been running 190 s.
        fleet.observe(&[loading("Spark-2", "org/DeepSeek-V4", 190.0, 12.0)], 1_000);
        // Still loading at t=1010, nothing to say.
        assert!(fleet
            .observe(&[loading("Spark-2", "org/DeepSeek-V4", 200.0, 12.0)], 1_010)
            .is_empty());
        // Ready at t=1020: started at 810, so 210 s.
        let events = fleet.observe(
            &[row("Spark-2", "online", "ready", "org/DeepSeek-V4", true)],
            1_020,
        );
        assert_eq!(
            events,
            vec![Event::ModelReady {
                node: "Spark-2".into(),
                model: "org/DeepSeek-V4".into(),
                loaded_in_secs: Some(210),
            }]
        );
        assert_eq!(
            events[0].message(),
            "DeepSeek-V4 is ready on Spark-2 (loaded in 4 min)"
        );
        // Staying ready is quiet.
        assert!(fleet
            .observe(
                &[row("Spark-2", "online", "ready", "org/DeepSeek-V4", true)],
                1_030
            )
            .is_empty());
    }

    #[test]
    fn duration_falls_back_to_first_sighting_without_elapsed() {
        let mut fleet = FleetState::new();
        let mut r = loading("n", "org/m", 0.0, 0.0);
        r.load_elapsed_seconds = None;
        r.expected_ready_minutes = None;
        fleet.observe(std::slice::from_ref(&r), 100);
        fleet.observe(std::slice::from_ref(&r), 110);
        let events = fleet.observe(&[row("n", "online", "ready", "org/m", true)], 130);
        assert_eq!(
            events,
            vec![Event::ModelReady {
                node: "n".into(),
                model: "org/m".into(),
                loaded_in_secs: Some(30),
            }]
        );
        assert_eq!(
            events[0].message(),
            "m is ready on n (loaded in under a minute)"
        );
    }

    #[test]
    fn ready_without_prior_loading_is_quiet() {
        let mut fleet = FleetState::new();
        fleet.observe(&[row("a", "online", "idle", "", false)], 1);
        // Jumped straight to ready between polls with no loading phase seen: quiet.
        assert!(fleet
            .observe(&[row("a", "online", "ready", "org/m", true)], 2)
            .is_empty());
    }

    #[test]
    fn offline_and_back_notify_once_each() {
        let mut fleet = FleetState::new();
        fleet.observe(&[row("a", "online", "ready", "org/m", true)], 1);
        let down = vec![row("a", "offline", "idle", "", false)];
        assert_eq!(
            fleet.observe(&down, 2),
            vec![Event::NodeOffline { node: "a".into() }]
        );
        assert!(fleet.observe(&down, 3).is_empty());
        assert!(fleet.observe(&down, 4).is_empty());
        let up = vec![row("a", "online", "idle", "", false)];
        assert_eq!(
            fleet.observe(&up, 5),
            vec![Event::NodeOnline { node: "a".into() }]
        );
        assert!(fleet.observe(&up, 6).is_empty());
        assert_eq!(
            Event::NodeOffline { node: "a".into() }.message(),
            "a went offline"
        );
        assert_eq!(
            Event::NodeOnline { node: "a".into() }.message(),
            "a is back online"
        );
    }

    #[test]
    fn vanished_node_counts_as_offline_once_and_returns_online() {
        let mut fleet = FleetState::new();
        fleet.observe(
            &[
                row("a", "online", "ready", "org/m", true),
                row("b", "online", "idle", "", false),
            ],
            1,
        );
        let only_a = vec![row("a", "online", "ready", "org/m", true)];
        assert_eq!(
            fleet.observe(&only_a, 2),
            vec![Event::NodeOffline { node: "b".into() }]
        );
        assert!(fleet.observe(&only_a, 3).is_empty());
        let events = fleet.observe(
            &[
                row("a", "online", "ready", "org/m", true),
                row("b", "online", "idle", "", false),
            ],
            4,
        );
        assert_eq!(events, vec![Event::NodeOnline { node: "b".into() }]);
    }

    #[test]
    fn new_node_joining_is_quiet() {
        let mut fleet = FleetState::new();
        fleet.observe(&[row("a", "online", "ready", "org/m", true)], 1);
        let events = fleet.observe(
            &[
                row("a", "online", "ready", "org/m", true),
                row("z", "online", "idle", "", false),
            ],
            2,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn going_offline_while_loading_does_not_fake_a_ready() {
        let mut fleet = FleetState::new();
        fleet.observe(&[loading("a", "org/m", 5.0, 3.0)], 1);
        assert_eq!(
            fleet.observe(&[row("a", "offline", "idle", "", false)], 2),
            vec![Event::NodeOffline { node: "a".into() }]
        );
        // Comes back already serving: back online, but no "ready" claim.
        assert_eq!(
            fleet.observe(&[row("a", "online", "ready", "org/m", true)], 3),
            vec![Event::NodeOnline { node: "a".into() }]
        );
    }

    #[test]
    fn model_swap_notifies_for_the_new_model() {
        let mut fleet = FleetState::new();
        fleet.observe(&[row("a", "online", "ready", "org/old", true)], 1);
        assert!(fleet
            .observe(&[loading("a", "org/new", 1.0, 2.0)], 2)
            .is_empty());
        let events = fleet.observe(&[row("a", "online", "ready", "org/new", true)], 3);
        assert_eq!(
            events,
            vec![Event::ModelReady {
                node: "a".into(),
                model: "org/new".into(),
                loaded_in_secs: Some(2),
            }]
        );
    }
}
