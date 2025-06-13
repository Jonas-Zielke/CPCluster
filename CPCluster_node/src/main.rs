use cpcluster_common::config::Config;
use cpcluster_common::{
    is_local_ip, read_length_prefixed, write_length_prefixed, JoinInfo, NodeMessage, Task,
    TaskResult,
};
use log::{error, info, warn};
use meval::eval_str;
use reqwest::Client;
use rustls_native_certs as native_certs;
use std::{collections::HashMap, error::Error, fs, sync::Arc};
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{rustls, TlsConnector};

trait ReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> ReadWrite for T {}

async fn execute_task(task: Task, client: &Client) -> TaskResult {
    match task {
        Task::Compute { expression } => match eval_str(&expression) {
            Ok(v) => TaskResult::Number(v),
            Err(e) => TaskResult::Error(e.to_string()),
        },
        Task::HttpRequest { url } => match client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => TaskResult::Response(text),
                Err(e) => TaskResult::Error(e.to_string()),
            },
            Err(e) => TaskResult::Error(e.to_string()),
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let join_info = fs::read_to_string("join.json")?;
    let join_info: JoinInfo = serde_json::from_str(&join_info)?;

    let config = Config::load("config.json").unwrap_or_default();

    let mut stream: Option<Box<dyn ReadWrite + Unpin + Send>> = None;
    let open_tasks: Arc<Mutex<HashMap<String, NodeMessage>>> = Arc::new(Mutex::new(HashMap::new()));
    for addr in &config.master_addresses {
        match connect(
            addr,
            is_local_ip(&join_info.ip),
            &join_info.ip,
            config.ca_cert_path.as_deref(),
            config.ca_cert.as_deref(),
        )
        .await
        {
            Ok(s) => {
                info!("Connected to Master Node at {}", addr);
                stream = Some(s);
                break;
            }
            Err(e) => {
                warn!("Failed to connect to {}: {}", addr, e);
            }
        }
    }

    let mut stream = match stream {
        Some(s) => s,
        None => {
            error!("Unable to connect to any master node");
            return Ok(());
        }
    };

    // Send only the token for authentication
    write_length_prefixed(&mut stream, join_info.token.as_bytes()).await?;
    info!("Token sent for authentication");

    // Read and verify authentication response from the master node
    let auth_response = read_length_prefixed(&mut stream).await?;
    if auth_response == b"Invalid token" {
        error!("Authentication failed");
        return Ok(());
    }
    info!("Authentication successful");

    // Request to get the list of currently connected nodes
    let get_nodes_request = NodeMessage::GetConnectedNodes;
    send_message(&mut stream, get_nodes_request).await?;

    // Receive and display the list of connected nodes
    let buf = read_length_prefixed(&mut stream).await?;

    let response: NodeMessage = serde_json::from_slice(&buf)?;
    if let NodeMessage::ConnectedNodes(nodes) = response {
        info!("Currently connected nodes in the network: {:?}", nodes);
        if let Some(peer) = nodes.first() {
            send_message(&mut stream, NodeMessage::RequestConnection(peer.clone())).await?;
        }
    }

    // Heartbeat and message loop
    let http_client = Client::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(
        config.failover_timeout_ms / 2,
    ));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = send_message(&mut stream, NodeMessage::Heartbeat).await {
                    warn!("Heartbeat failed: {}", e);
                    match reconnect(&join_info, &config, &open_tasks).await {
                        Ok(s) => { stream = s; continue; }
                        Err(err) => { error!("Reconnect failed: {}", err); break; }
                    }
                }
            }
            res = read_length_prefixed(&mut stream) => {
                let buf = match res {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Read failed: {}", e);
                        match reconnect(&join_info, &config, &open_tasks).await {
                            Ok(s) => { stream = s; continue; }
                            Err(err) => { error!("Reconnect failed: {}", err); break; }
                        }
                    }
                };
                if let Ok(msg) = serde_json::from_slice::<NodeMessage>(&buf) {
                    match msg {
                        NodeMessage::ConnectionInfo(target, port) => {
                            let tasks = open_tasks.clone();
                            let ca_path = config.ca_cert_path.clone();
                            let ca_cert = config.ca_cert.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(
                                    target,
                                    port,
                                    tasks,
                                    ca_path.as_deref(),
                                    ca_cert.as_deref(),
                                ).await {
                                    error!("Direct connection error: {}", e);
                                }
                            });
                        }
                        NodeMessage::HeartbeatAck => {
                            info!("Received heartbeat acknowledgement from master");
                        }
                        NodeMessage::AssignTask { id, task } => {
                            let result = execute_task(task, &http_client).await;
                            let msg = NodeMessage::TaskResult {
                                id: id.clone(),
                                result,
                            };
                            open_tasks.lock().await.insert(id.clone(), msg.clone());
                            if let Err(e) = send_message(&mut stream, msg.clone()).await {
                                warn!("Failed to send task result: {}", e);
                            }
                            open_tasks.lock().await.remove(&id);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Attempt graceful disconnect if still connected
    let _ = send_message(&mut stream, NodeMessage::Disconnect).await;
    info!("Disconnected from Master Node");

    Ok(())
}

async fn send_message<S>(
    stream: &mut S,
    msg: NodeMessage,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    let msg_data = serde_json::to_vec(&msg)?;
    write_length_prefixed(stream, &msg_data).await?;
    info!("Sent message to Master Node: {:?}", msg);
    Ok(())
}

