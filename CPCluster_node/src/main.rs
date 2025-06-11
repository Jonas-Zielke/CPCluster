use std::{error::Error, fs, sync::Arc};
use cpcluster_protocol::{JoinInfo, NodeMessage};
use tokio::{net::{TcpStream, tcp::OwnedWriteHalf}, io::{AsyncReadExt, AsyncWriteExt}};


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let join_info = fs::read_to_string("join.json")?;
    let join_info: JoinInfo = serde_json::from_str(&join_info)?;

    let master_addr = format!("{}:{}", join_info.ip, join_info.port);
    let mut master_stream = TcpStream::connect(&master_addr).await?;
    println!("Connected to Master Node at {}", master_addr);

    // Send only the token for authentication
    master_stream.write_all(join_info.token.as_bytes()).await?;
    println!("Token sent for authentication");

    // Read and verify authentication response from the master node
    let mut auth_response = vec![0; 1024];
    let n = master_stream.read(&mut auth_response).await?;
    if n == 0 || &auth_response[..n] == b"Invalid token" {
        println!("Authentication failed");
        return Ok(());
    }
    println!("Authentication successful");

    // Request to get the list of currently connected nodes
    let get_nodes_request = NodeMessage::GetConnectedNodes;
    send_message(&mut master_stream, get_nodes_request).await?;

    // Receive and display the list of connected nodes
    let mut buf = vec![0; 1024];
    let n = master_stream.read(&mut buf).await?;
    if n == 0 {
        println!("Connection closed by Master Node");
        return Ok(());
    }

    let response: NodeMessage = serde_json::from_slice(&buf[..n])?;
    if let NodeMessage::ConnectedNodes(nodes) = response {
        println!("Currently connected nodes in the network: {:?}", nodes);
    }

    let (read_half, write_half) = master_stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));

    // Heartbeat loop
    let hb_writer = Arc::clone(&writer);
    tokio::spawn(async move {
        loop {
            if send_message_generic(&hb_writer, NodeMessage::Heartbeat).await.is_err() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    let mut reader = read_half;
    loop {
        let mut buf = vec![0; 1024];
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            println!("Master disconnected");
            break;
        }
        let msg: NodeMessage = serde_json::from_slice(&buf[..n])?;
        match msg {
            NodeMessage::AssignTask(id, data) => {
                println!("Processing task {} with data {}", id, data);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                send_message_generic(&writer, NodeMessage::TaskResult(id, "done".to_string())).await?;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn send_message(stream: &mut TcpStream, msg: NodeMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
    let msg_data = serde_json::to_vec(&msg)?;
    stream.write_all(&msg_data).await?;
    println!("Sent message to Master Node: {:?}", msg);
    Ok(())
}

async fn send_message_generic(writer: &tokio::sync::Mutex<OwnedWriteHalf>, msg: NodeMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut w = writer.lock().await;
    let data = serde_json::to_vec(&msg)?;
    w.write_all(&data).await?;
    Ok(())
}
