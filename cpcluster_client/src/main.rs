use cpcluster_client::{DEFAULT_TIMEOUT, execute_task, submit_and_wait};
use cpcluster_common::{
    JoinInfo, NodeMessage, Task, TaskResult, read_length_prefixed, write_length_prefixed,
};
use log::info;
use reqwest::Client;
use std::{borrow::Cow, env, error::Error, fs};
use tokio::net::TcpStream;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

fn join_path() -> String {
    env::var("CPCLUSTER_JOIN").unwrap_or_else(|_| "join.json".to_string())
}

async fn run_data_tests(stream: &mut TcpStream) -> Result<(), Box<dyn Error + Send + Sync>> {
    let compute_res = submit_and_wait(
        stream,
        Task::Compute {
            expression: Cow::Borrowed("1+1"),
        },
        Some(DEFAULT_TIMEOUT),
    )
    .await?;

    let number = match compute_res {
        TaskResult::Number(v) => v,
        _ => return Ok(()),
    };

    let mem_id = Uuid::new_v4().to_string();
    let _ = submit_and_wait(
        stream,
        Task::StoreData {
            key: mem_id.clone(),
            data: number.to_string().into_bytes(),
        },
        Some(DEFAULT_TIMEOUT),
    )
    .await?;

    let disk_id = format!("{}.txt", Uuid::new_v4());
    let content = format!("{}:{}", mem_id, number);
    let _ = submit_and_wait(
        stream,
        Task::DiskWrite {
            path: disk_id.clone(),
            data: content.into_bytes(),
        },
        Some(DEFAULT_TIMEOUT),
    )
    .await?;

    println!("RAM ID: {}", mem_id);
    println!("Storage ID: {}", disk_id);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    env_logger::init();
    let path = join_path();
    let mut join_info: JoinInfo = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if let Ok(token) = env::var("CPCLUSTER_TOKEN") {
        join_info.token = token;
    }
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
                                run_data_tests(&mut stream).await?;
                                return Ok(());
                            }
                            _ => sleep(Duration::from_millis(500)).await,
                        }
                    }
                } else {
                    run_data_tests(&mut stream).await?;
                    return Ok(());
                }
            }
            _ => sleep(Duration::from_millis(500)).await,
        }
    }
}
