use serde::{Deserialize, Serialize};

pub mod config;
pub use config::Config;

#[derive(Serialize, Deserialize)]
pub struct JoinInfo {
    pub token: String,
    pub ip: String,
    pub port: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum NodeMessage {
    RequestConnection(String),
    ConnectionInfo(String, u16),
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    Disconnect,
    Heartbeat,
}

/// Determine if an IP address is part of a private local network.
pub fn isLocalIp(ip: &str) -> bool {
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
    use super::isLocalIp;

    #[test]
    fn detects_private_ipv4() {
        assert!(isLocalIp("10.1.2.3"));
        assert!(isLocalIp("172.16.0.1"));
        assert!(isLocalIp("192.168.5.6"));
        assert!(isLocalIp("127.0.0.1"));
    }

    #[test]
    fn detects_public_ipv4() {
        assert!(!isLocalIp("8.8.8.8"));
        assert!(!isLocalIp("1.2.3.4"));
    }

    #[test]
    fn detects_ipv6() {
        assert!(isLocalIp("::1"));
        assert!(isLocalIp("fc00::1"));
        assert!(!isLocalIp("2001:4860:4860::8888"));
    }
}
