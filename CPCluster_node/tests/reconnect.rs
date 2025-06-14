use cpcluster_common::{
    read_length_prefixed, write_length_prefixed, Config, JoinInfo, NodeMessage,
};
use cpcluster_node::node::connect_to_master;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_with_backoff() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // pick unused port then release
    let tmp = TcpListener::bind("127.0.0.1:0").await?;
    let port = tmp.local_addr()?.port();
    drop(tmp);
    let addr = format!("127.0.0.1:{}", port);
    let join = JoinInfo {
        token: "tok".into(),
        ip: "127.0.0.1".into(),
        port,
    };
    let mut config = Config::default();
    config.master_addresses = vec![addr.clone()];
    config.max_retries = 5;

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let listener = TcpListener::bind(addr).await.expect("bind server");
        let (mut socket, _) = listener.accept().await.expect("accept connection");
        let token_bytes = read_length_prefixed(&mut socket).await.expect("read token");
        assert_eq!(
            std::str::from_utf8(&token_bytes).expect("valid UTF-8"),
            "tok"
        );
        write_length_prefixed(&mut socket, b"OK")
            .await
            .expect("send OK");
        let _ = read_length_prefixed(&mut socket)
            .await
            .expect("read register role");
        let _ = read_length_prefixed(&mut socket)
            .await
            .expect("read get connected nodes");
        let resp = NodeMessage::ConnectedNodes(Vec::new());
        write_length_prefixed(
            &mut socket,
            &serde_json::to_vec(&resp).expect("serialize response"),
        )
        .await
        .expect("send connected nodes");
    });

    connect_to_master(&join, &config).await?;
    Ok(())
}
