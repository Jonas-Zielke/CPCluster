use cpcluster_common::{read_length_prefixed, write_length_prefixed, NodeMessage};
use rcgen::generate_simple_self_signed;
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
use tokio::{net::TcpListener, io::{AsyncReadExt, AsyncWriteExt}};
use tokio_rustls::{rustls, TlsAcceptor, TlsConnector};

#[tokio::test]
async fn tls_handshake() {
    // generate certificate
    let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = cert.serialize_der().unwrap();
    let key_der = cert.serialize_private_key_der();
    let tls_config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![rustls::Certificate(cert_der.clone())], rustls::PrivateKey(key_der))
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = "testtoken".to_string();
    let token_srv = token.clone();
    let ports = Arc::new(Mutex::new(HashSet::from([addr.port() + 1])));
    let ports_srv = ports.clone();

    // master server
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = acceptor.accept(socket).await.unwrap();
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
        }
    });

    // client setup
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(&rustls::Certificate(cert_der)).unwrap();
    let client_config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_config));
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let server_name = rustls::ServerName::try_from("localhost").unwrap();
    let mut stream = connector.connect(server_name, tcp).await.unwrap();

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
    match msg {
        NodeMessage::ConnectionInfo(_, port) => assert_eq!(port, addr.port() + 1),
        other => panic!("unexpected message: {:?}", other),
    }

    assert!(ports.lock().unwrap().is_empty());
    server.await.unwrap();
}
