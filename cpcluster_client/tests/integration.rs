use cpcluster_client::{execute_task, submit_and_wait};
use cpcluster_common::{
    NodeMessage, Task, TaskResult, read_length_prefixed, write_length_prefixed,
};
use reqwest::Client;
use std::borrow::Cow;
use tokio::io::duplex;

#[tokio::test]
async fn submit_and_wait_returns_result() {
    let (mut client_stream, mut server_stream) = duplex(1024);

    let server = tokio::spawn(async move {
        // read submit message
        let buf = read_length_prefixed(&mut server_stream).await.unwrap();
        let msg: NodeMessage = serde_json::from_slice(&buf).unwrap();
        if let NodeMessage::SubmitTask { id, .. } = msg {
            let ack = NodeMessage::TaskAccepted(id.clone());
            write_length_prefixed(&mut server_stream, &serde_json::to_vec(&ack).unwrap())
                .await
                .unwrap();
            let buf = read_length_prefixed(&mut server_stream).await.unwrap();
            let req: NodeMessage = serde_json::from_slice(&buf).unwrap();
            if matches!(req, NodeMessage::GetTaskResult(_)) {
                let result = NodeMessage::TaskResult {
                    id,
                    result: TaskResult::Number(5.0),
                };
                write_length_prefixed(&mut server_stream, &serde_json::to_vec(&result).unwrap())
                    .await
                    .unwrap();
            }
        }
    });

    let task = Task::Compute {
        expression: Cow::Borrowed("2+3"),
    };

    let res = submit_and_wait(&mut client_stream, task).await.unwrap();
    server.await.unwrap();
    match res {
        TaskResult::Number(v) => assert!((v - 5.0).abs() < f64::EPSILON),
        other => panic!("unexpected result: {:?}", other),
    }
}

#[tokio::test]
async fn execute_task_compute() {
    let http_client = Client::new();
    let task = Task::Compute {
        expression: Cow::Borrowed("6/2"),
    };
    let result = execute_task(task, &http_client).await;
    match result {
        TaskResult::Number(v) => assert!((v - 3.0).abs() < f64::EPSILON),
        other => panic!("unexpected result: {:?}", other),
    }
}
