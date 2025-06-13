use cpcluster_common::{
    NodeMessage, Task, TaskResult, read_length_prefixed, write_length_prefixed,
};
use meval::eval_str;
use reqwest::Client;
use std::error::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

pub async fn submit_and_wait<S>(
    stream: &mut S,
    task: Task,
) -> Result<TaskResult, Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let id = Uuid::new_v4().to_string();
    let msg = NodeMessage::SubmitTask {
        id: id.clone(),
        task,
    };
    write_length_prefixed(stream, &serde_json::to_vec(&msg)?).await?;
    let _ = read_length_prefixed(stream).await?; // TaskAccepted
    loop {
        let req = NodeMessage::GetTaskResult(id.clone());
        write_length_prefixed(stream, &serde_json::to_vec(&req)?).await?;
        let buf = read_length_prefixed(stream).await?;
        match serde_json::from_slice::<NodeMessage>(&buf)? {
            NodeMessage::TaskResult { result, .. } => return Ok(result),
            _ => sleep(Duration::from_millis(500)).await,
        }
    }
}

pub async fn execute_task(task: Task, client: &Client) -> TaskResult {
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
        _ => TaskResult::Error("Unsupported task".into()),
    }
}
