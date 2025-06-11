use std::{error::Error, fs};
use cpcluster_common::{JoinInfo, NodeMessage};
use tokio::{net::TcpStream, io::{AsyncReadExt, AsyncWriteExt}};

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

    // Here you can proceed with additional actions, such as requesting a connection to another node.

    // Disconnect from the Master Node
    let disconnect_msg = NodeMessage::Disconnect;
    send_message(&mut master_stream, disconnect_msg).await?;
    println!("Disconnected from Master Node");

    Ok(())
}

async fn send_message(stream: &mut TcpStream, msg: NodeMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
    let msg_data = serde_json::to_vec(&msg)?;
    stream.write_all(&msg_data).await?;
    println!("Sent message to Master Node: {:?}", msg);
    Ok(())
}
