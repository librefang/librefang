use librefang::{Error, LibreFang};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn read_request_target(socket: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 1024];
        let read = socket.read(&mut chunk).await.unwrap();
        assert!(read > 0, "connection closed before request headers");
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request).unwrap();
    request.split_whitespace().nth(1).unwrap().to_string()
}

#[tokio::test]
async fn path_parameters_are_encoded_as_single_segments() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut targets = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            targets.push(read_request_target(&mut socket).await);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
        }
        targets
    });

    let client = LibreFang::new(format!("http://{address}"));
    client.agents.get_agent("a/b?c#d é").await.unwrap();
    let prefixed_client = LibreFang::new(format!("http://{address}/prefix"));
    prefixed_client.agents.get_agent("nested/id").await.unwrap();

    let dot_segment = client.agents.get_agent("..").await.unwrap_err();
    match dot_segment {
        Error::Api { status, body } => {
            assert_eq!(status, 0);
            assert_eq!(body, "invalid path segment: ..");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
    let mut invalid_stream = client
        .agents
        .send_message_stream(".", json!({"message": "hello"}));
    let stream_error = invalid_stream.recv().await.unwrap();
    assert_eq!(stream_error["status"], 0);
    assert_eq!(stream_error["error"], "invalid path segment: .");
    assert!(invalid_stream.recv().await.is_none());

    assert_eq!(
        server.await.unwrap(),
        [
            "/api/agents/a%2Fb%3Fc%23d%20%C3%A9",
            "/prefix/api/agents/nested%2Fid",
        ]
    );
}
