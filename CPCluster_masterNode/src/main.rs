use cpcluster_common::config::Config;
use cpcluster_common::{is_local_ip, JoinInfo, NodeMessage, Task, TaskResult};
use cpcluster_common::{read_length_prefixed, write_length_prefixed};
use log::{error, info, warn};
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    sync::Arc,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::{rustls, TlsAcceptor};
use uuid::Uuid;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeInfo {
    addr: String,
    last_heartbeat: u64,
    port: Option<u16>,
    active_tasks: HashMap<String, Task>,
    #[serde(default)]
    is_worker: bool,
}

#[derive(Debug, Clone)]
struct MasterNode {
    connected_nodes: Arc<Mutex<HashMap<String, NodeInfo>>>, // speichert Node-ID und Infos
    available_ports: Arc<Mutex<HashSet<u16>>>,              // verwaltet verfügbare Ports
    failover_timeout_ms: u64,
    pending_tasks: Arc<Mutex<HashMap<String, Task>>>,
    completed_tasks: Arc<Mutex<HashMap<String, TaskResult>>>,
}

fn assign_tasks_to_nodes(master: &MasterNode) {
    let tasks: Vec<(String, Task)> = {
        master
            .pending_tasks
            .blocking_lock()
            .iter()
            .map(|(id, t)| (id.clone(), t.clone()))
            .collect()
    };
    if tasks.is_empty() {
        return;
    }
    let mut nodes = master.connected_nodes.blocking_lock();
    nodes.retain(|_, n| n.is_worker);
    if nodes.is_empty() {
        return;
    }
    let keys: Vec<String> = nodes.keys().cloned().collect();
    for (i, (id, task)) in tasks.into_iter().enumerate() {
        let key = &keys[i % keys.len()];
        if let Some(node) = nodes.get_mut(key) {
            node.active_tasks.entry(id).or_insert(task);
        }
    }
}

