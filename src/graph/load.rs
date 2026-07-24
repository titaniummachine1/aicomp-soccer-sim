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
    #[serde(rename = "ownerFunctionSID", default)]
    pub owner_function_sid: String,
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
    /// Empty = root graph; else CreateFunction sID that owns this body node.
    pub owner_function_sid: String,
    pub ports: Vec<RawPort>,
}

#[derive(Debug, Clone)]
pub struct CreateFunctionDef {
    pub sid: String,
    pub name: String,
    /// DebugDrawLine / DebugDrawDisc body nodes owned by this CreateFunction.
    pub debug_draws: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TeamGraph {
    pub nodes: HashMap<String, GraphNode>,
    /// port_sid → PortRef
    pub ports: HashMap<String, PortRef>,
    /// For each input port_sid, the output port_sid that feeds it.
    pub input_source: HashMap<String, String>,
    pub controllers: [Option<String>; 4],
    /// Root-level SetVariable node sIDs (owner empty).
    pub set_variables: Vec<String>,
    /// Root-level DebugDrawLine / DebugDrawDisc sinks (owner empty).
    pub debug_draws: Vec<String>,
    /// Root-level Function call sIDs (owner empty) — Unity runs these even when unread.
    pub root_functions: Vec<String>,
    /// Function name → CreateFunction definition.
    pub create_functions: HashMap<String, CreateFunctionDef>,
    pub path: String,
}

pub fn load_team_graph(path: &Path) -> Result<TeamGraph, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))?;
    let raw: RawGraph = serde_json::from_str(&text).map_err(|e| format!("parse {path:?}: {e}"))?;
    Ok(index_graph(raw, path.display().to_string()))
}

pub fn index_graph(raw: RawGraph, path: String) -> TeamGraph {
    let mut nodes = HashMap::new();
    let mut ports = HashMap::new();
    let mut controllers: [Option<String>; 4] = [None, None, None, None];
    let mut set_variables = Vec::new();
    let mut debug_draws = Vec::new();
    let mut root_functions = Vec::new();
    let mut create_functions = HashMap::new();
    // CreateFunction sid → owned DebugDraw sids (filled after name map exists).
    let mut owned_debug_by_create: HashMap<String, Vec<String>> = HashMap::new();

    for n in raw.nodes {
        let modifier = normalize_modifier(&n.id, &n.modifier);
        if let Some(slot) = controller_slot(&n.id) {
            controllers[slot] = Some(n.sid.clone());
        }
        if n.id == "SetVariable" && n.owner_function_sid.is_empty() {
            set_variables.push(n.sid.clone());
        }
        if n.id == "DebugDrawLine" || n.id == "DebugDrawDisc" {
            if n.owner_function_sid.is_empty() {
                debug_draws.push(n.sid.clone());
            } else {
                owned_debug_by_create
                    .entry(n.owner_function_sid.clone())
                    .or_default()
                    .push(n.sid.clone());
            }
        }
        if n.id == "Function" && n.owner_function_sid.is_empty() {
            root_functions.push(n.sid.clone());
        }
        if n.id == "CreateFunction" {
            create_functions.insert(
                modifier.clone(),
                CreateFunctionDef {
                    sid: n.sid.clone(),
                    name: modifier.clone(),
                    debug_draws: Vec::new(),
                },
            );
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
                owner_function_sid: n.owner_function_sid,
                ports: n.ports,
            },
        );
    }

    for def in create_functions.values_mut() {
        if let Some(owned) = owned_debug_by_create.remove(&def.sid) {
            def.debug_draws = owned;
        }
    }

    let mut input_source = HashMap::new();
    let mut relay_relay: Vec<(String, String)> = Vec::new();
    for c in raw.connections {
        let Some(a) = ports.get(&c.port0) else {
            continue;
        };
        let Some(b) = ports.get(&c.port1) else {
            continue;
        };
        let pa = a.polarity;
        let pb = b.polarity;
        // Out(1) → In(0)
        if pa == 1 && pb == 0 {
            input_source.insert(c.port1.clone(), c.port0.clone());
        } else if pb == 1 && pa == 0 {
            input_source.insert(c.port0.clone(), c.port1.clone());
        // Out(1) → Relay(2)
        } else if pa == 1 && pb == 2 {
            input_source.insert(c.port1.clone(), c.port0.clone());
        } else if pb == 1 && pa == 2 {
            input_source.insert(c.port0.clone(), c.port1.clone());
        // Relay(2) → In(0)
        } else if pa == 2 && pb == 0 {
            input_source.insert(c.port1.clone(), c.port0.clone());
        } else if pb == 2 && pa == 0 {
            input_source.insert(c.port0.clone(), c.port1.clone());
        // Relay(2) ↔ Relay(2) — propagate after primary edges exist.
        } else if pa == 2 && pb == 2 {
            relay_relay.push((c.port0.clone(), c.port1.clone()));
        // Legacy: any non-zero → in(0)
        } else if pa != 0 && pb == 0 {
            input_source.insert(c.port1.clone(), c.port0.clone());
        } else if pb != 0 && pa == 0 {
            input_source.insert(c.port0.clone(), c.port1.clone());
        }
    }

    // Propagate sources along Relay↔Relay chains (CreateFunction → Relay → Relay → sink).
    let mut changed = true;
    while changed {
        changed = false;
        for (a, b) in &relay_relay {
            if let Some(src) = input_source.get(a).cloned() {
                if input_source.get(b).is_none() {
                    input_source.insert(b.clone(), src);
                    changed = true;
                }
            }
            if let Some(src) = input_source.get(b).cloned() {
                if input_source.get(a).is_none() {
                    input_source.insert(a.clone(), src);
                    changed = true;
                }
            }
        }
    }

    TeamGraph {
        nodes,
        ports,
        input_source,
        controllers,
        set_variables,
        debug_draws,
        root_functions,
        create_functions,
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
        // Prefer true inputs (polarity 0); Relay uses polarity 2 as bidirectional.
        node.ports
            .iter()
            .find(|p| p.id == port_name && p.polarity == 0)
            .or_else(|| {
                node.ports
                    .iter()
                    .find(|p| p.id == port_name && p.polarity == 2)
            })
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
