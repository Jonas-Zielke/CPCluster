use cpcluster_common::{read_length_prefixed, write_length_prefixed, NodeMessage};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn master_node_interaction() {
    // choose random port for master
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = "testtoken".to_string();
    let token_srv = token.clone();

    // ports available for allocation
    let ports = Arc::new(Mutex::new(HashSet::from([addr.port() + 1])));
    let ports_srv = ports.clone();

    // spawn simplified master server
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let token_bytes = read_length_prefixed(&mut socket).await.unwrap();
        assert_eq!(std::str::from_utf8(&token_bytes).unwrap(), token_srv);
        write_length_prefixed(&mut socket, b"OK").await.unwrap();

        let msg = read_length_prefixed(&mut socket).await.unwrap();
        let msg: NodeMessage = serde_json::from_slice(&msg).unwrap();
        assert!(matches!(msg, NodeMessage::GetConnectedNodes));
        let resp = NodeMessage::ConnectedNodes(vec!["node1".into()]);
        write_length_prefixed(&mut socket, &serde_json::to_vec(&resp).unwrap())
            .await
            .unwrap();

        let msg = read_length_prefixed(&mut socket).await.unwrap();
        let msg: NodeMessage = serde_json::from_slice(&msg).unwrap();
        if let NodeMessage::RequestConnection(id) = msg {
            let port = {
                let mut set = ports_srv.lock().unwrap();
                let p = *set.iter().next().unwrap();
                set.remove(&p);
                p
            };
            let resp = NodeMessage::ConnectionInfo(id, port);
            write_length_prefixed(&mut socket, &serde_json::to_vec(&resp).unwrap())
                .await
                .unwrap();
        } else {
            panic!("unexpected message: {:?}", msg);
        }
    });

    // spawn client node
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    write_length_prefixed(&mut stream, token.as_bytes())
        .await
        .unwrap();
    let resp = read_length_prefixed(&mut stream).await.unwrap();
    assert_eq!(&resp[..], b"OK");

    write_length_prefixed(
        &mut stream,
        &serde_json::to_vec(&NodeMessage::GetConnectedNodes).unwrap(),
    )
    .await
    .unwrap();
    let msg = read_length_prefixed(&mut stream).await.unwrap();
    let msg: NodeMessage = serde_json::from_slice(&msg).unwrap();
    assert!(matches!(msg, NodeMessage::ConnectedNodes(_)));

    write_length_prefixed(
        &mut stream,
        &serde_json::to_vec(&NodeMessage::RequestConnection("node1".into())).unwrap(),
    )
    .await
    .unwrap();
    let msg = read_length_prefixed(&mut stream).await.unwrap();
    let msg: NodeMessage = serde_json::from_slice(&msg).unwrap();
    let allocated_port = match msg {
        NodeMessage::ConnectionInfo(target, port) => {
            assert_eq!(target, "node1");
            port
        }
        other => panic!("unexpected message: {:?}", other),
    };

    // ensure port allocation removed it from available set
    assert!(ports.lock().unwrap().is_empty());
    assert_eq!(allocated_port, addr.port() + 1);

    // ensure server task completes
    server.await.unwrap();
}
