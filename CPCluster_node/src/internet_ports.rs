use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};

pub struct InternetPorts {
    tcp: Vec<Arc<TcpListener>>,
    udp: Vec<Arc<UdpSocket>>,
    tcp_idx: AtomicUsize,
    udp_idx: AtomicUsize,
}

impl Clone for InternetPorts {
    fn clone(&self) -> Self {
        Self {
            tcp: self.tcp.clone(),
            udp: self.udp.clone(),
            tcp_idx: AtomicUsize::new(self.tcp_idx.load(Ordering::Relaxed)),
            udp_idx: AtomicUsize::new(self.udp_idx.load(Ordering::Relaxed)),
        }
    }
}

impl InternetPorts {
    pub async fn bind(ports: &[u16]) -> Self {
        let mut tcp = Vec::new();
        let mut udp = Vec::new();
        for p in ports {
            if let Ok(listener) = TcpListener::bind(("0.0.0.0", *p)).await {
                tcp.push(Arc::new(listener));
            }
            if let Ok(sock) = UdpSocket::bind(("0.0.0.0", *p)).await {
                udp.push(Arc::new(sock));
            }
        }
        Self {
            tcp,
            udp,
            tcp_idx: AtomicUsize::new(0),
            udp_idx: AtomicUsize::new(0),
        }
    }

    pub fn tcp_port(&self) -> Option<u16> {
        if self.tcp.is_empty() {
            None
        } else {
            let idx = self.tcp_idx.fetch_add(1, Ordering::Relaxed) % self.tcp.len();
            self.tcp[idx].local_addr().ok().map(|a| a.port())
        }
    }

    pub fn udp_socket(&self) -> Option<Arc<UdpSocket>> {
        if self.udp.is_empty() {
            None
        } else {
            let idx = self.udp_idx.fetch_add(1, Ordering::Relaxed) % self.udp.len();
            Some(self.udp[idx].clone())
        }
    }
}
