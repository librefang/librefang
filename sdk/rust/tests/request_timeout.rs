use librefang::{Error, LibreFang};
use serde_json::json;
use std::future::pending;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{advance, timeout, Duration};

#[tokio::test(start_paused = true)]
async fn non_streaming_request_has_default_total_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        accepted_tx.send(()).unwrap();
        pending::<()>().await;
    });

    let client = LibreFang::new(format!("http://{address}"));
    let request = tokio::spawn(async move { client.system.health().await });
    accepted_rx.await.unwrap();
    advance(Duration::from_secs(61)).await;

    let result = timeout(Duration::from_secs(1), request)
        .await
        .expect("non-streaming SDK request did not time out")
        .unwrap();
    match result {
        Err(Error::Http(error)) => assert!(error.is_timeout(), "unexpected HTTP error: {error}"),
        other => panic!("expected reqwest timeout, got {other:?}"),
    }

    server.abort();
    let _ = server.await;
}

#[tokio::test(start_paused = true)]
async fn streaming_body_is_not_subject_to_total_request_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (headers_tx, headers_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .unwrap();
        headers_tx.send(()).unwrap();
        release_rx.await.unwrap();

        let payload = b"data: {\"after_timeout\":true}\n";
        socket
            .write_all(format!("{:X}\r\n", payload.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(payload).await.unwrap();
        socket.write_all(b"\r\n0\r\n\r\n").await.unwrap();
    });

    let client = LibreFang::new(format!("http://{address}"));
    let mut events = client
        .agents
        .send_message_stream("agent", json!({"message": "hello"}));
    headers_rx.await.unwrap();
    advance(Duration::from_secs(61)).await;
    release_tx.send(()).unwrap();

    assert_eq!(events.recv().await.unwrap(), json!({"after_timeout": true}));
    server.await.unwrap();
}
