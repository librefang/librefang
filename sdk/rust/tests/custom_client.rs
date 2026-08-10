use librefang::LibreFang;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn custom_client_headers_reach_generated_resources() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1024];
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "client closed before sending HTTP headers");
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer sdk-token")));

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            )
            .await
            .unwrap();
    });

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer sdk-token"));
    let http_client = reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()
        .unwrap();
    let client = LibreFang::with_client(format!("http://{address}"), http_client);

    assert_eq!(client.system.health().await.unwrap()["ok"], true);
    server.await.unwrap();
}
