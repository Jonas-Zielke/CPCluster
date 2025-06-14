use cpcluster_common::config::Config;
use cpcluster_common::{is_local_ip, JoinInfo, NodeMessage};
use cpcluster_common::{read_length_prefixed, write_length_prefixed};
use log::{error, info, warn};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, error::Error, fs, sync::Arc};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

pub mod shell;
pub mod state;
pub mod tls;

use shell::run_shell;
use state::{load_state, save_state, MasterNode, NodeInfo};
use tls::load_or_generate_tls_config;

async fn send_pending_tasks<S>(
    socket: &mut S,
    master: &MasterNode,
    addr: &str,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(node) = master.connected_nodes.lock().await.get(addr) {
        if !node.is_worker {
            return Ok(());
        }
    }
    loop {
        let next_task = {
            let mut pending = master.pending_tasks.lock().await;
            let nodes = master.connected_nodes.lock().await;
            if let Some((id, task)) = pending
                .iter()
                .find(|(id, _)| !nodes.values().any(|n| n.active_tasks.contains_key(*id)))
                .map(|(id, task)| (id.clone(), task.clone()))
            {
                pending.remove(&id);
                Some((id, task))
            } else {
                None
            }
        };
        let (id, task) = match next_task {
            Some(t) => t,
            None => break,
        };
        master
            .connected_nodes
            .lock()
            .await
            .entry(addr.to_string())
            .and_modify(|n| {
                n.active_tasks.insert(id.clone(), task.clone());
            });
        let msg = NodeMessage::AssignTask {
            id: id.clone(),
            task: task.clone(),
        };
        let data = serde_json::to_vec(&msg)?;
        if let Err(e) = write_length_prefixed(socket, &data).await {
            master
                .pending_tasks
                .lock()
                .await
                .insert(id.clone(), task.clone());
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

async fn allocate_port(master_node: &MasterNode) -> Option<u16> {
    let port = {
        let mut ports = master_node.available_ports.lock().await;
        let port_opt = ports.iter().cloned().next();
        if let Some(p) = port_opt {
            ports.remove(&p);
            Some(p)
        } else {
            None
        }
    };
    if port.is_some() {
        save_state(master_node).await;
    }
    port
}

async fn release_port(master_node: &MasterNode, addr: String) {
    let port = {
        let mut nodes = master_node.connected_nodes.lock().await;
        nodes.get_mut(&addr).and_then(|info| info.port.take())
    };
    if let Some(p) = port {
        master_node.available_ports.lock().await.insert(p);
    }
    save_state(master_node).await;
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

async fn cleanup_dead_nodes(master: &MasterNode) {
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
                pending.insert(id, task);
            }
        }
        release_port(master, addr.clone()).await;
    }
    save_state(master).await;
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
    if received_token == token {
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
        save_state(&master_node).await;
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
                        .insert(id.clone(), task);
                    save_state(&master_node).await;
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
                    save_state(&master_node).await;
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
                    save_state(&master_node).await;
                }
                _ => warn!("Unknown request"),
            }
            if let Err(e) = send_pending_tasks(&mut socket, &master_node, &addr).await {
                error!("Failed to send pending tasks: {}", e);
            }
        }
        master_node.connected_nodes.lock().await.remove(&addr);
        save_state(&master_node).await;
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
        available_ports: Arc::new(Mutex::new((config.min_port..=config.max_port).collect())),
        failover_timeout_ms: config.failover_timeout_ms,
        pending_tasks: Arc::new(Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(Mutex::new(HashMap::new())),
        state_file: config.state_file.clone(),
    });
    load_state(&master_node).await;
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
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();
    run("config.json", "join.json").await
}
