use librefang::LibreFang;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

async fn wait_for_len(receiver: &tokio::sync::mpsc::Receiver<serde_json::Value>, expected: usize) {
    timeout(Duration::from_secs(2), async {
        while receiver.len() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stream_channel_applies_bounded_backpressure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();

        let mut body = Vec::new();
        for index in 0..300 {
            body.extend_from_slice(format!("data: {{\"index\":{index}}}\n").as_bytes());
        }
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

    assert_eq!(events.max_capacity(), 256);
    wait_for_len(&events, 256).await;
    assert_eq!(events.recv().await.unwrap(), json!({"index": 0}));
    wait_for_len(&events, 256).await;

    let mut received = 1;
    while events.recv().await.is_some() {
        received += 1;
    }
    assert_eq!(received, 300);
    server.await.unwrap();
}

#[tokio::test]
async fn dropping_receiver_closes_connection_stalled_between_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (headers_sent, headers_received) = oneshot::channel();
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
        headers_sent.send(()).unwrap();

        let mut byte = [0_u8; 1];
        socket.read(&mut byte).await.unwrap()
    });

    let client = LibreFang::new(format!("http://{address}"));
    let events = client
        .agents
        .send_message_stream("agent", json!({"message": "hello"}));
    headers_received.await.unwrap();
    drop(events);

    let bytes_read = timeout(Duration::from_secs(2), server)
        .await
        .expect("stream connection remained open after receiver drop")
        .unwrap();
    assert_eq!(bytes_read, 0);
}
