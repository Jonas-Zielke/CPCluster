use cpcluster_common::{JoinInfo, NodeMessage};
use std::{error::Error, fs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let joinInfo = fs::read_to_string("join.json")?;
    let joinInfo: JoinInfo = serde_json::from_str(&joinInfo)?;

    let masterAddr = format!("{}:{}", joinInfo.ip, joinInfo.port);
    let mut masterStream = TcpStream::connect(&masterAddr).await?;
    println!("Connected to Master Node at {}", masterAddr);

    // Send only the token for authentication
    masterStream.write_all(joinInfo.token.as_bytes()).await?;
    println!("Token sent for authentication");

    // Read and verify authentication response from the master node
    let mut authResponse = vec![0; 1024];
    let n = masterStream.read(&mut authResponse).await?;
    if n == 0 || &authResponse[..n] == b"Invalid token" {
        println!("Authentication failed");
        return Ok(());
    }
    println!("Authentication successful");

    // Request to get the list of currently connected nodes
    let getNodesRequest = NodeMessage::GetConnectedNodes;
    sendMessage(&mut masterStream, getNodesRequest).await?;

    // Receive and display the list of connected nodes
    let mut buf = vec![0; 1024];
    let n = masterStream.read(&mut buf).await?;
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
    let disconnectMsg = NodeMessage::Disconnect;
    sendMessage(&mut masterStream, disconnectMsg).await?;
    println!("Disconnected from Master Node");

    Ok(())
}

async fn sendMessage(
    stream: &mut TcpStream,
    msg: NodeMessage,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let msg_data = serde_json::to_vec(&msg)?;
    stream.write_all(&msg_data).await?;
    println!("Sent message to Master Node: {:?}", msg);
    Ok(())
}
