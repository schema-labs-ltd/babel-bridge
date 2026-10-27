use super::{
    chatgpt_request_body, find_event_end, parse_stream_event, OpenAiApiFailure, TranslationService,
    UsageInfo, CHEAP_PRICING,
};
use crate::storage::OpenAiSettings;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct MockResponse {
    status: &'static str,
    body: String,
}

fn spawn_openai_mock(responses: Vec<MockResponse>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("mock server addr");
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept mock request");
            let _ = read_http_request(&mut stream);
            let body = response.body.as_bytes();
            write!(
                stream,
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.status,
                body.len()
            )
            .expect("write response headers");
            stream.write_all(body).expect("write response body");
        }
    });

    (format!("http://{}", addr), handle)
}

fn spawn_capturing_openai_mock(
    responses: Vec<MockResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("mock server addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = Arc::clone(&requests);
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept mock request");
            captured_requests
                .lock()
                .expect("lock captured requests")
                .push(read_http_request(&mut stream));
            let body = response.body.as_bytes();
            write!(
                stream,
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.status,
                body.len()
            )
            .expect("write response headers");
            stream.write_all(body).expect("write response body");
        }
    });

    (format!("http://{}", addr), requests, handle)
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");

    let mut data = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let bytes_read = stream.read(&mut buffer).expect("read mock request");
        if bytes_read == 0 {
            break data.len();
        }
        data.extend_from_slice(&buffer[..bytes_read]);
        if let Some(position) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };

    let headers = String::from_utf8_lossy(&data[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .or_else(|| {
            headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let expected_len = header_end + content_length;

    while data.len() < expected_len {
        let bytes_read = stream.read(&mut buffer).expect("read mock body");
        if bytes_read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..bytes_read]);
    }

    String::from_utf8(data).expect("mock request should be UTF-8")
}

fn mock_text(text: &str) -> MockResponse {
    MockResponse { status: "200 OK", body: serde_json::json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"output_text","text":text}]}]}).to_string() }
}

fn sse(events: &[serde_json::Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {}\n\n",
                event["type"].as_str().unwrap(),
                event
            )
        })
        .collect()
}

fn chatgpt_service(url: String, models: &[&str]) -> (TranslationService, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wa-chatgpt-translation-{}", uuid::Uuid::new_v4()));
    let auth = crate::chatgpt::ChatGptAuth::signed_in_for_test(&dir, "plan-token", models);
    (
        TranslationService::new_with_api_url(url).with_chatgpt(Arc::new(auth)),
        dir,
    )
}

mod chatgpt;
mod topics;
mod translate;
