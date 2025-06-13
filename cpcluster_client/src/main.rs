use cpcluster_common::{
    JoinInfo, NodeMessage, Task, TaskResult, read_length_prefixed, write_length_prefixed,
};
use log::info;
use meval::eval_str;
use once_cell::sync::Lazy;
use reqwest::Client;
use std::{
    borrow::Cow,
    collections::HashMap,
    error::Error,
    fs,
    sync::Mutex as StdMutex,
    time::Duration,
};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::{sleep, timeout};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

static DATA_STORE: Lazy<StdMutex<HashMap<String, Vec<u8>>>> =
    Lazy::new(|| StdMutex::new(HashMap::new()));

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
        Task::TcpIo { addr, port, data } => {
            match TcpStream::connect((addr.as_str(), port)).await {
                Ok(mut stream) => {
                    if let Err(e) = stream.write_all(&data).await {
                        return TaskResult::Error(e.to_string());
                    }
                    let mut buf = Vec::new();
                    match stream.read_to_end(&mut buf).await {
                        Ok(_) => TaskResult::Bytes(buf),
                        Err(e) => TaskResult::Error(e.to_string()),
                    }
                }
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::UdpIo { addr, port, data } => {
            match UdpSocket::bind("0.0.0.0:0").await {
                Ok(socket) => {
                    if let Err(e) = socket.send_to(&data, (addr.as_str(), port)).await {
                        return TaskResult::Error(e.to_string());
                    }
                    let mut buf = vec![0u8; 65535];
                    match timeout(Duration::from_secs(1), socket.recv_from(&mut buf)).await {
                        Ok(Ok((len, _))) => {
                            buf.truncate(len);
                            TaskResult::Bytes(buf)
                        }
                        _ => TaskResult::Bytes(Vec::new()),
                    }
                }
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::ComplexMath { expression } => match eval_str(&expression) {
            Ok(v) => TaskResult::Number(v),
            Err(e) => TaskResult::Error(e.to_string()),
        },
        Task::StoreData { key, data } => {
            let mut map = DATA_STORE.lock().unwrap();
            map.insert(key, data);
            TaskResult::Stored
        }
        Task::RetrieveData { key } => {
            let map = DATA_STORE.lock().unwrap();
            match map.get(&key).cloned() {
                Some(d) => TaskResult::Bytes(d),
                None => TaskResult::Error("Key not found".into()),
            }
        }
        Task::DiskWrite { path, data } => {
            match tokio::fs::write(&path, data).await {
                Ok(_) => TaskResult::Stored,
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
        Task::DiskRead { path } => {
            match tokio::fs::read(&path).await {
                Ok(d) => TaskResult::Bytes(d),
                Err(e) => TaskResult::Error(e.to_string()),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let join_info: JoinInfo = serde_json::from_str(&fs::read_to_string("join.json")?)?;
    let addr = format!("{}:{}", join_info.ip, join_info.port);
    info!("Connecting to master at {}", addr);
    let mut stream = TcpStream::connect(&addr).await?;

    write_length_prefixed(&mut stream, join_info.token.as_bytes()).await?;
    let resp = read_length_prefixed(&mut stream).await?;
    if resp == b"Invalid token" {
        return Err("authentication failed".into());
    }
    info!("Authenticated with master");

    // check if any nodes are connected; if none, execute tasks locally
    let nodes_req = NodeMessage::GetConnectedNodes;
    write_length_prefixed(&mut stream, &serde_json::to_vec(&nodes_req)?).await?;
    let buf = read_length_prefixed(&mut stream).await?;
    let local_execute = match serde_json::from_slice::<NodeMessage>(&buf)? {
        NodeMessage::ConnectedNodes(nodes) => {
            info!("{} worker(s) connected", nodes.len());
            nodes.is_empty()
        }
        _ => false,
    };

    let http_client = Client::new();

    let id = Uuid::new_v4().to_string();
    let task = Task::Compute {
        expression: Cow::Borrowed("1+2"),
    };

    if local_execute {
        info!("No workers connected, executing task locally");
        let result = execute_task(task.clone(), &http_client).await;
        info!("first result: {:?}", result);
        if let TaskResult::Number(v) = result {
            let expr = format!("{} * 3", v);
            let task2 = Task::Compute {
                expression: Cow::Owned(expr),
            };
            let result2 = execute_task(task2, &http_client).await;
            info!("chained result: {:?}", result2);
        }
        return Ok(());
    }

    let msg = NodeMessage::SubmitTask {
        id: id.clone(),
        task,
    };
    info!("Submitting task {}", id);
    write_length_prefixed(&mut stream, &serde_json::to_vec(&msg)?).await?;
    let _ = read_length_prefixed(&mut stream).await?; // TaskAccepted

    loop {
        let req = NodeMessage::GetTaskResult(id.clone());
        write_length_prefixed(&mut stream, &serde_json::to_vec(&req)?).await?;
        let buf = read_length_prefixed(&mut stream).await?;
        match serde_json::from_slice::<NodeMessage>(&buf)? {
            NodeMessage::TaskResult { result, .. } => {
                info!("first result: {:?}", result);
                if let TaskResult::Number(v) = result {
                    let expr = format!("{} * 3", v);
                    let id2 = Uuid::new_v4().to_string();
                    let task2 = Task::Compute {
                        expression: Cow::Owned(expr),
                    };
                    let msg2 = NodeMessage::SubmitTask {
                        id: id2.clone(),
                        task: task2,
                    };
                    write_length_prefixed(&mut stream, &serde_json::to_vec(&msg2)?).await?;
                    let _ = read_length_prefixed(&mut stream).await?;
                    loop {
                        let req2 = NodeMessage::GetTaskResult(id2.clone());
                        write_length_prefixed(&mut stream, &serde_json::to_vec(&req2)?).await?;
                        let buf2 = read_length_prefixed(&mut stream).await?;
                        match serde_json::from_slice::<NodeMessage>(&buf2)? {
                            NodeMessage::TaskResult { result: r, .. } => {
                                info!("chained result: {:?}", r);
                                return Ok(());
                            }
                            _ => sleep(Duration::from_millis(500)).await,
                        }
                    }
                } else {
                    return Ok(());
                }
            }
            _ => sleep(Duration::from_millis(500)).await,
        }
    }
}
