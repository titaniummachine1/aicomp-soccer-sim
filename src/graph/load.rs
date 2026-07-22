//! Parse AIComp `serializableNodes` / `serializableConnections` JSON.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::dropdowns;

#[derive(Debug, Clone, Deserialize)]
pub struct RawGraph {
    #[serde(rename = "serializableNodes")]
    pub nodes: Vec<RawNode>,
    #[serde(rename = "serializableConnections")]
    pub connections: Vec<RawConnection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawNode {
    pub id: String,
    #[serde(rename = "sID")]
    pub sid: String,
    #[serde(default)]
    pub modifier: serde_json::Value,
    #[serde(rename = "serializablePorts", default)]
    pub ports: Vec<RawPort>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPort {
    pub id: String,
    #[serde(rename = "sID")]
    pub sid: String,
    /// 0 = input, 1 = output (2 = relay rare).
    pub polarity: i32,
    #[serde(rename = "nodeSID", default)]
    pub node_sid: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawConnection {
    #[serde(rename = "port0SID")]
    pub port0: String,
    #[serde(rename = "port1SID")]
    pub port1: String,
}

#[derive(Debug, Clone)]
pub struct PortRef {
    pub node_sid: String,
    pub port_name: String,
    pub polarity: i32,
}

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: String,
    pub sid: String,
    /// Resolved modifier (dropdown label or raw constant string).
    pub modifier: String,
    pub ports: Vec<RawPort>,
}

#[derive(Debug, Clone)]
pub struct TeamGraph {
    pub nodes: HashMap<String, GraphNode>,
    /// port_sid → PortRef
    pub ports: HashMap<String, PortRef>,
    /// For each input port_sid, the output port_sid that feeds it.
    pub input_source: HashMap<String, String>,
    pub controllers: [Option<String>; 4],
    pub path: String,
}

pub fn load_team_graph(path: &Path) -> Result<TeamGraph, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
    let raw: RawGraph =
        serde_json::from_str(&text).map_err(|e| format!("parse {path:?}: {e}"))?;
    Ok(index_graph(raw, path.display().to_string()))
}

pub fn index_graph(raw: RawGraph, path: String) -> TeamGraph {
    let mut nodes = HashMap::new();
    let mut ports = HashMap::new();
    let mut controllers: [Option<String>; 4] = [None, None, None, None];

    for n in raw.nodes {
        let modifier = normalize_modifier(&n.id, &n.modifier);
        if let Some(slot) = controller_slot(&n.id) {
            controllers[slot] = Some(n.sid.clone());
        }
        for p in &n.ports {
            ports.insert(
                p.sid.clone(),
                PortRef {
                    node_sid: n.sid.clone(),
                    port_name: p.id.clone(),
                    polarity: p.polarity,
                },
            );
        }
        nodes.insert(
            n.sid.clone(),
            GraphNode {
                id: n.id,
                sid: n.sid,
                modifier,
                ports: n.ports,
            },
        );
    }

    let mut input_source = HashMap::new();
    for c in raw.connections {
        let Some(a) = ports.get(&c.port0) else {
            continue;
        };
        let Some(b) = ports.get(&c.port1) else {
            continue;
        };
        // Prefer out → in; also accept reverse just in case.
        if a.polarity != 0 && b.polarity == 0 {
            input_source.insert(c.port1.clone(), c.port0.clone());
        } else if b.polarity != 0 && a.polarity == 0 {
            input_source.insert(c.port0.clone(), c.port1.clone());
        }
    }

    TeamGraph {
        nodes,
        ports,
        input_source,
        controllers,
        path,
    }
}

fn normalize_modifier(node_id: &str, value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "0".into()
            } else {
                "1".into()
            }
        }
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    dropdowns::resolve(node_id, &raw).to_string()
}

fn controller_slot(id: &str) -> Option<usize> {
    match id {
        "SoccerController1" => Some(0),
        "SoccerController2" => Some(1),
        "SoccerController3" => Some(2),
        "SoccerController4" => Some(3),
        _ => None,
    }
}

impl TeamGraph {
    pub fn input_port_sid(&self, node_sid: &str, port_name: &str) -> Option<String> {
        let node = self.nodes.get(node_sid)?;
        node.ports
            .iter()
            .find(|p| p.id == port_name && p.polarity == 0)
            .map(|p| p.sid.clone())
    }

    pub fn output_port_sid(&self, node_sid: &str, port_name: &str) -> Option<String> {
        let node = self.nodes.get(node_sid)?;
        node.ports
            .iter()
            .find(|p| p.id == port_name && p.polarity != 0)
            .map(|p| p.sid.clone())
    }
}
