use cpcluster_common::{read_length_prefixed, write_length_prefixed, NodeMessage};
use cpcluster_common::{NodeRole, Task};
use cpcluster_masternode::state::{MasterNode, NodeInfo, PendingTask};
use cpcluster_masternode::{connect_to_peer, handle_peer_connection};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::Mutex;
use tokio::net::TcpListener;

#[tokio::test]
async fn master_node_interaction() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // choose random port for master
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let token = "testtoken".to_string();
    let token_srv = token.clone();

    // ports available for allocation
    let ports = Arc::new(StdMutex::new(HashSet::from([addr.port() + 1])));
    let ports_srv = ports.clone();

    // spawn simplified master server
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let token_bytes = read_length_prefixed(&mut socket).await.expect("read token");
        assert_eq!(
            std::str::from_utf8(&token_bytes).expect("valid UTF-8"),
            token_srv
        );
        write_length_prefixed(&mut socket, b"OK")
            .await
            .expect("send auth response");

        let msg = read_length_prefixed(&mut socket)
            .await
            .expect("read message");
        let msg: NodeMessage = serde_json::from_slice(&msg).expect("deserialize message");
        assert!(matches!(msg, NodeMessage::GetConnectedNodes));
        let resp = NodeMessage::ConnectedNodes(vec!["node1".into()]);
        write_length_prefixed(
            &mut socket,
            &serde_json::to_vec(&resp).expect("serialize response"),
        )
        .await
        .expect("send connected nodes");

        let msg = read_length_prefixed(&mut socket)
            .await
            .expect("read connection request");
        let msg: NodeMessage =
            serde_json::from_slice(&msg).expect("deserialize connection request");
        if let NodeMessage::RequestConnection(id) = msg {
            let port = {
                let mut set = ports_srv.lock().expect("lock ports");
                let p = *set.iter().next().expect("port available");
                set.remove(&p);
                p
            };
            let resp = NodeMessage::ConnectionInfo(id, port);
            write_length_prefixed(
                &mut socket,
                &serde_json::to_vec(&resp).expect("serialize connection info"),
            )
            .await
            .expect("send connection info");
        } else {
            panic!("unexpected message: {:?}", msg);
        }
    });

    // spawn client node
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    write_length_prefixed(&mut stream, token.as_bytes()).await?;
    let resp = read_length_prefixed(&mut stream).await?;
    assert_eq!(&resp[..], b"OK");

    write_length_prefixed(
        &mut stream,
        &serde_json::to_vec(&NodeMessage::GetConnectedNodes)?,
    )
    .await?;
    let msg = read_length_prefixed(&mut stream).await?;
    let msg: NodeMessage = serde_json::from_slice(&msg)?;
    assert!(matches!(msg, NodeMessage::ConnectedNodes(_)));

    write_length_prefixed(
        &mut stream,
        &serde_json::to_vec(&NodeMessage::RequestConnection("node1".into()))?,
    )
    .await?;
    let msg = read_length_prefixed(&mut stream).await?;
    let msg: NodeMessage = serde_json::from_slice(&msg)?;
    let allocated_port = match msg {
        NodeMessage::ConnectionInfo(target, port) => {
            assert_eq!(target, "node1");
            port
        }
        other => panic!("unexpected message: {:?}", other),
    };

    // ensure port allocation removed it from available set
    assert!(ports.lock().expect("lock ports").is_empty());
    assert_eq!(allocated_port, addr.port() + 1);

    // ensure server task completes
    server.await.expect("server task failed");
    Ok(())
}

