use cpcluster_common::{read_length_prefixed, write_length_prefixed, NodeMessage};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

#[tokio::test]
async fn node_interacts_with_master() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // choose random port for master
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let token = "testtoken".to_string();
    let token_srv = token.clone();
    let ports = Arc::new(Mutex::new(HashSet::from([addr.port() + 1])));
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
    match msg {
        NodeMessage::ConnectionInfo(_, port) => assert_eq!(port, addr.port() + 1),
        other => panic!("unexpected message: {:?}", other),
    }
    assert!(ports.lock().expect("lock ports").is_empty());
    server.await.expect("server task failed");
    Ok(())
}
