use cpcluster_common::{isLocalIp, JoinInfo, NodeMessage};
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

    let masterAddr = format!("{}:{}", joinInfo.ip, joinInfo.port);
    let mut stream: Box<dyn ReadWrite + Unpin + Send> = if isLocalIp(&joinInfo.ip) {
        let tcp = TcpStream::connect(&masterAddr).await?;
        Box::new(tcp)
    } else {
        let tcp = TcpStream::connect(&masterAddr).await?;
        let config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let serverName = rustls::ServerName::try_from(joinInfo.ip.as_str())?;
        let tls = connector.connect(serverName, tcp).await?;
        Box::new(tls)
    };
    println!("Connected to Master Node at {}", masterAddr);

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

    // Here you can proceed with additional actions, such as requesting a connection to another node.

    // Disconnect from the Master Node
    let disconnectMsg = NodeMessage::Disconnect;
    sendMessage(&mut stream, disconnectMsg).await?;
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
