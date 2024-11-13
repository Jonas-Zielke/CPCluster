use std::{collections::HashMap, error::Error, fs, sync::{Arc, Mutex}};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct JoinInfo {
    token: String,
    ip: String,
    port: String,
}

#[derive(Debug, Clone)]
struct MasterNode {
    connected_nodes: Arc<Mutex<HashMap<String, String>>>,  // speichert IP-Adressen als Strings
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Task {
    id: String,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
enum NodeMessage {
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    Task(Task),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let token = generate_token();
    let ip = "127.0.0.1".to_string();
    let port = "55000".to_string();

    let join_info = JoinInfo { token: token.clone(), ip: ip.clone(), port: port.clone() };
    fs::write("join.json", serde_json::to_string_pretty(&join_info)?)?;
    println!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    println!("Master Node listening on {}:{}", ip, port);

    let master_node = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
    });

    loop {
        let (stream, addr) = listener.accept().await?;
        let master_node = Arc::clone(&master_node);
        let token = token.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, master_node, token, addr.to_string()).await {
                eprintln!("Connection error: {:?}", e);
            }
        });
    }
}

async fn handle_connection(
    mut socket: TcpStream,
    master_node: Arc<MasterNode>,
    token: String,
    addr: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut buf = [0; 1024];
    let n = socket.read(&mut buf).await?;

    let received_token = std::str::from_utf8(&buf[..n])?.trim().to_string();
    if received_token == token {
        println!("Client authenticated with correct token");
        {
            // Node zum Netzwerk hinzufügen
            master_node.connected_nodes.lock().unwrap().insert(addr.clone(), addr.clone());
        }

        loop {
            let mut buf = [0; 1024];
            let n = socket.read(&mut buf).await?;
            if n == 0 {
                println!("Client disconnected");
                break;
            }

            // Nachricht empfangen und verarbeiten
            let request: NodeMessage = serde_json::from_slice(&buf[..n])?;
            match request {
                NodeMessage::GetConnectedNodes => {
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
                _ => println!("Unknown request"),
            }
        }

        // Bei Trennung den Node entfernen
        master_node.connected_nodes.lock().unwrap().remove(&addr);
    } else {
        println!("Client provided an invalid token");
        socket.write_all(b"Invalid token").await?;
    }

    Ok(())
}

fn generate_token() -> String {
    Uuid::new_v4().to_string()
}
