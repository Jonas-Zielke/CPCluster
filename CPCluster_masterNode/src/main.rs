use rustls::{Certificate, PrivateKey, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc as StdArc;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fs,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use uuid::Uuid;

const MIN_PORT: u16 = 55001;
const MAX_PORT: u16 = 55999;

// Informationen, die ein Node benötigt, um dem Cluster beizutreten
#[derive(Serialize, Deserialize)]
struct JoinInfo {
    token: String,
    ip: String,
    port: String,
}

// Zentrale Verwaltungsstruktur des Masters
#[derive(Debug, Clone)]
struct MasterNode {
    // speichert Node-ID und IP-Adresse
    connected_nodes: Arc<Mutex<HashMap<String, String>>>,
    // verwaltet verfügbare Ports
    available_ports: Arc<Mutex<HashSet<u16>>>,
}

// Nachrichten, die zwischen Node und Master ausgetauscht werden
#[derive(Serialize, Deserialize, Debug)]
enum NodeMessage {
    // Anfrage, sich mit einer anderen Node zu verbinden
    RequestConnection(String),
    // Verbindungsinfo: Ziel-IP und Port
    ConnectionInfo(String, u16),
    // Anforderung für die Liste verbundener Nodes
    GetConnectedNodes,
    // Antwort mit der Liste verbundener Nodes
    ConnectedNodes(Vec<String>),
    // Nachricht zum Beenden der Verbindung
    Disconnect,
}

#[tokio::main]
/// Einstiegspunkt des Master Nodes. Erstellt das Join-Token, richtet TLS ein und
/// wartet auf eingehende Verbindungen von Nodes.
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let token = generate_token();
    let ip = "127.0.0.1".to_string();
    let port = "55000".to_string();

    let cert_path = "certs/cert.pem";
    let key_path = "certs/key.pem";

    let tls_config = load_tls_config(cert_path, key_path)?;
    let acceptor = TlsAcceptor::from(tls_config);

    let join_info = JoinInfo {
        token: token.clone(),
        ip: ip.clone(),
        port: port.clone(),
    };
    fs::write("join.json", serde_json::to_string_pretty(&join_info)?)?;
    println!("Join information saved to join.json");

    let listener = TcpListener::bind(format!("{}:{}", ip, port)).await?;
    println!("Master Node listening on {}:{}", ip, port);

    let master_node = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
        available_ports: Arc::new(Mutex::new((MIN_PORT..=MAX_PORT).collect())),
    });

    loop {
        let (stream, addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let master_node = Arc::clone(&master_node);
        let token = token.clone();

        tokio::spawn(async move {
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    if let Err(e) =
                        handle_connection(tls_stream, master_node, token, addr.to_string()).await
                    {
                        eprintln!("Connection error: {:?}", e);
                    }
                }
                Err(e) => eprintln!("TLS accept error: {:?}", e),
            }
        });
    }
}

/// Bearbeitet eine einzelne TLS-Verbindung zu einer Node.
/// Validiert den Token und verarbeitet Nachrichten vom Client.
async fn handle_connection(
    mut socket: TlsStream<TcpStream>,
    master_node: Arc<MasterNode>,
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
        master_node
            .connected_nodes
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
                        // Hole die Adresse der Ziel-Node
                        let target_addr = master_node
                            .connected_nodes
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
                    release_port(&master_node, addr.clone());
                    break;
                }
                _ => println!("Unknown request"),
            }
        }

        // Entferne die Node aus der Liste der verbundenen Nodes
        master_node.connected_nodes.lock().unwrap().remove(&addr);
    } else {
        println!("Client provided an invalid token {}", received_token);
        socket.write_all(b"Invalid token").await?;
    }

    Ok(())
}

/// Erzeugt einen zufälligen Token zur Authentifizierung der Nodes.
fn generate_token() -> String {
    Uuid::new_v4().to_string()
}

/// Reserviert einen freien Port aus der Portliste.
fn allocate_port(master_node: &MasterNode) -> Option<u16> {
    let mut ports = master_node.available_ports.lock().unwrap();
    ports.iter().cloned().next().map(|port| {
        ports.remove(&port);
        port
    })
}

/// Gibt einen zuvor belegten Port wieder frei.
fn release_port(master_node: &MasterNode, addr: String) {
    let mut ports = master_node.available_ports.lock().unwrap();
    if let Some(port) = addr.split(':').nth(1).and_then(|p| p.parse::<u16>().ok()) {
        ports.insert(port);
    }
}

/// Lädt Zertifikat und privaten Schlüssel und erstellt die TLS-Konfiguration.
fn load_tls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<StdArc<ServerConfig>, Box<dyn Error + Send + Sync>> {
    let cert_file = &mut BufReader::new(File::open(cert_path)?);
    let key_file = &mut BufReader::new(File::open(key_path)?);

    let cert_chain = certs(cert_file)?.into_iter().map(Certificate).collect();
    let mut keys = pkcs8_private_keys(key_file)?
        .into_iter()
        .map(PrivateKey)
        .collect::<Vec<_>>();
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, keys.remove(0))?;

    Ok(StdArc::new(config))
}