#[tokio::test]
async fn port_released_after_disconnect() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let token = "testtoken".to_string();
    let token_srv = token.clone();

    let ports = Arc::new(StdMutex::new(HashSet::from([addr.port() + 1])));
    let ports_srv = ports.clone();
    let released = ports.clone();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let token_bytes = read_length_prefixed(&mut socket).await.expect("read token");
        assert_eq!(
            std::str::from_utf8(&token_bytes).expect("valid UTF-8"),
            token_srv
        );
        write_length_prefixed(&mut socket, b"OK")
            .await
            .expect("send auth");

        let msg = read_length_prefixed(&mut socket)
            .await
            .expect("read request");
        let msg: NodeMessage = serde_json::from_slice(&msg).expect("deserialize");
        let mut allocated_port = 0u16;
        if let NodeMessage::RequestConnection(id) = msg {
            let port = {
                let mut set = ports_srv.lock().expect("lock ports");
                let p = *set.iter().next().expect("port available");
                set.remove(&p);
                p
            };
            allocated_port = port;
            let resp = NodeMessage::ConnectionInfo(id, port);
            write_length_prefixed(&mut socket, &serde_json::to_vec(&resp).unwrap())
                .await
                .expect("send info");
        }

        let msg = read_length_prefixed(&mut socket)
            .await
            .expect("read disconnect");
        let msg: NodeMessage = serde_json::from_slice(&msg).expect("deserialize");
        if matches!(msg, NodeMessage::Disconnect) {
            released.lock().expect("lock").insert(allocated_port);
        }
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    write_length_prefixed(&mut stream, token.as_bytes()).await?;
    read_length_prefixed(&mut stream).await?; // OK

    write_length_prefixed(
        &mut stream,
        &serde_json::to_vec(&NodeMessage::RequestConnection("node1".into()))?,
    )
    .await?;
    let _ = read_length_prefixed(&mut stream).await?; // ConnectionInfo

    assert!(ports.lock().expect("lock").is_empty());

    write_length_prefixed(&mut stream, &serde_json::to_vec(&NodeMessage::Disconnect)?).await?;

    server.await.expect("server task");
    assert!(ports.lock().expect("lock").contains(&(addr.port() + 1)));
    Ok(())
}

#[tokio::test]
async fn masters_sync_state() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let master1 = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
        available_ports: Arc::new(Mutex::new(VecDeque::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(Mutex::new(HashMap::new())),
        state_file: "m1.json".into(),
    });
    let master2 = Arc::new(MasterNode {
        connected_nodes: Arc::new(Mutex::new(HashMap::new())),
        available_ports: Arc::new(Mutex::new(VecDeque::new())),
        failover_timeout_ms: 1000,
        pending_tasks: Arc::new(Mutex::new(HashMap::new())),
        completed_tasks: Arc::new(Mutex::new(HashMap::new())),
        state_file: "m2.json".into(),
    });

    master1.pending_tasks.lock().await.insert(
        "task1".into(),
        PendingTask {
            task: Task::Compute {
                expression: "1+1".into(),
            },
            target: None,
        },
    );
    master2.pending_tasks.lock().await.insert(
        "task2".into(),
        PendingTask {
            task: Task::Compute {
                expression: "2+2".into(),
            },
            target: None,
        },
    );

    master1.connected_nodes.lock().await.insert(
        "node1".into(),
        NodeInfo {
            addr: "node1".into(),
            last_heartbeat: 0,
            port: None,
            active_tasks: HashMap::new(),
            is_worker: true,
            role: NodeRole::Worker,
        },
    );
    master2.connected_nodes.lock().await.insert(
        "node2".into(),
        NodeInfo {
            addr: "node2".into(),
            last_heartbeat: 0,
            port: None,
            active_tasks: HashMap::new(),
            is_worker: true,
            role: NodeRole::Worker,
        },
    );

    let m2 = Arc::clone(&master2);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let token = read_length_prefixed(&mut stream).await?;
        assert_eq!(String::from_utf8(token)?, "MASTER_SYNC");
        handle_peer_connection(stream, m2).await?;
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    connect_to_peer(addr.to_string(), Arc::clone(&master1)).await;
    server.await??;

    assert!(master1.pending_tasks.lock().await.contains_key("task2"));
    assert!(master2.pending_tasks.lock().await.contains_key("task1"));
    assert!(master1.connected_nodes.lock().await.contains_key("node2"));
    assert!(master2.connected_nodes.lock().await.contains_key("node1"));
    Ok(())
}
