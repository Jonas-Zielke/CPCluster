use cpcluster_common::config::Config;
use cpcluster_common::{is_local_ip, JoinInfo, NodeMessage, Task};
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::{rustls, TlsAcceptor};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeInfo {
    addr: String,
    last_heartbeat: u64,
    port: Option<u16>,
    active_tasks: HashMap<String, Task>,
}

#[derive(Debug, Clone)]
struct MasterNode {
    connected_nodes: Arc<Mutex<HashMap<String, NodeInfo>>>, // speichert Node-ID und Infos
    available_ports: Arc<Mutex<HashSet<u16>>>,              // verwaltet verfügbare Ports
    failover_timeout_ms: u64,
    pending_tasks: Arc<Mutex<HashMap<String, Task>>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load("config.json").unwrap_or_default();
    config.save("config.json").ok();

    let token = generate_token();
    let addr = config
        .master_addresses
        .get(0)
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
    println!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    println!("Master Node listening on {}:{}", ip, port);

    // prepare TLS acceptor using configured or self-signed certificate
    let cert_path = config
        .cert_path
        .clone()
        .unwrap_or_else(|| "master_cert.pem".to_string());
    let key_path = config
        .key_path
        .clone()
        .unwrap_or_else(|| "master_key.pem".to_string());
    let tls_config = load_or_generate_tls_config(&cert_path, &key_path)?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let master_node = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
        available_ports: Arc::new(Mutex::new((config.min_port..=config.max_port).collect())),
        failover_timeout_ms: config.failover_timeout_ms,
        pending_tasks: Arc::new(Mutex::new(HashMap::new())),
    });
    load_state(&master_node);

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
                            handle_connection(tls_stream, master_node, token, addr.to_string()).await
                        {
                            eprintln!("TLS connection error: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("TLS accept failed: {:?}", e),
                }
            } else if let Err(e) =
                handle_connection(stream, master_node, token, addr.to_string()).await
            {
                eprintln!("Connection error: {:?}", e);
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
    let mut buf = [0; 1024];
    let n = socket.read(&mut buf).await?;

    let received_token = std::str::from_utf8(&buf[..n])?.trim().to_string();
    if received_token == token {
        println!("Client authenticated with correct token");

        // Inform the client that authentication succeeded so it can
        // continue with the protocol. Without this message the client
        // would block waiting for a response.
        socket.write_all(b"OK").await?;

        // Füge die Node zur verbundenen Liste hinzu
        master_node.connected_nodes.lock().unwrap().insert(
            addr.clone(),
            NodeInfo {
                addr: addr.clone(),
                last_heartbeat: now_ms(),
                port: None,
                active_tasks: HashMap::new(),
            },
        );
        save_state(&master_node);

        loop {
            let mut buf = [0; 1024];
            let n = socket.read(&mut buf).await?;
            if n == 0 {
                println!("Client disconnected");
                break;
            }

            let request: NodeMessage = serde_json::from_slice(&buf[..n])?;
            match request {
                NodeMessage::GetConnectedNodes => {
                    // Sende die Liste aller verbundenen Nodes
                    let connected_nodes = master_node
                        .connected_nodes
                        .lock()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect::<Vec<String>>();

                    let response = NodeMessage::ConnectedNodes(connected_nodes);
                    let response_data = serde_json::to_vec(&response)?;
                    socket.write_all(&response_data).await?;
                    println!("Sent connected nodes list to client");
                }
                NodeMessage::RequestConnection(target_id) => {
                    // Prüfe, ob ein freier Port verfügbar ist
                    if let Some(port) = allocate_port(&master_node) {
                        master_node
                            .connected_nodes
                            .lock()
                            .unwrap()
                            .entry(addr.clone())
                            .and_modify(|n| n.port = Some(port));
                        // Hole die Adresse der Ziel-Node
                        let target_addr = master_node
                            .connected_nodes
                            .lock()
                            .unwrap()
                            .get(&target_id)
                            .cloned()
                            .map(|n| n.addr);

                        if let Some(target_addr) = target_addr {
                            // Sende die Verbindungsinformation an die anfragende Node
                            let response = NodeMessage::ConnectionInfo(target_addr, port);
                            let response_data = serde_json::to_vec(&response)?;
                            socket.write_all(&response_data).await?;
                            println!("Connection info sent to {} on port {}", target_id, port);
                        } else {
                            println!("Target Node not found");
                        }
                    } else {
                        println!("No ports available");
                    }
                }
                NodeMessage::Disconnect => {
                    // Entferne die Node und gebe den Port frei
                    println!("Node disconnected and port released.");
                    release_port(&master_node, addr.clone());
                    master_node.connected_nodes.lock().unwrap().remove(&addr);
                    save_state(&master_node);
                    break;
                }
                NodeMessage::Heartbeat => {
                    master_node
                        .connected_nodes
                        .lock()
                        .unwrap()
                        .entry(addr.clone())
                        .and_modify(|n| n.last_heartbeat = now_ms());
                }
                NodeMessage::TaskResult { id, .. } => {
                    master_node
                        .connected_nodes
                        .lock()
                        .unwrap()
                        .entry(addr.clone())
                        .and_modify(|n| {
                            n.active_tasks.remove(&id);
                        });
                    master_node.pending_tasks.lock().unwrap().remove(&id);
                }
                _ => println!("Unknown request"),
            }
        }

        // Entferne die Node aus der Liste der verbundenen Nodes
        master_node.connected_nodes.lock().unwrap().remove(&addr);
        save_state(&master_node);
    } else {
        println!("Client provided an invalid token {}", received_token);
        socket.write_all(b"Invalid token").await?;
    }

    Ok(())
}

fn generate_token() -> String {
    Uuid::new_v4().to_string()
}

fn allocate_port(master_node: &MasterNode) -> Option<u16> {
    let mut ports = master_node.available_ports.lock().unwrap();
    ports.iter().cloned().next().map(|port| {
        ports.remove(&port);
        save_state(master_node);
        port
    })
}

fn release_port(master_node: &MasterNode, addr: String) {
    if let Some(info) = master_node.connected_nodes.lock().unwrap().get_mut(&addr) {
        if let Some(port) = info.port.take() {
            master_node.available_ports.lock().unwrap().insert(port);
        }
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
        let nodes = master.connected_nodes.lock().unwrap();
        for (addr, info) in nodes.iter() {
            if now.saturating_sub(info.last_heartbeat) > timeout {
                stale.push(addr.clone());
            }
        }
    }
    for addr in stale {
        println!("Node {} timed out", addr);
        if let Some(info) = master.connected_nodes.lock().unwrap().remove(&addr) {
            let mut pending = master.pending_tasks.lock().unwrap();
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
        let cert = generate_simple_self_signed(vec!["localhost".to_string()])?;
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
}

fn load_state(master: &MasterNode) {
    if let Ok(data) = fs::read_to_string("master_state.json") {
        if let Ok(state) = serde_json::from_str::<MasterState>(&data) {
            *master.connected_nodes.lock().unwrap() = state.connected_nodes;
            *master.available_ports.lock().unwrap() = state.available_ports.into_iter().collect();
            *master.pending_tasks.lock().unwrap() = state.pending_tasks;
        }
    }
}

fn save_state(master: &MasterNode) {
    let state = MasterState {
        connected_nodes: master.connected_nodes.lock().unwrap().clone(),
        available_ports: master
            .available_ports
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect(),
        pending_tasks: master.pending_tasks.lock().unwrap().clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&state) {
        let _ = fs::write("master_state.json", data);
    }
}
