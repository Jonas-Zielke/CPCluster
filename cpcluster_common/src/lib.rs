//! Shared types for CPCluster nodes, clients, and utilities.
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

pub mod config;
pub use config::Config;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Serialize, Deserialize)]
pub struct JoinInfo {
    pub token: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Task {
    Compute {
        expression: Cow<'static, str>,
    },
    HttpRequest {
        url: String,
    },
    /// Send data over a TCP connection and return the response bytes.
    TcpIo {
        addr: String,
        port: u16,
        data: Vec<u8>,
    },
    /// Send a UDP datagram and optionally receive a response.
    UdpIo {
        addr: String,
        port: u16,
        data: Vec<u8>,
    },
    /// Perform a complex mathematical computation.
    ComplexMath {
        expression: Cow<'static, str>,
    },
    /// Store arbitrary bytes in memory under the given key.
    StoreData {
        key: String,
        data: Vec<u8>,
    },
    /// Retrieve previously stored data by key.
    RetrieveData {
        key: String,
    },
    /// Write bytes to disk at the given path.
    DiskWrite {
        path: String,
        data: Vec<u8>,
    },
    /// Read bytes from disk at the given path.
    DiskRead {
        path: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TaskResult {
    Number(f64),
    Response(String),
    /// Raw bytes returned from an operation (e.g. TCP/UDP I/O or disk read).
    Bytes(Vec<u8>),
    /// Data was successfully stored in memory or on disk.
    Stored,
    Error(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NodeMessage {
    RequestConnection(String),
    ConnectionInfo(String, u16),
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    SubmitTask { id: String, task: Task },
    GetTaskResult(String),
    Disconnect,
    Heartbeat,
    HeartbeatAck,
    AssignTask { id: String, task: Task },
    TaskResult { id: String, result: TaskResult },
    TaskAccepted(String),
    DirectMessage(String),
}

/// Write a length-prefixed binary message to the provided async writer.
pub async fn write_length_prefixed<S>(
    stream: &mut S,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(data).await?;
    Ok(())
}

/// Read a length-prefixed binary message from the provided async reader.
pub async fn read_length_prefixed<S>(
    stream: &mut S,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Determine if an IP address is part of a private local network.
pub fn is_local_ip(ip: &str) -> bool {
    if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
        match addr {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                // 10.0.0.0/8
                if octets[0] == 10 {
                    return true;
                }
                // 172.16.0.0/12
                if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                    return true;
                }
                // 192.168.0.0/16
                if octets[0] == 192 && octets[1] == 168 {
                    return true;
                }
                // 127.0.0.0/8 loopback
                if octets[0] == 127 {
                    return true;
                }
            }
            std::net::IpAddr::V6(v6) => {
                // localhost ::1
                if v6.is_loopback() {
                    return true;
                }
                // Unique local addresses fc00::/7
                if v6.segments()[0] & 0xfe00 == 0xfc00 {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_local_ip;

    #[test]
    fn detects_private_ipv4() {
        assert!(is_local_ip("10.1.2.3"));
        assert!(is_local_ip("172.16.0.1"));
        assert!(is_local_ip("192.168.5.6"));
        assert!(is_local_ip("127.0.0.1"));
    }

    #[test]
    fn detects_public_ipv4() {
        assert!(!is_local_ip("8.8.8.8"));
        assert!(!is_local_ip("1.2.3.4"));
    }

    #[test]
    fn detects_ipv6() {
        assert!(is_local_ip("::1"));
        assert!(is_local_ip("fc00::1"));
        assert!(!is_local_ip("2001:4860:4860::8888"));
    }
}