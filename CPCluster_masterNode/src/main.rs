use cpcluster_common::{JoinInfo, NodeMessage};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

const MIN_PORT: u16 = 55001;
const MAX_PORT: u16 = 55999;

#[derive(Debug, Clone)]
struct MasterNode {
    connectedNodes: Arc<Mutex<HashMap<String, String>>>, // speichert Node-ID und IP-Adresse
    availablePorts: Arc<Mutex<HashSet<u16>>>,            // verwaltet verfügbare Ports
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let token = generate_token();
    let ip = "127.0.0.1".to_string();
    let port = "55000".to_string();

    let joinInfo = JoinInfo {
        token: token.clone(),
        ip: ip.clone(),
        port: port.clone(),
    };
    fs::write("join.json", serde_json::to_string_pretty(&joinInfo)?)?;
    println!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    println!("Master Node listening on {}:{}", ip, port);

    let masterNode = Arc::new(MasterNode {
        connectedNodes: Arc::new(Mutex::new(HashMap::new())),
        availablePorts: Arc::new(Mutex::new((MIN_PORT..=MAX_PORT).collect())),
    });

    loop {
        let (stream, addr) = listener.accept().await?;
        let masterNode = Arc::clone(&masterNode);
        let token = token.clone();

        tokio::spawn(async move {
            if let Err(e) = handleConnection(stream, masterNode, token, addr.to_string()).await {
                eprintln!("Connection error: {:?}", e);
            }
        });
    }
}

async fn handleConnection(
    mut socket: TcpStream,
    masterNode: Arc<MasterNode>,
    token: String,
    addr: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
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
            .insert(addr.clone(), addr.clone());

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
                    // Prüfe, ob ein freier Port verfügbar ist
                    if let Some(port) = allocatePort(&masterNode) {
                        // Hole die Adresse der Ziel-Node
                        let target_addr = masterNode
                            .connectedNodes
                            .lock()
                            .unwrap()
                            .get(&target_id)
                            .cloned();

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
                    releasePort(&masterNode, addr.clone());
                    break;
                }
                _ => println!("Unknown request"),
            }
        }

        // Entferne die Node aus der Liste der verbundenen Nodes
        masterNode.connectedNodes.lock().unwrap().remove(&addr);
    } else {
        println!("Client provided an invalid token {}", received_token);
        socket.write_all(b"Invalid token").await?;
    }

    Ok(())
}

fn generate_token() -> String {
    Uuid::new_v4().to_string()
}

fn allocatePort(masterNode: &MasterNode) -> Option<u16> {
    let mut ports = masterNode.availablePorts.lock().unwrap();
    ports.iter().cloned().next().map(|port| {
        ports.remove(&port);
        port
    })
}

fn releasePort(masterNode: &MasterNode, addr: String) {
    let mut ports = masterNode.availablePorts.lock().unwrap();
    if let Some(port) = addr.split(':').nth(1).and_then(|p| p.parse::<u16>().ok()) {
        ports.insert(port);
    }
}
