use crate::{Task, TaskResult};
use meval::eval_str;
use reqwest::Client;

/// Execute simple tasks that don't require node-specific resources.
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
