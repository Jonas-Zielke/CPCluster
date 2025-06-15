use cpcluster_common::config::Config;
use cpcluster_common::{is_local_ip, JoinInfo, NodeMessage, NodeRole, Task, TaskResult};
use cpcluster_common::{read_length_prefixed, write_length_prefixed};
use log::{error, info, warn};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fs,
    sync::Arc,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

pub mod shell;
pub mod state;
pub mod tls;

use shell::run_shell;
use state::{load_state, spawn_save_state, MasterNode, NodeInfo, PendingTask};
use tls::load_or_generate_tls_config;

#[derive(serde::Serialize, serde::Deserialize)]
struct PeerState {
    connected_nodes: HashMap<String, NodeInfo>,
    pending_tasks: HashMap<String, PendingTask>,
    completed_tasks: HashMap<String, TaskResult>,
}

async fn collect_state(master: &MasterNode) -> PeerState {
    PeerState {
        connected_nodes: master.connected_nodes.lock().await.clone(),
        pending_tasks: master.pending_tasks.lock().await.clone(),
        completed_tasks: master.completed_tasks.lock().await.clone(),
    }
}

async fn merge_state(master: &Arc<MasterNode>, state: PeerState) {
    {
        let mut nodes = master.connected_nodes.lock().await;
        for (k, v) in state.connected_nodes {
            nodes.entry(k).or_insert(v);
        }
    }
    {
        let mut pending = master.pending_tasks.lock().await;
        for (k, v) in state.pending_tasks {
            pending.entry(k).or_insert(v);
        }
    }
    {
        let mut completed = master.completed_tasks.lock().await;
        for (k, v) in state.completed_tasks {
            completed.entry(k).or_insert(v);
        }
    }
    spawn_save_state(master);
}

pub async fn handle_peer_connection<S>(
    mut socket: S,
    master_node: Arc<MasterNode>,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let data = read_length_prefixed(&mut socket).await?;
    let state: PeerState = serde_json::from_slice(&data)?;
    merge_state(&master_node, state).await;
    let local_state = collect_state(&master_node).await;
    let resp = serde_json::to_vec(&local_state)?;
    write_length_prefixed(&mut socket, &resp).await?;
    Ok(())
}

pub async fn connect_to_peer(addr: String, master_node: Arc<MasterNode>) {
    match tokio::net::TcpStream::connect(&addr).await {
        Ok(mut stream) => {
            if write_length_prefixed(&mut stream, b"MASTER_SYNC")
                .await
                .is_ok()
            {
                if let Err(e) = async {
                    let state = collect_state(&master_node).await;
                    let data = serde_json::to_vec(&state)?;
                    write_length_prefixed(&mut stream, &data).await?;
                    let resp = read_length_prefixed(&mut stream).await?;
                    let other: PeerState = serde_json::from_slice(&resp)?;
                    merge_state(&master_node, other).await;
                    Ok::<(), Box<dyn Error + Send + Sync>>(())
                }
                .await
                {
                    warn!("Peer sync with {} failed: {}", addr, e);
                }
            }
        }
        Err(e) => warn!("Failed to connect to peer {}: {}", addr, e),
    }
}

fn role_for_task(task: &Task) -> NodeRole {
    match task {
        Task::GetStorage | Task::DiskWrite { .. } | Task::DiskRead { .. } => NodeRole::Disk,
        Task::Tcp { .. } | Task::Udp { .. } => NodeRole::Internet,
        _ => NodeRole::Worker,
    }
}

async fn send_pending_tasks<S>(
    socket: &mut S,
    master: &Arc<MasterNode>,
    addr: &str,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let role = match master.connected_nodes.lock().await.get(addr) {
        Some(node) if node.is_worker => node.role.clone(),
        _ => return Ok(()),
    };
    loop {
        let next_task = {
            let mut pending = master.pending_tasks.lock().await;
            let nodes = master.connected_nodes.lock().await;
            if let Some((id, ptask)) = pending
                .iter()
                .find(|(id, p)| {
                    (p.target.as_deref().map(|t| t == addr).unwrap_or(true))
                        && role_for_task(&p.task) == role
                        && !nodes.values().any(|n| n.active_tasks.contains_key(*id))
                })
                .map(|(id, p)| (id.clone(), p.clone()))
            {
                pending.remove(&id);
                Some((id, ptask))
            } else {
                None
            }
        };
        let (id, ptask) = match next_task {
            Some(t) => t,
            None => break,
        };
        master
            .connected_nodes
            .lock()
            .await
            .entry(addr.to_string())
            .and_modify(|n| {
                n.active_tasks.insert(id.clone(), ptask.task.clone());
            });
        let msg = NodeMessage::AssignTask {
            id: id.clone(),
            task: ptask.task.clone(),
        };
        let data = serde_json::to_vec(&msg)?;
        if let Err(e) = write_length_prefixed(socket, &data).await {
            master
                .pending_tasks
                .lock()
                .await
                .insert(id.clone(), ptask.clone());
            master
                .connected_nodes
                .lock()
                .await
                .entry(addr.to_string())
                .and_modify(|n| {
                    n.active_tasks.remove(&id);
                });
            return Err(e);
        }
    }
    Ok(())
}

