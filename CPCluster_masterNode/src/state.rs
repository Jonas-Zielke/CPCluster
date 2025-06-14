use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use cpcluster_common::{NodeRole, Task, TaskResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub addr: String,
    pub last_heartbeat: u64,
    pub port: Option<u16>,
    pub active_tasks: HashMap<String, Task>,
    #[serde(default)]
    pub is_worker: bool,
    #[serde(default)]
    pub role: NodeRole,
}

#[derive(Debug, Clone)]
pub struct MasterNode {
    pub connected_nodes: Arc<Mutex<HashMap<String, NodeInfo>>>,
    pub available_ports: Arc<Mutex<HashSet<u16>>>,
    pub failover_timeout_ms: u64,
    pub pending_tasks: Arc<Mutex<HashMap<String, Task>>>,
    pub completed_tasks: Arc<Mutex<HashMap<String, TaskResult>>>,
    pub state_file: String,
}

#[derive(Serialize, Deserialize)]
struct MasterState {
    connected_nodes: HashMap<String, NodeInfo>,
    available_ports: Vec<u16>,
    pending_tasks: HashMap<String, Task>,
    #[serde(default)]
    completed_tasks: HashMap<String, TaskResult>,
}

pub async fn load_state(master: &MasterNode) {
    if let Ok(data) = tokio::fs::read_to_string(&master.state_file).await {
        if let Ok(state) = serde_json::from_str::<MasterState>(&data) {
            *master.connected_nodes.lock().await = state.connected_nodes;
            *master.available_ports.lock().await = state.available_ports.into_iter().collect();
            *master.pending_tasks.lock().await = state.pending_tasks;
            *master.completed_tasks.lock().await = state.completed_tasks;
        }
    }
}

pub async fn save_state(master: &MasterNode) {
    let state = MasterState {
        connected_nodes: master.connected_nodes.lock().await.clone(),
        available_ports: master
            .available_ports
            .lock()
            .await
            .iter()
            .cloned()
            .collect(),
        pending_tasks: master.pending_tasks.lock().await.clone(),
        completed_tasks: master.completed_tasks.lock().await.clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&state) {
        let _ = tokio::fs::write(&master.state_file, data).await;
    }
}
