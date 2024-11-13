use std::{error::Error, fs, time::Duration};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::from_slice;

#[derive(Serialize, Deserialize)]
struct JoinInfo {
    token: String,
    ip: String,
    port: String,
}

#[derive(Serialize, Deserialize, Debug)]
enum NodeMessage {
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    Task(Task),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Task {
    id: String,
    data: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Lade die Join-Informationen
    let join_info: JoinInfo = serde_json::from_str(&fs::read_to_string("join.json")?)?;
    let master_addr = format!("{}:{}", join_info.ip, join_info.port);

    // Verbindung zur Master-Node herstellen
    let mut socket = TcpStream::connect(&master_addr).await?;
    println!("Connected to Master Node at {}", master_addr);

    // Token senden und auf Authentifizierung warten
    socket.write_all(join_info.token.as_bytes()).await?;
    println!("Token sent for authentication");

    // Erfolgreiche Authentifizierung annehmen und ConnectedNodes abfragen
    tokio::time::sleep(Duration::from_secs(1)).await;
    let get_nodes_msg = serde_json::to_vec(&NodeMessage::GetConnectedNodes)?;
    socket.write_all(&get_nodes_msg).await?;
    println!("Sent message to Master Node: GetConnectedNodes");

    // Warte auf Antworten in einer Schleife
    let mut buf = vec![0; 1024];
    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            println!("Connection closed by Master Node");
            break;
        }

        // Antwort von der Master-Node analysieren
        let response: NodeMessage = from_slice(&buf[..n])?;
        match response {
            NodeMessage::ConnectedNodes(nodes) => {
                println!("Connected nodes in the network: {:?}", nodes);
            }
            NodeMessage::Task(task) => {
                println!("Received task from Master Node: {:?}", task);
                // Hier könnte die Logik zur Ausführung der Aufgabe hinzugefügt werden
            }
            _ => {
                println!("Received unexpected response from Master Node: {:?}", response);
            }
        }

        buf.clear();  // Buffer nach jeder Nachricht leeren
    }

    Ok(())
}
