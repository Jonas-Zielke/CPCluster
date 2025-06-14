use cpcluster_common::{Task, TaskResult};
use cpcluster_node::{execute_node_task, memory_store::MemoryStore};
use reqwest::Client;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

#[tokio::test]
async fn tcp_and_udp_tasks() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // TCP echo
    let tcp_listener = TcpListener::bind("127.0.0.1:0").await?;
    let tcp_addr = tcp_listener.local_addr()?;
    tokio::spawn(async move {
        let (mut socket, _) = tcp_listener.accept().await.expect("accept tcp");
        let mut buf = [0u8; 5];
        socket.read_exact(&mut buf).await.expect("read tcp");
        socket.write_all(b"world").await.expect("write tcp");
    });
    let client = Client::new();
    let store = MemoryStore::new();
    let res = execute_node_task(
        Task::Tcp {
            addr: tcp_addr.to_string(),
            data: b"hello".to_vec(),
        },
        &client,
        "./",
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Bytes(ref b) if b == b"world"));

    // UDP echo
    let udp_socket = UdpSocket::bind("127.0.0.1:0").await?;
    let udp_addr = udp_socket.local_addr()?;
    tokio::spawn(async move {
        let mut buf = [0u8; 5];
        let (n, peer) = udp_socket.recv_from(&mut buf).await.expect("udp recv");
        assert_eq!(&buf[..n], b"ping");
        udp_socket.send_to(b"pong", &peer).await.expect("udp send");
    });
    let res = execute_node_task(
        Task::Udp {
            addr: udp_addr.to_string(),
            data: b"ping".to_vec(),
        },
        &client,
        "./",
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Bytes(ref b) if b == b"pong"));
    Ok(())
}

#[tokio::test]
async fn complex_and_storage_tasks() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = Client::new();
    let store = MemoryStore::new();
    // complex math
    let res = execute_node_task(
        Task::ComplexMath {
            expression: "1+2*i+(3-4*i)".into(),
        },
        &client,
        "./",
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Response(ref s) if s.trim() == "4-2i"));

    // RAM store and retrieve
    let res = execute_node_task(
        Task::StoreData {
            key: "k".into(),
            data: b"data".to_vec(),
        },
        &client,
        "./",
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Stored));
    let res = execute_node_task(
        Task::RetrieveData { key: "k".into() },
        &client,
        "./",
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Bytes(ref b) if b == b"data"));
    Ok(())
}

#[tokio::test]
async fn disk_tasks() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempdir()?;
    let path = dir.path().to_str().expect("path to str");
    let client = Client::new();
    let store = MemoryStore::new();
    let res = execute_node_task(
        Task::DiskWrite {
            path: "file.bin".into(),
            data: b"d".to_vec(),
        },
        &client,
        path,
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Written));
    let res = execute_node_task(
        Task::DiskRead {
            path: "file.bin".into(),
        },
        &client,
        path,
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Bytes(ref b) if b == b"d"));
    Ok(())
}

#[tokio::test]
async fn disk_path_safety() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempdir()?;
    let base = dir.path();
    let evil_path = base.join("..\u{002f}evil.txt");
    tokio::fs::write(&evil_path, b"bad").await?;
    let client = Client::new();
    let store = MemoryStore::new();

    let res = execute_node_task(
        Task::DiskWrite {
            path: "../evil.txt".into(),
            data: b"x".to_vec(),
        },
        &client,
        base.to_str().expect("base to str"),
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Error(_)));

    let res = execute_node_task(
        Task::DiskRead {
            path: "../evil.txt".into(),
        },
        &client,
        base.to_str().expect("base to str"),
        &store,
        None,
        None,
    )
    .await;
    assert!(matches!(res, TaskResult::Error(_)));
    Ok(())
}
