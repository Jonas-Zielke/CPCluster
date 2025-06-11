use rustls::{ClientConfig, RootCertStore, ServerName};
use rustls_pemfile::certs;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc as StdArc;
use std::{error::Error, fs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{client::TlsStream, TlsConnector};

// Informationen, die zum Herstellen der Verbindung mit dem Master benötigt werden
#[derive(Serialize, Deserialize)]
struct JoinInfo {
    token: String,
    ip: String,
    port: String,
}

// Nachrichten zwischen Node und Master
#[derive(Serialize, Deserialize, Debug)]
enum NodeMessage {
    RequestConnection(String),  // Verbindung zu einer Node anfordern
    ConnectionInfo(String, u16), // Verbindungsdetails erhalten
    GetConnectedNodes,          // Liste der verbundenen Nodes anfragen
    ConnectedNodes(Vec<String>),
    Disconnect,                 // Verbindung sauber beenden
}

#[tokio::main]
/// Startet die Node, verbindet sich zum Master und führt das Basisprotokoll aus.
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let join_info = fs::read_to_string("join.json")?;
    let join_info: JoinInfo = serde_json::from_str(&join_info)?;

    let master_addr = format!("{}:{}", join_info.ip, join_info.port);
    let tcp_stream = TcpStream::connect(&master_addr).await?;
    println!("Connected to Master Node at {}", master_addr);

    let tls_config = load_tls_config("certs/cert.pem")?;
    let connector = TlsConnector::from(tls_config);
    let server_name = ServerName::try_from(join_info.ip.as_str())?;
    let mut master_stream = connector.connect(server_name, tcp_stream).await?;

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

/// Hilfsfunktion zum Senden serialisierter Nachrichten über einen TLS-Stream.
async fn send_message(
    stream: &mut TlsStream<TcpStream>,
    msg: NodeMessage,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let msg_data = serde_json::to_vec(&msg)?;
    stream.write_all(&msg_data).await?;
    println!("Sent message to Master Node: {:?}", msg);
    Ok(())
}

/// Lädt das Zertifikat des Masters und baut damit die TLS-Clientkonfiguration auf.
fn load_tls_config(cert_path: &str) -> Result<StdArc<ClientConfig>, Box<dyn Error + Send + Sync>> {
    let mut cert_file = BufReader::new(File::open(cert_path)?);
    let certs = certs(&mut cert_file)?;
    let mut root_store = RootCertStore::empty();
    root_store.add_parsable_certificates(&certs);

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(StdArc::new(config))
}
