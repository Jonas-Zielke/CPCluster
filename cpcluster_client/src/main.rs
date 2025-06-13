use cpcluster_common::{read_length_prefixed, write_length_prefixed, JoinInfo, NodeMessage, Task, TaskResult};
use std::{borrow::Cow, error::Error, fs};
use tokio::net::TcpStream;
use uuid::Uuid;
use tokio::time::{sleep, Duration};
use serde_json;
use env_logger;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let join_info: JoinInfo = serde_json::from_str(&fs::read_to_string("join.json")?)?;
    let addr = format!("{}:{}", join_info.ip, join_info.port);
    let mut stream = TcpStream::connect(&addr).await?;

    write_length_prefixed(&mut stream, join_info.token.as_bytes()).await?;
    let resp = read_length_prefixed(&mut stream).await?;
    if resp == b"Invalid token" { return Err("authentication failed".into()); }

    let id = Uuid::new_v4().to_string();
    let task = Task::Compute { expression: Cow::Borrowed("1+2") };
    let msg = NodeMessage::SubmitTask { id: id.clone(), task };
    write_length_prefixed(&mut stream, &serde_json::to_vec(&msg)?).await?;
    let _ = read_length_prefixed(&mut stream).await?; // TaskAccepted

    loop {
        let req = NodeMessage::GetTaskResult(id.clone());
        write_length_prefixed(&mut stream, &serde_json::to_vec(&req)?).await?;
        let buf = read_length_prefixed(&mut stream).await?;
        match serde_json::from_slice::<NodeMessage>(&buf)? {
            NodeMessage::TaskResult { result, .. } => {
                println!("first result: {:?}", result);
                if let TaskResult::Number(v) = result {
                    let expr = format!("{} * 3", v);
                    let id2 = Uuid::new_v4().to_string();
                    let task2 = Task::Compute { expression: Cow::Owned(expr) };
                    let msg2 = NodeMessage::SubmitTask { id: id2.clone(), task: task2 };
                    write_length_prefixed(&mut stream, &serde_json::to_vec(&msg2)?).await?;
                    let _ = read_length_prefixed(&mut stream).await?;
                    loop {
                        let req2 = NodeMessage::GetTaskResult(id2.clone());
                        write_length_prefixed(&mut stream, &serde_json::to_vec(&req2)?).await?;
                        let buf2 = read_length_prefixed(&mut stream).await?;
                        match serde_json::from_slice::<NodeMessage>(&buf2)? {
                            NodeMessage::TaskResult { result: r, .. } => {
                                println!("chained result: {:?}", r);
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
