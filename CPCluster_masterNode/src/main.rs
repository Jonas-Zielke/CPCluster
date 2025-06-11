use cpcluster_common::{isLocalIp, JoinInfo, NodeMessage};
use cpcluster_common::config::Config;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{rustls, TlsAcceptor};
use rcgen::generate_simple_self_signed;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};


#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeInfo {
    addr: String,
    last_heartbeat: u64,
    assigned_port: Option<u16>,
}

#[derive(Debug, Clone)]
struct MasterNode {
    connectedNodes: Arc<Mutex<HashMap<String, NodeInfo>>>, // speichert Node-ID und Infos
    availablePorts: Arc<Mutex<HashSet<u16>>>,              // verwaltet verfügbare Ports
    failover_timeout_ms: u64,
    node_channels: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<NodeMessage>>>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load("config.json").unwrap_or_default();
    config.save("config.json").ok();

    let token = generateToken();
    let addr = config
        .master_addresses
        .get(0)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:55000".to_string());
    let mut parts = addr.split(':');
    let ip = parts.next().unwrap_or("127.0.0.1").to_string();
    let port = parts.next().unwrap_or("55000").to_string();

    let joinInfo = JoinInfo {
        token: token.clone(),
        ip: ip.clone(),
        port: port.clone(),
    };
    fs::write("join.json", serde_json::to_string_pretty(&joinInfo)?)?;
    println!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    println!("Master Node listening on {}:{}", ip, port);

    // prepare TLS acceptor using a self-signed certificate
    let tlsConfig = generateTlsConfig()?;
    let tlsAcceptor = TlsAcceptor::from(Arc::new(tlsConfig));

    let masterNode = Arc::new(MasterNode {
        connectedNodes: Arc::new(Mutex::new(HashMap::new())),
        availablePorts: Arc::new(Mutex::new((config.min_port..=config.max_port).collect())),
        failover_timeout_ms: config.failover_timeout_ms,
        node_channels: Arc::new(Mutex::new(HashMap::new())),
    });
    loadState(&masterNode);

    // Cleanup task to remove nodes that stopped sending heartbeats
    let cleanup_master = Arc::clone(&masterNode);
    tokio::spawn(async move {
        let interval = Duration::from_millis(config.failover_timeout_ms);
        loop {
            tokio::time::sleep(interval).await;
            cleanupDeadNodes(&cleanup_master);
        }
    });

