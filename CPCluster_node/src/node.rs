use crate::{
    disk_store::DiskStore, execute_task, internet_ports::InternetPorts, memory_store::MemoryStore,
};
use cpcluster_common::config::Config;
use cpcluster_common::{
    is_local_ip, read_length_prefixed, write_length_prefixed, JoinInfo, NodeMessage,
};
use log::{error, info, warn};
use reqwest::Client;
use rustls_native_certs as native_certs;
use std::{collections::HashMap, error::Error, fs, io, sync::Arc};
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{rustls, TlsConnector};

pub trait ReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> ReadWrite for T {}

pub async fn run(join_info: JoinInfo, config: Config) -> Result<(), Box<dyn Error + Send + Sync>> {
    let stream = connect_to_master(&join_info, &config).await?;
    heartbeat_loop(stream, join_info, config).await
}

pub async fn connect_to_master(
    join_info: &JoinInfo,
    config: &Config,
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    let mut attempt = 0;
    let mut delay = std::time::Duration::from_millis(100);
    loop {
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
                    info!("Connected to Master Node at {}", addr);
                    write_length_prefixed(&mut s, join_info.token.as_bytes()).await?;
                    let auth_response = read_length_prefixed(&mut s).await?;
                    if auth_response == b"Invalid token" {
                        error!("Authentication failed");
                        continue;
                    }
                    send_message(&mut s, NodeMessage::RegisterRole(config.role.clone())).await?;
                    send_message(&mut s, NodeMessage::GetConnectedNodes).await?;
                    if let Ok(buf) = read_length_prefixed(&mut s).await {
                        if let Ok(NodeMessage::ConnectedNodes(nodes)) = serde_json::from_slice(&buf)
                        {
                            info!("Currently connected nodes in the network: {:?}", nodes);
                            if let Some(peer) = nodes.first() {
                                send_message(&mut s, NodeMessage::RequestConnection(peer.clone()))
                                    .await?;
                            }
                        }
                    }
                    return Ok(s);
                }
                Err(e) => warn!("Failed to connect to {}: {}", addr, e),
            }
        }
        attempt += 1;
        if attempt > config.max_retries {
            break;
        }
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
    Err(Box::new(std::io::Error::other(
        "Unable to connect to any master node",
    )))
}

async fn heartbeat_loop(
    mut stream: Box<dyn ReadWrite + Unpin + Send>,
    join_info: JoinInfo,
    config: Config,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let open_tasks: Arc<Mutex<HashMap<String, NodeMessage>>> = Arc::new(Mutex::new(HashMap::new()));
    let memory = MemoryStore::new();
    let disk_store = if config.role == cpcluster_common::NodeRole::Disk {
        Some(DiskStore::new(
            std::path::PathBuf::from(&config.storage_dir),
            config.disk_space_mb,
        ))
    } else {
        None
    };
    let internet_ports = if config.role == cpcluster_common::NodeRole::Internet {
        if let Some(ports) = &config.internet_ports {
            Some(Arc::new(InternetPorts::bind(ports).await))
        } else {
            Some(Arc::new(InternetPorts::bind(&[]).await))
        }
    } else {
        None
    };
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
                            let storage = config.storage_dir.clone();
                            let mem = memory.clone();
                            let ds = disk_store.clone();
                            let internet = internet_ports.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(
                                    target,
                                    port,
                                    tasks,
                                    ca_path.as_deref(),
                                    ca_cert.as_deref(),
                                    &storage,
                                    mem,
                                    ds,
                                    internet.as_deref(),
                                ).await {
                                    error!("Direct connection error: {}", e);
                                }
                            });
                        }
                        NodeMessage::HeartbeatAck => {
                            info!("Received heartbeat acknowledgement from master");
                        }
                        NodeMessage::AssignTask { id, task } => {
                            let result = execute_task(
                                task,
                                &http_client,
                                &config.storage_dir,
                                &memory,
                                disk_store.as_ref(),
                                internet_ports.as_deref(),
                            ).await;
                            let msg = NodeMessage::TaskResult { id: id.clone(), result };
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
        let mut pem = std::io::BufReader::new(fs::File::open(path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("failed to open CA certificate '{}': {}", path, e),
            )
        })?);
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

pub async fn reconnect(
    join_info: &JoinInfo,
    config: &Config,
    open_tasks: &Arc<Mutex<HashMap<String, NodeMessage>>>,
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    let mut attempt = 0;
    let mut delay = std::time::Duration::from_millis(100);
    loop {
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
                    write_length_prefixed(&mut s, join_info.token.as_bytes()).await?;
                    let auth_resp = read_length_prefixed(&mut s).await?;
                    if auth_resp == b"Invalid token" {
                        error!("Authentication failed during reconnect");
                        continue;
                    }
                    if let Err(e) = send_message(&mut s, NodeMessage::GetConnectedNodes).await {
                        warn!("Failed to request connected nodes: {}", e);
                    } else if let Ok(buf) = read_length_prefixed(&mut s).await {
                        if let Ok(NodeMessage::ConnectedNodes(nodes)) = serde_json::from_slice(&buf)
                        {
                            info!("Currently connected nodes in the network: {:?}", nodes);
                        }
                    }
                    let tasks = open_tasks.lock().await.clone();
                    for task in tasks.values() {
                        if let Err(e) = send_message(&mut s, task.clone()).await {
                            warn!("Failed to resend task: {}", e);
                        }
                    }
                    return Ok(s);
                }
                Err(e) => warn!("Failed to connect to {}: {}", addr, e),
            }
        }
        attempt += 1;
        if attempt > config.max_retries {
            break;
        }
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
    Err(Box::new(std::io::Error::other(
        "Unable to reconnect to any master node",
    )))
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    target: String,
    port: u16,
    tasks: Arc<Mutex<HashMap<String, NodeMessage>>>,
    ca_path: Option<&str>,
    ca_cert: Option<&str>,
    storage_dir: &str,
    memory: MemoryStore,
    disk: Option<DiskStore>,
    internet_ports: Option<&InternetPorts>,
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
            let result = execute_task(
                task,
                &client,
                storage_dir,
                &memory,
                disk.as_ref(),
                internet_ports,
            )
            .await;
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