fn run_shell(master: Arc<MasterNode>) {
    use std::io::{self, BufRead, Write};
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    println!(
        "CPCluster shell ready.\nCommands:\n  nodes                - list connected nodes\n  tasks                - list active and pending tasks\n  task <id>            - show status or result of a task\n  addtask <type> <arg> - queue a task (type: compute|http)\n  exit                 - quit"
    );
    print!("> ");
    let _ = std::io::stdout().flush();
    let mut exit_shell = false;
    while let Some(Ok(line)) = lines.next() {
        for cmd in line.split(';') {
            match cmd.trim() {
                "nodes" => {
                    let nodes = master.connected_nodes.blocking_lock();
                    if nodes.is_empty() {
                        println!("No connected nodes");
                    } else {
                        for addr in nodes.keys() {
                            println!("{}", addr);
                        }
                    }
                }
                "tasks" => {
                    let nodes = master.connected_nodes.blocking_lock();
                    if nodes.is_empty() {
                        println!("No active tasks");
                    } else {
                        for (addr, info) in nodes.iter() {
                            for (id, task) in info.active_tasks.iter() {
                                println!("{}: {} -> {:?}", addr, id, task);
                            }
                        }
                    }
                    drop(nodes);
                    let pending = master.pending_tasks.blocking_lock();
                    if pending.is_empty() {
                        println!("No pending tasks");
                    } else {
                        println!("Pending tasks:");
                        for (id, task) in pending.iter() {
                            println!("{} -> {:?}", id, task);
                        }
                    }
                    drop(pending);
                    let completed = master.completed_tasks.blocking_lock();
                    if completed.is_empty() {
                        println!("No completed tasks");
                    } else {
                        println!("Completed tasks:");
                        for (id, res) in completed.iter() {
                            println!("{} -> {:?}", id, res);
                        }
                    }
                }
                cmd if cmd.trim_start().starts_with("task") => {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.len() != 2 {
                        println!("Usage: task <id>");
                    } else {
                        let id = parts[1];
                        if master.pending_tasks.blocking_lock().contains_key(id) {
                            println!("Task {} is pending", id);
                        } else if master
                            .connected_nodes
                            .blocking_lock()
                            .values()
                            .any(|n| n.active_tasks.contains_key(id))
                        {
                            println!("Task {} is running", id);
                        } else if let Some(result) = master.completed_tasks.blocking_lock().get(id)
                        {
                            println!("Task {} finished: {:?}", id, result);
                        } else {
                            println!("Task {} not found", id);
                        }
                    }
                }
                cmd if cmd.trim_start().starts_with("addtask") => {
                    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
                    if parts.len() < 3 {
                        println!("Usage: addtask compute <expression> | addtask http <url>");
                    } else {
                        let id = uuid::Uuid::new_v4().to_string();
                        let task = match parts[1] {
                            "compute" => Task::Compute {
                                expression: parts[2].to_string().into(),
                            },
                            "http" => Task::HttpRequest {
                                url: parts[2].to_string(),
                            },
                            _ => {
                                println!("Unknown task type. Use 'compute' or 'http'.");
                                continue;
                            }
                        };
                        master
                            .pending_tasks
                            .blocking_lock()
                            .insert(id.clone(), task);
                        save_state(&master);
                        println!("Queued task {}", id);
                    }
                }
                "exit" | "quit" => {
                    println!("Exiting shell");
                    exit_shell = true;
                    break;
                }
                "" => {}
                _ => println!("Unknown command"),
            }
            if exit_shell {
                break;
            }
        }
        if exit_shell {
            break;
        }
        print!("> ");
        let _ = std::io::stdout().flush();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let config = Config::load("config.json").unwrap_or_default();
    config.save("config.json").ok();

    let token = generate_token();
    let addr = config
        .master_addresses
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:55000".to_string());
    let mut parts = addr.split(':');
    let ip = parts.next().unwrap_or("127.0.0.1").to_string();
    let port = parts.next().unwrap_or("55000").to_string();

    let join_info = JoinInfo {
        token: token.clone(),
        ip: ip.clone(),
        port: port.clone(),
    };
    fs::write("join.json", serde_json::to_string_pretty(&join_info)?)?;
    info!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    info!("Master Node listening on {}:{}", ip, port);

    // prepare TLS acceptor using configured or self-signed certificate
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
    });
    load_state(&master_node);

    let shell_master = Arc::clone(&master_node);
    std::thread::spawn(move || {
        run_shell(shell_master);
    });

    // Cleanup task to remove nodes that stopped sending heartbeats
    let cleanup_master = Arc::clone(&master_node);
    tokio::spawn(async move {
        let interval = Duration::from_millis(config.failover_timeout_ms);
        loop {
            tokio::time::sleep(interval).await;
            cleanup_dead_nodes(&cleanup_master);
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
                    Err(e) => error!("TLS accept failed: {:?}", e),
                }
            } else if let Err(e) =
                handle_connection(stream, master_node, token, addr.to_string()).await
            {
                error!("Connection error: {:?}", e);
            }
        });
    }
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

        // Inform the client that authentication succeeded so it can
        // continue with the protocol. Without this message the client
        // would block waiting for a response.
        write_length_prefixed(&mut socket, b"OK").await?;

        // Füge die Node zur verbundenen Liste hinzu
        master_node.connected_nodes.lock().await.insert(
            addr.clone(),
            NodeInfo {
                addr: addr.clone(),
                last_heartbeat: now_ms(),
                port: None,
                active_tasks: HashMap::new(),
                is_worker: false,
            },
        );
        save_state(&master_node);
        // assign any pending tasks to this node immediately
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
                NodeMessage::GetConnectedNodes => {
                    // Sende die Liste aller verbundenen Nodes
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
                    // Prüfe, ob ein freier Port verfügbar ist
                    if let Some(port) = allocate_port(&master_node) {
                        master_node
                            .connected_nodes
                            .lock()
                            .await
                            .entry(addr.clone())
                            .and_modify(|n| n.port = Some(port));
                        // Hole die Adresse der Ziel-Node
                        let target_addr = master_node
                            .connected_nodes
                            .lock()
                            .await
                            .get(&target_id)
                            .cloned()
                            .map(|n| n.addr);

                        if let Some(target_addr) = target_addr {
                            // Sende die Verbindungsinformation an die anfragende Node
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
                    save_state(&master_node);
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
                    // Entferne die Node und gebe den Port frei
                    info!("Node disconnected and port released.");
                    release_port(&master_node, addr.clone());
                    master_node.connected_nodes.lock().await.remove(&addr);
                    save_state(&master_node);
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
                    save_state(&master_node);
                }
                _ => warn!("Unknown request"),
            }
            // try to dispatch new tasks that might be pending
            if let Err(e) = send_pending_tasks(&mut socket, &master_node, &addr).await {
                error!("Failed to send pending tasks: {}", e);
            }
        }

        // Entferne die Node aus der Liste der verbundenen Nodes
        master_node.connected_nodes.lock().await.remove(&addr);
        save_state(&master_node);
    } else {
        warn!("Client provided an invalid token {}", received_token);
        write_length_prefixed(&mut socket, b"Invalid token").await?;
    }

    Ok(())
}

