use cpcluster_common::{
    NodeMessage, Task, TaskResult, read_length_prefixed, write_length_prefixed,
};
use std::error::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Duration, Instant, sleep, timeout};
use uuid::Uuid;

/// Default timeout applied when [`submit_and_wait`] callers pass `None`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn submit_and_wait<S>(
    stream: &mut S,
    task: Task,
    timeout_duration: Option<Duration>,
) -> Result<TaskResult, Box<dyn Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let timeout_duration = timeout_duration.unwrap_or(DEFAULT_TIMEOUT);
    let id = Uuid::new_v4().to_string();
    let msg = NodeMessage::SubmitTask {
        id: id.clone(),
        task,
    };
    write_length_prefixed(stream, &serde_json::to_vec(&msg)?).await?;
    let _ = read_length_prefixed(stream).await?; // TaskAccepted
    let deadline = Instant::now() + timeout_duration;
    loop {
        let req = NodeMessage::GetTaskResult(id.clone());
        write_length_prefixed(stream, &serde_json::to_vec(&req)?).await?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("task timed out".into());
        }
        let buf = match timeout(remaining, read_length_prefixed(stream)).await {
            Ok(res) => res?,
            Err(_) => return Err("task timed out".into()),
        };
        match serde_json::from_slice::<NodeMessage>(&buf)? {
            NodeMessage::TaskResult { result, .. } => return Ok(result),
            _ => {
                if Instant::now() >= deadline {
                    return Err("task timed out".into());
                }
                sleep(Duration::from_millis(500)).await
            }
        }
    }
}

pub use cpcluster_common::execute_task;
