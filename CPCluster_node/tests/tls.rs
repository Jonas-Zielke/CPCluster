use cpcluster_common::{read_length_prefixed, write_length_prefixed, NodeMessage};
use rcgen::generate_simple_self_signed;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio_rustls::{rustls, TlsAcceptor, TlsConnector};

#[tokio::test]
async fn node_tls_interaction() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cert = generate_simple_self_signed(vec!["localhost".into()])?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    let tls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::Certificate(cert_der.clone())],
            rustls::PrivateKey(key_der),
        )
        .expect("create TLS config");
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let token = "testtoken".to_string();
    let token_srv = token.clone();
    let ports = Arc::new(Mutex::new(HashSet::from([addr.port() + 1])));
    let ports_srv = ports.clone();

    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept connection");
        let mut socket = acceptor.accept(socket).await.expect("TLS accept");
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

    let mut root_store = rustls::RootCertStore::empty();
    root_store
        .add(&rustls::Certificate(cert_der))
        .expect("add cert");
    let client_config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let server_name = rustls::ServerName::try_from("localhost")?;
    let mut stream = connector.connect(server_name, tcp).await?;

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

#[tokio::test]
async fn tls_certificate_mismatch() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cert = generate_simple_self_signed(vec!["localhost".into()])?;
    let cert_der = cert.serialize_der()?;
    let key_der = cert.serialize_private_key_der();
    let tls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::Certificate(cert_der.clone())],
            rustls::PrivateKey(key_der),
        )
        .expect("create TLS config");
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
        let _ = acceptor.accept(socket).await;
    });

    let other = generate_simple_self_signed(vec!["localhost".into()])?;
    let other_der = other.serialize_der()?;
    let mut root_store = rustls::RootCertStore::empty();
    root_store
        .add(&rustls::Certificate(other_der))
        .expect("add cert");
    let client_config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let server_name = rustls::ServerName::try_from("localhost")?;
    let res = connector.connect(server_name, tcp).await;
    assert!(res.is_err(), "handshake should fail");
    server.await.expect("server task failed");
    Ok(())
}
