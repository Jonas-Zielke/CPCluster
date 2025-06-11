use cpcluster_common::{isLocalIp, JoinInfo, NodeMessage};
use cpcluster_common::config::Config;
use std::{error::Error, fs, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{rustls, TlsConnector};

struct NoCertificateVerification;

trait ReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite + ?Sized> ReadWrite for T {}

impl rustls::client::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let joinInfo = fs::read_to_string("join.json")?;
    let joinInfo: JoinInfo = serde_json::from_str(&joinInfo)?;

    let config = Config::load("config.json").unwrap_or_default();

    let mut stream: Option<Box<dyn ReadWrite + Unpin + Send>> = None;
    let mut open_tasks: Vec<NodeMessage> = Vec::new();
    for addr in &config.master_addresses {
        match connect(addr, isLocalIp(&joinInfo.ip), &joinInfo.ip).await {
            Ok(s) => {
                println!("Connected to Master Node at {}", addr);
                stream = Some(s);
                break;
            }
            Err(e) => {
                println!("Failed to connect to {}: {}", addr, e);
            }
        }
    }

    let mut stream = match stream {
        Some(s) => s,
        None => {
            println!("Unable to connect to any master node");
            return Ok(());
        }
    };

    // Send only the token for authentication
    stream.write_all(joinInfo.token.as_bytes()).await?;
    println!("Token sent for authentication");

    // Read and verify authentication response from the master node
    let mut authResponse = vec![0; 1024];
    let n = stream.read(&mut authResponse).await?;
    if n == 0 || &authResponse[..n] == b"Invalid token" {
        println!("Authentication failed");
        return Ok(());
    }
    println!("Authentication successful");

    // Request to get the list of currently connected nodes
    let getNodesRequest = NodeMessage::GetConnectedNodes;
    sendMessage(&mut stream, getNodesRequest).await?;

    // Receive and display the list of connected nodes
    let mut buf = vec![0; 1024];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        println!("Connection closed by Master Node");
        return Ok(());
    }

    let response: NodeMessage = serde_json::from_slice(&buf[..n])?;
    if let NodeMessage::ConnectedNodes(nodes) = response {
        println!("Currently connected nodes in the network: {:?}", nodes);
    }

    // Periodic heartbeat loop
    loop {
        if let Err(e) = sendMessage(&mut stream, NodeMessage::Heartbeat).await {
            println!("Heartbeat failed: {}", e);
            match reconnect(&joinInfo, &config, &open_tasks).await {
                Ok(s) => {
                    stream = s;
                    continue;
                }
                Err(err) => {
                    println!("Reconnect failed: {}", err);
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(config.failover_timeout_ms)).await;
    }

    // Attempt graceful disconnect if still connected
    let _ = sendMessage(&mut stream, NodeMessage::Disconnect).await;
    println!("Disconnected from Master Node");

    Ok(())
}

async fn sendMessage<S>(
    stream: &mut S,
    msg: NodeMessage,
) -> Result<(), Box<dyn Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    let msg_data = serde_json::to_vec(&msg)?;
    stream.write_all(&msg_data).await?;
    println!("Sent message to Master Node: {:?}", msg);
    Ok(())
}

async fn connect(
    addr: &str,
    local: bool,
    ip: &str,
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    if local {
        let tcp = TcpStream::connect(addr).await?;
        Ok(Box::new(tcp))
    } else {
        let tcp = TcpStream::connect(addr).await?;
        let config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let serverName = rustls::ServerName::try_from(ip)?;
        let tls = connector.connect(serverName, tcp).await?;
        Ok(Box::new(tls))
    }
}

async fn reconnect(
    join_info: &JoinInfo,
    config: &Config,
    open_tasks: &[NodeMessage],
) -> Result<Box<dyn ReadWrite + Unpin + Send>, Box<dyn Error + Send + Sync>> {
    for addr in &config.master_addresses {
        match connect(addr, isLocalIp(&join_info.ip), &join_info.ip).await {
            Ok(mut s) => {
                println!("Reconnected to Master Node at {}", addr);

                // re-authenticate
                s.write_all(join_info.token.as_bytes()).await?;
                let mut auth_resp = vec![0; 1024];
                let n = s.read(&mut auth_resp).await?;
                if n == 0 || &auth_resp[..n] == b"Invalid token" {
                    println!("Authentication failed during reconnect");
                    continue;
                }
                println!("Re-authentication successful");

                // request nodes again
                if let Err(e) = sendMessage(&mut s, NodeMessage::GetConnectedNodes).await {
                    println!("Failed to request connected nodes: {}", e);
                } else {
                    let mut buf = vec![0; 1024];
                    if let Ok(n) = s.read(&mut buf).await {
                        if n > 0 {
                            if let Ok(NodeMessage::ConnectedNodes(nodes)) = serde_json::from_slice(&buf[..n]) {
                                println!("Currently connected nodes in the network: {:?}", nodes);
                            }
                        }
                    }
                }

                // resend open tasks
                for task in open_tasks {
                    if let Err(e) = sendMessage(&mut s, task.clone()).await {
                        println!("Failed to resend task: {}", e);
                    }
                }

                return Ok(s);
            }
            Err(e) => {
                println!("Failed to connect to {}: {}", addr, e);
            }
        }
    }

    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        "Unable to reconnect to any master node",
    )))
}
