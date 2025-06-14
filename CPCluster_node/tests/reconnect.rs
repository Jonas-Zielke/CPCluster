use cpcluster_common::{
    read_length_prefixed, write_length_prefixed, Config, JoinInfo, NodeMessage,
};
use cpcluster_node::node::connect_to_master;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_with_backoff() {
    // pick unused port then release
    let tmp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = tmp.local_addr().unwrap().port();
    drop(tmp);
    let addr = format!("127.0.0.1:{}", port);
    let join = JoinInfo {
        token: "tok".into(),
        ip: "127.0.0.1".into(),
        port,
    };
    let config = Config {
        master_addresses: vec![addr.clone()],
        max_retries: 5,
        ..Default::default()
    };

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let listener = TcpListener::bind(addr).await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        let token_bytes = read_length_prefixed(&mut socket).await.unwrap();
        assert_eq!(std::str::from_utf8(&token_bytes).unwrap(), "tok");
        write_length_prefixed(&mut socket, b"OK").await.unwrap();
        let _ = read_length_prefixed(&mut socket).await.unwrap(); // RegisterRole
        let _ = read_length_prefixed(&mut socket).await.unwrap(); // GetConnectedNodes
        let resp = NodeMessage::ConnectedNodes(Vec::new());
        write_length_prefixed(&mut socket, &serde_json::to_vec(&resp).unwrap())
            .await
            .unwrap();
    });

    let res = connect_to_master(&join, &config).await;
    assert!(res.is_ok());
}