    loop {
        let (stream, addr) = listener.accept().await?;
        let masterNode = Arc::clone(&masterNode);
        let token = token.clone();

        let acceptor = tlsAcceptor.clone();
        tokio::spawn(async move {
            let useTls = !isLocalIp(&addr.ip().to_string());
            if useTls {
                match acceptor.accept(stream).await {
                    Ok(tlsStream) => {
                        if let Err(e) = handleConnection(tlsStream, masterNode, token, addr.to_string()).await {
                            eprintln!("TLS connection error: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("TLS accept failed: {:?}", e),
                }
            } else if let Err(e) = handleConnection(stream, masterNode, token, addr.to_string()).await {
                eprintln!("Connection error: {:?}", e);
            }
        });
    }
}

async fn handleConnection<S>(
    mut socket: S,
    masterNode: Arc<MasterNode>,
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
        masterNode
            .connectedNodes
            .lock()
            .unwrap()
            .insert(
                addr.clone(),
                NodeInfo {
                    addr: addr.clone(),
                    last_heartbeat: now_ms(),
                    assigned_port: None,
                },
            );
        saveState(&masterNode);

        // Channel to deliver messages to this node
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<NodeMessage>();
        masterNode
            .node_channels
            .lock()
            .unwrap()
            .insert(addr.clone(), tx);

        loop {
            let mut buf = [0; 1024];
            let read_fut = socket.read(&mut buf);
            tokio::select! {
                result = read_fut => {
                    let n = result?;
                    if n == 0 {
                        println!("Client disconnected");
                        break;
                    }

                    let request: NodeMessage = serde_json::from_slice(&buf[..n])?;
                    match request {
                        NodeMessage::GetConnectedNodes => {
                            // Sende die Liste aller verbundenen Nodes
                            let connectedNodes = masterNode
                                .connectedNodes
                                .lock()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect::<Vec<String>>();

                    let response = NodeMessage::ConnectedNodes(connectedNodes);
                    let response_data = serde_json::to_vec(&response)?;
                    socket.write_all(&response_data).await?;
                    println!("Sent connected nodes list to client");
                }
                        NodeMessage::RequestConnection(target_id) => {
                            if let Some(port) = allocatePort(&masterNode) {
                                let target_addr = {
                                    let mut nodes = masterNode.connectedNodes.lock().unwrap();
                                    let target_addr = nodes.get(&target_id).cloned().map(|n| n.addr.clone());
                                    if let Some(_) = target_addr {
                                        if let Some(n) = nodes.get_mut(&addr) {
                                            n.assigned_port = Some(port);
                                        }
                                        if let Some(n) = nodes.get_mut(&target_id) {
                                            n.assigned_port = Some(port);
                                        }
                                    }
                                    target_addr
                                };

                                if let Some(target_addr) = target_addr {
                                    let response = NodeMessage::ConnectionInfo(target_addr.clone(), port);
                                    let response_data = serde_json::to_vec(&response)?;
                                    socket.write_all(&response_data).await?;

                                    if let Some(tx) = masterNode.node_channels.lock().unwrap().get(&target_id) {
                                        let _ = tx.send(NodeMessage::ConnectionInfo(addr.clone(), port));
                                    }

                                    println!("Connection info sent to {} on port {}", target_id, port);
                                } else {
                                    println!("Target Node not found");
                                    releasePort(&masterNode, addr.clone());
                                }
                            } else {
                                println!("No ports available");
                            }
                        }
                        NodeMessage::Disconnect => {
                            // Entferne die Node und gebe den Port frei
                            println!("Node disconnected and port released.");
                            releasePort(&masterNode, addr.clone());
                            masterNode.connectedNodes.lock().unwrap().remove(&addr);
                            masterNode.node_channels.lock().unwrap().remove(&addr);
                            saveState(&masterNode);
                            break;
                        }
                        NodeMessage::Heartbeat => {
                            masterNode
                                .connectedNodes
                                .lock()
                                .unwrap()
                                .entry(addr.clone())
                                .and_modify(|n| n.last_heartbeat = now_ms());
                        }
                        _ => println!("Unknown request"),
                    }
                }
                Some(msg) = rx.recv() => {
                    let data = serde_json::to_vec(&msg)?;
                    socket.write_all(&data).await?;
                }
            }
        }

        // Entferne die Node aus der Liste der verbundenen Nodes
        masterNode.connectedNodes.lock().unwrap().remove(&addr);
        masterNode.node_channels.lock().unwrap().remove(&addr);
        saveState(&masterNode);
    } else {
        println!("Client provided an invalid token {}", received_token);
        socket.write_all(b"Invalid token").await?;
    }

    Ok(())
}

fn generateToken() -> String {
    Uuid::new_v4().to_string()
}

fn allocatePort(masterNode: &MasterNode) -> Option<u16> {
    let port = {
        let mut ports = masterNode.availablePorts.lock().unwrap();
        ports.iter().cloned().next().map(|p| {
            ports.remove(&p);
            p
        })
    };
    if port.is_some() {
        saveState(masterNode);
    }
    port
}

fn releasePort(masterNode: &MasterNode, addr: String) {
    let port = {
        let mut nodes = masterNode.connectedNodes.lock().unwrap();
        if let Some(info) = nodes.get_mut(&addr) {
            let p = info.assigned_port;
            info.assigned_port = None;
            p
        } else {
            None
        }
    };

    if let Some(port) = port {
        {
            let mut ports = masterNode.availablePorts.lock().unwrap();
            ports.insert(port);
        }
        saveState(masterNode);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn cleanupDeadNodes(master: &MasterNode) {
    let timeout = master.failover_timeout_ms * 2;
    let now = now_ms();
    let mut stale = Vec::new();
    {
        let nodes = master.connectedNodes.lock().unwrap();
        for (addr, info) in nodes.iter() {
            if now.saturating_sub(info.last_heartbeat) > timeout {
                stale.push(addr.clone());
            }
        }
    }
    for addr in stale {
        println!("Node {} timed out", addr);
        releasePort(master, addr.clone());
        master.connectedNodes.lock().unwrap().remove(&addr);
    }
    saveState(master);
}

fn generateTlsConfig() -> Result<rustls::ServerConfig, Box<dyn Error + Send + Sync>> {
    let cert = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    let config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![rustls::Certificate(cert_der)], rustls::PrivateKey(key_der))?;
    Ok(config)
}

#[derive(Serialize, Deserialize)]
struct MasterState {
    connected_nodes: HashMap<String, NodeInfo>,
    available_ports: Vec<u16>,
}

fn loadState(master: &MasterNode) {
    if let Ok(data) = fs::read_to_string("master_state.json") {
        if let Ok(state) = serde_json::from_str::<MasterState>(&data) {
            *master.connectedNodes.lock().unwrap() = state.connected_nodes;
            *master.availablePorts.lock().unwrap() = state.available_ports.into_iter().collect();
        }
    }
}

fn saveState(master: &MasterNode) {
    let state = MasterState {
        connected_nodes: master.connectedNodes.lock().unwrap().clone(),
        available_ports: master.availablePorts.lock().unwrap().iter().cloned().collect(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&state) {
        let _ = fs::write("master_state.json", data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_master() -> MasterNode {
        MasterNode {
            connectedNodes: Arc::new(Mutex::new(HashMap::new())),
            availablePorts: Arc::new(Mutex::new(HashSet::from([6000u16]))),
            failover_timeout_ms: 1000,
            node_channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn release_port_returns_it_to_pool() {
        let master = test_master();
        master.connectedNodes.lock().unwrap().insert(
            "node".into(),
            NodeInfo { addr: "node".into(), last_heartbeat: 0, assigned_port: Some(6000) },
        );

        // port is removed from available pool
        allocatePort(&master);
        assert!(master.availablePorts.lock().unwrap().is_empty());

        releasePort(&master, "node".into());
        assert!(master.availablePorts.lock().unwrap().contains(&6000));
        assert!(master.connectedNodes.lock().unwrap().get("node").unwrap().assigned_port.is_none());
    }
}