fn generate_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn allocate_port(master_node: &Arc<MasterNode>) -> Option<u16> {
    let port = {
        let mut ports = master_node.available_ports.lock().await;
        ports.pop_front()
    };
    if port.is_some() {
        spawn_save_state(master_node);
    }
    port
}

async fn release_port(master_node: &Arc<MasterNode>, addr: String) {
    let port = {
        let mut nodes = master_node.connected_nodes.lock().await;
        nodes.get_mut(&addr).and_then(|info| info.port.take())
    };
    if let Some(p) = port {
        master_node.available_ports.lock().await.push_back(p);
    }
    spawn_save_state(master_node);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

async fn cleanup_dead_nodes(master: &Arc<MasterNode>) {
    let timeout = master.failover_timeout_ms * 2;
    let now = now_ms();
    let mut stale = Vec::new();
    {
        let nodes = master.connected_nodes.lock().await;
        for (addr, info) in nodes.iter() {
            if now.saturating_sub(info.last_heartbeat) > timeout {
                stale.push(addr.clone());
            }
        }
    }
    for addr in stale {
        warn!("Node {} timed out", addr);
        let info_opt = { master.connected_nodes.lock().await.remove(&addr) };
        if let Some(info) = info_opt {
            let mut pending = master.pending_tasks.lock().await;
            for (id, task) in info.active_tasks {
                pending.insert(id, PendingTask { task, target: None });
            }
        }
        release_port(master, addr.clone()).await;
    }
    spawn_save_state(master);
}

async fn handle_connection<S>(
    mut socket: S,
    master_node: Arc<MasterNode>,
    token: String,
    addr: String,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let token_bytes = read_length_prefixed(&mut socket).await?;
    let received_token = String::from_utf8(token_bytes)?.trim().to_string();
    if received_token == "MASTER_SYNC" {
        return handle_peer_connection(socket, master_node).await;
    } else if received_token == token {
        info!("Client authenticated with correct token");
        write_length_prefixed(&mut socket, b"OK").await?;
        master_node.connected_nodes.lock().await.insert(
            addr.clone(),
            NodeInfo {
                addr: addr.clone(),
                last_heartbeat: now_ms(),
                port: None,
                active_tasks: HashMap::new(),
                is_worker: false,
                role: cpcluster_common::NodeRole::Worker,
            },
        );
        spawn_save_state(&master_node);
        if let Err(e) = send_pending_tasks(&mut socket, &master_node, &addr).await {
            error!("Failed to send pending tasks: {}", e);
        }
        loop {
            let data = match read_length_prefixed(&mut socket).await {
                Ok(d) => d,
                Err(_) => {
                    warn!("Client disconnected");
                    break;
                }
            };
            let request: NodeMessage = serde_json::from_slice(&data)?;
            match request {
                NodeMessage::RegisterRole(role) => {
                    master_node
                        .connected_nodes
                        .lock()
                        .await
                        .entry(addr.clone())
                        .and_modify(|n| n.role = role);
                }
                NodeMessage::GetConnectedNodes => {
                    let connected_nodes = master_node
                        .connected_nodes
                        .lock()
                        .await
                        .keys()
                        .cloned()
                        .collect::<Vec<String>>();
                    let response = NodeMessage::ConnectedNodes(connected_nodes);
                    let response_data = serde_json::to_vec(&response)?;
                    write_length_prefixed(&mut socket, &response_data).await?;
                    info!("Sent connected nodes list to client");
                }
                NodeMessage::RequestConnection(target_id) => {
                    if let Some(port) = allocate_port(&master_node).await {
                        master_node
                            .connected_nodes
                            .lock()
                            .await
                            .entry(addr.clone())
                            .and_modify(|n| n.port = Some(port));
                        let target_addr = master_node
                            .connected_nodes
                            .lock()
                            .await
                            .get(&target_id)
                            .cloned()
                            .map(|n| n.addr);
                        if let Some(target_addr) = target_addr {
                            let response = NodeMessage::ConnectionInfo(target_addr, port);
                            let response_data = serde_json::to_vec(&response)?;
                            write_length_prefixed(&mut socket, &response_data).await?;
                            info!("Connection info sent to {} on port {}", target_id, port);
                        } else {
                            warn!("Target Node not found");
                        }
                    } else {
                        warn!("No ports available");
                    }
                }
                NodeMessage::SubmitTask { id, task } => {
                    master_node
                        .pending_tasks
                        .lock()
                        .await
                        .insert(id.clone(), PendingTask { task, target: None });
                    spawn_save_state(&master_node);
                    let ack = serde_json::to_vec(&NodeMessage::TaskAccepted(id))?;
                    write_length_prefixed(&mut socket, &ack).await?;
                }
                NodeMessage::GetTaskResult(id) => {
                    let result_opt = master_node.completed_tasks.lock().await.get(&id).cloned();
                    if let Some(result) = result_opt {
                        let resp = NodeMessage::TaskResult { id, result };
                        let data = serde_json::to_vec(&resp)?;
                        write_length_prefixed(&mut socket, &data).await?;
                    } else {
                        let resp = NodeMessage::DirectMessage("Pending".into());
                        let data = serde_json::to_vec(&resp)?;
                        write_length_prefixed(&mut socket, &data).await?;
                    }
                }
                NodeMessage::Disconnect => {
                    info!("Node disconnected and port released.");
                    release_port(&master_node, addr.clone()).await;
                    master_node.connected_nodes.lock().await.remove(&addr);
                    spawn_save_state(&master_node);
                    break;
                }
                NodeMessage::Heartbeat => {
                    master_node
                        .connected_nodes
                        .lock()
                        .await
                        .entry(addr.clone())
                        .and_modify(|n| {
                            n.last_heartbeat = now_ms();
                            n.is_worker = true;
                        });
                    let ack = serde_json::to_vec(&NodeMessage::HeartbeatAck)?;
                    write_length_prefixed(&mut socket, &ack).await?;
                    info!("Heartbeat received from {}", addr);
                }
                NodeMessage::TaskResult { id, result } => {
                    master_node
                        .connected_nodes
                        .lock()
                        .await
                        .entry(addr.clone())
                        .and_modify(|n| {
                            n.active_tasks.remove(&id);
                        });
                    master_node.pending_tasks.lock().await.remove(&id);
                    master_node
                        .completed_tasks
                        .lock()
                        .await
                        .insert(id.clone(), result);
                    spawn_save_state(&master_node);
                }
                _ => warn!("Unknown request"),
            }
            if let Err(e) = send_pending_tasks(&mut socket, &master_node, &addr).await {
                error!("Failed to send pending tasks: {}", e);
            }
        }
        master_node.connected_nodes.lock().await.remove(&addr);
        spawn_save_state(&master_node);
    } else {
        warn!("Client provided an invalid token {}", received_token);
        write_length_prefixed(&mut socket, b"Invalid token").await?;
    }
    Ok(())
}

pub async fn run(config_path: &str, join_path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load(config_path).unwrap_or_default();
    config.save(config_path)?;
    let token = generate_token();
    let addr = config
        .master_addresses
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:55000".to_string());
    let mut parts = addr.split(':');
    let ip = parts.next().unwrap_or("127.0.0.1").to_string();
    let port: u16 = parts.next().unwrap_or("55000").parse().unwrap_or(55000);
    let join_info = JoinInfo {
        token: token.clone(),
        ip: ip.clone(),
        port,
    };
    fs::write(join_path, serde_json::to_string_pretty(&join_info)?)?;
    info!("Join information saved to {}", join_path);
    let listener = TcpListener::bind((ip.as_str(), port)).await?;
    info!("Master Node listening on {}:{}", ip, port);
    let cert_path = config
        .cert_path
        .clone()
        .unwrap_or_else(|| "master_cert.pem".to_string());
    let key_path = config
        .key_path
        .clone()
        .unwrap_or_else(|| "master_key.pem".to_string());
    let tls_config = load_or_generate_tls_config(&cert_path, &key_path, &ip)?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let master_node = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
        available_ports: Arc::new(Mutex::new(VecDeque::from(
            (config.min_port..=config.max_port).collect::<Vec<_>>(),
        ))),
        failover_timeout_ms: config.failover_timeout_ms,
        pending_tasks: Arc::new(Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(Mutex::new(HashMap::new())),
        state_file: config.state_file.clone(),
    });
    load_state(&master_node).await;
    if let Some(peers) = config.peer_masters.clone() {
        for peer in peers {
            let m = Arc::clone(&master_node);
            tokio::spawn(connect_to_peer(peer, m));
        }
    }
    let shell_master = Arc::clone(&master_node);
    let rt_handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        run_shell(shell_master, rt_handle);
    });
    let cleanup_master = Arc::clone(&master_node);
    tokio::spawn(async move {
        let interval = Duration::from_millis(config.failover_timeout_ms);
        loop {
            tokio::time::sleep(interval).await;
            cleanup_dead_nodes(&cleanup_master).await;
        }
    });
    loop {
        let (stream, addr) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let master_node = Arc::clone(&master_node);
        let token = token.clone();
        let acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            let use_tls = !is_local_ip(&addr.ip().to_string());
            if use_tls {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        if let Err(e) =
                            handle_connection(tls_stream, master_node, token, addr.to_string())
                                .await
                        {
                            error!("TLS connection error: {:?}", e);
                        }
                    }
                    Err(e) => error!(
                        "TLS accept from {} failed (kind: {:?}): {}",
                        addr,
                        e.kind(),
                        e
                    ),
                }
            } else if let Err(e) =
                handle_connection(stream, master_node, token, addr.to_string()).await
            {
                error!("Connection error: {:?}", e);
            }
        });
    }
}

pub async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load("CPCluster_masterNode/config/config.json").unwrap_or_default();
    let level = config
        .log_level
        .as_deref()
        .and_then(|l| l.parse().ok())
        .unwrap_or(log::LevelFilter::Info);
    env_logger::Builder::new().filter_level(level).init();
    run("CPCluster_masterNode/config/config.json", "join.json").await
}