fn generate_token() -> String {
    Uuid::new_v4().to_string()
}

fn allocate_port(master_node: &MasterNode) -> Option<u16> {
    let port = {
        let mut ports = master_node.available_ports.blocking_lock();
        let port_opt = ports.iter().cloned().next();
        if let Some(p) = port_opt {
            ports.remove(&p);
            Some(p)
        } else {
            None
        }
    };
    if port.is_some() {
        save_state(master_node);
    }
    port
}

fn release_port(master_node: &MasterNode, addr: String) {
    let port = {
        let mut nodes = master_node.connected_nodes.blocking_lock();
        nodes.get_mut(&addr).and_then(|info| info.port.take())
    };
    if let Some(p) = port {
        master_node.available_ports.blocking_lock().insert(p);
    }
    save_state(master_node);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn cleanup_dead_nodes(master: &MasterNode) {
    let timeout = master.failover_timeout_ms * 2;
    let now = now_ms();
    let mut stale = Vec::new();
    {
        let nodes = master.connected_nodes.blocking_lock();
        for (addr, info) in nodes.iter() {
            if now.saturating_sub(info.last_heartbeat) > timeout {
                stale.push(addr.clone());
            }
        }
    }
    for addr in stale {
        warn!("Node {} timed out", addr);
        let info_opt = { master.connected_nodes.blocking_lock().remove(&addr) };
        if let Some(info) = info_opt {
            let mut pending = master.pending_tasks.blocking_lock();
            for (id, task) in info.active_tasks {
                pending.insert(id, task);
            }
        }
        release_port(master, addr);
    }
    save_state(master);
}

fn load_or_generate_tls_config(
    cert_path: &str,
    key_path: &str,
    host: &str,
) -> Result<rustls::ServerConfig, Box<dyn Error + Send + Sync>> {
    if std::path::Path::new(cert_path).exists() && std::path::Path::new(key_path).exists() {
        let mut cert_file = std::io::BufReader::new(fs::File::open(cert_path)?);
        let mut key_file = std::io::BufReader::new(fs::File::open(key_path)?);
        let certs = rustls_pemfile::certs(&mut cert_file)?
            .into_iter()
            .map(rustls::Certificate)
            .collect();
        let keys = rustls_pemfile::pkcs8_private_keys(&mut key_file)?;
        let key = keys.first().ok_or("no private key found")?.clone();
        let config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(certs, rustls::PrivateKey(key))?;
        Ok(config)
    } else {
        let cert = generate_simple_self_signed(vec![host.to_string()])?;
        fs::write(cert_path, cert.serialize_pem()?)?;
        fs::write(key_path, cert.serialize_private_key_pem())?;
        let cert_der = cert.serialize_der()?;
        let key_der = cert.serialize_private_key_der();
        let config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::Certificate(cert_der)],
                rustls::PrivateKey(key_der),
            )?;
        Ok(config)
    }
}

#[derive(Serialize, Deserialize)]
struct MasterState {
    connected_nodes: HashMap<String, NodeInfo>,
    available_ports: Vec<u16>,
    pending_tasks: HashMap<String, Task>,
    #[serde(default)]
    completed_tasks: HashMap<String, TaskResult>,
}

fn load_state(master: &MasterNode) {
    if let Ok(data) = fs::read_to_string("master_state.json") {
        if let Ok(state) = serde_json::from_str::<MasterState>(&data) {
            *master.connected_nodes.blocking_lock() = state.connected_nodes;
            *master.available_ports.blocking_lock() = state.available_ports.into_iter().collect();
            *master.pending_tasks.blocking_lock() = state.pending_tasks;
            *master.completed_tasks.blocking_lock() = state.completed_tasks;
        }
    }
    assign_tasks_to_nodes(master);
}

fn save_state(master: &MasterNode) {
    let state = MasterState {
        connected_nodes: master.connected_nodes.blocking_lock().clone(),
        available_ports: master
            .available_ports
            .blocking_lock()
            .iter()
            .cloned()
            .collect(),
        pending_tasks: master.pending_tasks.blocking_lock().clone(),
        completed_tasks: master.completed_tasks.blocking_lock().clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&state) {
        let _ = fs::write("master_state.json", data);
    }
    assign_tasks_to_nodes(master);
}