async fn connect(
    addr: &str,
    local: bool,
    ip: &str,
    ca_path: Option<&str>,
    ca_cert: Option<&str>,
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    if local {
        let tcp = TcpStream::connect(addr).await?;
        Ok(Box::new(tcp))
    } else {
        let tcp = TcpStream::connect(addr).await?;
        let config = build_tls_config(ca_path, ca_cert)?;
        let connector = TlsConnector::from(Arc::new(config));
        let server_name = rustls::ServerName::try_from(ip)?;
        let tls = connector.connect(server_name, tcp).await?;
        Ok(Box::new(tls))
    }
}

fn build_tls_config(
    ca_path: Option<&str>,
    ca_cert: Option<&str>,
) -> Result<rustls::ClientConfig, Box<dyn Error + Send + Sync>> {
    let mut root_store = rustls::RootCertStore::empty();
    if let Some(pem_str) = ca_cert {
        let mut pem = std::io::BufReader::new(pem_str.as_bytes());
        for cert in rustls_pemfile::certs(&mut pem)? {
            root_store.add(&rustls::Certificate(cert)).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{:?}", e))
            })?;
        }
    } else if let Some(path) = ca_path {
        let mut pem = std::io::BufReader::new(fs::File::open(path)?);
        for cert in rustls_pemfile::certs(&mut pem)? {
            root_store.add(&rustls::Certificate(cert)).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{:?}", e))
            })?;
        }
    } else {
        let native = native_certs::load_native_certs();
        if !native.errors.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "could not load platform certs: {:?}",
                native.errors
            ))));
        }
        for cert in native.certs {
            root_store
                .add(&rustls::Certificate(cert.as_ref().to_vec()))
                .map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{:?}", e))
                })?;
        }
    }
    Ok(rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

async fn reconnect(
    join_info: &JoinInfo,
    config: &Config,
    open_tasks: &Arc<Mutex<HashMap<String, NodeMessage>>>,
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    for addr in &config.master_addresses {
        match connect(
            addr,
            is_local_ip(&join_info.ip),
            &join_info.ip,
            config.ca_cert_path.as_deref(),
            config.ca_cert.as_deref(),
        )
        .await
        {
            Ok(mut s) => {
                info!("Reconnected to Master Node at {}", addr);

                // re-authenticate
                write_length_prefixed(&mut s, join_info.token.as_bytes()).await?;
                let auth_resp = read_length_prefixed(&mut s).await?;
                if auth_resp == b"Invalid token" {
                    error!("Authentication failed during reconnect");
                    continue;
                }
                info!("Re-authentication successful");

                // request nodes again
                if let Err(e) = send_message(&mut s, NodeMessage::GetConnectedNodes).await {
                    warn!("Failed to request connected nodes: {}", e);
                } else if let Ok(buf) = read_length_prefixed(&mut s).await {
                    if let Ok(NodeMessage::ConnectedNodes(nodes)) = serde_json::from_slice(&buf) {
                        info!("Currently connected nodes in the network: {:?}", nodes);
                    }
                }

                // resend open tasks
                let tasks = open_tasks.lock().await.clone();
                for task in tasks.values() {
                    if let Err(e) = send_message(&mut s, task.clone()).await {
                        warn!("Failed to resend task: {}", e);
                    }
                }

                return Ok(s);
            }
            Err(e) => {
                warn!("Failed to connect to {}: {}", addr, e);
            }
        }
    }

    Err(Box::new(std::io::Error::other(
        "Unable to reconnect to any master node",
    )))
}

async fn handle_connection(
    target: String,
    port: u16,
    tasks: Arc<Mutex<HashMap<String, NodeMessage>>>,
    ca_path: Option<&str>,
    ca_cert: Option<&str>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let addr = format!("{}:{}", target, port);
    let use_tls = !is_local_ip(&target);
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    let connect_fut = TcpStream::connect(&addr);
    let mut stream: Box<dyn ReadWrite + Unpin + Send>;
    tokio::select! {
        Ok(sock) = connect_fut => {
            if use_tls {
                let config = build_tls_config(ca_path, ca_cert)?;
                let connector = TlsConnector::from(Arc::new(config));
                let server_name = rustls::ServerName::try_from(target.as_str())?;
                let tls = connector.connect(server_name, sock).await?;
                stream = Box::new(tls);
            } else {
                stream = Box::new(sock);
            }
        }
        Ok((sock, _)) = listener.accept() => {
            if use_tls {
                let config = build_tls_config(ca_path, ca_cert)?;
                let connector = TlsConnector::from(Arc::new(config));
                let server_name = rustls::ServerName::try_from(target.as_str())?;
                let tls = connector.connect(server_name, sock).await?;
                stream = Box::new(tls);
            } else {
                stream = Box::new(sock);
            }
        }
    }

    let client = Client::new();
    loop {
        let buf = match read_length_prefixed(&mut stream).await {
            Ok(b) => b,
            Err(_) => break,
        };
        if let Ok(NodeMessage::AssignTask { id, task }) = serde_json::from_slice(&buf) {
            let result = execute_task(task, &client).await;
            let msg = NodeMessage::TaskResult {
                id: id.clone(),
                result,
            };
            tasks.lock().await.insert(id.clone(), msg.clone());
            send_message(&mut stream, msg.clone()).await?;
            tasks.lock().await.remove(&id);
        }
    }
    Ok(())
}
