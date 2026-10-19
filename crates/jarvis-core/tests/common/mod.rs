//! Tiny HTTP mock: serves one canned response per accepted connection.
#![allow(dead_code)]

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub enum Canned {
    /// Full response with a body.
    Json { status: u16, body: String },
    /// NDJSON lines written with a delay between them, then the connection closes.
    Stream { lines: Vec<String>, delay: Duration },
    /// Accept and never answer.
    Hang,
}

pub fn ok(body: impl Into<String>) -> Canned {
    Canned::Json { status: 200, body: body.into() }
}

pub fn chat_line(content: &str, done: bool) -> String {
    serde_json::json!({ "model": "m", "message": { "role": "assistant", "content": content }, "done": done }).to_string()
}

pub fn chat_once_body(content: &str) -> Canned {
    ok(chat_line(content, true))
}

async fn read_request(sock: &mut TcpStream) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            while buf.len() < end + 4 + len {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            return;
        }
    }
}

/// Starts the server and returns its base URL.
pub async fn serve(responses: Vec<Canned>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for canned in responses {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                read_request(&mut sock).await;
                match canned {
                    Canned::Json { status, body } => {
                        let resp = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                    }
                    Canned::Stream { lines, delay } => {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n";
                        let _ = sock.write_all(head.as_bytes()).await;
                        for line in lines {
                            if sock.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                return;
                            }
                            let _ = sock.flush().await;
                            tokio::time::sleep(delay).await;
                        }
                    }
                    Canned::Hang => {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

/// A URL where nothing is listening.
pub async fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}
