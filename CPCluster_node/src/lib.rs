use cpcluster_common::{Task, TaskResult};
use meval::shunting_yard::to_rpn;
use meval::tokenizer::{tokenize, Operation, Token};
use num_complex::Complex64;
pub mod disk_store;
pub mod internet_ports;
pub mod memory_store;
pub mod node;
pub use cpcluster_common::execute_task;

use disk_store::DiskStore;
use internet_ports::InternetPorts;
use memory_store::MemoryStore;
use reqwest::Client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream, UdpSocket};

fn eval_complex(expr: &str) -> Result<Complex64, String> {
    let tokens = tokenize(expr).map_err(|e| e.to_string())?;
    let rpn = to_rpn(&tokens).map_err(|e| format!("{:?}", e))?;
    let mut stack: Vec<Complex64> = Vec::new();
    for t in rpn {
        match t {
            Token::Number(n) => stack.push(Complex64::new(n, 0.0)),
            Token::Var(v) => {
                if v == "i" {
                    stack.push(Complex64::new(0.0, 1.0));
                } else {
                    return Err(format!("unknown var {}", v));
                }
            }
            Token::Binary(op) => {
                let b = stack.pop().ok_or("stack underflow")?;
                let a = stack.pop().ok_or("stack underflow")?;
                let res = match op {
                    Operation::Plus => a + b,
                    Operation::Minus => a - b,
                    Operation::Times => a * b,
                    Operation::Div => a / b,
                    Operation::Pow => a.powc(b),
                    Operation::Rem => Complex64::new(a.re % b.re, a.im),
                };
                stack.push(res);
            }
            Token::Unary(op) => {
                let a = stack.pop().ok_or("stack underflow")?;
                stack.push(match op {
                    Operation::Plus => a,
                    Operation::Minus => -a,
                    _ => return Err("unsupported unary".into()),
                });
            }
            _ => return Err("unsupported token".into()),
        }
    }
    stack.pop().ok_or("no result".into())
}

