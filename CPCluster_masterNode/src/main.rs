use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::error::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use rustls::{Certificate, PrivateKey, ServerConfig};
use rustls_pemfile::{certs, pkcs8_private_keys};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::process::Command;
use std::path::Path;
use std::io::BufReader;

#[derive(Debug, Clone)]
struct MasterNode {
    tasks: Arc<Mutex<Vec<Task>>>,
    connected_nodes: Arc<Mutex<HashMap<String, TcpStream>>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Task {
    id: String,
    data: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cert_path = "certs/cert.pem";
    let key_path = "certs/key.pem";

    // Zertifikate überprüfen und ggf. generieren
    check_and_generate_certs(cert_path, key_path)?;

    // TLS-Serverkonfiguration laden
    let tls_server_config = load_tls_server_config(cert_path, key_path)?;
    let acceptor = TlsAcceptor::from(tls_server_config.clone());

    // TCP-Listener auf Port 55000
    let listener = TcpListener::bind("0.0.0.0:55000").await?;
    println!("Master Node listening on port 55000");

    let master_node = Arc::new(MasterNode {
        tasks: Arc::new(Mutex::new(Vec::new())),
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
    });

    loop {
        let (socket, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let master_node = Arc::clone(&master_node);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(acceptor, socket, master_node).await {
                eprintln!("Connection error: {:?}", e);
            }
        });
    }
}

async fn handle_connection(
    acceptor: TlsAcceptor,
    socket: TcpStream,
    master_node: Arc<MasterNode>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut tls_stream = acceptor.accept(socket).await?;

    let mut buf = [0; 1024];
    let n = tls_stream.read(&mut buf).await?;

    let command: &str = std::str::from_utf8(&buf[..n])?;
    match command {
        "SEND_TASK" => {
            let task = Task {
                id: uuid::Uuid::new_v4().to_string(),
                data: vec![],
            };
            master_node.tasks.lock().unwrap().push(task.clone());
            let response = serde_json::to_vec(&task)?;
            tls_stream.write_all(&response).await?;
        }
        "GET_TASK" => {
            let tasks = master_node.tasks.lock().unwrap().clone();
            let response = serde_json::to_vec(&tasks)?;
            tls_stream.write_all(&response).await?;
        }
        "GET_NODES" => {
            let nodes = master_node.connected_nodes.lock().unwrap().keys().cloned().collect::<Vec<_>>();
            let response = serde_json::to_vec(&nodes)?;
            tls_stream.write_all(&response).await?;
        }
        _ => eprintln!("Unknown command: {}", command),
    }

    Ok(())
}

fn load_tls_server_config(cert_path: &str, key_path: &str) -> Result<Arc<ServerConfig>, Box<dyn Error + Send + Sync>> {
    let cert_file = &mut BufReader::new(File::open(cert_path)?);
    let certs = certs(cert_file)?.into_iter().map(Certificate).collect();

    let key_file = &mut BufReader::new(File::open(key_path)?);
    let mut keys: Vec<PrivateKey> = pkcs8_private_keys(key_file)?.into_iter().map(PrivateKey).collect();

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certs, keys.remove(0))?;

    Ok(Arc::new(config))
}


fn check_and_generate_certs(cert_path: &str, key_path: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    if !Path::new(cert_path).exists() || !Path::new(key_path).exists() {
        println!("Certificates not found, generating new ones...");
        std::fs::create_dir_all("certs")?;

        let output = Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "rsa:4096", "-keyout", key_path, "-out", cert_path,
                "-days", "365", "-nodes", "-subj", "/CN=localhost"
            ])
            .output()?;

        if !output.status.success() {
            return Err(format!("Failed to generate certificates: {:?}", output.stderr).into());
        }
        println!("Certificates generated successfully.");
    }
    Ok(())
}
