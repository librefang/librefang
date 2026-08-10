use librefang::LibreFang;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn truncated_response_reports_stream_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();

        let body = b"data: {\"received\":true}\n";
        let headers = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100\r\nConnection: close\r\n\r\n";
        socket.write_all(headers).await.unwrap();
        socket.write_all(body).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let client = LibreFang::new(format!("http://{address}"));
    let mut events = client
        .agents
        .send_message_stream("agent", json!({"message": "hello"}));

    let first = timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first, json!({"received": true}));

    let failure = timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .expect("truncated response must emit an error event");
    assert_eq!(failure["status"], 0);
    assert!(failure["error"]
        .as_str()
        .is_some_and(|message| message.starts_with("stream error: ")));

    assert!(timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .is_none());
    server.await.unwrap();
}
