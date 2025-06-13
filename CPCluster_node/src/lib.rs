use cpcluster_common::{Task, TaskResult};
use meval::shunting_yard::to_rpn;
use meval::tokenizer::{tokenize, Operation, Token};
use num_complex::Complex64;
use once_cell::sync::Lazy;
use reqwest::Client;
use std::collections::HashMap;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Mutex;

static MEMORY: Lazy<Mutex<HashMap<String, Vec<u8>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

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

pub async fn execute_task(task: Task, client: &Client, storage_dir: &str) -> TaskResult {
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
        Task::Tcp { addr, data } => match TcpStream::connect(&addr).await {
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
        },
        Task::Udp { addr, data } => match UdpSocket::bind("0.0.0.0:0").await {
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
        },
        Task::ComplexMath { expression } => match eval_complex(&expression) {
            Ok(c) => TaskResult::Response(c.to_string()),
            Err(e) => TaskResult::Error(e),
        },
        Task::StoreData { key, data } => {
            MEMORY.lock().await.insert(key, data);
            TaskResult::Stored
        }
        Task::RetrieveData { key } => match MEMORY.lock().await.get(&key).cloned() {
            Some(d) => TaskResult::Bytes(d),
            None => TaskResult::Error("Key not found".into()),
        },
        Task::DiskWrite { path, data } => {
            let full = Path::new(storage_dir).join(&path);
            if let Some(parent) = full.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return TaskResult::Error(e.to_string());
                }
            }
            match tokio::fs::write(&full, &data).await {
                Ok(_) => TaskResult::Written,
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::DiskRead { path } => {
            let full = Path::new(storage_dir).join(&path);
            match tokio::fs::read(&full).await {
                Ok(d) => TaskResult::Bytes(d),
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
    }
}
