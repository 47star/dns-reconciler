use std::{net::SocketAddr, time::Duration};

use dns_reconciler::{cloudflare::client::CloudflareClient, dns::desired_state::DesiredRecord};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
async fn cloudflare_client_lists_a_records() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut stream).await;
        assert!(request.starts_with("GET /client/v4/zones/zone-id/dns_records?"));
        assert!(request.contains("type=A"));
        assert!(request.contains("page=1"));
        assert!(request.contains("per_page=100"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer token"));

        respond_json(
            &mut stream,
            r#"{
                "success": true,
                "errors": [],
                "messages": [],
                "result": [{
                    "id": "id-1",
                    "name": "host01.dhcp.example.com",
                    "type": "A",
                    "content": "192.0.2.10",
                    "ttl": 300,
                    "proxied": false
                }],
                "result_info": {
                    "page": 1,
                    "per_page": 100,
                    "total_pages": 1,
                    "count": 1,
                    "total_count": 1
                }
            }"#,
        )
        .await;
    });

    let client = test_client(addr);
    let records = client.list_a_records().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "host01.dhcp.example.com");

    server.await.unwrap();
}

#[tokio::test]
async fn cloudflare_client_creates_updates_and_deletes_records() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut create_stream, _) = listener.accept().await.unwrap();
        let create_request = read_http_request(&mut create_stream).await;
        assert!(create_request.starts_with("POST /client/v4/zones/zone-id/dns_records "));
        assert!(create_request.contains(r#""type":"A""#));
        assert!(create_request.contains(r#""name":"host01.dhcp.example.com""#));
        respond_json(&mut create_stream, success_body()).await;

        let (mut update_stream, _) = listener.accept().await.unwrap();
        let update_request = read_http_request(&mut update_stream).await;
        assert!(update_request.starts_with("PUT /client/v4/zones/zone-id/dns_records/id-1 "));
        assert!(update_request.contains(r#""content":"192.0.2.10""#));
        respond_json(&mut update_stream, success_body()).await;

        let (mut delete_stream, _) = listener.accept().await.unwrap();
        let delete_request = read_http_request(&mut delete_stream).await;
        assert!(delete_request.starts_with("DELETE /client/v4/zones/zone-id/dns_records/id-1 "));
        respond_json(&mut delete_stream, success_body()).await;
    });

    let client = test_client(addr);
    let desired = DesiredRecord {
        name: "host01.dhcp.example.com".to_string(),
        content: "192.0.2.10".parse().unwrap(),
        ttl: 300,
        proxied: false,
    };

    client.create_record(&desired).await.unwrap();
    client.update_record("id-1", &desired).await.unwrap();
    client.delete_record("id-1").await.unwrap();

    server.await.unwrap();
}

fn test_client(addr: SocketAddr) -> CloudflareClient {
    CloudflareClient::new(
        format!("http://{addr}/client/v4"),
        "zone-id".to_string(),
        "token".to_string(),
        Duration::from_secs(2),
    )
    .unwrap()
}

async fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break buffer.len();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
    };

    let content_length = parse_content_length(&buffer[..header_end + 4]);
    while buffer.len() < header_end + 4 + content_length {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    String::from_utf8_lossy(&buffer).into_owned()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

async fn respond_json(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

fn success_body() -> &'static str {
    r#"{"success":true,"errors":[],"messages":[],"result":{"id":"id-1"}}"#
}
