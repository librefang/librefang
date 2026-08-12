use librefang::LibreFang;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn final_sse_event_without_newline_is_flushed_at_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();

        let body = b"data: {\"final\":true}";
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let client = LibreFang::new(format!("http://{address}"));
    let mut events = client
        .agents
        .send_message_stream("agent", json!({"message": "hello"}));

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .expect("final event was dropped at clean EOF");
    assert_eq!(event, json!({"final": true}));
    assert!(timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn truncated_utf8_at_eof_surfaces_an_error_event_instead_of_dropping_silently() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();

        // "data: " followed by the first byte of a multi-byte UTF-8 codepoint
        // (0xE2 starts a 3-byte sequence), with no continuation bytes and no
        // trailing newline before the connection closes.
        let mut body = b"data: ".to_vec();
        body.push(0xE2);
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let client = LibreFang::new(format!("http://{address}"));
    let mut events = client
        .agents
        .send_message_stream("agent", json!({"message": "hello"}));

    let event = timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .expect("truncated utf-8 at EOF was dropped silently instead of surfacing an error");
    assert_eq!(event["status"], json!(0));
    assert!(event["error"]
        .as_str()
        .unwrap()
        .contains("invalid utf-8 in SSE line"));
    server.await.unwrap();
}