pub async fn execute_node_task(
    task: Task,
    client: &Client,
    storage_dir: &str,
    store: &MemoryStore,
    disk: Option<&DiskStore>,
    internet: Option<&InternetPorts>,
) -> TaskResult {
    match task {
        Task::Compute { expression } => match meval::eval_str(&expression) {
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
        Task::Tcp { addr, data } => {
            let connect_res = if let Some(inet) = internet.and_then(|i| i.tcp_port()) {
                let socket = TcpSocket::new_v4().map_err(|e| TaskResult::Error(e.to_string()));
                match socket {
                    Ok(s) => {
                        if s.bind(std::net::SocketAddr::from(([0, 0, 0, 0], inet)))
                            .is_ok()
                        {
                            s.connect(addr.parse().unwrap()).await
                        } else {
                            TcpStream::connect(&addr).await
                        }
                    }
                    Err(_) => TcpStream::connect(&addr).await,
                }
            } else {
                TcpStream::connect(&addr).await
            };
            match connect_res {
                Ok(mut stream) => {
                    if let Err(e) = stream.write_all(&data).await {
                        return TaskResult::Error(e.to_string());
                    }
                    let mut buf = vec![0u8; 1024];
                    match stream.read(&mut buf).await {
                        Ok(n) => TaskResult::Bytes(buf[..n].to_vec()),
                        Err(e) => TaskResult::Error(e.to_string()),
                    }
                }
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::Udp { addr, data } => {
            if let Some(sock) = internet.and_then(|i| i.udp_socket()) {
                if let Err(e) = sock.send_to(&data, &addr).await {
                    return TaskResult::Error(e.to_string());
                }
                let mut buf = vec![0u8; 1024];
                match sock.recv_from(&mut buf).await {
                    Ok((n, _)) => TaskResult::Bytes(buf[..n].to_vec()),
                    Err(e) => TaskResult::Error(e.to_string()),
                }
            } else {
                match UdpSocket::bind("0.0.0.0:0").await {
                    Ok(socket) => {
                        if let Err(e) = socket.send_to(&data, &addr).await {
                            return TaskResult::Error(e.to_string());
                        }
                        let mut buf = vec![0u8; 1024];
                        match socket.recv_from(&mut buf).await {
                            Ok((n, _)) => TaskResult::Bytes(buf[..n].to_vec()),
                            Err(e) => TaskResult::Error(e.to_string()),
                        }
                    }
                    Err(e) => TaskResult::Error(e.to_string()),
                }
            }
        }
        Task::ComplexMath { expression } => match eval_complex(&expression) {
            Ok(c) => TaskResult::Response(c.to_string()),
            Err(e) => TaskResult::Error(e),
        },
        Task::StoreData { key, data } => {
            if let Some(ds) = disk {
                match ds.store(key, data).await {
                    Ok(_) => TaskResult::Stored,
                    Err(e) => TaskResult::Error(e.to_string()),
                }
            } else {
                store.store(key, data).await;
                TaskResult::Stored
            }
        }
        Task::RetrieveData { key } => {
            if let Some(ds) = disk {
                match ds.load(&key).await {
                    Some(d) => TaskResult::Bytes(d),
                    None => TaskResult::Error("Key not found".into()),
                }
            } else {
                match store.load(&key).await {
                    Some(d) => TaskResult::Bytes(d),
                    None => TaskResult::Error("Key not found".into()),
                }
            }
        }
        Task::DiskWrite { path, data } => {
            if let Err(e) = tokio::fs::create_dir_all(storage_dir).await {
                return TaskResult::Error(e.to_string());
            }
            let base = match tokio::fs::canonicalize(storage_dir).await {
                Ok(p) => p,
                Err(e) => return TaskResult::Error(e.to_string()),
            };
            let full = base.join(&path);
            if let Some(parent) = full.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return TaskResult::Error(e.to_string());
                }
            }
            let canonical = if full.exists() {
                match tokio::fs::canonicalize(&full).await {
                    Ok(p) => p,
                    Err(e) => return TaskResult::Error(e.to_string()),
                }
            } else {
                match tokio::fs::canonicalize(full.parent().unwrap()).await {
                    Ok(p) => p.join(full.file_name().unwrap()),
                    Err(e) => return TaskResult::Error(e.to_string()),
                }
            };
            if !canonical.starts_with(&base) {
                return TaskResult::Error("Invalid path".into());
            }
            match tokio::fs::write(&canonical, &data).await {
                Ok(_) => TaskResult::Written,
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::DiskRead { path } => {
            let base = match tokio::fs::canonicalize(storage_dir).await {
                Ok(p) => p,
                Err(e) => return TaskResult::Error(e.to_string()),
            };
            let full = base.join(&path);
            let canonical = match tokio::fs::canonicalize(&full).await {
                Ok(p) => p,
                Err(e) => return TaskResult::Error(e.to_string()),
            };
            if !canonical.starts_with(&base) {
                return TaskResult::Error("Invalid path".into());
            }
            match tokio::fs::read(&canonical).await {
                Ok(d) => TaskResult::Bytes(d),
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::GetGlobalRam => {
            let stats = store.stats().await;
            if stats.is_empty() {
                TaskResult::Response("No entries found\n".to_string())
            } else {
                let mut resp = String::new();
                for (id, size) in stats {
                    resp.push_str(&format!("{}: {} bytes\n", id, size));
                }
                TaskResult::Response(resp)
            }
        }
        Task::GetStorage => {
            if let Some(ds) = disk {
                match ds.stats().await {
                    Ok((files, free)) => {
                        let mut resp = format!("free: {} bytes\n", free);
                        if files.is_empty() {
                            resp.push_str("No entries found\n");
                        } else {
                            for (id, size) in files {
                                resp.push_str(&format!("{}: {} bytes\n", id, size));
                            }
                        }
                        TaskResult::Response(resp)
                    }
                    Err(e) => TaskResult::Error(e.to_string()),
                }
            } else {
                TaskResult::Error("No disk store configured".into())
            }
        }
    }
}
