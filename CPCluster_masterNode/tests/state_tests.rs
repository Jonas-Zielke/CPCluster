use cpcluster_common::Task;
use cpcluster_masternode::state::{load_state, save_state, MasterNode, PendingTask};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn master_state_persists_pending_tasks(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempdir()?;
    let state_path = dir.path().join("master_state.json");

    let master = MasterNode {
        connected_nodes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        available_ports: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        state_file: state_path.to_string_lossy().into_owned(),
    };

    master.pending_tasks.lock().await.insert(
        "task1".into(),
        PendingTask {
            task: Task::Compute {
                expression: "1+1".into(),
            },
            target: None,
        },
    );

    save_state(&master).await;

    let new_master = MasterNode {
        connected_nodes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        available_ports: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        state_file: state_path.to_string_lossy().into_owned(),
    };

    load_state(&new_master).await;

    let pending = new_master.pending_tasks.lock().await;
    assert!(pending.contains_key("task1"));

    Ok(())
}
