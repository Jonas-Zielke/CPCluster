use cpcluster_common::Task;
use cpcluster_masternode::state::{load_state, save_state, MasterNode};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn master_state_persists_pending_tasks(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempdir()?;
    let orig = std::env::current_dir()?;
    std::env::set_current_dir(dir.path())?;

    let master = MasterNode {
        connected_nodes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        available_ports: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    };

    master.pending_tasks.lock().await.insert(
        "task1".into(),
        Task::Compute {
            expression: "1+1".into(),
        },
    );

    save_state(&master).await;

    let new_master = MasterNode {
        connected_nodes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        available_ports: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    };

    load_state(&new_master).await;

    let pending = new_master.pending_tasks.lock().await;
    assert!(pending.contains_key("task1"));

    std::env::set_current_dir(orig)?;
    Ok(())
}
